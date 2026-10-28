//! Where the UI's data comes from and where its changes go
//! (docs/BACKEND_SPRINT_1.md, 2.3).
//!
//! - Reads: `load_mail`, `load_calendar` and friends build the UI's view
//!   models from the store, which syncs keep current.
//! - Writes: the UI changes its view model at once (the optimistic update)
//!   and hands the backend a `MailOp` or `CalOp`, which queues it in the
//!   outbox and commits it in the background.
//! - What happened: `BackendEvent`s, which `PigeonApp` handles one arm per
//!   variant (`app/backend_events.rs`).
//!
//! Demo mode (`Source::Demo`) is the same backend on an in-memory store
//! seeded from the placeholder data; ops commit by applying to that store.

pub mod attachments;
pub mod auth;
pub mod calendar;
pub mod categories;
pub mod compose;
pub mod demo;
pub mod gmail;
pub mod health;
pub mod http;
pub mod images;
pub mod labels;
pub mod mime;
pub mod new_mail;
pub mod outbox;
pub mod people;
pub mod photo;
pub mod store;
pub mod sync;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::{NaiveDateTime, Utc};
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Task};
use serde::{Deserialize, Serialize};

use self::{
    auth::{AccountRecord, Auth, ClientConfig, Keychain, Secret, SignInError, TokenStore},
    http::{GoogleTransport, Request, SystemClock, Transport, TransportError, UreqTransport},
    outbox::Outbox,
    store::Store,
};
use crate::{
    app::ComposeKind,
    calendar::{CalendarData, Event, RsvpStatus, SeriesScope},
    data::{
        Account, Attachment, Contact, DraftOrigin, Folder, Label, MailData, Message, Quote, Thread,
    },
    search::GmailQuery,
};

/// An op in the outbox: its row id.
pub type OpId = i64;

/// How long an op the UI can undo waits before it goes: as long as the
/// toast that offers the Undo.
pub const UNDO_WINDOW: Duration = Duration::from_secs(5);

/// How long after a committed write the sync reads it back (2.6).
const SYNC_AFTER_WRITE: Duration = Duration::from_secs(2);

/// How long settings changes settle before they're written.
const SETTINGS_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The placeholder data, in memory.
    Demo,
    /// The signed-in Google account (W2).
    Google,
    /// Nobody signed in: the sign-in screen (W2).
    SignedOut,
}

/// What the status line says (ACC-5, STA-1/3/4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    Demo,
    SignedOut,
    Syncing {
        done: usize,
        total: usize,
    },
    /// Unix ms.
    Synced {
        at: i64,
    },
    Offline {
        waiting: usize,
    },
    /// The refresh token stopped working: "Sign in again".
    AuthExpired,
    /// A request is waiting out a long backoff: "Waiting for Gmail…" (W11).
    Waiting,
}

/// The browser sign-in, for the sign-in screen (ACC-3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SignInState {
    #[default]
    Idle,
    /// The consent page is open in the browser.
    Waiting,
    /// Why the last try didn't work, in plain words.
    Failed(String),
}

/// The signed-in account's connection to Google.
struct Google {
    /// Authenticated, limited, retrying: what sync and commits use.
    transport: Arc<GoogleTransport>,
    auth: Arc<Auth>,
    /// The bare network, for the token endpoint and revoking.
    raw: Arc<dyn Transport>,
}

impl Google {
    fn new(client: ClientConfig, refresh_token: Secret) -> Self {
        let raw: Arc<dyn Transport> = Arc::new(UreqTransport::new());
        let clock = Arc::new(SystemClock);
        let auth = Arc::new(Auth::new(client, refresh_token, raw.clone(), clock.clone()));
        Self {
            transport: Arc::new(GoogleTransport::new(raw.clone(), auth.clone(), clock)),
            auth,
            raw,
        }
    }
}

const PEOPLE_ME: &str =
    "https://people.googleapis.com/v1/people/me?personFields=names,emailAddresses,photos";

/// The signed-in user's name and primary address, and where their photo is.
struct Profile {
    claims: auth::Claims,
    /// `photo::pick`'s answer.
    photo: Option<String>,
}

fn fetch_profile(transport: &dyn Transport) -> Result<Profile, TransportError> {
    #[derive(Deserialize)]
    struct Person {
        #[serde(default)]
        names: Vec<Field>,
        #[serde(default, rename = "emailAddresses")]
        email_addresses: Vec<Field>,
        #[serde(default)]
        photos: Vec<photo::Photo>,
    }
    #[derive(Deserialize)]
    struct Field {
        #[serde(rename = "displayName")]
        display_name: Option<String>,
        value: Option<String>,
        #[serde(default)]
        metadata: Metadata,
    }
    #[derive(Default, Deserialize)]
    struct Metadata {
        #[serde(default)]
        primary: bool,
    }
    let response = transport.send(&Request::get(PEOPLE_ME, 1))?;
    if !response.is_success() {
        return Err(TransportError::Network(format!(
            "people/me answered {}",
            response.status
        )));
    }
    let person: Person = response
        .json()
        .map_err(|_| TransportError::Network("people/me unreadable".into()))?;
    let primary = |fields: &[Field]| -> Option<usize> {
        fields
            .iter()
            .position(|f| f.metadata.primary)
            .or((!fields.is_empty()).then_some(0))
    };
    Ok(Profile {
        claims: auth::Claims {
            name: primary(&person.names)
                .and_then(|ix| person.names[ix].display_name.clone())
                .unwrap_or_default(),
            email: primary(&person.email_addresses)
                .and_then(|ix| person.email_addresses[ix].value.clone())
                .unwrap_or_default(),
        },
        photo: photo::pick(&person.photos),
    })
}

/// A change to mail, in Gmail's terms: label ids, `INBOX` and `UNREAD`
/// included (see `gmail`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MailOp {
    /// Read, star, labels, spam, archive.
    Modify {
        threads: Vec<String>,
        add: Vec<String>,
        remove: Vec<String>,
    },
    Trash {
        threads: Vec<String>,
    },
    Untrash {
        threads: Vec<String>,
    },
    SaveDraft {
        /// The Drafts thread's id.
        local_id: String,
        draft: Outgoing,
        /// W3 (DRF-5): which local save this is; an older one never
        /// overwrites a newer one.
        #[serde(default)]
        revision: u64,
    },
    DeleteDraft {
        local_id: String,
    },
    Send {
        draft: Outgoing,
        /// The draft it was sent from, which goes.
        local_id: Option<String>,
        /// The thread it goes in: the one it answers, or a new
        /// `sent-local-*` one.
        thread_id: String,
    },
}

impl MailOp {
    /// The ops that move threads from `from` to `to`. Sent and Drafts
    /// aren't places to move to.
    pub fn moves(threads: Vec<String>, from: Folder, to: Folder) -> Vec<MailOp> {
        let labels = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut ops = Vec::new();
        if from == Folder::Trash && to != Folder::Trash {
            ops.push(MailOp::Untrash {
                threads: threads.clone(),
            });
        }
        let (add, remove) = match to {
            Folder::Trash => {
                ops.push(MailOp::Trash { threads });
                return ops;
            }
            Folder::Inbox => (labels(&[gmail::INBOX]), labels(&[gmail::SPAM])),
            Folder::Archive => (Vec::new(), labels(&[gmail::INBOX, gmail::SPAM])),
            Folder::Spam => (labels(&[gmail::SPAM]), labels(&[gmail::INBOX])),
            Folder::Sent | Folder::Drafts => return ops,
        };
        ops.push(MailOp::Modify {
            threads,
            add,
            remove,
        });
        ops
    }

    /// Whether it waits out the undo window: sends, discards, moves and
    /// deletes. Read, star and label changes go at once.
    pub fn undoable(&self) -> bool {
        match self {
            MailOp::Modify { add, remove, .. } => add
                .iter()
                .chain(remove)
                .any(|l| gmail::FOLDER_LABELS.contains(&l.as_str())),
            MailOp::Trash { .. }
            | MailOp::Untrash { .. }
            | MailOp::DeleteDraft { .. }
            | MailOp::Send { .. } => true,
            MailOp::SaveDraft { .. } => false,
        }
    }
}

/// A message written here, to save as a draft or send.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Outgoing {
    pub kind: ComposeKind,
    pub from: Contact,
    pub to: Vec<Contact>,
    pub cc: Vec<Contact>,
    pub bcc: Vec<Contact>,
    pub subject: String,
    /// The body as typed.
    pub markdown: String,
    /// What the message carries: the body, and for a send the quote too.
    pub html: String,
    pub attachments: Vec<Attachment>,
    /// The thread a reply answers.
    pub reply_to: Option<String>,
    /// ME-010: the message in it a reply answers, by id; empty for the
    /// thread's last. Its Message-ID threads the reply.
    #[serde(default)]
    pub answers: String,
    pub quoted: Option<Quote>,
    /// W3 (MST-1): the alias's Reply-To, if it has one.
    #[serde(default)]
    pub reply_to_address: String,
    /// W3 (MST-2): the alias's signature as HTML, when the draft carries
    /// one; `html` already has it, the plain-text part gets its text.
    #[serde(default)]
    pub signature: String,
    /// W2 (ATT-7): held copies of files taken out of the draft, released
    /// with the send like the ones it carries.
    #[serde(default)]
    pub dropped_files: Vec<String>,
}

impl Outgoing {
    /// The files on disk it has or had: its attachments', and the held
    /// copies taken out of it.
    pub fn file_paths(&self) -> impl Iterator<Item = &str> {
        self.attachments
            .iter()
            .map(|a| a.path.as_str())
            .chain(self.dropped_files.iter().map(String::as_str))
    }

    /// The message, from the user, dated now, with `labels`.
    pub fn to_message(&self, labels: Vec<String>) -> Message {
        // From the text as shown, without the markdown's syntax.
        let body = crate::markdown::to_html(&self.markdown);
        let snippet: String = crate::html::plain_text(&crate::html::parse(&body))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(140)
            .collect();
        Message {
            label_ids: labels,
            to: self.to.clone(),
            cc: self.cc.clone(),
            bcc: self.bcc.clone(),
            reply_to: (!self.reply_to_address.is_empty())
                .then(|| Contact {
                    name: String::new(),
                    email: self.reply_to_address.clone(),
                })
                .into_iter()
                .collect(),
            snippet,
            body: crate::data::Body::Loaded {
                html: self.html.clone(),
            },
            attachments: self.attachments.clone(),
            ..Message::new(self.from.clone())
        }
    }

    /// The Drafts thread it's saved as.
    pub fn to_draft_thread(&self, local_id: String) -> Thread {
        let origin = DraftOrigin {
            kind: self.kind,
            markdown: self.markdown.clone(),
            reply_to: self.reply_to.clone(),
            answers: self.answers.clone(),
            quoted: self.quoted.clone(),
            signature: !self.signature.is_empty(),
        };
        Thread::local(
            local_id,
            self.subject.clone(),
            Folder::Drafts,
            vec![self.to_message(vec![gmail::DRAFT.into()])],
            Some(origin),
        )
    }
}

/// Whether Google emails the guests about a change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendUpdates {
    All,
    #[default]
    None,
}

#[allow(clippy::large_enum_variant)] // Queued one at a time; not worth boxing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CalOp {
    Insert {
        event: Event,
        send_updates: SendUpdates,
    },
    Patch {
        id: String,
        before: Event,
        after: Event,
        scope: SeriesScope,
        send_updates: SendUpdates,
    },
    /// The occurrence at `occurrence` (where the rule puts it), the ones
    /// after it, or all of it.
    Delete {
        id: String,
        occurrence: Option<NaiveDateTime>,
        scope: SeriesScope,
        send_updates: SendUpdates,
    },
    Move {
        id: String,
        to_calendar: String,
    },
    Rsvp {
        id: String,
        status: RsvpStatus,
    },
    SetSelected {
        calendar: String,
        selected: bool,
    },
    // W5: "This and following" as one op, so the new series goes in only
    // once the old one was shortened (`before` to `after`).
    Split {
        id: String,
        before: Event,
        after: Event,
        new: Event,
        send_updates: SendUpdates,
    },
}

impl CalOp {
    /// Deletes wait out the undo window; the rest go at once.
    pub fn undoable(&self) -> bool {
        matches!(self, CalOp::Delete { .. })
    }
}

/// An outbox row's op.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Mail(MailOp),
    Calendar(CalOp),
}

impl Op {
    pub fn kind(&self) -> &'static str {
        match self {
            Op::Mail(op) => match op {
                MailOp::Modify { .. } => "mail.modify",
                MailOp::Trash { .. } => "mail.trash",
                MailOp::Untrash { .. } => "mail.untrash",
                MailOp::SaveDraft { .. } => "mail.save_draft",
                MailOp::DeleteDraft { .. } => "mail.delete_draft",
                MailOp::Send { .. } => "mail.send",
            },
            Op::Calendar(op) => match op {
                CalOp::Insert { .. } => "calendar.insert",
                CalOp::Patch { .. } => "calendar.patch",
                CalOp::Delete { .. } => "calendar.delete",
                CalOp::Move { .. } => "calendar.move",
                CalOp::Rsvp { .. } => "calendar.rsvp",
                CalOp::SetSelected { .. } => "calendar.set_selected",
                CalOp::Split { .. } => "calendar.split",
            },
        }
    }

    /// What it changes, so ops on one thing go in order: the threads, the
    /// draft, the event or the calendar.
    pub fn resource(&self) -> String {
        match self {
            Op::Mail(op) => match op {
                MailOp::Modify { threads, .. }
                | MailOp::Trash { threads }
                | MailOp::Untrash { threads } => threads.join(","),
                MailOp::SaveDraft { local_id, .. } | MailOp::DeleteDraft { local_id } => {
                    local_id.clone()
                }
                // Sends go one at a time, and after the saves of their
                // draft (W3, DRF-5).
                MailOp::Send {
                    local_id: Some(local_id),
                    ..
                } => format!("send,{local_id}"),
                MailOp::Send { .. } => "send".into(),
            },
            Op::Calendar(op) => match op {
                CalOp::Insert { event, .. } => event.id.clone(),
                CalOp::Patch { id, .. }
                | CalOp::Delete { id, .. }
                | CalOp::Move { id, .. }
                | CalOp::Rsvp { id, .. } => id.clone(),
                // Both series: an undo's ops on either wait for it.
                CalOp::Split { id, new, .. } => format!("{id},{}", new.id),
                CalOp::SetSelected { calendar, .. } => format!("calendar:{calendar}"),
            },
        }
    }

    fn undoable(&self) -> bool {
        match self {
            Op::Mail(op) => op.undoable(),
            Op::Calendar(op) => op.undoable(),
        }
    }
}

/// What the backend tells the app. `PigeonApp` reloads the ids named from
/// the store (`load_threads`, `load_events`), which keeps the selection.
#[derive(Clone, Debug, PartialEq)]
pub enum BackendEvent {
    ThreadsChanged(Vec<String>),
    ThreadsRemoved(Vec<String>),
    IdReplaced {
        old: String,
        new: String,
    },
    LabelsChanged,
    CountsChanged,
    EventsChanged(Vec<String>),
    EventsRemoved(Vec<String>),
    CalendarsChanged,
    ContactsChanged,
    Status(SyncStatus),
    OpCommitted(OpId),
    OpFailed {
        op: OpId,
        message: String,
    },
    SignedIn,
    SignedOut,
    /// The account's send-as addresses (aliases, names, signatures) were
    /// read again and changed.
    AccountChanged,
    // Sprint 2 W1.
    /// Mail that just arrived and is news (NTF-1); never from a full sync.
    NewMail(Vec<new_mail::NewMail>),
}

/// A view that can load older threads (LST-7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewKey {
    Folder(Folder),
    Starred,
    /// By label id.
    Label(String),
    /// W4a: an Inbox category (FOL-11).
    Category(categories::Category),
}

/// What a server search found (SRC-7): threads by id, newest first, and
/// Gmail's estimate of how many there are in all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    pub thread_ids: Vec<String>,
    pub estimate: Option<usize>,
    /// For the next page, if there is one.
    pub next_page: Option<String>,
}

/// Where loading a view's older threads is (LST-7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Older {
    #[default]
    More,
    Loading,
    /// Gmail has no older ones.
    Done,
}

/// A view's paging past the cache: its query, and where the next page is.
#[derive(Default)]
struct OlderPages {
    query: Option<GmailQuery>,
    next: Option<String>,
    state: Older,
}

/// Threads per page of a view's older threads, and pages tried at most
/// per "Load older" when they're all cached already.
const OLDER_PAGE: usize = 50;
const OLDER_TRIES: usize = 5;

pub struct Backend {
    source: Source,
    /// Mail, calendars and contacts: in memory in demo mode.
    store: Store,
    /// App-level settings: always the file in the data folder, so they
    /// persist in demo mode too. The same store as `store` for Google.
    prefs: Store,
    outbox: Outbox,
    account: Option<Account>,
    status: SyncStatus,
    pending_settings: Option<serde_json::Map<String, serde_json::Value>>,
    /// Ops are going; the worker reschedules itself once they're done.
    committing: bool,
    _worker: Option<Task<()>>,
    /// The sync that reads back a committed write.
    _sync_soon: Option<Task<()>>,
    _save_settings: Option<Task<()>>,
    // Signing in (W2).
    /// The account as `accounts.json` has it, with its granted scopes.
    record: Option<AccountRecord>,
    google: Option<Google>,
    /// Where refresh tokens live: the Keychain.
    tokens: Arc<dyn TokenStore>,
    sign_in: SignInState,
    sign_in_cancel: Option<Arc<AtomicBool>>,
    _sign_in: Option<Task<()>>,
    _profile: Option<Task<()>>,
    /// The user's cached profile photo, once there is one.
    profile_photo: Option<PathBuf>,
    _photo: Option<Task<()>>,
    /// Mail sync and body fetches (W3, `sync`).
    sync: sync::SyncState,
    /// W11: offline, waiting and the queue, over sync's status (`health`).
    health: health::Health,
    /// W9: each view's older pages (LST-7).
    older: Vec<(ViewKey, OlderPages)>,
    /// W4a: Inbox category counts (FOL-11).
    categories: categories::CategoryState,
}

impl EventEmitter<BackendEvent> for Backend {}

/// `PIGEON_DATA_DIR`, else `~/Library/Application Support/Pigeon`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PIGEON_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join("Library/Application Support/Pigeon")
}

/// `PIGEON_DEMO=1`: the placeholder data, whoever is signed in.
pub fn demo_requested() -> bool {
    std::env::var("PIGEON_DEMO").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// W9: a view's threads older than `oldest` (ms), for "Load older".
fn older_query(view: &ViewKey, oldest: Option<i64>) -> Option<GmailQuery> {
    let mut query = GmailQuery::view(view)?;
    if let Some(ms) = oldest {
        let before = format!("before:{}", ms.div_euclid(1000));
        query.q = [query.q.as_str(), &before]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    Some(query)
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// One op's commit, as the worker hands it back.
struct Commit {
    id: OpId,
    outcome: outbox::Outcome,
    events: Vec<BackendEvent>,
    calendar: bool,
}

fn log_outbox(result: anyhow::Result<()>) {
    if let Err(e) = result {
        eprintln!("Pigeon: outbox: {e:#}");
    }
}

/// Commit one op to Google, off the main thread: what became of it, and
/// what it changed for the app to reload, reported before the outcome.
/// Both `transport` (Calendar) and `gmail` send at the user's priority.
/// No network or no sign-in, on any op, is `Later`: it stays queued.
fn commit_google(
    transport: &Arc<dyn Transport>,
    gmail: &gmail::Gmail,
    store: &Store,
    account: &str,
    op: &Op,
) -> (outbox::Outcome, Vec<BackendEvent>) {
    match op {
        // Thread actions (W5).
        Op::Mail(op @ (MailOp::Modify { .. } | MailOp::Trash { .. } | MailOp::Untrash { .. })) => (
            outbox::commit_threads(gmail, store, account, op),
            Vec::new(),
        ),
        // Drafts and sends (W6).
        Op::Mail(op) => {
            let (result, events) = compose::commit_compose(gmail, store, account, op);
            let outcome = match result {
                Ok(()) => outbox::Outcome::Committed,
                Err(e) => match e.downcast::<TransportError>() {
                    Ok(e) if e.goes_later() => outbox::Outcome::Later(e),
                    Ok(e) => outbox::Outcome::failed(e.to_string()),
                    Err(e) => outbox::Outcome::failed(e.to_string()),
                },
            };
            (outcome, events)
        }
        // Calendar writes (W7).
        Op::Calendar(op) => {
            let committed = calendar::commit(&**transport, store, account, op);
            let outcome = match (committed.retry, committed.error) {
                (Some(e), _) => outbox::Outcome::Later(e),
                (None, Some(message)) => outbox::Outcome::failed(message),
                (None, None) => outbox::Outcome::Committed,
            };
            (outcome, committed.changes.backend_events())
        }
    }
}

/// Demo mode: the op applies to the in-memory store, and that's all.
fn commit_demo(store: &Store, account: &str, op: &Op) -> (outbox::Outcome, Vec<BackendEvent>) {
    let result = match op {
        Op::Mail(op) => store.apply_mail_op(account, op),
        Op::Calendar(op) => store.apply_cal_op(account, account, op),
    };
    let outcome = match result {
        Ok(()) => outbox::Outcome::Committed,
        Err(e) => outbox::Outcome::failed(e.to_string()),
    };
    (outcome, Vec::new())
}

impl Backend {
    /// Demo if `PIGEON_DEMO=1`, else Google if `accounts.json` names an
    /// account, else SignedOut. The account's refresh token comes from the
    /// Keychain after, off the main thread, so the window doesn't wait on
    /// it (MA-072); without one, it's SignedOut then.
    pub fn open(cx: &mut App) -> Entity<Backend> {
        let prefs = Store::open(&data_dir().join("pigeon.sqlite")).unwrap_or_else(|e| {
            eprintln!("Pigeon: settings won't be kept: {e:#}");
            Store::open_in_memory().expect("an in-memory store always opens")
        });
        let tokens: Arc<dyn TokenStore> = Arc::new(Keychain);
        let (source, session) = if demo_requested() {
            (Source::Demo, None)
        } else {
            match auth::load_accounts(&data_dir()).into_iter().next() {
                Some(record) => (Source::Google, Some((record, None))),
                None => (Source::SignedOut, None),
            }
        };
        let backend = cx.new(|cx| Backend::new(source, prefs, tokens, session, cx));
        if source == Source::Google {
            backend.update(cx, |b, cx| b.read_token(cx));
        }
        backend
    }

    /// The refresh token from the Keychain, in the background (MA-072).
    fn read_token(&mut self, cx: &mut Context<Self>) {
        let Some(email) = self.record.as_ref().map(|r| r.email.clone()) else {
            return;
        };
        let tokens = self.tokens.clone();
        self._sign_in = Some(cx.spawn(async move |this, cx| {
            let token = cx
                .background_executor()
                .spawn(async move { tokens.get(&email) })
                .await;
            this.update(cx, |this, cx| this.token_read(token, cx)).ok();
        }));
    }

    fn token_read(&mut self, token: anyhow::Result<Option<Secret>>, cx: &mut Context<Self>) {
        if self.source != Source::Google || self.google.is_some() {
            return;
        }
        let token = match token {
            Ok(Some(token)) => token,
            Ok(None) => {
                crate::logging::info("auth: no refresh token in the Keychain; signed out");
                self.switch(Source::SignedOut, None, cx);
                return cx.emit(BackendEvent::SignedOut);
            }
            Err(e) => {
                eprintln!("Pigeon: {e:#}");
                self.switch(Source::SignedOut, None, cx);
                return cx.emit(BackendEvent::SignedOut);
            }
        };
        // No client: the cache shows, and the status says sign in again.
        let Ok(client) = ClientConfig::load(&data_dir()) else {
            return;
        };
        self.google = Some(Google::new(client, token));
        self.check_profile(cx);
        if let Some(transport) = self.transport() {
            self.start_mail_sync(transport, cx);
        }
        self.schedule(cx);
    }

    /// `session`'s token is `None` while it's still being read
    /// (`read_token`).
    fn new(
        source: Source,
        prefs: Store,
        tokens: Arc<dyn TokenStore>,
        session: Option<(AccountRecord, Option<Secret>)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut record = None;
        let mut google = None;
        let (store, account, status) = match source {
            Source::Demo => {
                let store = Store::open_in_memory().expect("an in-memory store always opens");
                let account = demo::seed(&store).expect("the placeholder data seeds");
                (store, Some(account), SyncStatus::Demo)
            }
            Source::Google => {
                let (rec, refresh_token) = session.expect("Google mode has an account");
                let account = Account {
                    name: rec.name.clone(),
                    email: rec.email.clone(),
                    aliases: Vec::new(),
                    send_as: Vec::new(),
                };
                // Without the client, nothing refreshes: the cache shows,
                // and signing in again says what's missing.
                let status = match ClientConfig::load(&data_dir()) {
                    Ok(client) => {
                        google = refresh_token.map(|token| Google::new(client, token));
                        SyncStatus::Syncing { done: 0, total: 0 }
                    }
                    Err(e) => {
                        eprintln!("Pigeon: {e:#}");
                        SyncStatus::AuthExpired
                    }
                };
                record = Some(rec);
                (prefs.clone(), Some(account), status)
            }
            Source::SignedOut => (prefs.clone(), None, SyncStatus::SignedOut),
        };
        let email = account
            .as_ref()
            .map(|a| a.email.clone())
            .unwrap_or_default();
        let mut this = Self {
            source,
            outbox: Outbox::new(store.clone(), email),
            store,
            prefs,
            account,
            health: health::Health::new(status.clone()),
            status,
            pending_settings: None,
            committing: false,
            _worker: None,
            _sync_soon: None,
            _save_settings: None,
            record,
            google,
            tokens,
            sign_in: SignInState::Idle,
            sign_in_cancel: None,
            _sign_in: None,
            _profile: None,
            profile_photo: None,
            _photo: None,
            sync: Default::default(),
            older: Vec::new(),
            categories: Default::default(),
        };
        if this.source == Source::Google {
            this.profile_photo = photo::cached(&data_dir(), this.email());
        }
        this.start_categories();
        // Whatever was left queued at the last quit goes now.
        this.schedule(cx);
        this.check_profile(cx);
        if let Some(transport) = this.transport() {
            this.start_mail_sync(transport, cx);
        }
        this.start_health(cx);
        this
    }

    /// Start over on another source, keeping the settings.
    fn switch(
        &mut self,
        source: Source,
        session: Option<(AccountRecord, Secret)>,
        cx: &mut Context<Self>,
    ) {
        self.flush_settings();
        if let Some(cancel) = &self.sign_in_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        let session = session.map(|(record, token)| (record, Some(token)));
        *self = Self::new(source, self.prefs.clone(), self.tokens.clone(), session, cx);
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub fn is_demo(&self) -> bool {
        self.source == Source::Demo
    }

    /// Name, email and aliases (ACC-1, ACC-2).
    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    /// The user's profile photo on disk, if Google has one for them.
    pub fn profile_photo(&self) -> Option<&Path> {
        self.profile_photo.as_deref()
    }

    fn email(&self) -> &str {
        self.account.as_ref().map_or("", |a| a.email.as_str())
    }

    pub fn status(&self) -> &SyncStatus {
        &self.status
    }

    // Reads, from the store: fast and local.

    /// Everything for the mail panes, newest first, without the messages'
    /// bodies (`Body::NotLoaded`), so the first frame doesn't wait on them
    /// (MA-072). Changes queued at the last quit show as made (PER-2). On
    /// selection, `load_threads` reads the stored bodies; `fetch_body` the
    /// rest.
    pub fn load_mail_heads(&self) -> MailData {
        let mut data = MailData {
            account: self.account.clone().unwrap_or_default(),
            labels: self.load_labels(),
            threads: self
                .store
                .load_all_thread_heads(self.email())
                .unwrap_or_else(|e| {
                    eprintln!("Pigeon: couldn't read mail from the store: {e:#}");
                    Vec::new()
                }),
        };
        self.outbox.overlay(&mut data.threads);
        data.sort_threads();
        data
    }

    /// The threads with a send still queued (offline, or in its undo
    /// window), by thread id: "Waiting to send" (MA-069).
    pub fn queued_sends(&self) -> std::collections::HashSet<String> {
        self.outbox.queued_sends()
    }

    pub fn load_labels(&self) -> Vec<Label> {
        self.store.load_labels(self.email()).unwrap_or_default()
    }

    /// The stored threads among `ids`, with the changes still queued for
    /// them; a missing one is gone.
    pub fn load_threads(&self, ids: &[String]) -> Vec<Thread> {
        let mut threads = self
            .store
            .load_threads(self.email(), ids)
            .unwrap_or_default();
        self.outbox.overlay(&mut threads);
        self.overlay_bodies(&mut threads);
        threads
    }

    pub fn load_calendar(&self) -> CalendarData {
        CalendarData {
            account: self.account.clone().unwrap_or_default(),
            calendars: self.store.load_calendars(self.email()).unwrap_or_default(),
            events: self
                .store
                .load_all_events(self.email())
                .unwrap_or_else(|e| {
                    eprintln!("Pigeon: couldn't read events from the store: {e:#}");
                    Vec::new()
                }),
        }
    }

    /// The calendars alone, without their events (MC-010).
    pub fn load_calendars(&self) -> Vec<crate::calendar::Calendar> {
        self.store.load_calendars(self.email()).unwrap_or_default()
    }

    /// Events with calendar changes still queued (`Outbox::queued_events`).
    pub fn queued_events(&self) -> std::collections::HashSet<String> {
        self.outbox.queued_events()
    }

    /// Calendars with a show/hide still queued (`Outbox::queued_selections`).
    pub fn queued_selections(&self) -> std::collections::HashSet<String> {
        self.outbox.queued_selections()
    }

    pub fn load_events(&self, ids: &[String]) -> Vec<Event> {
        self.store
            .load_events(self.email(), ids)
            .unwrap_or_default()
    }

    /// Everyone to suggest as a recipient, best first, for `ContactBook`.
    pub fn contacts(&self) -> Vec<Contact> {
        match &self.account {
            Some(account) => self.store.contacts(account).unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Fetch the bodies of a thread's messages that have none (RDR-6);
    /// emits `ThreadsChanged` once they're stored. Demo bodies are all
    /// there already.
    pub fn fetch_body(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        // W6: a Gmail draft comes whole, from drafts.get (DRF-3).
        if self.fetch_draft(thread_id, cx) {
            return;
        }
        self.fetch_bodies(thread_id, cx);
    }

    // Writes.

    pub fn mail(&mut self, op: MailOp, cx: &mut Context<Self>) -> OpId {
        if let MailOp::Send { draft, .. } = &op {
            self.note_recipients(draft, cx);
        }
        // W3 (DRF-5): a save waiting out its 30 s goes first, now.
        if let MailOp::Send {
            local_id: Some(id), ..
        }
        | MailOp::DeleteDraft { local_id: id } = &op
        {
            log_outbox(self.outbox.hurry(id, now_ms()));
        }
        self.queue(Op::Mail(op), cx)
    }

    pub fn calendar(&mut self, op: CalOp, cx: &mut Context<Self>) -> OpId {
        self.queue(Op::Calendar(op), cx)
    }

    /// A change that goes at once even when it could wait for Undo: the
    /// inverse of one that already went (Undo after the window).
    pub fn mail_now(&mut self, op: MailOp, cx: &mut Context<Self>) -> OpId {
        self.push(Op::Mail(op), 0, cx)
    }

    fn queue(&mut self, op: Op, cx: &mut Context<Self>) -> OpId {
        let delay = if op.undoable() {
            UNDO_WINDOW.as_millis() as i64
        } else {
            0
        };
        self.push(op, delay, cx)
    }

    fn push(&mut self, op: Op, delay: i64, cx: &mut Context<Self>) -> OpId {
        match self.outbox.push(op, delay, now_ms()) {
            Ok(id) => {
                if let SyncStatus::Offline { .. } = self.status {
                    let waiting = self.outbox.waiting();
                    self.set_status(SyncStatus::Offline { waiting }, cx);
                }
                self.schedule(cx);
                // W11: "Offline · N changes waiting" counts it.
                self.refresh_status(cx);
                id
            }
            Err(e) => {
                eprintln!("Pigeon: couldn't queue a change: {e:#}");
                0
            }
        }
    }

    /// Undo inside the undo window: the op never goes. False once it has
    /// started to go; the caller then queues the inverse.
    pub fn cancel(&mut self, op: OpId) -> bool {
        self.outbox.cancel(op)
    }

    /// Wake when the next op is due, and commit what's due then. Nothing
    /// goes to Google while its sign-in needs renewing (STA-4); the queue
    /// waits for the next sign-in.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.committing {
            return;
        }
        let can_commit = match self.source {
            Source::Demo => true,
            Source::Google => self.google.is_some() && self.status != SyncStatus::AuthExpired,
            Source::SignedOut => false,
        };
        let Some(due) = self.outbox.next_due().filter(|_| can_commit) else {
            self._worker = None;
            return;
        };
        let wait = Duration::from_millis((due - now_ms()).max(0) as u64);
        self._worker = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let Ok(Some((store, account, google, due))) = this.update(cx, |this, _| {
                let due = this.outbox.take_due(now_ms()).ok()?;
                this.committing = true;
                let google = this.transport().map(|t| {
                    let user: Arc<dyn Transport> = Arc::new(outbox::UserPriority(t));
                    (user.clone(), gmail::Gmail::new(user))
                });
                Some((this.store.clone(), this.email().to_string(), google, due))
            }) else {
                return;
            };
            let results = cx
                .background_executor()
                .spawn(async move {
                    due.into_iter()
                        .map(|(id, op)| {
                            let calendar = matches!(op, Op::Calendar(_));
                            let (outcome, events) = match &google {
                                Some((transport, gmail)) => {
                                    commit_google(transport, gmail, &store, &account, &op)
                                }
                                None => commit_demo(&store, &account, &op),
                            };
                            Commit {
                                id,
                                outcome,
                                events,
                                calendar,
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                this.committed(results, cx);
                this.committing = false;
                this.schedule(cx);
            })
            .ok();
        }));
    }

    /// Put commit outcomes into effect: rows done, failed (with the
    /// rollback already stored) or back in the queue.
    fn committed(&mut self, results: Vec<Commit>, cx: &mut Context<Self>) {
        let (mut mail, mut calendar) = (false, false);
        for Commit {
            id,
            outcome,
            events,
            calendar: is_calendar,
        } in results
        {
            // The row is done (or failed, or waiting) first: what the commit
            // changed is then held back only for ops still queued after it.
            match &outcome {
                outbox::Outcome::Committed => log_outbox(self.outbox.finish(id)),
                outbox::Outcome::Failed { message, .. } => {
                    log_outbox(self.outbox.fail(id, message))
                }
                outbox::Outcome::Later(error) => log_outbox(self.outbox.retry_later(
                    id,
                    now_ms() + outbox::RETRY_LATER_MS,
                    &error.to_string(),
                )),
            }
            // What the commit changed (W6, W7), before it's reported.
            for event in events {
                cx.emit(event);
            }
            match outcome {
                outbox::Outcome::Committed => {
                    if is_calendar {
                        calendar = true;
                    } else {
                        mail = true;
                    }
                    cx.emit(BackendEvent::OpCommitted(id));
                }
                outbox::Outcome::Failed {
                    message,
                    changed,
                    removed,
                } => {
                    calendar |= is_calendar;
                    if !changed.is_empty() {
                        cx.emit(BackendEvent::ThreadsChanged(changed));
                    }
                    if !removed.is_empty() {
                        cx.emit(BackendEvent::ThreadsRemoved(removed));
                    }
                    cx.emit(BackendEvent::OpFailed { op: id, message });
                }
                outbox::Outcome::Later(error) => self.note_error(&error, cx),
            }
        }
        // W4a: demo counts follow its commits.
        if mail && self.source == Source::Demo {
            self.refresh_category_counts(cx);
        }
        if self.source == Source::Google {
            if mail {
                self.sync_soon(cx);
            }
            // W7: a calendar sync shortly after a calendar write.
            if calendar {
                self.sync_calendar_soon(cx);
            }
        }
    }

    /// Read back what a write changed, once Gmail has it (2.6).
    fn sync_soon(&mut self, cx: &mut Context<Self>) {
        self._sync_soon = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SYNC_AFTER_WRITE).await;
            this.update(cx, |this, cx| this.run_sync(cx)).ok();
        }));
    }

    // Google only; nothing to do on the placeholder data.

    // search and paging (W9) -------------------------------------------

    /// Whether Gmail can be searched and paged: signed in to Google.
    pub fn can_search(&self) -> bool {
        self.transport().is_some()
    }

    /// Search all of Gmail (SRC-7): a page of `query`'s threads, at the
    /// user's priority. Ones the store lacks are fetched and stored first
    /// (`ThreadsChanged`). Demo search is local only; offline it fails at
    /// once, as a network error.
    pub fn search(
        &mut self,
        query: &GmailQuery,
        page: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<SearchResult, gmail::GmailError>> {
        let Some(transport) = self.transport() else {
            return Task::ready(Ok(SearchResult::default()));
        };
        if transport.is_offline() {
            return Task::ready(Err(gmail::GmailError::Network("offline".into())));
        }
        let (store, account, query) = (self.store.clone(), self.email().to_string(), query.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let gmail = gmail::Gmail::new(Arc::new(outbox::UserPriority(transport)));
                    gmail::list_and_store(
                        &gmail,
                        &store,
                        &account,
                        &query,
                        gmail::SEARCH_PAGE,
                        page.as_deref(),
                    )
                })
                .await;
            this.update(cx, |this, cx| match &result {
                Ok(found) if !found.stored.is_empty() => {
                    cx.emit(BackendEvent::ThreadsChanged(found.stored.clone()))
                }
                Ok(_) => {}
                Err(e) => this.note_gmail_error(e, cx),
            })
            .ok();
            result.map(|found| SearchResult {
                thread_ids: found.ids,
                estimate: found.estimate,
                next_page: found.next_page,
            })
        })
    }

    /// Where loading `view`'s older threads is.
    pub fn older(&self, view: &ViewKey) -> Older {
        self.older
            .iter()
            .find(|(v, _)| v == view)
            .map_or(Older::More, |(_, pages)| pages.state)
    }

    /// Fetch the next page of a view, past the sync window (LST-7): first
    /// the threads before `oldest` (ms), the view's oldest cached one, then
    /// page on. Emits `ThreadsChanged` for the threads stored and
    /// `CountsChanged` once done, for the "Load older" row.
    pub fn load_older(&mut self, view: ViewKey, oldest: Option<i64>, cx: &mut Context<Self>) {
        let Some(transport) = self.transport() else {
            return;
        };
        let Some(query) = older_query(&view, oldest) else {
            return;
        };
        let ix = match self.older.iter().position(|(v, _)| *v == view) {
            Some(ix) => ix,
            None => {
                self.older.push((view.clone(), OlderPages::default()));
                self.older.len() - 1
            }
        };
        let pages = &mut self.older[ix].1;
        if pages.state != Older::More {
            return;
        }
        pages.state = Older::Loading;
        let query = pages.query.get_or_insert(query).clone();
        let page = pages.next.clone();
        let (store, account) = (self.store.clone(), self.email().to_string());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let gmail = gmail::Gmail::new(Arc::new(outbox::UserPriority(transport)));
                    gmail::older_pages(
                        &gmail,
                        &store,
                        &account,
                        &query,
                        page,
                        OLDER_TRIES,
                        OLDER_PAGE,
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                let Some((_, pages)) = this.older.iter_mut().find(|(v, _)| *v == view) else {
                    return;
                };
                match result {
                    Ok((stored, next)) => {
                        pages.state = if next.is_some() {
                            Older::More
                        } else {
                            Older::Done
                        };
                        pages.next = next;
                        if !stored.is_empty() {
                            cx.emit(BackendEvent::ThreadsChanged(stored));
                        }
                    }
                    Err(e) => {
                        pages.state = Older::More;
                        eprintln!("Pigeon: couldn't load older conversations: {e}");
                        this.note_gmail_error(&e, cx);
                    }
                }
                cx.emit(BackendEvent::CountsChanged);
            })
            .ok();
        })
        .detach();
    }

    fn note_gmail_error(&mut self, error: &gmail::GmailError, cx: &mut Context<Self>) {
        match error {
            gmail::GmailError::Network(m) => {
                self.note_error(&TransportError::Network(m.clone()), cx)
            }
            gmail::GmailError::Status { status: 401, .. } => {
                self.note_error(&TransportError::AuthExpired, cx)
            }
            _ => {}
        }
    }
    // end search and paging (W9) ---------------------------------------

    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        cx.emit(BackendEvent::Status(self.status.clone()));
        self.sync.calendar_due = true;
        self.run_sync(cx);
    }

    // The account (W2).

    /// Google's APIs, authenticated, retrying and within quota. `None` in
    /// demo mode and when signed out. Sync calls it through `Transport`
    /// (background priority); user actions use `send_user`.
    pub fn transport(&self) -> Option<Arc<GoogleTransport>> {
        self.google.as_ref().map(|g| g.transport.clone())
    }

    /// The scopes Google granted (ACC-6); empty unless signed in.
    pub fn granted_scopes(&self) -> &[String] {
        self.record.as_ref().map_or(&[], |r| r.scopes.as_slice())
    }

    /// Whether a feature's scope was granted. Demo mode has everything.
    pub fn has_scope(&self, scope: &str) -> bool {
        match self.source {
            Source::Demo => true,
            Source::Google => self.record.as_ref().is_some_and(|r| r.has_scope(scope)),
            Source::SignedOut => false,
        }
    }

    /// Signed in, but Google stopped accepting it, or a feature's scope is
    /// missing: signing in again fixes both.
    pub fn needs_sign_in(&self) -> bool {
        self.source == Source::Google
            && (self.status == SyncStatus::AuthExpired
                || [
                    auth::GMAIL_SCOPE,
                    auth::CALENDAR_SCOPE,
                    auth::CONTACTS_SCOPE,
                ]
                .iter()
                .any(|s| !self.has_scope(s)))
    }

    pub fn sign_in_state(&self) -> &SignInState {
        &self.sign_in
    }

    /// What the status line says, changed by sync (W3, W4).
    pub fn set_status(&mut self, status: SyncStatus, cx: &mut Context<Self>) {
        // W11: shown through `health`, which keeps Offline and an expired
        // sign-in over sync's progress.
        self.health.note(status);
        self.refresh_status(cx);
    }

    /// A request that failed for good: an expired sign-in, or offline once
    /// the transport has seen two network failures in a row.
    pub fn note_error(&mut self, error: &TransportError, cx: &mut Context<Self>) {
        match error {
            TransportError::AuthExpired => self.set_status(SyncStatus::AuthExpired, cx),
            TransportError::Network(_) => {
                if self
                    .google
                    .as_ref()
                    .is_some_and(|g| g.transport.is_offline())
                {
                    let waiting = self.outbox.waiting();
                    self.set_status(SyncStatus::Offline { waiting }, cx);
                }
            }
            TransportError::TooLarge => {}
        }
    }

    /// Who is signed in, from `people/me` (ACC-1): the name may have
    /// changed since sign-in. Also the first authenticated call, so an
    /// expired sign-in or no network shows at once.
    fn check_profile(&mut self, cx: &mut Context<Self>) {
        let Some(transport) = self.transport() else {
            return;
        };
        self._profile = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { fetch_profile(&*transport) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(profile) => {
                    this.update_profile(profile.claims);
                    this.refresh_photo(profile.photo, cx);
                    // Sync owns the status; the app picks the account's name up with the status.
                    cx.emit(BackendEvent::Status(this.status.clone()));
                }
                Err(e) => {
                    eprintln!("Pigeon: couldn't read the profile: {e}");
                    this.note_error(&e, cx)
                }
            })
            .ok();
        }));
    }

    /// Keep the profile photo current: fetched when missing or a day old,
    /// dropped when the user has none. Offline, the cached one stays.
    fn refresh_photo(&mut self, url: Option<String>, cx: &mut Context<Self>) {
        let dir = data_dir();
        let path = photo::path(&dir, self.email());
        let Some(url) = url else {
            if self.profile_photo.take().is_some() {
                photo::forget(&dir, self.email());
                cx.emit(BackendEvent::AccountChanged);
            }
            return;
        };
        if self.profile_photo.is_some() && photo::is_fresh(&path, std::time::SystemTime::now()) {
            return;
        }
        self._photo = Some(cx.spawn(async move |this, cx| {
            let fetched = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    photo::download(&images::ImageTransport::default(), &url, &fetched)
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    if this.profile_photo.replace(path).is_none() {
                        cx.emit(BackendEvent::AccountChanged);
                    }
                }
                Err(e) => eprintln!("Pigeon: couldn't fetch the profile photo: {e:#}"),
            })
            .ok();
        }));
    }

    fn update_profile(&mut self, claims: auth::Claims) {
        let (Some(account), Some(record)) = (&mut self.account, &mut self.record) else {
            return;
        };
        if claims.name.is_empty() || claims.name == record.name {
            return;
        }
        account.name = claims.name.clone();
        record.name = claims.name;
        let record = record.clone();
        if let Err(e) = auth::save_accounts(&data_dir(), &[record]) {
            eprintln!("Pigeon: couldn't save the account: {e:#}");
        }
    }

    /// The browser sign-in (ACC-3); again after "Sign in again" (STA-4),
    /// which keeps the cache and the queue.
    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.sign_in == SignInState::Waiting
            || self.source == Source::Google && !self.needs_sign_in()
        {
            return;
        }
        let dir = data_dir();
        let client = match ClientConfig::load(&dir) {
            Ok(client) => client,
            Err(e) => return self.sign_in_failed(format!("{e:#}."), cx),
        };
        let (listener, port) = match auth::listen() {
            Ok(listening) => listening,
            Err(e) => return self.sign_in_failed(format!("Couldn't start signing in: {e}"), cx),
        };
        let verifier = auth::random_token(64);
        let state = auth::random_token(16);
        let redirect = format!("http://127.0.0.1:{port}");
        let hint = self.record.as_ref().map(|r| r.email.clone());
        cx.open_url(&auth::auth_url(
            &client,
            &redirect,
            &auth::challenge(&verifier),
            &state,
            hint.as_deref(),
        ));
        let cancel = Arc::new(AtomicBool::new(false));
        self.sign_in_cancel = Some(cancel.clone());
        self.sign_in = SignInState::Waiting;
        cx.emit(BackendEvent::Status(self.status.clone()));
        let tokens = self.tokens.clone();
        self._sign_in = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let code =
                        auth::wait_for_code(&listener, &state, &cancel, auth::SIGN_IN_TIMEOUT)?;
                    auth::finish_sign_in(
                        &UreqTransport::new(),
                        &client,
                        &code,
                        &verifier,
                        &redirect,
                        &*tokens,
                        &dir,
                    )
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(session) => {
                    this.switch(Source::Google, Some(session), cx);
                    cx.emit(BackendEvent::SignedIn);
                }
                Err(SignInError::Cancelled) => {}
                Err(e) => this.sign_in_failed(e.to_string(), cx),
            })
            .ok();
        }));
    }

    fn sign_in_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.sign_in = SignInState::Failed(message);
        self.sign_in_cancel = None;
        cx.emit(BackendEvent::Status(self.status.clone()));
    }

    /// Esc on the waiting screen.
    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.sign_in_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self._sign_in = None;
        self.sign_in = SignInState::Idle;
        cx.emit(BackendEvent::Status(self.status.clone()));
    }

    /// From the sign-in screen: the placeholder data, until Sign In or
    /// Sign Out goes back to it (ME-050).
    pub fn use_demo(&mut self, cx: &mut Context<Self>) {
        if self.source == Source::Demo {
            return;
        }
        self.switch(Source::Demo, None, cx);
        cx.emit(BackendEvent::SignedIn);
    }

    /// The demo data chosen on the sign-in screen, not by `PIGEON_DEMO`:
    /// it can go back there (ME-050).
    pub fn demo_by_choice(&self) -> bool {
        self.source == Source::Demo && !demo_requested()
    }

    /// Back to the sign-in screen from demo data chosen there (ME-050).
    pub fn leave_demo(&mut self, cx: &mut Context<Self>) {
        if !self.demo_by_choice() {
            return;
        }
        self.switch(Source::SignedOut, None, cx);
        cx.emit(BackendEvent::SignedOut);
    }

    /// Revoke the grant, delete the Keychain item, the `accounts.json`
    /// entry and the account's cached rows, then the sign-in screen (ACC-7).
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self.source != Source::Google {
            return;
        }
        let email = self.email().to_string();
        let raw: Arc<dyn Transport> = self
            .google
            .as_ref()
            .map_or_else(|| Arc::new(UreqTransport::new()) as _, |g| g.raw.clone());
        let refresh_token = self.google.as_ref().map(|g| g.auth.refresh_token().clone());
        let tokens = self.tokens.clone();
        let store = self.store.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(e) =
                    auth::forget(&*raw, refresh_token.as_ref(), &email, &*tokens, &data_dir())
                {
                    eprintln!("Pigeon: signing out: {e:#}");
                }
                photo::forget(&data_dir(), &email);
                if let Err(e) = store.delete_account(&email) {
                    eprintln!("Pigeon: couldn't clear the cache: {e:#}");
                }
            })
            .detach();
        self.switch(Source::SignedOut, None, cx);
        cx.emit(BackendEvent::SignedOut);
    }

    // Settings (PER-3).

    /// The saved settings, by key, to apply before the first frame.
    pub fn load_settings(&self) -> serde_json::Map<String, serde_json::Value> {
        self.prefs.load_settings().unwrap_or_default()
    }

    /// Save settings once they settle.
    pub fn save_settings(
        &mut self,
        settings: serde_json::Map<String, serde_json::Value>,
        cx: &mut Context<Self>,
    ) {
        self.pending_settings = Some(settings);
        self._save_settings = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTINGS_DEBOUNCE).await;
            let Ok(Some((prefs, settings))) = this.update(cx, |this, _| {
                Some((this.prefs.clone(), this.pending_settings.take()?))
            }) else {
                return;
            };
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = prefs.save_settings(&settings) {
                        eprintln!("Pigeon: couldn't save settings: {e:#}");
                    }
                })
                .await;
        }));
    }

    // Sprint 2 W1 (NTF-1, NTF-2, NTF-3) -------------------------------

    /// Notifications on: poll every minute in the background instead of
    /// every five.
    pub fn set_notifications(&mut self, on: bool) {
        self.sync.notifications = on;
    }

    /// The Mac woke: sync mail and the calendar now.
    pub fn wake(&mut self, cx: &mut Context<Self>) {
        self.sync.calendar_due = true;
        self.run_sync(cx);
    }

    /// Whether these threads are read, or out of the Inbox, now: their
    /// notifications can go.
    pub fn threads_done(&self, ids: &[String]) -> bool {
        let threads = self.load_threads(ids);
        ids.iter().all(|id| {
            threads
                .iter()
                .find(|t| &t.id == id)
                .is_none_or(|t| !t.unread || t.folder != Folder::Inbox)
        })
    }
    // end sprint 2 W1 ---------------------------------------------------

    // Sprint 2 W7b ------------------------------------------------------

    /// The Inbox threads without their bodies, read off the main thread,
    /// with the changes still queued shown as made: the menu bar's recent
    /// mail (MC-009). Threads a queued change touches come too, since it
    /// may bring them back to the Inbox.
    pub fn load_inbox_heads(&self, cx: &mut Context<Self>) -> Task<Vec<Thread>> {
        let store = self.store.clone();
        let email = self.email().to_string();
        let also = self.outbox.queued_threads();
        cx.spawn(async move |this, cx| {
            let mut threads = cx
                .background_executor()
                .spawn(async move { store.load_inbox_heads(&email, &also).unwrap_or_default() })
                .await;
            this.update(cx, |this, _| this.outbox.overlay(&mut threads))
                .ok();
            threads
        })
    }
    // end sprint 2 W7b --------------------------------------------------

    // Sprint 2 W3 (DRF-5) -----------------------------------------------

    /// A save while typing: stored here now, at `revision`, and on Gmail
    /// after `delay_ms` (newer saves before then replace it). False when a
    /// newer save, a send or a discard came first.
    pub fn save_draft(
        &mut self,
        local_id: String,
        draft: Outgoing,
        revision: u64,
        delay_ms: i64,
        cx: &mut Context<Self>,
    ) -> bool {
        let email = self.email().to_string();
        match compose::save_locally(&self.store, &email, &local_id, &draft, revision) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => eprintln!("Pigeon: couldn't save the draft here: {e:#}"),
        }
        self.push(
            Op::Mail(MailOp::SaveDraft {
                local_id,
                draft,
                revision,
            }),
            delay_ms.max(0),
            cx,
        );
        true
    }
    // end sprint 2 W3 ---------------------------------------------------

    // Sprint 2 W6 -------------------------------------------------------

    /// The contact card for `email` (CON-5), from the store only; `name` is
    /// the one the mail gave, for people who aren't contacts.
    pub fn contact_card(&self, email: &str, name: &str) -> crate::contacts::ContactCard {
        let fallback = || crate::contacts::ContactCard {
            name: name.to_string(),
            emails: vec![email.to_string()],
            ..Default::default()
        };
        let Some(account) = &self.account else {
            return fallback();
        };
        self.store
            .contact_card(account, email, name)
            .unwrap_or_else(|e| {
                eprintln!("Pigeon: couldn't read the contact card: {e:#}");
                fallback()
            })
    }
    // end sprint 2 W6 ---------------------------------------------------

    /// Write settings still waiting to settle: on quit.
    pub fn flush_settings(&mut self) {
        self._save_settings = None;
        if let Some(settings) = self.pending_settings.take()
            && let Err(e) = self.prefs.save_settings(&settings)
        {
            eprintln!("Pigeon: couldn't save settings: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_map_to_gmail_ops() {
        let t = || vec!["t".to_string()];
        let s = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            MailOp::moves(t(), Folder::Inbox, Folder::Archive),
            [MailOp::Modify {
                threads: t(),
                add: vec![],
                remove: s(&["INBOX", "SPAM"])
            }]
        );
        assert_eq!(
            MailOp::moves(t(), Folder::Trash, Folder::Inbox),
            [
                MailOp::Untrash { threads: t() },
                MailOp::Modify {
                    threads: t(),
                    add: s(&["INBOX"]),
                    remove: s(&["SPAM"])
                }
            ]
        );
        assert_eq!(
            MailOp::moves(t(), Folder::Inbox, Folder::Trash),
            [MailOp::Trash { threads: t() }]
        );
        assert!(MailOp::moves(t(), Folder::Inbox, Folder::Sent).is_empty());
    }

    #[test]
    fn only_moves_wait_for_undo() {
        let modify = |label: &str| MailOp::Modify {
            threads: vec![],
            add: vec![label.into()],
            remove: vec![],
        };
        assert!(!modify(gmail::STARRED).undoable());
        assert!(!modify("work").undoable());
        assert!(modify(gmail::SPAM).undoable());
        assert!(
            MailOp::DeleteDraft {
                local_id: "d".into()
            }
            .undoable()
        );
    }

    #[test]
    fn older_pages_start_before_the_oldest_cached_thread() {
        let label =
            older_query(&ViewKey::Label("Label_7".into()), Some(1_793_000_000_999)).unwrap();
        assert_eq!(label.q, "before:1793000000");
        assert_eq!(label.label.as_deref(), Some("Label_7"));
        let archive = older_query(&ViewKey::Folder(Folder::Archive), Some(1_000)).unwrap();
        assert_eq!(
            archive.q,
            "-in:inbox -in:spam -in:trash -in:drafts before:1"
        );
        let empty = older_query(&ViewKey::Starred, None).unwrap();
        assert_eq!(
            (empty.q.as_str(), empty.label.as_deref()),
            ("", Some("STARRED"))
        );
        assert!(older_query(&ViewKey::Folder(Folder::Drafts), None).is_none());
    }

    #[test]
    fn data_dir_is_overridable() {
        // Read-only: the variable is only looked at, never set, so tests
        // running at once don't race.
        let dir = data_dir();
        match std::env::var_os("PIGEON_DATA_DIR") {
            Some(d) if !d.is_empty() => assert_eq!(dir, PathBuf::from(d)),
            _ => assert!(dir.ends_with("Library/Application Support/Pigeon")),
        }
    }
}

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

// Sprint 1 lands every seam first (W1); the Google workstreams use what
// demo mode doesn't yet.
#![allow(dead_code)]

pub mod auth;
pub mod calendar;
pub mod compose;
pub mod demo;
pub mod gmail;
pub mod health;
pub mod http;
pub mod images;
pub mod mime;
pub mod outbox;
pub mod people;
pub mod store;
pub mod sync;

use std::{
    path::PathBuf,
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
    search::Query,
};

/// An op in the outbox: its row id.
pub type OpId = i64;

/// How long an op the UI can undo waits before it goes: as long as the
/// toast that offers the Undo.
pub const UNDO_WINDOW: Duration = Duration::from_secs(5);

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
    "https://people.googleapis.com/v1/people/me?personFields=names,emailAddresses";

/// The signed-in user's name and primary address.
fn fetch_profile(transport: &dyn Transport) -> Result<auth::Claims, TransportError> {
    #[derive(Deserialize)]
    struct Person {
        #[serde(default)]
        names: Vec<Field>,
        #[serde(default, rename = "emailAddresses")]
        email_addresses: Vec<Field>,
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
    Ok(auth::Claims {
        name: primary(&person.names)
            .and_then(|ix| person.names[ix].display_name.clone())
            .unwrap_or_default(),
        email: primary(&person.email_addresses)
            .and_then(|ix| person.email_addresses[ix].value.clone())
            .unwrap_or_default(),
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
    pub quoted: Option<Quote>,
}

impl Outgoing {
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
            quoted: self.quoted.clone(),
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
                // Sends go one at a time.
                MailOp::Send { .. } => "send".into(),
            },
            Op::Calendar(op) => match op {
                CalOp::Insert { event, .. } => event.id.clone(),
                CalOp::Patch { id, .. }
                | CalOp::Delete { id, .. }
                | CalOp::Move { id, .. }
                | CalOp::Rsvp { id, .. } => id.clone(),
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
    IdReplaced { old: String, new: String },
    LabelsChanged,
    CountsChanged,
    EventsChanged(Vec<String>),
    EventsRemoved(Vec<String>),
    CalendarsChanged,
    ContactsChanged,
    Status(SyncStatus),
    OpCommitted(OpId),
    OpFailed { op: OpId, message: String },
    SignedIn,
    SignedOut,
}

/// A view that can load older threads (LST-7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewKey {
    Folder(Folder),
    Starred,
    /// By label id.
    Label(String),
}

/// What a server search found (SRC-7): threads by id, newest first, and
/// Gmail's estimate of how many there are in all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    pub thread_ids: Vec<String>,
    pub estimate: Option<usize>,
}

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
    /// Mail sync and body fetches (W3, `sync`).
    sync: sync::SyncState,
    /// W11: offline, waiting and the queue, over sync's status (`health`).
    health: health::Health,
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

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

impl Backend {
    /// Demo if `PIGEON_DEMO=1`, else Google if `accounts.json` names an
    /// account whose refresh token is in the Keychain, else SignedOut.
    pub fn open(cx: &mut App) -> Entity<Backend> {
        let prefs = Store::open(&data_dir().join("pigeon.sqlite")).unwrap_or_else(|e| {
            eprintln!("Pigeon: settings won't be kept: {e:#}");
            Store::open_in_memory().expect("an in-memory store always opens")
        });
        let tokens: Arc<dyn TokenStore> = Arc::new(Keychain);
        let (source, session) = if demo_requested() {
            (Source::Demo, None)
        } else {
            match auth::signed_in(&data_dir(), &*tokens) {
                Some(session) => (Source::Google, Some(session)),
                None => (Source::SignedOut, None),
            }
        };
        cx.new(|cx| Backend::new(source, prefs, tokens, session, cx))
    }

    fn new(
        source: Source,
        prefs: Store,
        tokens: Arc<dyn TokenStore>,
        session: Option<(AccountRecord, Secret)>,
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
                };
                // Without the client, nothing refreshes: the cache shows,
                // and signing in again says what's missing.
                let status = match ClientConfig::load(&data_dir()) {
                    Ok(client) => {
                        google = Some(Google::new(client, refresh_token));
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
            _save_settings: None,
            record,
            google,
            tokens,
            sign_in: SignInState::Idle,
            sign_in_cancel: None,
            _sign_in: None,
            _profile: None,
            sync: Default::default(),
        };
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

    fn email(&self) -> &str {
        self.account.as_ref().map_or("", |a| a.email.as_str())
    }

    pub fn status(&self) -> &SyncStatus {
        &self.status
    }

    // Reads, from the store: fast and local.

    /// Everything for the mail panes, newest first. At startup.
    pub fn load_mail(&self) -> MailData {
        let mut data = MailData {
            account: self.account.clone().unwrap_or_default(),
            labels: self.load_labels(),
            threads: self
                .store
                .load_all_threads(self.email())
                .unwrap_or_else(|e| {
                    eprintln!("Pigeon: couldn't read mail from the store: {e:#}");
                    Vec::new()
                }),
        };
        data.sort_threads();
        data
    }

    pub fn load_labels(&self) -> Vec<Label> {
        self.store.load_labels(self.email()).unwrap_or_default()
    }

    /// The stored threads among `ids`; a missing one is gone.
    pub fn load_threads(&self, ids: &[String]) -> Vec<Thread> {
        let mut threads = self
            .store
            .load_threads(self.email(), ids)
            .unwrap_or_default();
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
        self.queue(Op::Mail(op), cx)
    }

    pub fn calendar(&mut self, op: CalOp, cx: &mut Context<Self>) -> OpId {
        self.queue(Op::Calendar(op), cx)
    }

    fn queue(&mut self, op: Op, cx: &mut Context<Self>) -> OpId {
        let delay = if op.undoable() {
            UNDO_WINDOW.as_millis() as i64
        } else {
            0
        };
        match self.outbox.push(op, delay, now_ms()) {
            Ok(id) => {
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

    /// Wake when the next op is due, and commit what's due then.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.committing {
            return;
        }
        let Some(due) = self.outbox.next_due() else {
            self._worker = None;
            return;
        };
        let wait = Duration::from_millis((due - now_ms()).max(0) as u64);
        self._worker = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let Ok(Some((store, account, demo, due, transport))) = this.update(cx, |this, _| {
                let due = this.outbox.take_due(now_ms()).ok()?;
                this.committing = true;
                Some((
                    this.store.clone(),
                    this.email().to_string(),
                    this.is_demo(),
                    due,
                    this.transport(),
                ))
            }) else {
                return;
            };
            let me = account.clone();
            let results = cx
                .background_executor()
                .spawn(async move {
                    // W6: drafts and sends, at the user's priority.
                    let calendar_transport = transport.clone();
                    let compose_gmail =
                        transport.map(|t| gmail::Gmail::new(Arc::new(compose::UserCalls(t))));
                    due.into_iter()
                        .map(|(id, op)| {
                            // W5: Google commits go through gmail.rs and
                            // calendar.rs here.
                            let mut calendar = false;
                            let (result, events) = if demo {
                                let result = match &op {
                                    Op::Mail(op) => store.apply_mail_op(&account, op),
                                    Op::Calendar(op) => store.apply_cal_op(&account, &me, op),
                                };
                                (result, Vec::new())
                            } else if let (Op::Mail(op), Some(gmail)) = (&op, &compose_gmail)
                                && compose::handles(op)
                            {
                                // W6: drafts and sends.
                                compose::commit_compose(gmail, &store, &account, op)
                            } else if let (Op::Calendar(op), Some(transport)) =
                                (&op, &calendar_transport)
                            {
                                // W7: calendar writes.
                                calendar = true;
                                let committed = calendar::commit(
                                    &calendar::UserPriority(transport.clone()),
                                    &store,
                                    &account,
                                    op,
                                );
                                (committed.result(), committed.changes.backend_events())
                            } else {
                                (
                                    Err(anyhow::anyhow!("not connected to Google yet")),
                                    Vec::new(),
                                )
                            };
                            (id, result, events, calendar)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                for (id, result, events, calendar) in results {
                    // What the commit changed, before it's reported.
                    for event in events {
                        cx.emit(event);
                    }
                    // W7: a calendar sync shortly after a calendar write.
                    if calendar {
                        this.sync_calendar_soon(cx);
                    }
                    match result {
                        Ok(()) => {
                            this.outbox.finish(id).ok();
                            cx.emit(BackendEvent::OpCommitted(id));
                        }
                        Err(e) => {
                            this.outbox.fail(id, &e.to_string()).ok();
                            cx.emit(BackendEvent::OpFailed {
                                op: id,
                                message: e.to_string(),
                            });
                        }
                    }
                }
                this.committing = false;
                this.schedule(cx);
            })
            .ok();
        }));
    }

    // Google only; nothing to do on the placeholder data.

    /// Search all of Gmail (SRC-7). Demo search is local only.
    pub fn search(&mut self, _query: &Query, _cx: &mut Context<Self>) -> Task<SearchResult> {
        Task::ready(SearchResult::default())
    }

    /// Fetch the next page of a view, past the sync window (LST-7).
    pub fn load_older(&mut self, _view: ViewKey, _cx: &mut Context<Self>) {}

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
                Ok(claims) => {
                    this.update_profile(claims);
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

    /// From the sign-in screen: the placeholder data, until relaunch.
    pub fn use_demo(&mut self, cx: &mut Context<Self>) {
        if self.source == Source::Demo {
            return;
        }
        self.switch(Source::Demo, None, cx);
        cx.emit(BackendEvent::SignedIn);
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

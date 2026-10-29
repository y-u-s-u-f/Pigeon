//! The sync scheduler (docs/BACKEND_SPRINT_1.md, 2.6): one loop on the
//! `Backend` that wakes on a timer, on window activation and on Sync Now.
//! Each pass runs the mail sync as a plan of steps, each a blocking call
//! off the main thread whose result is stored and reported before the next
//! starts, so the Inbox paints after its first page. Then the calendar pass.
//!
//! - Full sync (SYN-1): profile and labels, the first Inbox page, drafts,
//!   the first page of Sent, Starred, Spam and Trash, then a backfill to
//!   1,000 threads or 90 days. Threads whose history id is unchanged are
//!   skipped. Partial sync starts from the history id taken first.
//! - Partial sync (SYN-2): `history.list` from the stored history id, the
//!   touched threads refetched; a 404 means it's too old: full sync again.
//! - Bodies (RDR-6): fetched raw when a thread stays selected for 150 ms.
//! - Background bodies (AI-3): new Inbox mail's, one conversation at a
//!   time, for the AI pass; apart from the reader's.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{Context, Task};

use super::{
    Backend, BackendEvent, SyncStatus, ViewKey,
    categories::{self, Category},
    gmail::{self, Gmail, GmailError},
    http::Transport,
    mime,
    store::{ALIASES_KEY, SEND_AS_KEY, Store},
};
use crate::data::{Body, Folder, Thread};

/// The history id partial sync continues from.
const HISTORY_KEY: &str = "gmail_history_id";
/// The history id taken at the start of a full sync, kept until it ends.
const FULL_START_KEY: &str = "gmail_full_h0";
pub(super) const LAST_SYNC_KEY: &str = "last_sync";

const POLL_ACTIVE: Duration = Duration::from_secs(45);
const POLL_INACTIVE: Duration = Duration::from_secs(5 * 60);
/// Sprint 2 W1: with the window closed or behind, while notifications are
/// on (NTF-1).
const POLL_BACKGROUND: Duration = Duration::from_secs(60);
const BODY_DEBOUNCE: Duration = Duration::from_millis(150);
/// AI-3: the background body queue says what it stored this often.
const BODIES_READY_EVERY: usize = 10;
/// AI-3: the pass runs at least this often.
const AI_PASS_EVERY: Duration = Duration::from_secs(5 * 60);
/// How often the send-as addresses are read again, after the first
/// partial sync of a launch: signatures and aliases change in Gmail.
const SEND_AS_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The backfill's bounds (6.4 item 2).
const BACKFILL_MAX: usize = 1000;
const BACKFILL_QUERY: &str = "newer_than:90d";

/// The scheduler's state, on the `Backend`.
#[derive(Default)]
pub struct SyncState {
    gmail: Option<Gmail>,
    running: bool,
    /// Asked to sync while a pass ran: go again once it ends.
    again: bool,
    pub(super) active: bool,
    /// Sprint 2 W1: notifications are on, so the background poll is quicker.
    pub(super) notifications: bool,
    /// Network failures in a row (2.9).
    failures: u32,
    /// A full sync has finished once; every view is loaded.
    pub(super) full_done: bool,
    /// Views whose first page the running full sync has stored.
    loaded: Vec<Folder>,
    /// Threads fetched and queued in this pass, for the status line.
    progress: (usize, usize),
    /// The rest of a pass that stopped on an error: the next one resumes it.
    resume: VecDeque<Step>,
    last_error: Option<String>,
    _pass: Option<Task<()>>,
    _timer: Option<Task<()>>,
    _body_debounce: Option<Task<()>>,
    /// Messages whose bodies are being fetched, and those that failed.
    body_loading: HashSet<String>,
    body_errors: HashMap<String, String>,
    /// AI-3: conversations whose bodies the pass waits for, in order, and
    /// those stored since `BodiesReady` last said so.
    body_queue: VecDeque<String>,
    body_queue_running: bool,
    bodies_stored: Vec<String>,
    _ai_pass: Option<Task<()>>,
    /// The calendar pass (W4): when it last ran, whether one runs now,
    /// and whether the next mail pass should run it early (focus, Sync Now).
    calendar_last: Option<Instant>,
    calendar_running: bool,
    pub(super) calendar_due: bool,
    _calendar: Option<Task<()>>,
    /// The contacts pass (W8): when it last ran, and whether one runs now.
    contacts_last: Option<Instant>,
    contacts_running: bool,
    _contacts: Option<Task<()>>,
    /// When the send-as addresses were last asked for.
    send_as_asked: Option<Instant>,
}

/// One step of a mail sync plan.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    /// Profile (the history id to continue from), labels, aliases.
    Start,
    /// The send-as addresses again (aliases, names, signatures).
    SendAs,
    List {
        kind: ListKind,
        page: Option<String>,
        seen: usize,
    },
    /// Up to `gmail::BATCH_SIZE` threads, by id: one HTTP batch.
    Fetch(Vec<String>),
    Drafts,
    Labels,
    /// The view's first page is stored.
    Loaded(Folder),
    /// Move on to `history`, with the messages history added since the
    /// last one: those still news are notified about (NTF-1).
    SetHistory {
        history: u64,
        added: Vec<(String, String)>,
    },
    /// The full sync is done: continue from the history id taken at Start.
    Finish,
    Partial,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ListKind {
    Label(&'static str, usize),
    /// W4a: a search's first page.
    Query(&'static str, usize),
    Backfill,
}

fn full_plan() -> Vec<Step> {
    let list = |label, max| Step::List {
        kind: ListKind::Label(label, max),
        page: None,
        seen: 0,
    };
    vec![
        Step::Start,
        list(gmail::INBOX, 100),
        // W4a: Primary's first page, however many promotions came first.
        Step::List {
            kind: ListKind::Query(Category::Primary.view_query(), 50),
            page: None,
            seen: 0,
        },
        Step::Loaded(Folder::Inbox),
        Step::Drafts,
        Step::Loaded(Folder::Drafts),
        list(gmail::SENT, 50),
        Step::Loaded(Folder::Sent),
        list(gmail::STARRED, 50),
        list(gmail::SPAM, 25),
        list(gmail::TRASH, 25),
        Step::List {
            kind: ListKind::Backfill,
            page: None,
            seen: 0,
        },
        Step::Finish,
    ]
}

/// What a step did.
#[derive(Debug, Default)]
struct Outcome {
    changed: Vec<String>,
    removed: Vec<String>,
    labels: bool,
    /// Sprint 2 W1: mail to notify about (NTF-1).
    new_mail: Vec<super::new_mail::NewMail>,
    aliases: Option<Vec<String>>,
    /// W3 (MST-1): every send-as address, the main one too.
    send_as: Option<Vec<crate::data::SendAs>>,
    loaded: Option<Folder>,
    full_done: bool,
    /// W4a: history touched the Inbox, so its category counts are stale.
    inbox: bool,
    /// Threads queued to fetch, and fetched.
    queued: usize,
    fetched: usize,
    /// Steps to run next, before the rest of the plan.
    next: Vec<Step>,
    /// Throw away the rest of the plan for `next`.
    replan: bool,
    error: Option<GmailError>,
}

impl Outcome {
    fn failed(error: GmailError) -> Self {
        Self {
            error: Some(error),
            ..Default::default()
        }
    }
}

/// The blocking half: Gmail calls and store writes, off the main thread.
#[derive(Clone)]
struct MailSync {
    gmail: Gmail,
    store: Store,
    account: String,
}

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return Outcome::failed(e.into()),
        }
    };
}

impl From<anyhow::Error> for GmailError {
    fn from(e: anyhow::Error) -> Self {
        GmailError::Parse(format!("store: {e:#}"))
    }
}

impl MailSync {
    fn run(&self, step: Step) -> Outcome {
        match step {
            Step::Start => self.start(),
            Step::List { kind, page, seen } => self.list(kind, page, seen),
            Step::Fetch(ids) => self.fetch(&ids),
            Step::Drafts => self.drafts(),
            Step::Labels => self.labels(),
            Step::SendAs => match self.send_as() {
                Ok(outcome) => outcome,
                // Offline: the pass stops here and resumes with it.
                Err(e) if e.is_network() => Outcome::failed(e),
                // Aliases matter less than the mail; carry on without them.
                Err(e) => {
                    eprintln!("Pigeon: couldn't read the send-as addresses: {e}");
                    Outcome::default()
                }
            },
            Step::Loaded(folder) => Outcome {
                loaded: Some(folder),
                ..Default::default()
            },
            Step::SetHistory { history, added } => Outcome {
                new_mail: tri!(super::new_mail::checkpoint(
                    &self.store,
                    &self.account,
                    &added,
                    HISTORY_KEY,
                    history,
                    super::now_ms(),
                )),
                ..Default::default()
            },
            Step::Finish => {
                if let Some(h0) = tri!(self.store.sync_state(&self.account, FULL_START_KEY)) {
                    tri!(self.store.set_sync_state(&self.account, HISTORY_KEY, &h0));
                }
                Outcome {
                    full_done: true,
                    ..Default::default()
                }
            }
            Step::Partial => self.partial(),
        }
    }

    fn start(&self) -> Outcome {
        let profile: gmail::Profile = tri!(self.gmail.call(&gmail::profile_request()));
        tri!(self.store.set_sync_state(
            &self.account,
            FULL_START_KEY,
            &profile.history_id.to_string()
        ));
        let mut outcome = self.labels();
        if outcome.error.is_some() {
            return outcome;
        }
        // Aliases matter less than the mail; carry on without them.
        if let Ok(send_as) = self.send_as() {
            outcome.aliases = send_as.aliases;
            outcome.send_as = send_as.send_as;
        }
        outcome
    }

    /// The send-as addresses (W3, MST-1), stored and reported.
    fn send_as(&self) -> Result<Outcome, GmailError> {
        let list: gmail::SendAsList = self.gmail.call(&gmail::send_as_request())?;
        let send_as: Vec<crate::data::SendAs> =
            list.send_as.iter().map(gmail::SendAs::to_send_as).collect();
        let aliases: Vec<String> = list
            .send_as
            .into_iter()
            .filter(|s| !s.is_primary && !s.send_as_email.eq_ignore_ascii_case(&self.account))
            .map(|s| s.send_as_email)
            .collect();
        if let Ok(json) = serde_json::to_string(&aliases) {
            self.store
                .set_sync_state(&self.account, ALIASES_KEY, &json)
                .ok();
        }
        if let Ok(json) = serde_json::to_string(&send_as) {
            self.store
                .set_sync_state(&self.account, SEND_AS_KEY, &json)
                .ok();
        }
        Ok(Outcome {
            aliases: Some(aliases),
            send_as: Some(send_as),
            ..Default::default()
        })
    }

    fn labels(&self) -> Outcome {
        let list: gmail::LabelList = tri!(self.gmail.call(&gmail::labels_list_request()));
        // Counts come only from labels.get: for the sidebar's folders and
        // labels.
        let counted: Vec<usize> = list
            .labels
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                gmail::shows_label(l)
                    || [gmail::INBOX, gmail::SPAM, gmail::DRAFT].contains(&l.id.as_str())
            })
            .map(|(ix, _)| ix)
            .collect();
        let requests: Vec<_> = counted
            .iter()
            .map(|&ix| gmail::label_get_request(&list.labels[ix].id))
            .collect();
        let mut labels = list.labels;
        for (ix, result) in counted
            .into_iter()
            .zip(self.gmail.batch::<gmail::GLabel>(&requests))
        {
            if let Ok(full) = result {
                labels[ix] = full;
            }
        }
        tri!(self.store.replace_gmail_labels(&self.account, &labels));
        Outcome {
            labels: true,
            ..Default::default()
        }
    }

    fn list(&self, kind: ListKind, page: Option<String>, seen: usize) -> Outcome {
        let request = match kind {
            ListKind::Label(label, max) => {
                gmail::threads_list_request(Some(label), None, max, page.as_deref())
            }
            ListKind::Query(q, max) => {
                gmail::threads_list_request(None, Some(q), max, page.as_deref())
            }
            ListKind::Backfill => {
                gmail::threads_list_request(None, Some(BACKFILL_QUERY), 100, page.as_deref())
            }
        };
        let list: gmail::ThreadList = tri!(self.gmail.call(&request));
        let stored = tri!(self.store.thread_history_ids(&self.account));
        let changed = changed_threads(&list.threads, &stored);
        let seen = seen + list.threads.len();
        let mut next: Vec<Step> = changed
            .chunks(gmail::BATCH_SIZE)
            .map(|chunk| Step::Fetch(chunk.to_vec()))
            .collect();
        if kind == ListKind::Backfill
            && seen < BACKFILL_MAX
            && let Some(page) = list.next_page_token
        {
            next.push(Step::List {
                kind,
                page: Some(page),
                seen,
            });
        }
        Outcome {
            queued: changed.len(),
            next,
            ..Default::default()
        }
    }

    fn fetch(&self, ids: &[String]) -> Outcome {
        let requests: Vec<_> = ids.iter().map(|id| gmail::thread_get_request(id)).collect();
        let mut threads: Vec<Thread> = Vec::new();
        let mut removed = Vec::new();
        let mut failed = Vec::new();
        let mut error = None;
        let me = self.store.me(&self.account);
        for (id, result) in ids
            .iter()
            .zip(self.gmail.batch::<gmail::GThread>(&requests))
        {
            match result {
                Ok(t) => match gmail::thread_from_gmail(&t, &me) {
                    Some(thread) => threads.push(thread),
                    // Only drafts left: no conversation.
                    None => removed.push(id.clone()),
                },
                Err(e) if e.is_not_found() => removed.push(id.clone()),
                Err(e) => {
                    failed.push(id.clone());
                    error = Some(e);
                }
            }
        }
        let changed: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
        let fetched = changed.len() + removed.len();
        tri!(self.store.put_synced_threads(&self.account, threads));
        tri!(self.store.delete_threads(&self.account, &removed));
        // The next pass picks up only the ones that failed.
        let next = if failed.is_empty() {
            Vec::new()
        } else {
            vec![Step::Fetch(failed)]
        };
        Outcome {
            changed,
            removed,
            fetched,
            next,
            error,
            ..Default::default()
        }
    }

    /// Gmail's drafts as Drafts threads (DRF-2). A draft saved from here
    /// keeps its local id; the others are `draft-<id>`.
    fn drafts(&self) -> Outcome {
        let mut drafts = Vec::new();
        let mut page = None;
        for _ in 0..5 {
            let list: gmail::DraftList = tri!(
                self.gmail
                    .call(&gmail::drafts_list_request(page.as_deref()))
            );
            drafts.extend(list.drafts);
            page = list.next_page_token;
            if page.is_none() {
                break;
            }
        }
        let stored = tri!(self.store.stored_drafts(&self.account));
        // By Gmail draft id, and by local id (MC-024).
        let by_draft: HashMap<&str, (&String, &String)> = stored
            .iter()
            .filter_map(|(local, d, message)| Some((d.as_deref()?, (local, message))))
            .rev()
            .collect();
        let by_local: HashMap<&str, &String> = stored
            .iter()
            .rev()
            .map(|(local, _, message)| (local.as_str(), message))
            .collect();
        let local_for = |draft_id: &str| {
            by_draft
                .get(draft_id)
                .map(|(local, message)| ((*local).clone(), (*message).clone()))
        };
        let mut wanted = Vec::new();
        let mut keep: HashSet<String> = HashSet::new();
        for draft in &drafts {
            let (thread_id, stored_message) = local_for(&draft.id).unwrap_or_else(|| {
                let id = gmail::draft_thread_id(&draft.id);
                let message = by_local
                    .get(id.as_str())
                    .map(|m| (*m).clone())
                    .unwrap_or_default();
                (id, message)
            });
            keep.insert(thread_id.clone());
            if stored_message != draft.message.id {
                wanted.push((thread_id, draft.id.clone(), draft.message.id.clone()));
            }
        }
        let requests: Vec<_> = wanted
            .iter()
            .map(|(_, _, message)| gmail::message_metadata_request(message))
            .collect();
        let mut threads = Vec::new();
        let mut error = None;
        for ((thread_id, draft_id, _), result) in wanted
            .iter()
            .zip(self.gmail.batch::<gmail::GMessage>(&requests))
        {
            match result {
                Ok(m) => {
                    let mut thread = gmail::draft_thread(draft_id, &m);
                    thread.id = thread_id.clone();
                    threads.push((thread, draft_id.clone()));
                }
                Err(e) if e.is_not_found() => {}
                Err(e) => error = Some(e),
            }
        }
        // Drafts that came from Gmail and are gone from it. Local ones not
        // yet saved there (no draft id) stay.
        let removed: Vec<String> = stored
            .iter()
            .filter(|(local, draft_id, _)| {
                (draft_id.is_some() || gmail::is_server_draft(local)) && !keep.contains(local)
            })
            .map(|(local, _, _)| local.clone())
            .collect();
        let changed: Vec<String> = threads.iter().map(|(t, _)| t.id.clone()).collect();
        let ids: Vec<(String, String)> = threads
            .iter()
            .map(|(t, d)| (t.id.clone(), d.clone()))
            .collect();
        tri!(
            self.store
                .put_synced_threads(&self.account, threads.into_iter().map(|(t, _)| t).collect())
        );
        for (thread_id, draft_id) in ids {
            tri!(
                self.store
                    .set_draft_id(&self.account, &thread_id, &draft_id)
            );
        }
        tri!(self.store.delete_threads(&self.account, &removed));
        Outcome {
            changed,
            removed,
            error,
            ..Default::default()
        }
    }

    fn partial(&self) -> Outcome {
        let Some(start) = tri!(self.store.sync_state(&self.account, HISTORY_KEY))
            .and_then(|h| h.parse::<u64>().ok())
        else {
            return Outcome {
                next: full_plan(),
                replan: true,
                ..Default::default()
            };
        };
        let mut history = Vec::new();
        let mut latest = start;
        let mut page: Option<String> = None;
        loop {
            match self
                .gmail
                .call::<gmail::HistoryList>(&gmail::history_request(start, page.as_deref()))
            {
                Ok(list) => {
                    history.extend(list.history);
                    latest = latest.max(list.history_id.unwrap_or(start));
                    page = list.next_page_token;
                    if page.is_none() {
                        break;
                    }
                }
                // Too old to continue from: sync again from scratch,
                // skipping what hasn't changed.
                Err(e) if e.is_not_found() => {
                    return Outcome {
                        next: full_plan(),
                        replan: true,
                        ..Default::default()
                    };
                }
                Err(e) => return Outcome::failed(e),
            }
        }
        partial_plan(&history, latest)
    }

    /// Fetch, parse and store the bodies of `messages` (id, Gmail's size
    /// estimate) in `thread_id`; the messages that failed, with why.
    fn fetch_bodies(&self, thread_id: &str, messages: &[(String, u64)]) -> Vec<(String, String)> {
        let mut answers = Vec::new();
        for chunk in body_chunks(messages) {
            let requests: Vec<_> = chunk
                .iter()
                .map(|(id, _)| gmail::message_raw_request(id))
                .collect();
            answers.extend(
                chunk
                    .iter()
                    .map(|(id, _)| id)
                    .zip(self.gmail.batch::<gmail::GMessage>(&requests)),
            );
        }
        let mut bodies = Vec::new();
        let mut failed = Vec::new();
        for (id, result) in answers {
            let parsed = result.map_err(|e| e.to_string()).and_then(|m| {
                m.raw
                    .as_deref()
                    .and_then(mime::decode_raw)
                    .and_then(|raw| mime::parse_message(&raw))
                    .ok_or_else(|| "The message couldn't be read".to_string())
            });
            match parsed {
                Ok(body) => bodies.push((id.clone(), body)),
                Err(e) => failed.push((id.clone(), e)),
            }
        }
        if let Err(e) = self.store.put_bodies(&self.account, thread_id, bodies) {
            eprintln!("Pigeon: couldn't store message bodies: {e:#}");
        }
        failed
    }
}

/// Most of a raw-body batch's answer, estimated: under `http`'s limit on
/// what's read, with room for the batch's own framing (MC-012).
const BODY_BATCH_BYTES: u64 = 40 * 1024 * 1024;

/// `messages` (id, size estimate) in batches whose raw answers, in base64
/// at 4/3 of the size, stay under `BODY_BATCH_BYTES`; a message too big
/// for that goes alone. In order.
fn body_chunks(messages: &[(String, u64)]) -> Vec<&[(String, u64)]> {
    let mut chunks = Vec::new();
    let (mut start, mut bytes) = (0, 0);
    for (ix, (_, size)) in messages.iter().enumerate() {
        let size = size.saturating_mul(4) / 3;
        if ix > start && bytes + size > BODY_BATCH_BYTES {
            chunks.push(&messages[start..ix]);
            (start, bytes) = (ix, 0);
        }
        bytes += size;
    }
    if start < messages.len() {
        chunks.push(&messages[start..]);
    }
    chunks
}

/// Puts the steps `outcome` asks for at the front of `plan`. A step that
/// failed without saying how to go on goes back to the front itself, so
/// the next pass resumes there instead of starting over.
fn advance(plan: &mut VecDeque<Step>, step: Step, outcome: &mut Outcome) {
    if outcome.replan {
        plan.clear();
    }
    if outcome.error.is_some() && outcome.next.is_empty() {
        outcome.next.push(step);
    }
    for step in outcome.next.drain(..).rev() {
        plan.push_front(step);
    }
}

/// The listed threads the store lacks, or has at another history id.
fn changed_threads(listed: &[gmail::ThreadRef], stored: &HashMap<String, u64>) -> Vec<String> {
    listed
        .iter()
        .filter(|t| t.history_id.is_none() || stored.get(&t.id) != t.history_id.as_ref())
        .map(|t| t.id.clone())
        .collect()
}

/// What follows a page of history: refetch what it touched, re-read the
/// drafts and counts if anything changed, then move on to `latest`.
fn partial_plan(history: &[gmail::History], latest: u64) -> Outcome {
    let touched = gmail::touched_threads(history);
    let mut next: Vec<Step> = touched
        .chunks(gmail::BATCH_SIZE)
        .map(|chunk| Step::Fetch(chunk.to_vec()))
        .collect();
    if gmail::touches_drafts(history) {
        next.push(Step::Drafts);
    }
    if !history.is_empty() {
        next.push(Step::Labels);
    }
    next.push(Step::SetHistory {
        history: latest,
        added: super::new_mail::added_messages(history),
    });
    Outcome {
        queued: touched.len(),
        inbox: categories::touches_inbox(history),
        next,
        ..Default::default()
    }
}

impl Backend {
    /// Start syncing mail over `transport` (Google mode). The main agent
    /// wires W2's authenticated transport in here once signed in.
    pub fn start_mail_sync(&mut self, transport: Arc<dyn Transport>, cx: &mut Context<Self>) {
        self.sync.gmail = Some(Gmail::new(transport));
        self.sync.active = true;
        let email = self.email().to_string();
        self.sync.full_done = self
            .store
            .sync_state(&email, HISTORY_KEY)
            .ok()
            .flatten()
            .is_some();
        if let Some(account) = &mut self.account
            && let Ok(Some(json)) = self.store.sync_state(&email, ALIASES_KEY)
        {
            account.aliases = serde_json::from_str(&json).unwrap_or_default();
        }
        if let Some(account) = &mut self.account
            && let Ok(Some(json)) = self.store.sync_state(&email, SEND_AS_KEY)
        {
            account.send_as = serde_json::from_str(&json).unwrap_or_default();
        }
        self.run_sync(cx);
    }

    /// The window came to the front or left it: sync on focus, and poll
    /// less often in the background.
    pub fn set_window_active(&mut self, active: bool, cx: &mut Context<Self>) {
        let was = self.sync.active;
        self.sync.active = active;
        if active && !was {
            self.sync.calendar_due = true;
            self.run_sync(cx);
        }
    }

    /// Whether a view's first page is in the store (STA-1, LST-9): always
    /// in demo mode, else once the first full sync has got to it.
    pub fn view_loaded(&self, view: &ViewKey) -> bool {
        self.source != super::Source::Google
            || self.sync.full_done
            || matches!(view, ViewKey::Folder(folder) if self.sync.loaded.contains(folder))
            || matches!(view, ViewKey::Category(_) if self.sync.loaded.contains(&Folder::Inbox))
    }

    /// Why the last sync failed, while nothing loaded (STA-1).
    pub fn sync_error(&self) -> Option<&str> {
        self.sync.last_error.as_deref()
    }

    /// Gmail's thread counts by label id: (unread, total). Empty in demo
    /// mode (FOL-2, LST-6).
    pub fn label_counts(&self) -> HashMap<String, (u64, u64)> {
        self.store.label_counts(self.email()).unwrap_or_default()
    }

    /// Run a sync pass now, or right after the one running.
    pub(super) fn run_sync(&mut self, cx: &mut Context<Self>) {
        let Some(gmail) = self.sync.gmail.clone() else {
            return;
        };
        if self.sync.running {
            self.sync.again = true;
            return;
        }
        self.sync.running = true;
        self.sync.again = false;
        self.sync._timer = None;
        let worker = MailSync {
            gmail,
            store: self.store.clone(),
            account: self.email().to_string(),
        };
        // A pass that stopped on an error goes on from there.
        let plan = if !self.sync.resume.is_empty() {
            std::mem::take(&mut self.sync.resume)
        } else if self.sync.full_done {
            self.sync.progress = (0, 0);
            let mut plan = VecDeque::from([Step::Partial]);
            // Once a launch (an account synced before W3 lacks them), then
            // every few hours.
            if self
                .sync
                .send_as_asked
                .is_none_or(|at| at.elapsed() >= SEND_AS_EVERY)
            {
                self.sync.send_as_asked = Some(Instant::now());
                plan.push_front(Step::SendAs);
            }
            plan
        } else {
            self.sync.progress = (0, 0);
            self.set_status(SyncStatus::Syncing { done: 0, total: 0 }, cx);
            // Start reads them.
            self.sync.send_as_asked = Some(Instant::now());
            full_plan().into()
        };
        self.sync._pass = Some(cx.spawn(async move |this, cx| {
            let mut plan = plan;
            let mut error = None;
            while let Some(step) = plan.pop_front() {
                let w = worker.clone();
                let again = step.clone();
                let mut outcome = cx
                    .background_executor()
                    .spawn(async move { w.run(step) })
                    .await;
                advance(&mut plan, again, &mut outcome);
                error = outcome.error.take();
                if this
                    .update(cx, |this, cx| this.apply_outcome(outcome, cx))
                    .is_err()
                {
                    return;
                }
                if error.is_some() {
                    break;
                }
            }
            this.update(cx, |this, cx| this.pass_finished(error, plan, cx))
                .ok();
        }));
    }

    fn apply_outcome(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        let mut account_changed = false;
        if let Some(aliases) = outcome.aliases
            && let Some(account) = &mut self.account
            && account.aliases != aliases
        {
            account.aliases = aliases;
            account_changed = true;
        }
        if let Some(send_as) = outcome.send_as
            && let Some(account) = &mut self.account
            && account.send_as != send_as
        {
            account.send_as = send_as;
            account_changed = true;
        }
        if account_changed {
            cx.emit(BackendEvent::AccountChanged);
        }
        if !outcome.changed.is_empty() {
            let changed = outcome.changed.clone();
            cx.emit(BackendEvent::ThreadsChanged(outcome.changed));
            // ACT-11: new mail wakes a snoozed conversation.
            self.wake_on_reply(&changed, cx);
        }
        if !outcome.removed.is_empty() {
            cx.emit(BackendEvent::ThreadsRemoved(outcome.removed));
        }
        if !outcome.new_mail.is_empty() {
            let new_mail = outcome.new_mail.clone();
            cx.emit(BackendEvent::NewMail(outcome.new_mail));
            // AI-3, AI-5.
            self.ai_new_mail(&new_mail, cx);
        }
        if outcome.labels {
            cx.emit(BackendEvent::LabelsChanged);
            cx.emit(BackendEvent::CountsChanged);
        }
        if let Some(folder) = outcome.loaded {
            self.sync.loaded.push(folder);
            // Shows the view's empty state, if it is empty.
            cx.emit(BackendEvent::CountsChanged);
        }
        if outcome.inbox || outcome.full_done {
            self.inbox_touched();
        }
        if outcome.full_done {
            self.sync.full_done = true;
            cx.emit(BackendEvent::CountsChanged);
        }
        self.sync.progress.0 += outcome.fetched;
        self.sync.progress.1 += outcome.queued;
        if !self.sync.full_done {
            let (done, total) = self.sync.progress;
            self.set_status(SyncStatus::Syncing { done, total }, cx);
        }
    }

    /// `rest` is what's left of a plan that stopped on `error`.
    fn pass_finished(
        &mut self,
        error: Option<GmailError>,
        rest: VecDeque<Step>,
        cx: &mut Context<Self>,
    ) {
        self.sync.running = false;
        self.sync.resume = rest;
        let email = self.email().to_string();
        let retry = match error {
            None => {
                self.sync.failures = 0;
                self.sync.last_error = None;
                let at = super::now_ms();
                self.store
                    .set_sync_state(&email, LAST_SYNC_KEY, &at.to_string())
                    .ok();
                self.set_status(SyncStatus::Synced { at }, cx);
                // W4a: Inbox category counts, if the Inbox changed.
                self.refresh_category_counts(cx);
                None
            }
            Some(GmailError::Status { status: 401, .. }) => {
                self.set_status(SyncStatus::AuthExpired, cx);
                None
            }
            Some(e) => {
                eprintln!("Pigeon: mail sync stopped: {e}");
                self.sync.last_error = Some(e.to_string());
                if e.is_network() {
                    self.sync.failures += 1;
                    // W11: or the transport's two failures in a row.
                    if self.sync.failures >= 2 || self.transport().is_some_and(|t| t.is_offline()) {
                        let waiting = self.outbox.waiting();
                        self.set_status(SyncStatus::Offline { waiting }, cx);
                    }
                } else if self.sync.full_done {
                    self.set_status(
                        SyncStatus::Synced {
                            at: super::now_ms(),
                        },
                        cx,
                    );
                }
                // Shows the failed state where nothing has loaded.
                cx.emit(BackendEvent::CountsChanged);
                let backoff = 15u64 << self.sync.failures.saturating_sub(1).min(3);
                Some(Duration::from_secs(backoff.min(120)))
            }
        };

        // calendar pass (W4): replace `sync_calendar` below.
        self.sync_calendar(cx);
        // contacts pass (W8).
        self.sync_contacts(cx);

        if self.sync.again {
            return self.run_sync(cx);
        }
        let wait = retry.unwrap_or(if self.sync.active {
            POLL_ACTIVE
        } else if self.sync.notifications {
            POLL_BACKGROUND
        } else {
            POLL_INACTIVE
        });
        self.sync._timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| this.run_sync(cx)).ok();
        }));
    }

    // calendar pass (W4) ----------------------------------------------
    /// The calendar's sync (SYN-4), run after each mail pass: every 2
    /// minutes, or at once on focus and Sync Now.
    fn sync_calendar(&mut self, cx: &mut Context<Self>) {
        const INTERVAL: Duration = Duration::from_secs(120);
        if self.sync.calendar_running || !self.has_scope(super::auth::CALENDAR_SCOPE) {
            return;
        }
        let due = self.sync.calendar_due
            || self
                .sync
                .calendar_last
                .is_none_or(|last| last.elapsed() >= INTERVAL);
        let Some(transport) = self.transport().filter(|_| due) else {
            return;
        };
        self.sync.calendar_due = false;
        self.sync.calendar_running = true;
        let store = self.store.clone();
        let email = self.email().to_string();
        self.sync._calendar = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { super::calendar::sync_pass(&*transport, &store, &email) })
                .await;
            this.update(cx, |this, cx| {
                this.sync.calendar_running = false;
                this.sync.calendar_last = Some(Instant::now());
                match result {
                    Ok(changes) => {
                        for e in &changes.errors {
                            eprintln!("Pigeon: calendar sync skipped a calendar: {e}");
                        }
                        for event in changes.backend_events() {
                            cx.emit(event);
                        }
                    }
                    Err(e) => eprintln!("Pigeon: calendar sync stopped: {e:#}"),
                }
            })
            .ok();
        }));
    }
    // end calendar pass (W4) ------------------------------------------

    // contacts pass (W8) ----------------------------------------------
    /// The contacts' sync (SYN-6), run after each mail pass: on launch,
    /// then every 30 minutes.
    fn sync_contacts(&mut self, cx: &mut Context<Self>) {
        const INTERVAL: Duration = Duration::from_secs(30 * 60);
        use super::people::{self, Source};
        let sources: Vec<Source> = [
            (Source::Connection, super::auth::CONTACTS_SCOPE),
            (Source::Other, people::OTHER_CONTACTS_SCOPE),
        ]
        .into_iter()
        .filter(|(_, scope)| self.has_scope(scope))
        .map(|(source, _)| source)
        .collect();
        let due = self
            .sync
            .contacts_last
            .is_none_or(|last| last.elapsed() >= INTERVAL);
        if self.sync.contacts_running || sources.is_empty() || !due {
            return;
        }
        let Some(transport) = self.transport() else {
            return;
        };
        self.sync.contacts_running = true;
        let store = self.store.clone();
        let email = self.email().to_string();
        self.sync._contacts = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { people::sync_pass(&*transport, &store, &email, &sources) })
                .await;
            this.update(cx, |this, cx| {
                this.sync.contacts_running = false;
                match result {
                    Ok(changed) => {
                        this.sync.contacts_last = Some(Instant::now());
                        if changed {
                            cx.emit(BackendEvent::ContactsChanged);
                        }
                    }
                    // Offline: try again after the next mail pass.
                    Err(e) if people::is_network(&e) => {}
                    Err(e) => {
                        this.sync.contacts_last = Some(Instant::now());
                        eprintln!("Pigeon: contacts sync stopped: {e:#}");
                    }
                }
            })
            .ok();
        }));
    }

    /// Who a message is sent to joins the book at once, in demo mode too.
    pub(super) fn note_recipients(&mut self, draft: &super::Outgoing, cx: &mut Context<Self>) {
        let recipients: Vec<_> = [&draft.to, &draft.cc, &draft.bcc]
            .into_iter()
            .flatten()
            .filter(|c| !self.account.as_ref().is_some_and(|a| a.is_me(&c.email)))
            .cloned()
            .collect();
        if recipients.is_empty() {
            return;
        }
        match self.store.add_sent_recipients(self.email(), &recipients) {
            Ok(()) => cx.emit(BackendEvent::ContactsChanged),
            Err(e) => eprintln!("Pigeon: couldn't add recipients to contacts: {e:#}"),
        }
    }
    // end contacts pass (W8) ------------------------------------------

    // Bodies (RDR-6).

    /// Fetch the bodies `thread_id`'s messages lack, once it has stayed
    /// selected for 150 ms; a newer call replaces one still waiting.
    pub(super) fn fetch_bodies(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        let Some(gmail) = self.sync.gmail.clone() else {
            return;
        };
        let thread_id = thread_id.to_string();
        self.sync._body_debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BODY_DEBOUNCE).await;
            let Ok(Some((worker, messages))) = this.update(cx, |this, cx| {
                let thread = this.load_threads(std::slice::from_ref(&thread_id)).pop()?;
                let messages: Vec<(String, u64)> = thread
                    .messages
                    .iter()
                    .filter(|m| matches!(m.body, Body::NotLoaded | Body::Failed(_)))
                    .map(|m| (m.id.clone(), m.size_estimate))
                    .collect();
                if messages.is_empty() {
                    return None;
                }
                for (id, _) in &messages {
                    this.sync.body_errors.remove(id);
                    this.sync.body_loading.insert(id.clone());
                }
                cx.emit(BackendEvent::ThreadsChanged(vec![thread_id.clone()]));
                let worker = MailSync {
                    gmail,
                    store: this.store.clone(),
                    account: this.email().to_string(),
                };
                Some((worker, messages))
            }) else {
                return;
            };
            // On its own, so a newer selection doesn't cancel it halfway.
            cx.spawn(async move |cx| {
                let ids = messages.clone();
                let tid = thread_id.clone();
                let failed = cx
                    .background_executor()
                    .spawn(async move { worker.fetch_bodies(&tid, &ids) })
                    .await;
                this.update(cx, |this, cx| {
                    for (id, _) in &messages {
                        this.sync.body_loading.remove(id);
                    }
                    this.sync.body_errors.extend(failed);
                    cx.emit(BackendEvent::ThreadsChanged(vec![thread_id.clone()]));
                    cx.emit(BackendEvent::BodiesReady(vec![thread_id]));
                    this.ai_pass(cx);
                })
                .ok();
            })
            .detach();
        }));
    }

    // AI-3: the background body queue.

    /// Fetch these conversations' missing bodies in the background, one
    /// conversation at a time, after those already waiting.
    pub(super) fn queue_bodies(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if self.sync.gmail.is_none() {
            return;
        }
        for id in ids {
            if !self.sync.body_queue.contains(&id) {
                self.sync.body_queue.push_back(id);
            }
        }
        self.next_bodies(cx);
    }

    /// The next conversation with bodies to fetch; `BodiesReady` every
    /// `BODIES_READY_EVERY` conversations and when the queue is empty.
    fn next_bodies(&mut self, cx: &mut Context<Self>) {
        let Some(gmail) = self.sync.gmail.clone() else {
            return;
        };
        if self.sync.body_queue_running {
            return;
        }
        while let Some(id) = self.sync.body_queue.pop_front() {
            let messages: Vec<(String, u64)> = self
                .store
                .load_threads(self.email(), std::slice::from_ref(&id))
                .unwrap_or_default()
                .iter()
                .flat_map(|t| &t.messages)
                .filter(|m| matches!(m.body, Body::NotLoaded))
                .filter(|m| {
                    !self.sync.body_errors.contains_key(&m.id)
                        && !self.sync.body_loading.contains(&m.id)
                })
                .map(|m| (m.id.clone(), m.size_estimate))
                .collect();
            if messages.is_empty() {
                continue;
            }
            self.sync.body_queue_running = true;
            let worker = MailSync {
                gmail,
                store: self.store.clone(),
                account: self.email().to_string(),
            };
            let account = worker.account.clone();
            cx.spawn(async move |this, cx| {
                let tid = id.clone();
                let failed = cx
                    .background_executor()
                    .spawn(async move { worker.fetch_bodies(&tid, &messages) })
                    .await;
                this.update(cx, |this, cx| {
                    // Signed out or switched meanwhile: the queue went with it.
                    if this.email() != account {
                        return;
                    }
                    this.sync.body_queue_running = false;
                    this.sync.body_errors.extend(failed);
                    this.sync.bodies_stored.push(id);
                    if this.sync.bodies_stored.len() >= BODIES_READY_EVERY {
                        let ids = std::mem::take(&mut this.sync.bodies_stored);
                        cx.emit(BackendEvent::BodiesReady(ids));
                        this.ai_pass(cx);
                    }
                    this.next_bodies(cx);
                })
                .ok();
            })
            .detach();
            return;
        }
        if !self.sync.bodies_stored.is_empty() {
            let ids = std::mem::take(&mut self.sync.bodies_stored);
            cx.emit(BackendEvent::BodiesReady(ids));
            self.ai_pass(cx);
        }
    }

    /// AI-3: the pass every few minutes, with AI on.
    pub(super) fn start_ai_pass(&mut self, cx: &mut Context<Self>) {
        self.ai_pass(cx);
        self.sync._ai_pass = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(AI_PASS_EVERY).await;
                if this.update(cx, |this, cx| this.ai_pass(cx)).is_err() {
                    return;
                }
            }
        }));
    }

    /// Bodies being fetched show as loading, failed ones with why.
    pub(super) fn overlay_bodies(&self, threads: &mut [Thread]) {
        if self.sync.body_loading.is_empty() && self.sync.body_errors.is_empty() {
            return;
        }
        for message in threads.iter_mut().flat_map(|t| &mut t.messages) {
            if matches!(message.body, Body::Loaded { .. }) {
                continue;
            }
            if self.sync.body_loading.contains(&message.id) {
                message.body = Body::Loading;
            } else if let Some(e) = self.sync.body_errors.get(&message.id) {
                message.body = Body::Failed(e.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::http::{FakeTransport, batch_body};

    const ACCOUNT: &str = "me@example.com";
    const BATCH: &str = "/batch/gmail/v1";

    fn sync(fake: FakeTransport) -> (MailSync, Arc<FakeTransport>) {
        let fake = Arc::new(fake);
        let sync = MailSync {
            gmail: Gmail::new(fake.clone()),
            store: Store::open_in_memory().unwrap(),
            account: ACCOUNT.into(),
        };
        (sync, fake)
    }

    /// MC-012: big raw bodies go in batches the read limit takes; one
    /// too big for a batch goes alone.
    #[test]
    fn body_batches_stay_under_the_read_limit() {
        const MB: u64 = 1024 * 1024;
        let messages: Vec<(String, u64)> = [1, 20, 20, 50, 2, 2]
            .iter()
            .enumerate()
            .map(|(ix, mb)| (format!("m{ix}"), mb * MB))
            .collect();
        let ids =
            |chunk: &[(String, u64)]| chunk.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let chunks: Vec<Vec<String>> = body_chunks(&messages).into_iter().map(ids).collect();
        assert_eq!(
            chunks,
            [vec!["m0", "m1"], vec!["m2"], vec!["m3"], vec!["m4", "m5"]]
        );
        assert!(body_chunks(&[]).is_empty());
    }

    #[test]
    fn unchanged_history_ids_are_skipped() {
        let listed: Vec<gmail::ThreadRef> = serde_json::from_str(
            r#"[{"id":"a","historyId":"5"},{"id":"b","historyId":"7"},{"id":"c","historyId":"1"}]"#,
        )
        .unwrap();
        let stored = HashMap::from([("a".to_string(), 5), ("b".to_string(), 6)]);
        assert_eq!(changed_threads(&listed, &stored), ["b", "c"]);
    }

    #[test]
    fn list_queues_only_changed_threads() {
        let (sync, _) = sync(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/threads",
            &[("labelIds", "INBOX")],
            &[(
                200,
                r#"{"threads":[{"id":"thread-a1","historyId":"1052"},{"id":"n","historyId":"3"}]}"#,
            )],
        ));
        let t: gmail::GThread = serde_json::from_str(include_str!(
            "../../tests/fixtures/google/gmail/thread_metadata.json"
        ))
        .unwrap();
        sync.store
            .put_synced_threads(
                ACCOUNT,
                vec![gmail::thread_from_gmail(&t, &Default::default()).unwrap()],
            )
            .unwrap();
        let outcome = sync.list(ListKind::Label(gmail::INBOX, 100), None, 0);
        assert_eq!(outcome.queued, 1);
        assert_eq!(outcome.next, [Step::Fetch(vec!["n".into()])]);
    }

    #[test]
    fn fetch_stores_threads_and_drops_gone_ones() {
        let thread = include_str!("../../tests/fixtures/google/gmail/thread_metadata.json");
        let gone = r#"{"error":{"code":404,"message":"Not Found"}}"#;
        let body = batch_body(&[(200, thread), (404, gone)]);
        let (sync, fake) = sync(FakeTransport::new().on("POST", BATCH, &[], &[(200, &body)]));
        sync.store
            .put_threads(
                ACCOUNT,
                &[Thread::local(
                    "old".into(),
                    "Old".into(),
                    Folder::Archive,
                    vec![crate::data::Message::new(Default::default())],
                    None,
                )],
            )
            .unwrap();
        let outcome = sync.fetch(&["thread-a1".into(), "old".into()]);
        assert_eq!(outcome.changed, ["thread-a1"]);
        assert_eq!(outcome.removed, ["old"]);
        assert!(outcome.error.is_none());
        let stored = sync.store.load_all_threads(ACCOUNT).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].messages.len(), 2);
        // One batch, costing 40 per thread.
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].cost, 80);
    }

    #[test]
    fn partial_refetches_touched_threads() {
        let (sync, _) = sync(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/history",
            &[("startHistoryId", "1900")],
            &[(
                200,
                include_str!("../../tests/fixtures/google/gmail/history.json"),
            )],
        ));
        sync.store
            .set_sync_state(ACCOUNT, HISTORY_KEY, "1900")
            .unwrap();
        let outcome = sync.partial();
        assert!(!outcome.replan);
        assert_eq!(
            outcome.next,
            [
                Step::Fetch(vec![
                    "thread-a1".into(),
                    "thread-b2".into(),
                    "thread-c3".into(),
                    "thread-d4".into()
                ]),
                Step::Drafts,
                Step::Labels,
                Step::SetHistory {
                    history: 2000,
                    // The new Inbox message carries through to the end.
                    added: vec![("msg-b9".into(), "thread-b2".into())]
                }
            ]
        );
    }

    #[test]
    fn stale_history_resyncs() {
        let (sync, _) = sync(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/history",
            &[],
            &[(
                404,
                r#"{"error":{"code":404,"message":"Requested entity was not found."}}"#,
            )],
        ));
        sync.store
            .set_sync_state(ACCOUNT, HISTORY_KEY, "1")
            .unwrap();
        let outcome = sync.partial();
        assert!(outcome.replan);
        assert_eq!(outcome.next, full_plan());
        assert!(outcome.error.is_none());
    }

    #[test]
    fn only_inbox_history_makes_the_category_counts_stale() {
        let history: gmail::HistoryList = serde_json::from_str(include_str!(
            "../../tests/fixtures/google/gmail/history.json"
        ))
        .unwrap();
        assert!(partial_plan(&history.history, 2000).inbox);
        // A draft deleted and a star added: the Inbox didn't change.
        assert!(!partial_plan(&history.history[2..], 2000).inbox);
        assert!(!partial_plan(&[], 2000).inbox);
    }

    #[test]
    fn quiet_history_only_moves_on() {
        let outcome = partial_plan(&[], 77);
        assert_eq!(
            outcome.next,
            [Step::SetHistory {
                history: 77,
                added: Vec::new()
            }]
        );
    }

    #[test]
    fn network_failure_stops_the_step() {
        let (sync, _) =
            sync(FakeTransport::new().on("GET", "/gmail/v1/users/me/profile", &[], &[(0, "")]));
        assert!(sync.start().error.unwrap().is_network());
    }

    #[test]
    fn bodies_are_fetched_raw_and_parsed() {
        use base64::Engine as _;
        let eml = include_str!("../../tests/fixtures/google/gmail/attachments.eml");
        let raw = base64::engine::general_purpose::URL_SAFE.encode(eml);
        let thread = include_str!("../../tests/fixtures/google/gmail/thread_metadata.json");
        let t: gmail::GThread = serde_json::from_str(thread).unwrap();
        let message = format!(r#"{{"id":"msg-a1","raw":"{raw}"}}"#);
        let body = batch_body(&[
            (200, &message),
            (400, r#"{"error":{"message":"Bad Request"}}"#),
        ]);
        let (sync, fake) = sync(FakeTransport::new().on("POST", BATCH, &[], &[(200, &body)]));
        sync.store
            .put_synced_threads(
                ACCOUNT,
                vec![gmail::thread_from_gmail(&t, &Default::default()).unwrap()],
            )
            .unwrap();
        let failed = sync.fetch_bodies(
            "thread-a1",
            &[("msg-a1".into(), 1000), ("msg-a2".into(), 1000)],
        );
        assert_eq!(
            failed,
            [("msg-a2".to_string(), "Gmail: Bad Request".to_string())]
        );
        assert_eq!(fake.requests()[0].cost, 120);
        let stored = &sync.store.load_all_threads(ACCOUNT).unwrap()[0];
        let m = stored.messages.iter().find(|m| m.id == "msg-a1").unwrap();
        assert!(m.html().contains("See attached"));
        assert_eq!(m.attachments.len(), 2);
        assert!(stored.search.all.contains("see attached"));
        assert_eq!(stored.loaded, crate::data::Loaded::Metadata);
        // A later metadata refetch keeps the body.
        sync.store
            .put_synced_threads(
                ACCOUNT,
                vec![gmail::thread_from_gmail(&t, &Default::default()).unwrap()],
            )
            .unwrap();
        let stored = &sync.store.load_all_threads(ACCOUNT).unwrap()[0];
        let m = stored.messages.iter().find(|m| m.id == "msg-a1").unwrap();
        assert_eq!(m.attachments.len(), 2);
        assert!(stored.search.all.contains("see attached"));
    }

    /// Runs a plan the way the scheduler does, without the app.
    fn drive(sync: &MailSync, plan: Vec<Step>) -> Vec<Outcome> {
        drive_rest(sync, plan.into()).0
    }

    /// Also what's left when a step fails, for the next pass.
    fn drive_rest(sync: &MailSync, mut plan: VecDeque<Step>) -> (Vec<Outcome>, VecDeque<Step>) {
        let mut out = Vec::new();
        while let Some(step) = plan.pop_front() {
            let mut outcome = sync.run(step.clone());
            advance(&mut plan, step, &mut outcome);
            let stop = outcome.error.is_some();
            out.push(outcome);
            if stop {
                break;
            }
        }
        (out, plan)
    }

    #[test]
    fn rate_limited_threads_retry_and_the_fetch_completes() {
        let thread = include_str!("../../tests/fixtures/google/gmail/thread_metadata.json");
        let busy = r#"{"error":{"code":429,"message":"Too many concurrent requests for user."}}"#;
        let first = batch_body(&[(429, busy), (404, r#"{"error":{"code":404}}"#)]);
        let second = batch_body(&[(200, thread)]);
        let fake =
            Arc::new(FakeTransport::new().on("POST", BATCH, &[], &[(200, &first), (200, &second)]));
        let sync = MailSync {
            gmail: Gmail::with_clock(
                fake.clone(),
                Arc::new(crate::backend::http::FakeClock::new()),
            ),
            store: Store::open_in_memory().unwrap(),
            account: ACCOUNT.into(),
        };
        let outcome = sync.fetch(&["thread-a1".into(), "gone".into()]);
        assert!(outcome.error.is_none());
        assert!(outcome.next.is_empty());
        assert_eq!(outcome.changed, ["thread-a1"]);
        assert_eq!(outcome.removed, ["gone"]);
        // The retry carried only the rate-limited thread.
        let requests = fake.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].cost, 40);
    }

    #[test]
    fn a_failed_step_leaves_the_rest_to_resume() {
        let thread = include_str!("../../tests/fixtures/google/gmail/thread_metadata.json");
        let denied = r#"{"error":{"code":400,"message":"Bad Request"}}"#;
        let body = batch_body(&[(200, thread), (400, denied)]);
        let (worker, _) = sync(FakeTransport::new().on("POST", BATCH, &[], &[(200, &body)]));
        let plan = VecDeque::from([
            Step::Fetch(vec!["thread-a1".into(), "bad".into()]),
            Step::Drafts,
            Step::Finish,
        ]);
        let (outcomes, rest) = drive_rest(&worker, plan);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error.is_some());
        // Only the failed thread is fetched again, then the plan goes on.
        assert_eq!(
            rest,
            [Step::Fetch(vec!["bad".into()]), Step::Drafts, Step::Finish]
        );
        // A step that fails as a whole goes back as it was.
        let (sync, _) =
            sync(FakeTransport::new().on("GET", "/gmail/v1/users/me/drafts", &[], &[(0, "")]));
        let (_, rest) = drive_rest(&sync, VecDeque::from([Step::Drafts, Step::Finish]));
        assert_eq!(rest, [Step::Drafts, Step::Finish]);
    }

    #[test]
    fn full_sync_fills_the_store_then_continues_from_its_start() {
        let thread = include_str!("../../tests/fixtures/google/gmail/thread_metadata.json");
        let body = batch_body(&[(200, thread)]);
        let labels = r#"{"labels":[{"id":"INBOX","name":"INBOX","type":"system"},
            {"id":"Label_7","name":"Work","type":"user"}]}"#;
        let one = r#"{"threads":[{"id":"thread-a1","historyId":"1052"}]}"#;
        let (sync, fake) = sync(
            FakeTransport::new()
                .on(
                    "GET",
                    "/gmail/v1/users/me/profile",
                    &[],
                    &[(200, r#"{"emailAddress":"me@example.com","historyId":"900"}"#)],
                )
                .on("GET", "/gmail/v1/users/me/labels", &[], &[(200, labels)])
                .on(
                    "GET",
                    "/gmail/v1/users/me/settings/sendAs",
                    &[],
                    &[(200, r#"{"sendAs":[{"sendAsEmail":"me@example.com","isPrimary":true,"isDefault":true},{"sendAsEmail":"alias@example.com","displayName":"Work Me","replyToAddress":"team@example.com","signature":"<b>Me</b>","verificationStatus":"accepted"}]}"#)],
                )
                .on("GET", "/gmail/v1/users/me/threads", &[("labelIds", "INBOX")], &[(200, one)])
                .on("GET", "/gmail/v1/users/me/threads", &[("q", "newer_than%3A90d")], &[(200, one)])
                .on("GET", "/gmail/v1/users/me/threads", &[], &[(200, "{}")])
                .on("GET", "/gmail/v1/users/me/drafts", &[], &[(200, "{}")])
                // Label counts first, then the thread.
                .on(
                    "POST",
                    BATCH,
                    &[],
                    &[
                        (
                            200,
                            &batch_body(&[
                                (200, r#"{"id":"INBOX","name":"INBOX","type":"system","threadsUnread":3,"threadsTotal":9}"#),
                                (200, r#"{"id":"Label_7","name":"Work","type":"user","threadsUnread":1,"threadsTotal":2}"#),
                            ]),
                        ),
                        (200, &body),
                    ],
                ),
        );
        let outcomes = drive(&sync, full_plan());
        assert!(outcomes.iter().all(|o| o.error.is_none()));
        assert!(outcomes.last().unwrap().full_done);
        let aliases = outcomes.iter().find_map(|o| o.aliases.clone()).unwrap();
        assert_eq!(aliases, ["alias@example.com"]);
        // W3 (MST-1): with their names, reply-to and signatures.
        let send_as = outcomes.iter().find_map(|o| o.send_as.clone()).unwrap();
        assert_eq!(send_as.len(), 2);
        assert!(send_as[0].is_default && send_as[0].is_primary);
        assert_eq!(send_as[1].name, "Work Me");
        assert_eq!(send_as[1].reply_to, "team@example.com");
        assert_eq!(send_as[1].signature, "<b>Me</b>");
        assert!(!send_as[1].pending);
        let stored = sync
            .store
            .sync_state(ACCOUNT, SEND_AS_KEY)
            .unwrap()
            .unwrap();
        assert!(stored.contains("Work Me"));
        let threads = sync.store.load_all_threads(ACCOUNT).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(
            sync.store
                .sync_state(ACCOUNT, HISTORY_KEY)
                .unwrap()
                .as_deref(),
            Some("900")
        );
        let counts = sync.store.label_counts(ACCOUNT).unwrap();
        assert_eq!(counts["INBOX"], (3, 9));
        let labels = sync.store.load_labels(ACCOUNT).unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].name, "Work");
        // The backfill lists the Inbox thread again, unchanged: one fetch.
        let fetches = fake
            .requests()
            .iter()
            .filter(|r| r.path() == BATCH && r.cost == 40)
            .count();
        assert_eq!(fetches, 1);
        // W4a: Primary's first page is asked for by category.
        assert!(fake.requests().iter().any(|r| {
            r.query()
                .contains(&("q", "in%3Ainbox%20category%3Aprimary"))
        }));
    }

    /// An account whose full sync ran before W3 has no send-as stored: the
    /// launch's first partial pass reads them, and an offline one resumes
    /// with them.
    #[test]
    fn a_partial_pass_reads_the_send_as_addresses() {
        let send_as = r#"{"sendAs":[{"sendAsEmail":"me@example.com","isPrimary":true,"isDefault":true,"signature":"Cheers, Me"},{"sendAsEmail":"alias@example.com","verificationStatus":"accepted"}]}"#;
        let (sync, _) = sync(
            FakeTransport::new()
                .on(
                    "GET",
                    "/gmail/v1/users/me/settings/sendAs",
                    &[],
                    &[(0, ""), (200, send_as)],
                )
                .on(
                    "GET",
                    "/gmail/v1/users/me/history",
                    &[],
                    &[(200, r#"{"historyId":"1900"}"#)],
                ),
        );
        sync.store
            .set_sync_state(ACCOUNT, HISTORY_KEY, "1900")
            .unwrap();
        assert!(
            sync.store
                .sync_state(ACCOUNT, SEND_AS_KEY)
                .unwrap()
                .is_none()
        );
        let plan = VecDeque::from([Step::SendAs, Step::Partial]);
        let (outcomes, rest) = drive_rest(&sync, plan);
        assert!(outcomes[0].error.as_ref().unwrap().is_network());
        assert_eq!(rest, [Step::SendAs, Step::Partial]);
        let outcomes = drive(&sync, rest.into());
        assert!(outcomes.iter().all(|o| o.error.is_none()));
        let read = outcomes[0].send_as.clone().unwrap();
        assert_eq!(read[0].signature, "Cheers, Me");
        assert_eq!(
            outcomes[0].aliases.as_deref(),
            Some(&["alias@example.com".to_string()][..])
        );
        let stored = sync
            .store
            .sync_state(ACCOUNT, SEND_AS_KEY)
            .unwrap()
            .unwrap();
        assert!(stored.contains("Cheers, Me"));
    }

    #[test]
    fn drafts_become_drafts_threads() {
        let list = r#"{"drafts":[{"id":"r-1","message":{"id":"dm1","threadId":"dm1"}}]}"#;
        let meta = r#"{"id":"dm1","threadId":"dm1","labelIds":["DRAFT"],"snippet":"Hi",
            "internalDate":"1793000000000",
            "payload":{"headers":[{"name":"To","value":"x@example.com"},{"name":"Subject","value":"Plan"}]}}"#;
        let body = batch_body(&[(200, meta)]);
        let (sync, _) = sync(
            FakeTransport::new()
                .on("GET", "/gmail/v1/users/me/drafts", &[], &[(200, list)])
                .on("POST", BATCH, &[], &[(200, &body)]),
        );
        // A Gmail draft no longer there goes.
        let old = gmail::draft_thread(
            "r-old",
            &gmail::GMessage {
                id: "old-m".into(),
                ..Default::default()
            },
        );
        sync.store.put_threads(ACCOUNT, &[old]).unwrap();
        let outcome = sync.drafts();
        assert_eq!(outcome.changed, ["draft-r-1"]);
        assert_eq!(outcome.removed, ["draft-r-old"]);
        let stored = sync.store.load_all_threads(ACCOUNT).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].folder, Folder::Drafts);
        assert_eq!(stored[0].subject, "Plan");
        assert_eq!(
            sync.store.stored_drafts(ACCOUNT).unwrap(),
            [("draft-r-1".into(), Some("r-1".into()), "dm1".into())]
        );
        // Unchanged: nothing fetched again.
        assert!(sync.drafts().changed.is_empty());
    }
}

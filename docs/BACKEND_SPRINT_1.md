# Backend sprint 1: link Pigeon to Google

Plan for turning every 🟡 item in [`ROADMAP.md`](ROADMAP.md) into 🟢: working against the user's real Google account (Gmail, Calendar, People), local-first, with no server. Item IDs are the roadmap's. Google facts were checked against developers.google.com on 2026-10-26; where they change a decision, the change is listed under [Open questions](#open-questions).

## Contents

1. [Goal and scope](#1-goal-and-scope)
2. [Architecture](#2-architecture)
3. [Workstreams](#3-workstreams)
4. [Execution order](#4-execution-order)
5. [Testing](#5-testing)
6. [Done, roadmap rules, risks, open questions](#6-done-roadmap-rules-risks-open-questions)

---

## 1. Goal and scope

**Goal.** Pigeon runs end to end on the user's Google account. Mail, labels, drafts, calendars, events and contacts come from Google, are cached in SQLite, and stay current through polling. Every action the UI offers today writes back to Google. The placeholder JSON stays as a demo mode that uses the same code paths.

**Constraints.** Only one account. No server, no Pub/Sub, no webhooks. Light crates only (see 2.1). Every new action is in ⌘K with a shortcut.

### 1.1 🟡 items covered (93), by workstream

| Workstream | Items |
| --- | --- |
| **W1 Foundation** (model, store, demo mode) | DAT-1, DAT-2, DAT-3, DAT-5, DAT-6 |
| **W2 Auth and HTTP** | ACC-1 (profile name), CAL-1 (scopes, no-calendar empty state) |
| **W3 Gmail read** | FOL-1, FOL-2, FOL-3, FOL-4, LST-1, LST-2, LST-3, LST-4, LST-5, LST-6, LST-9, RDR-2, RDR-3, RDR-4, RDR-5, RDR-7, ATT-3, ATT-5, LNK-1, PAL-3 |
| **W4 Calendar read** | CAL-2, CAL-3, CAL-4, CAL-4a, CAL-5, CAL-12, CAL-17, CAL-18, CAL-19 |
| **W5 Mail actions** | RDR-1, ACT-1, ACT-1a, ACT-2, ACT-3, ACT-4, ACT-5, ACT-6, FOL-5, FOL-7, UND-3, PAL-1 |
| **W6 Compose, drafts, send** | CMP-1 … CMP-11, CMP-13, DRF-1, DRF-2, DRF-3, DRF-4, UND-1, UND-2, ATT-6 |
| **W7 Calendar writes** | CAL-6, CAL-7, CAL-8, CAL-9, CAL-10, CAL-11, CAL-13, CAL-14, CAL-15, CAL-16, CAL-21, CAL-40 |
| **W8 Contacts and identity** | ACC-2, CON-1, CON-2 |
| **W9 Search and paging** | SRC-1 … SRC-8, PAL-2 |
| **W10 Reader extras** | CAL-20, ATT-1 |
| **W11 States and settings** | (only pulled-in ⚪ items, below) |

Some 🟡 items need no backend at all: LNK-1, CAL-4, CAL-4a, CAL-18, RDR-7, LST-9 and PAL-1. They turn 🟢 once a tester confirms them on real data.

**What 🟢 means for a few items that are hard to pin down:**

- **CAL-15:** Reminders are read from and saved to Google (`reminders`). Firing local notifications stays ⚪ (NTF-3), and Google's own apps already notify.
- **CAL-17:** The Join link is read from Google. Editing follows Open question 9.
- **LST-4:** The Archived header counts the cached archived threads, and "Load older" (LST-7) fetches more. Gmail has no cheap server count for "not in Inbox".
- **SRC-5:** Matches the union of the local cache (substring) and Gmail's server search (whole words).

### 1.2 ⚪ items pulled in, and why

| Item | Why the 🟡 items need it | Extent this sprint |
| --- | --- | --- |
| ACC-3 Sign in / remove | Nothing works without a grant. | Sign in, sign out, one account. No "add second account". |
| ACC-5 Per-account sync status | The user must see whether data is current (STA-1). | Status line in the sidebar and Settings › Accounts. |
| ACC-6 Scopes and consent | Granular consent can drop scopes. Features must switch off cleanly. | Read the granted scopes and disable Mail, Calendar or Contacts features that lack theirs. |
| ACC-7 Token refresh, revoke | Access tokens last about 1 h. | Refresh, retry once on 401, revoke on sign-out. |
| SYN-1 Gmail full sync | Fills the cache. | Bounded window, see 2.6. |
| SYN-2 Gmail partial sync | New mail and changes made elsewhere. | `history.list` polling. |
| SYN-4 Calendar incremental sync | Event changes made elsewhere. | `syncToken` per calendar. |
| SYN-6 Contacts sync | CON-1 suggestions from real contacts. | `connections.list` + `otherContacts.list` with sync tokens (Open question 4). |
| SYN-7 Batching, quotas, backoff | Gmail's per-user quota (2.6) makes batching and a rate limiter mandatory. | HTTP batch ≤ 50, token bucket, backoff. |
| STA-1 Loading states | Launch must not wait on the network. LST-9 must not flash "Nothing". | List skeleton, reader spinner, per-view loaded state. |
| STA-2 Failure and rollback | Writes are optimistic. | Rollback plus error toast. |
| STA-3 Offline and queued changes | The write queue must survive no network. | Offline indicator and durable queue. No offline search of uncached mail. |
| STA-4 Expired sign-in | Testing-mode refresh tokens expire after 7 days. | "Sign in again" banner. Queue and cache are kept. |
| STA-5 Conflicts | Calendar ETags, the open thread changing underneath. | 412 → reload the event and show a toast. Refresh the open thread on sync. Never overwrite draft text. |
| PER-1 Local store | The cache. | SQLite. |
| PER-2 Durable outbox | Delayed commits (undo) must survive a quit. | `outbox` table. Quit flushes due sends. |
| PER-3 Persisted settings | Store decision. ATT-1's image setting needs it. | `settings` table, applied before the first frame. |
| RDR-6 Body on demand | Bodies aren't in the list sync. | Fetched on open and cached. |
| LST-7 Paging | Older mail and label views beyond the sync window. | "Load older" at the end of each view. |
| LST-8 Live refresh | Synced changes must reach the list without losing selection. | Uses `update_threads()`. |
| DRF-6 Draft sync | DRF-2 must show server drafts. | `drafts.list` + history. Last write wins, but never while the draft is open. |
| DAT-4 Threading headers | CMP-2/3/11 threading in other clients. | Stored per message and set on replies. |
| CMP-12 Send failure feedback | Rollback of a failed send. | Error toast, and the message goes back to Drafts. |
| CAL-22 Read-only permissions | Edits to shared, holiday or other people's events would fail with 403 constantly. | Read-only panel by `accessRole`, `organizer.self` and `guestsCanModify`. RSVP stays editable. |

### 1.3 Out of scope

All other ⚪ items, including:

- Multiple accounts (ACC-4)
- Label management, permanent delete, nested labels, categories and Important (FOL-6, 8–12)
- Push (SYN-3, SYN-5)
- Notifications and background running (NTF-*)
- Raw source (RDR-8) and cid images (ATT-2)
- Opening, downloading or forwarding attachments (ATT-4, ATT-7)
- Mail settings (MST-*), including a From picker. Aliases are read for ACC-2 only.
- Contact management, photos and directory (CON-3 … CON-8)
- Repeat-rule editing (CAL-29)
- Event types, sharing, free/busy and calendar settings
- Save while typing (DRF-5)
- PLN-*, persisted calendar UI state (CAL-23), extra search operators (SRC-9)
- App signing, notarisation, public OAuth verification (ACC-8)

---

## 2. Architecture

### 2.1 Crates

All of these are added in W1, so later workstreams never touch `Cargo.toml`.

| Crate | Use | Note |
| --- | --- | --- |
| `ureq` 3 (rustls, gzip) | HTTP | `rustls` and `ring` are already in `Cargo.lock` (via GPUI). |
| `rusqlite` (`bundled`) | Cache | Compiles SQLite once. FTS is not used. |
| `security-framework` | Keychain, `SecRandom` for PKCE | Already in `Cargo.lock`. |
| `mail-parser`, `mail-builder` | MIME in and out | Pure Rust. |
| `base64`, `sha2` | base64url (`raw`, PKCE), the S256 challenge | Already in `Cargo.lock`, so they cost no compile time. Listed explicitly. |
| `uuid` (`v4`) | Local IDs, calendar event IDs | Already in `Cargo.lock`. A UUID's 32 lowercase hex digits are valid base32hex, so Google accepts them as client-chosen event IDs. |
| `iana-time-zone` | Local IANA zone for event writes | Already pulled in by `chrono`'s `clock` feature. |

No tokio, no async HTTP, no OAuth crate, no RRULE crate (a small subset parser, see 2.7). The loopback server is `std::net::TcpListener`. The browser opens with `cx.open_url()`.

### 2.2 Module layout

```
src/backend/
  mod.rs        Backend entity, BackendEvent, MailOp/CalOp, Source (Demo | Google), public API (2.3)
  auth.rs       google_client.json, loopback + PKCE, token exchange/refresh/revoke, Keychain, accounts.json
  http.rs       Transport trait (+ FakeTransport for tests), UreqTransport, auth header, retries,
                backoff, quota limiter, HTTP batch encode/decode
  gmail.rs      Gmail API types (serde) and calls; Gmail → model mapping (folder derivation, DAT-2)
  calendar.rs   Calendar API types and calls; Event ↔ Google event mapping; RRULE ↔ Rule
  people.rs     People API types and calls
  mime.rs       outgoing MIME (mail-builder), incoming raw parse (mail-parser), iCalendar UID/METHOD
  store/
    mod.rs      Store (rusqlite Connection, WAL), migrations runner, settings
    schema.sql  every table (2.5), written once in W1
    mail.rs     mail reads (→ MailData/Thread) and writes
    calendar.rs calendar reads (→ CalendarData) and writes
    contacts.rs contacts reads and writes, frequency ranking
    outbox.rs   queue rows
  sync.rs       sync scheduler: mail, calendar and contacts loops, resync rules
  outbox.rs     write queue: delay/undo, coalescing, pending-op overlay, commit, rollback
  demo.rs       seeds an in-memory Store from assets/*.json; writes apply locally only
src/contacts.rs ContactBook, moved out of src/ui/composer.rs in W1 (pure move)
src/app/compose.rs  draft/send/discard/undo handlers, moved out of src/app.rs in W1 (pure move)
src/ui/onboarding.rs  sign-in screen
src/ui/status.rs      sync status line, offline/sign-in-again banner
tests/fixtures/google/{oauth,gmail,calendar,people}/*.json, *.eml   synthetic fixtures
```

### 2.3 The data interface the UI uses

The seam keeps the UI's in-memory view models (`MailData`, `CalendarData`) and their synchronous rendering. What changes is where they come from and where changes go:

- **Reads.** `MailData` and `CalendarData` are built from the `Store`, not from JSON. `PigeonApp::data` stays a `MailData`, and `CalendarView::data` stays a `CalendarData`. A sync never replaces them whole. It reports which IDs changed, and the app reloads just those threads or events from the store through `update_threads()` (`src/app.rs`), which already keeps selection and scroll.
- **Writes.** Handlers mutate the view model exactly as today (that is the optimistic update). They also hand a `MailOp` or `CalOp` to the `Backend`. The backend writes it to the store and the outbox and commits it to Google in the background.
- **Failure.** The backend re-reads the server's truth for the affected IDs, writes it to the store and emits `OpFailed` plus `ThreadsChanged` or `EventsChanged`. The app reloads those IDs, which is the rollback, and shows a toast.

```rust
// src/backend/mod.rs (shape, not final code)
pub struct Backend { store: Store, source: Source, outbox: Outbox, sync: SyncState, /* … */ }
impl EventEmitter<BackendEvent> for Backend {}

impl Backend {
    pub fn open(cx: &mut App) -> Entity<Backend>;          // Demo if PIGEON_DEMO=1, else Google if signed in, else SignedOut
    pub fn account(&self) -> Option<&Account>;             // name, email, aliases (ACC-1, ACC-2)
    pub fn status(&self) -> &SyncStatus;                   // ACC-5, STA-*
    pub fn load_mail(&self) -> MailData;                   // startup, from the cache (fast, local)
    pub fn load_threads(&self, ids: &[String]) -> Vec<Thread>;
    pub fn load_calendar(&self) -> CalendarData;
    pub fn load_events(&self, ids: &[String]) -> Vec<Event>;
    pub fn contacts(&self) -> Vec<Contact>;                // ranked, for ContactBook
    pub fn fetch_body(&mut self, thread_id: &str, cx);     // RDR-6; emits ThreadsChanged when stored
    pub fn mail(&mut self, op: MailOp, cx) -> OpId;
    pub fn calendar(&mut self, op: CalOp, cx) -> OpId;
    pub fn cancel(&mut self, op: OpId) -> bool;            // undo inside the delay window
    pub fn search(&mut self, query: &Query, cx) -> Task<SearchResult>;   // SRC-7
    pub fn load_older(&mut self, view: ViewKey, cx);       // LST-7
    pub fn sync_now(&mut self, cx);
    pub fn sign_in(&mut self, cx); pub fn sign_out(&mut self, cx);
}

pub enum MailOp {
    Modify { threads: Vec<String>, add: Vec<String>, remove: Vec<String> },   // read, star, labels, spam, archive
    Trash { threads: Vec<String> }, Untrash { threads: Vec<String> },
    SaveDraft { local_id: String, draft: Outgoing },
    DeleteDraft { local_id: String },
    Send { draft: Outgoing, local_id: Option<String> },
}
pub enum CalOp {
    Insert { event: Event, send_updates: SendUpdates },
    Patch { id: String, before: Event, after: Event, scope: SeriesScope, send_updates: SendUpdates },
    Delete { id: String, occurrence: Option<NaiveDateTime>, scope: SeriesScope, send_updates: SendUpdates },
    Move { id: String, to_calendar: String },
    Rsvp { id: String, status: RsvpStatus },
    SetSelected { calendar: String, selected: bool },
}
pub enum BackendEvent {
    ThreadsChanged(Vec<String>), ThreadsRemoved(Vec<String>), IdReplaced { old: String, new: String },
    LabelsChanged, CountsChanged,
    EventsChanged(Vec<String>), EventsRemoved(Vec<String>), CalendarsChanged,
    ContactsChanged, Status(SyncStatus),
    OpCommitted(OpId), OpFailed { op: OpId, message: String },
    SignedIn, SignedOut,
}
```

**Ownership.** `PigeonApp` owns the `Entity<Backend>` and subscribes to it. `CalendarView` gets a clone of the handle and issues its own `CalOp`s. Composers keep getting the contact book. `ContactBook` becomes `Rc<RefCell<ContactBook>>` (or an `Entity`) so `ContactsChanged` reaches open composers, as CON-1 asks.

**Demo mode** is the same `Backend` with `Source::Demo`: an in-memory store (`Store::open_in_memory()`) seeded by `demo.rs` from `assets/mailbox.json` and `assets/calendar.json`. The outbox marks ops committed at once, with no transport. That gives one code path for UI work and `cargo test`, and removes `MailData::load()` and `CalendarData::load()` as data sources (DAT-1). The JSON is still compiled in through `include_str!`.

### 2.4 Model changes (W1)

**Mail (`src/data.rs`, DAT-2/3/5)**

- `Thread`:
  - `id` is the Gmail `threadId`, or `draft-local-<uuid>` / `sent-local-<uuid>` until the server answers (DAT-5). `IdReplaced` swaps it in place.
  - `folder` is derived from Gmail labels by `gmail::derive_folder()` in this order:

    | Thread has | `folder` |
    | --- | --- |
    | `TRASH` | Trash |
    | `SPAM` | Spam |
    | `INBOX` | Inbox |
    | only `SENT` messages | Sent |
    | anything else | Archive |

  - Drafts are not folders of conversations. Each Gmail draft is its own one-message `Thread` with `folder = Drafts`, as today. A reply draft keeps `origin.reply_to` = its `threadId`.
  - DRAFT messages are left out of the conversation they belong to. The reader shows the inline composer for a reply draft instead (`reply_draft_of()`).
  - New: `label_ids` (system and user labels, the union over messages), `history_id`, `loaded: Loaded` (`Metadata | Full`).
  - `unread` is true when any message has `UNREAD`. `starred` is true when any message has `STARRED`.
- `Message`: new `id`, `label_ids`, `internal_date` (sort key, replaces `date` for ordering), `message_id`, `in_reply_to`, `references`, `reply_to` (DAT-4), `size_estimate`, `body: Body` (`NotLoaded | Loading | Loaded { html } | Failed(String)`; replaces `html: String`). `Attachment` gains `attachment_id`, `part_id`. `Invite` gains `ical_uid` (`event_id` is resolved from it).
- `Account`: `aliases: Vec<String>` from `sendAs.list`. `MailData::is_me()` checks them all (ACC-2).
- `SearchIndex`: built once when a thread is stored and saved as `threads.search_text`, so loads don't re-parse HTML (DAT-6).

**Calendar (`src/calendar/mod.rs`, `recurrence.rs`)**

- `Event` gains:
  - `etag`, `ical_uid`, `status`, `time_zone: Option<String>`, `html_link`
  - `organizer_self`, `guests_can_modify`, `attendees_omitted`, `event_type`
  - `conference_uri` (CAL-17), `reminder_use_default`
- `recurrence: Option<Recurrence>` becomes `Option<Rule>`. `Rule` is `{ freq, interval, by_day: Vec<(Option<i8>, Weekday)>, by_month_day, count, until }`. The six keywords become constructors, and `Rule::label()` gives "Every weekday" and so on. `exceptions` (EXDATE / cancelled instances) and `recurring_event_id` / `original_start` stay as they are; they already match Google's model.
- `Calendar` gains `access_role`, `primary`, `time_zone`, `default_reminder`. `visible` maps to `calendarList.selected`.
- Times stay wall-clock `NaiveDateTime` in the user's local zone (`timeZone=<local IANA>` on reads, converted with `chrono::Local`). The event's own `timeZone` is kept for writes (CAL-12).

### 2.5 Cache schema

One file, `~/Library/Application Support/Pigeon/pigeon.sqlite` (mode 0600, WAL). Account-scoped tables start with an `account` column, so sign-out is `DELETE … WHERE account = ?` and ACC-4 can come later without a migration. Raw Google JSON is kept where we must write back without losing fields we don't model (events).

| Table | Key | Main columns |
| --- | --- | --- |
| `settings` | `key` | `value` (JSON). App-level: appearance, reduce motion, auto-advance, search scope, density, text scale, pane widths, images. PER-3. |
| `labels` | `account, id` | `name, type, bg, fg, threads_unread, threads_total` |
| `threads` | `account, id` | `history_id, subject, snippet, last_date, folder, unread, starred, has_attachments, label_ids, search_text, in_window, loaded` |
| `messages` | `account, id` | `thread_id, history_id, internal_date, label_ids, from, to, cc, bcc, reply_to, subject, snippet, message_id, in_reply_to, references, size_estimate, attachments (JSON)` |
| `bodies` | `account, message_id` | `html, plain, invite_uid, invite_method, bytes, last_opened`. LRU, capped at 300 MB by default. Eviction drops bodies only. |
| `drafts` | `account, local_id` | `draft_id, message_id, thread_id, markdown, origin (JSON), updated_at` |
| `calendars` | `account, id` | `summary, bg, fg, selected, hidden, access_role, is_primary, time_zone, default_reminder, sync_token` |
| `events` | `account, calendar_id, id` | `etag, ical_uid, status, start, end, all_day, recurring_event_id, original_start, updated, json` |
| `contacts` | `account, resource_name, email` | `source (connection/other), name, etag`. Frequency is computed from `messages`. |
| `sync_state` | `account, key` | `value`: `gmail_history_id`, `calendar_list_sync_token`, `people_sync_token`, `other_contacts_sync_token`, `window_oldest`, `last_sync`, `last_error` |
| `outbox` | `id` | `account, kind, payload (JSON), not_before, state (pending/inflight/failed/done), attempts, next_attempt, error, created_at` |
| `images` | `url_hash` | `url, mime, bytes, fetched_at` (ATT-1, capped) |

Migrations live in `store/schema.sql` as numbered blocks applied in order. Workstreams that need a schema change add a new block with a number reserved in [section 4](#4-execution-order). Nobody edits an earlier block.

### 2.6 Threading and the sync algorithms

**Threads.**

- All HTTP and SQLite work runs off the main thread: `cx.spawn(async move |this, cx| { let r = cx.background_executor().spawn(async move { blocking_call() }).await; this.update(cx, …) })`. This is the pattern `show_toast()` and the calendar tick already use.
- `ureq` calls block a background worker, so the backend caps requests in flight at 4 with a counting semaphore.
- `Store` wraps one `rusqlite::Connection` in `Arc<Mutex<_>>`. Writes are short transactions. Startup reads (`load_mail`, `load_calendar`) take tens of ms on the main thread before the first frame, which is acceptable and keeps the first paint instant.
- The sync scheduler is a single `cx.spawn` loop on `Backend` that wakes on a timer, on window activation (hooked through the existing `observe_window_activation` in `PigeonApp::new`), and 2 s after any committed write.

**Gmail quotas (checked 2026-10-26; changed May 2026).**

- Limit: 6,000 units per minute per user.
- Costs: `threads.get` 40 units (120 with `format=full`), `messages.get` 20 (60 with `raw`/`full`), `threads.list` 10, `history.list` 2, `threads.modify` 10, `messages.send` 100, `drafts.update` 15, `labels.get` 1.
- A batch of *n* calls costs *n* calls. Batches hold at most 100 calls; Google recommends ≤ 50.

So `http.rs` has a token-bucket limiter in quota units: 5,000/min for background sync, with 1,000/min kept for user actions, which always go first. Every call declares its cost.

**Gmail full sync (SYN-1).** Each step stores and emits as it goes, so the Inbox paints after step 4.

1. `getProfile`: email, and record `historyId` H0 *before* listing, so changes during the sync are caught later.
2. `labels.list`, then `labels.get` for the system and user labels in one batch (counts, FOL-2).
3. `settings.sendAs.list` for aliases (ACC-2). The name comes from the OpenID userinfo / `people/me` (ACC-1).
4. `threads.list labelIds=INBOX maxResults=100`, then `threads.get format=metadata` in batches of 50 (`metadataHeaders` = From, To, Cc, Bcc, Reply-To, Subject, Date, Message-ID, In-Reply-To, References; `fields=` to trim). This costs about 4,000 units, around 45 s on a fresh quota, and the first 50 rows show after roughly 20 s.
5. `drafts.list`, then build the Drafts threads. Content is fetched with `drafts.get format=raw` when a draft is opened.
6. `labelIds=SENT` and `STARRED`, first 50 each.
7. Backfill with `threads.list` (all mail, no spam or trash) page by page until 1,000 threads or 90 days, whichever comes first (Open question 2). Threads whose stored `history_id` equals the listed one are skipped.
8. Start partial sync from H0.

**Gmail partial sync (SYN-2).** Every 45 s while the window is active, on focus, and 2 s after a committed write:

- Call `history.list startHistoryId=H` and page through.
- Collect the touched thread IDs and refetch them with `threads.get format=metadata` (batched). A thread that answers 404 is removed.
- Re-derive the folder, store, and emit `ThreadsChanged` (LST-8). Then `labels.get` the touched labels (FOL-2, LST-6).
- Store the response's `historyId`.
- A 404 on `history.list` means H is too old. Re-run steps 2 and 4–7, skipping unchanged `history_id`s, then continue.
- If the open thread changed, the reader reloads it but keeps its expanded and focused state (STA-5).

**Bodies (RDR-6).**

- When a thread stays selected for 150 ms (so `j`/`k` key repeat doesn't spend quota), fetch each message without a body with `messages.get format=raw`, batched.
- Parse with `mail-parser`: charset, RFC 2047, `text/html` or `text/plain`, attachments, `text/calendar` UID and METHOD.
- Store in `bodies` and in `search_text`. The reader shows "Loading…" until then. Failures show "Couldn't load this message" with a Retry button (palette: "Retry Loading Message").

**Calendar sync (SYN-4).**

- `colors.get` once, cached. `calendarList.list` with a `syncToken` after the first run.
- For each calendar in the list: `events.list singleEvents=false maxResults=2500 timeZone=<local>`. The full sync also sends `timeMin=now−180 days`.
- Page to the end and store `nextSyncToken`. Later calls send `syncToken` only; Google forbids `timeMin`/`timeMax` together with a sync token.
- `status=cancelled` items delete events, or become `exceptions` on their series.
- A 410 wipes that calendar's events and runs a full sync again.
- Polled every 2 min while active, on focus, and after writes.
- **Recurrence (CAL-5):** series are stored with their RRULE and expanded locally by `recurrence::occurrences()`. RRULEs outside the supported subset (`FREQ`, `INTERVAL`, `BYDAY` with ordinals, `BYMONTHDAY`, `COUNT`, `UNTIL`; plus `EXDATE`) are flagged. For those, `events.instances` (next 400 days) is stored as concrete occurrences.

**Contacts sync (SYN-6).**

- `people.connections.list personFields=names,emailAddresses requestSyncToken=true` and `otherContacts.list readMask=names,emailAddresses requestSyncToken=true`, then incremental with the tokens.
- Polled every 30 min and on launch. An expired token triggers a full list.
- Ranking: contacts first, then other contacts, ordered by how often the address appears in cached mail. The existing `ContactBook::suggest()` ranking and `is_automated()` filter stay.

### 2.7 Write queue, optimistic updates, rollback

**Every op** becomes a `outbox` row, written in the same transaction as the optimistic change to the store (PER-2).

**Delay and undo.**

- `not_before` is `now + 5 s` (`TOAST_DURATION`) for ops the UI can undo: Send (UND-1), DeleteDraft (UND-2), Modify or Trash from Archive / Delete / Move to (UND-3), and calendar Delete (CAL-10).
- Undo calls `Backend::cancel(op)`: the row is deleted, the store change reverted, and the UI restores its state as today.
- If the row was already in flight, Undo sends the inverse instead (for example `untrash` + `addLabelIds INBOX`). A send that already went out shows "Already sent".
- Read, star and label changes commit at once.

**The worker.**

- Picks due rows in order per resource: per thread ID, per event ID, and per draft local ID. Sends are serialised.
- Coalesces: consecutive Modifies of one thread merge, and a SaveDraft replaces an older pending SaveDraft of the same draft.
- Bulk ops (ACT-6) go out as one HTTP batch of `threads.modify` / `threads.trash` / `threads.untrash`, up to 50 per batch. A partial failure rolls back only the failed threads.

**Pending-op overlay.** When a sync refetches a thread or event that still has pending ops, those ops are re-applied before storing. Otherwise a poll in the 5 s undo window would bring an archived thread back.

**Commit results.**

- Send: the server's `id` and `threadId` replace `sent-local-*` (`IdReplaced`).
- SaveDraft: the server `draft.id` is stored in `drafts`.
- Insert and Patch: the new `etag` is stored.

**Retries and failure.**

| Response | What happens |
| --- | --- |
| Network error, 429, 5xx, 403 `rateLimitExceeded` / `userRateLimitExceeded` | Truncated exponential backoff: 1, 2, 4 … 64 s plus up to 1 s jitter, per Google's guidance. At most 8 attempts, then failed. |
| 401 | One token refresh, then retry. |
| 412 (event ETag) | Refetch the event, store the server version, and show the toast "Changed elsewhere; reloaded". The op is dropped (STA-5). |
| Other 4xx | Failed at once. |

**Rollback (STA-2).** A failed op refetches the server's state for its IDs, stores it, emits the change, and shows a toast with the reason ("Couldn't archive 2 conversations: …").

- Failed sends become drafts again in Drafts, with the toast "Not sent: <reason>" (CMP-12).
- Failed draft saves keep the local text and retry later. A draft's text is never discarded.

**Quit.** Pending sends inside their undo window are committed at once on Quit. `Quit` waits up to 5 s for in-flight rows, then quits; anything left is flushed on the next launch. Other pending rows stay queued for the next launch.

### 2.8 Auth and tokens

- `google_client.json`: read from App Support at runtime. Only the `installed` key is used. The client secret is never logged and never committed.
- **Sign-in (ACC-3):**
  1. Bind `127.0.0.1:0`.
  2. Build the auth URL with `code_challenge` = S256 of a 64-byte random verifier (`SecRandomCopyBytes`), a random `state`, the scopes, `access_type=offline`, `prompt=consent`, `include_granted_scopes=true`.
  3. Open it in the browser. The sign-in screen shows "Waiting for your browser… (Esc cancels)".
  4. Accept one request whose `state` matches, reply with a small "You can close this tab" page, and stop listening.
  5. Exchange the code at `token_uri`.
  6. Read `scope` (the granted scopes, ACC-6) and `id_token` (email and name).
  7. Store the refresh token in the Keychain (generic password, service `Pigeon Google`, account = email) and `{email, name, scopes}` in `accounts.json`.
  8. The listener times out after 5 min.
- **Startup:** `accounts.json` + Keychain item → Google mode, with no sign-in. Neither → the sign-in screen. `PIGEON_DEMO=1` → demo mode, whatever is signed in.
- **Access tokens** are kept in memory with their expiry. A refresh starts 60 s before expiry; if several requests need it at once, only one refresh runs and the rest wait for its result. `invalid_grant` puts the app in the "Sign in again" state (STA-4); the cache and outbox are kept, and signing in again resumes them.
- **Sign out (ACC-7):** revoke at `https://oauth2.googleapis.com/revoke`, delete the Keychain item, the `accounts.json` entry and the account's rows, then show the sign-in screen.
- **Granted scopes:** a feature whose scope wasn't granted is hidden or shows an empty state. For example, Calendar without `calendar` shows "Calendar access wasn't granted. Sign in again to allow it." (CAL-1).

### 2.9 Errors, offline, rate limits

- **Offline.** Two consecutive network failures put the status at `Offline`. Polls pause and back off (15 s → 2 min), and the outbox waits. Reads come from the cache. Uncached bodies show "Offline: this message isn't downloaded yet". The status line shows "Offline · 3 changes waiting" (STA-3).
- **Rate limits.** Handled by the limiter (2.6) and backoff (2.7). A long backoff shows in the status line ("Waiting for Gmail…").
- **Errors** reach the user only as toasts or the status line, in plain words ("Gmail didn't accept the message: invalid recipient"). Raw JSON is never shown.

### 2.10 Security

- Never in logs, panics, `Debug` output, test output or the repo: access or refresh tokens, `Authorization` headers, the client secret, authorization codes, message bodies, or subjects and addresses beyond what the UI shows.
- `http.rs` logs only `METHOD host/path status ms`. Query strings are redacted. `Debug` on token types prints `***`.
- `google_client.json`, `accounts.json` and `pigeon.sqlite` live outside the repo. App Support files are 0600 and the folder 0700.
- Fixtures under `tests/fixtures/google/` are synthetic: example.com addresses and made-up IDs. Nothing recorded from the real account is committed.
- Remote images are never fetched without the user's choice (ATT-1). Fetches carry no cookies and no referer.
- The loopback listener binds 127.0.0.1 only, checks `state`, and serves one request.
- `cargo test` never touches the network, the Keychain or App Support. It uses `FakeTransport`, `Store::open_in_memory()` and a `TokenStore` fake.

### 2.11 Front-end changes

| Where | Change | Items |
| --- | --- | --- |
| `src/ui/onboarding.rs` (new) | First run: app name, "Sign in with Google" (Enter), "Try with demo data" (`d`), waiting state with Cancel (Esc), and errors (no `google_client.json`, consent denied, timeout). Focused and keyboard-only like the rest of the app. | ACC-3 |
| `src/ui/status.rs` (new) | Status line under the account card in the sidebar: "Synced just now" / "Syncing 340 of 1,000" / "Offline · 3 changes waiting" / "Sign in again" (button). The same text appears in Settings › Accounts. | ACC-5, STA-1/3/4 |
| `src/ui/thread_list.rs` | Skeleton rows while a view's first page loads. "Nothing in …" only once the view is loaded. "Load older" row at the end (Enter, or reaching it with `j`). "About N results" plus a "Searching Gmail…" line. | STA-1, LST-9, LST-7, SRC-6/7 |
| `src/ui/reader.rs` | Loading state per message, Retry, read-only invite card when the event isn't found. "Load images" bar (ATT-1). | RDR-6, CAL-20, ATT-1 |
| `src/ui/sidebar.rs` | Server counts. The account card from the real profile. | FOL-2, ACC-1 |
| `src/app.rs` `render_toast()` | Error toasts (no Undo, a warning tone from `theme.rs`). Send toast: "Sending… Undo", then "Message sent". | STA-2, UND-1, CMP-8/12 |
| `src/ui/calendar/event_panel.rs` | Read-only mode (fields not editable, RSVP still is). "Send updates to guests?" prompt (Send / Don't send / Cancel), reusing `SeriesPrompt`. | CAL-22, CAL-13 |
| `src/ui/settings.rs` | Accounts section: email, name, granted scopes, last sync, Sync Now, Sign Out. Mail section: "Load remote images: Ask / Always / Never". | ACC-5/7, ATT-1, PER-3 |
| `src/actions.rs` (W1 only) | New commands: Sign In, Sign Out, Sync Now (`cmd-r`), Retry Loading Message, Load Images (`shift-i` is taken; use `cmd-shift-i`), Load Older Conversations, Use Demo Data. All in `COMMANDS` and ⌘K. | PAL-1 |

---

## 3. Workstreams

Each workstream is one `implementer`, in its own worktree and branch, owning the files listed. "Shared, append-only" means a workstream may add a variant or arm in a file another owns, but nothing else. All workstreams update their roadmap entries' **Today** and **Needs** text (status stays 🟡; see 6.2).

### W1 Foundation (Phase 0)

- **Goal:** land every shared seam, so later phases work in parallel without touching hot files. No network yet.
- **Items:** DAT-1, DAT-2 (mapping function and tests), DAT-3, DAT-5, DAT-6. Prepares PER-1, PER-2, PER-3.
- **Owns:**
  - `Cargo.toml`, `src/main.rs`
  - `src/backend/mod.rs`, `store/mod.rs`, `store/schema.sql`, `store/mail.rs`, `store/calendar.rs`, `store/outbox.rs`, `demo.rs`
  - `http.rs` (the `Transport` trait, `Request`/`Response`, `FakeTransport`, batch encode/decode; no ureq yet)
  - `src/data.rs`, `src/calendar/mod.rs`, `src/calendar/recurrence.rs` (`Rule` replacing `Recurrence`, keywords only)
  - `src/actions.rs`, `src/settings.rs`
  - New `src/contacts.rs` and `src/app/compose.rs` (pure moves)
  - `src/app.rs` and every UI file whose types change, mechanically (`Message::html` → `body`, `Recurrence` → `Rule`)
- **Work:**
  - The model changes from 2.4. The store with the full schema. Demo mode through the store.
  - `PigeonApp` gets `Entity<Backend>`. Handlers emit `MailOp`/`CalOp`, which demo mode commits at once.
  - The stub actions from 2.11, with handlers that do nothing outside demo.
  - Settings saved to and loaded from the `settings` table, appearance applied before the first frame (PER-3).
  - The two pure-move refactors.
- **Acceptance:**
  - `PIGEON_DEMO=1 cargo run --release` (and plain `cargo run` with no account; until W2, it falls back to demo) behaves exactly as before: same 124 threads, same 167 events, every action works.
  - All existing tests pass; their only changes are type renames.
- **Unit tests:**
  - Store round trip (thread → rows → thread).
  - `derive_folder` table.
  - Demo seed counts.
  - Outbox delay, cancel and coalesce.
  - Batch request encode and multipart response decode (fixture).
  - `Rule` keyword labels.

### W2 Auth and HTTP (Phase 1)

- **Goal:** a signed-in, authenticated transport with backoff, a quota limiter and a status line.
- **Items:** ACC-1. Pulled in: ACC-3, ACC-5 (basic), ACC-6, ACC-7, SYN-7, STA-4.
- **Owns:** `src/backend/auth.rs`, `http.rs` (the `UreqTransport` half, limiter, backoff), `src/ui/onboarding.rs`, `src/ui/status.rs`.
- **Shared, append-only:** `backend/mod.rs` (`sign_in`/`sign_out` bodies, `Status`).
- **Depends on:** W1.
- **APIs:**
  - OAuth: `accounts.google.com/o/oauth2/v2/auth`, `oauth2.googleapis.com/token`, `/revoke`
  - Keychain generic password
  - `people/me?personFields=names,emailAddresses` (or the `id_token` claims)
- **Acceptance:**
  - With the existing grant (see Open question 1), launch shows the user's real name and email and no sign-in screen.
  - Without one, the sign-in screen appears; signing in in the browser lands in the app.
  - Sign Out returns to the sign-in screen and removes the Keychain item.
  - A revoked token shows "Sign in again".
  - No token appears in any output (grep the test log for `ya29.` and `1//`).
- **Unit tests:**
  - PKCE challenge against RFC 7636's test vector.
  - Auth URL parameters.
  - Loopback request parsing: right state, wrong state, `error=access_denied`.
  - Token response parsing, including granted `scope`.
  - Refresh single-flight.
  - 401 → refresh → retry.
  - Backoff schedule on 429 and 503 (fake clock).
  - Limiter accounting.
  - Redaction of `Debug` and log lines.

### W3 Gmail read (Phase 1)

- **Goal:** real mail in the list and the reader, kept current.
- **Items:** see 1.1. Pulled in: SYN-1, SYN-2, RDR-6, LST-8, STA-1 (list and reader parts).
- **Owns:**
  - `src/backend/gmail.rs`, `mime.rs` (parse half), `sync.rs` (mail loop and scheduler skeleton)
  - `store/mail.rs` reads and writes beyond the W1 basics
  - `src/ui/thread_list.rs`, `src/ui/reader.rs`, `src/ui/sidebar.rs`
- **Shared, append-only:** `app.rs` (the `BackendEvent` mail arms in the subscription).
- **Depends on:** W1 (`FakeTransport` is enough for development). Merges after W2.
- **APIs:** `getProfile`, `labels.list`, `labels.get`, `threads.list`, `threads.get` (`metadata`), `messages.get` (`raw`), `history.list`, `drafts.list`.
- **Acceptance:**
  - The Inbox shows the account's recent threads with correct sender, subject, snippet, date, unread, star and labels. Counts match Gmail's labels (FOL-2).
  - Opening a thread shows every message, with bodies rendered by `html.rs`.
  - A mail sent to the account while Pigeon runs appears within 60 s, and selection doesn't jump.
  - Restarting shows the cached list at once.
  - A stale `historyId` (forced in a test) resyncs.
- **Unit tests:**
  - Metadata thread fixture → `Thread` (folder, flags, headers).
  - History fixture → touched IDs.
  - 404 history → resync path.
  - Raw `.eml` fixtures:
    - ISO-8859-1 body
    - RFC 2047 subject
    - `multipart/alternative`
    - attachments
    - `text/calendar` REQUEST and CANCEL
  - Skip unchanged `history_id`.

### W4 Calendar read (Phase 1)

- **Goal:** real calendars and events, recurrence and zones right, kept current.
- **Items:** see 1.1. Pulled in: SYN-4.
- **Owns:**
  - `src/backend/calendar.rs` (read half), `store/calendar.rs` beyond W1
  - `src/calendar/recurrence.rs`, `src/calendar/time.rs` (if needed)
  - `src/ui/calendar/header.rs` (the calendars menu)
- **Shared, append-only:** `sync.rs` (the calendar loop function, in its own block), `app.rs` (calendar event arms).
- **Depends on:** W1. Merges after W3.
- **APIs:** `calendarList.list`, `colors.get`, `events.list` (`syncToken`, `timeMin`, `timeZone`), `events.instances`, `events.list iCalUID=`.
- **Acceptance:**
  - Week view matches Google Calendar for this week and the next: titles, times, all-day, colours, recurring series, moved and cancelled occurrences.
  - Hidden calendars stay hidden.
  - Changes made elsewhere appear within about 2 min, or on focus.
  - A 410 resyncs.
  - DST: a weekly series in the user's zone stays at the same wall time across the change.
- **Unit tests:**
  - RRULE parse and expand: `WEEKLY;BYDAY=MO,WE`, `MONTHLY;BYDAY=2TU`, `MONTHLY;BYDAY=-1FR`, `COUNT`, `UNTIL`, `EXDATE`, `INTERVAL`.
  - Unsupported-rule flag.
  - Event JSON → `Event` (timed with offset, all-day exclusive end, cancelled instance → exception, modified instance).
  - Sync-token paging.
  - 410 handling.

### W5 Mail actions (Phase 2)

- **Goal:** every thread action commits to Gmail, with undo and rollback.
- **Items:** see 1.1. Pulled in: STA-2, STA-3 (queue side), PER-2 (worker).
- **Owns:** `src/backend/outbox.rs` (worker, commit, rollback, overlay), `src/app/mail.rs`.
- **Shared, append-only:** `gmail.rs` (`modify`/`trash`/`untrash` call functions in a new `// writes` block).
- **Depends on:** Phase 1.
- **APIs:** `threads.modify`, `threads.trash`, `threads.untrash`, HTTP batch.
- **Mapping:**

  | Action | Gmail call |
  | --- | --- |
  | Archive | remove `INBOX` |
  | Move to Inbox | add `INBOX`, and untrash first when in Trash |
  | Spam | add `SPAM`, remove `INBOX` |
  | Not spam | remove `SPAM`, add `INBOX` |
  | Delete | `trash` |
  | Read / unread | `UNREAD` on the whole thread (`threads.modify`; Open question 7) |
  | Star | `STARRED` on the thread |
  | Labels | add or remove the label ID |
- **Acceptance:**
  - Each action is reflected in Gmail within seconds; Undo within 5 s means Gmail never changes.
  - A forced failure rolls back with a toast.
  - Offline actions queue and apply on reconnect.
  - Quitting with queued actions applies them on the next launch.
- **Unit tests:**
  - Op → request mapping for each action.
  - Batch partial failure → rollback of only the failed IDs.
  - Overlay keeps an archived thread out of the Inbox during a poll.
  - Undo before commit → no request.
  - Undo after commit → inverse request.
  - Backoff then fail → rollback.

### W6 Compose, drafts and send (Phase 2)

- **Goal:** real sending, drafts that live in Gmail, undo send.
- **Items:** see 1.1. Pulled in: DRF-6, DAT-4, CMP-12.
- **Owns:** `src/app/compose.rs`, `src/ui/composer.rs`, `src/backend/mime.rs` (build half; new `fn build_*`), `src/markdown.rs` (if the plain part needs it).
- **Shared, append-only:** `gmail.rs` (drafts and send calls).
- **Depends on:** Phase 1.
- **APIs:** `messages.send` (with `threadId`; the upload endpoint `uploadType=multipart` above 5 MB), `drafts.create`, `drafts.update`, `drafts.get` (`raw`), `drafts.send`, `drafts.delete`.
- **MIME:**
  - Structure: `multipart/mixed` (only with attachments) holding `multipart/alternative` (`text/plain` = the markdown as typed; `text/html` = `markdown::to_html()`), then attachments read from disk at send time.
  - Headers: `From` (primary address and name), `To`, `Cc`, `Bcc`, `Subject` (RFC 2047), `Date`, `Message-ID`.
  - Replies: `In-Reply-To` and `References` from the thread's last message, plus `threadId` (CMP-2/3/11). Reply All honours `Reply-To`.
  - Forward: the quote as a `<blockquote>` in the HTML (CMP-10), no `threadId`.
  - Drafts add `X-Pigeon-Markdown: 1` so DRF-3 reopens with the markdown. Sends don't carry it.
  - Over 25 MB, the composer refuses ("Attachments are over Gmail's 25 MB limit").
- **Flow:**
  - Save on close: `SaveDraft`, then `drafts.create` and later `drafts.update`.
  - Send: `Send` with a 5 s delay; a saved draft goes through `drafts.send`, otherwise `messages.send`.
  - The sent copy comes back through history; the optimistic message is matched by `Message-ID` and replaced (CMP-13).
  - Discard: delayed `drafts.delete`.
- **Acceptance:**
  - A new message, a reply, a reply-all and a forward to the user's own address arrive correctly: threaded in Gmail web, formatting intact, attachments open.
  - Undo inside 5 s means nothing is sent.
  - Drafts appear in Gmail's Drafts, edits sync, and a draft written in Gmail web appears in Pigeon.
  - Discard and its Undo work.
  - An invalid recipient rejected by Gmail returns the message to Drafts with the reason.
- **Unit tests:**
  - MIME snapshots (plain, HTML, attachment, RFC 2047, reply headers).
  - `References` chain building.
  - Draft round trip with markdown recovery.
  - Send failure → back to Drafts.
  - `IdReplaced` on send.

### W7 Calendar writes (Phase 2)

- **Goal:** every calendar edit commits, with ETags, series scopes and guest updates.
- **Items:** see 1.1. Pulled in: CAL-22, STA-5 (events).
- **Owns:** `src/ui/calendar/mod.rs`, `event_panel.rs`, `drag.rs`, `week_view.rs`, `month_view.rs`, `agenda.rs`, `mini_month.rs`.
- **Shared, append-only:** `backend/calendar.rs` (writes block), `outbox.rs` (`CalOp` commit arm).
- **Depends on:** Phase 1.
- **APIs:** `events.insert` (client ID = UUID hex, CAL-11), `events.patch` with `If-Match`, `events.delete`, `events.move`, `events.instances`, `calendarList.patch` (`selected`); `sendUpdates` throughout.
- **Rules:**
  - Panel edits are debounced: one patch on close or after 1.5 s idle, carrying only the changed fields.
  - Attendees are written only if `!attendees_omitted`.
  - Reminders: `overrides=[{popup, m}]`; "None" = `useDefault=false` with no overrides.
  - Busy/free → `transparency`.
  - All-day → `start.date` / `end.date`.
  - Timed → `dateTime` + `timeZone` (the event's zone, else the local IANA zone).
- **Series (CAL-6):**
  - This event: patch or delete the instance, found with `events.instances` around `original_start`.
  - Following: patch the series' RRULE `UNTIL`.
  - All: patch the series.
- **Guests:**
  - When an edit touches an event that has guests other than the user, ask "Send updates to guests?"; the answer maps to `sendUpdates` (`all` or `none`).
  - RSVP patches only the `self` attendee's `responseStatus` (CAL-14), debounced.
  - Deleting someone else's event = decline + hide (CAL-22).
- **Acceptance:**
  - Create, drag, resize, edit every field, delete, undo, move to another calendar: all reflected in Google Calendar.
  - "This / following / all" on a test series is right in Google.
  - Guests receive invites only when chosen.
  - An event changed elsewhere and then edited here reloads with a toast.
  - Read-only events can't be edited.
- **Unit tests:**
  - `Event` → patch body diff.
  - `sendUpdates` choice.
  - Instance lookup.
  - `UNTIL` rewrite.
  - 412 handling.
  - Read-only rules table.

### W8 Contacts and identity (Phase 2)

- **Goal:** real recipient suggestions and aliases.
- **Items:** ACC-2, CON-1, CON-2. Pulled in: SYN-6.
- **Owns:** `src/contacts.rs`, `src/backend/people.rs`, `store/contacts.rs`.
- **Shared, append-only:** `sync.rs` (contacts loop function).
- **Depends on:** Phase 1.
- **APIs:** `people.connections.list`, `otherContacts.list` (sync tokens), `settings.sendAs.list` (fetched in W3; W8 uses the stored aliases).
- **Rules:**
  - New recipients from sends join the book at once.
  - `ContactsChanged` updates open composers.
  - CON-2: an unmatched name before a comma stays as text with the error "No contact named “x”" instead of being dropped.
- **Acceptance:**
  - Typing a known contact's name or address suggests them, ranked as before.
  - Aliases count as "me" in the reader and in reply recipients.
  - Someone just written to is suggested in the next composer.
- **Unit tests:**
  - Connections and otherContacts fixture → contacts.
  - Sync-token expiry.
  - Ranking with frequency.
  - Alias `is_me`.

### W9 Search and paging (Phase 3)

- **Goal:** search over all of Gmail, not just the cache; older mail on demand.
- **Items:** SRC-1 … SRC-8, PAL-2. Pulled in: LST-7.
- **Owns:** `src/search.rs` (`Query::to_gmail_q()`), `src/ui/command_palette.rs`.
- **Shared, append-only:** `app/mail.rs` (search functions; W5 has merged by then), `backend/mod.rs` (`search`, `load_older`).
- **APIs:** `threads.list q=… maxResults=25`, then `threads.get metadata` for unknown IDs; `pageToken`; `resultSizeEstimate`.
- **Rules:**
  - Local results show instantly. The server query is debounced 300 ms, and stale responses are dropped (SRC-8).
  - Results merge newest first. "About N results" when the server estimate is larger (SRC-6).
  - Scope mapping (SRC-2/4): `in:archive` → `-in:inbox -in:spam -in:trash -in:drafts`; `in:anywhere` → `includeSpamTrash=true`; `label:` → the Gmail label name, quoted.
  - The palette searches the local cache only, which is instant and capped at 30.
- **Acceptance:**
  - A search for a word that only appears in a two-year-old mail finds it.
  - Operators behave as in Gmail.
  - Scrolling to the end of a label view loads older threads.
- **Unit tests:**
  - `to_gmail_q` table for every operator and scope.
  - Debounce and staleness.
  - Merge and dedupe.

### W10 Reader extras (Phase 3)

- **Goal:** invites tied to real events, remote images on request.
- **Items:** CAL-20, ATT-1.
- **Owns:** `src/ui/invite.rs`, `src/html.rs`, `src/ui/html_view.rs`.
- **Shared, append-only:** `store/` (the `images` table already exists).
- **APIs:** `events.list iCalUID=` (fallback when not cached). Plain HTTPS GET for images through `ureq` (no cookies, 10 MB cap, image MIME only).
- **Rules:**
  - Invites: the `text/calendar` UID from W3 resolves to the cached event by `ical_uid`. Answers go through W7's `CalOp::Rsvp`. "Remove from calendar" deletes only the user's copy, without notifying.
  - Images: `Block::Image` keeps `src`. "Load images" per message, or always per setting. Tracking pixels stay dropped. Decoded through GPUI's image support.
- **Acceptance:**
  - A real invitation shows its card; Yes, Maybe and No reach the organizer.
  - Images load only when asked, and are cached.
- **Unit tests:**
  - ICS UID/METHOD extraction.
  - `src` kept, and the `is_safe_url` image rules.
  - Image cache cap.

### W11 States and settings (Phase 3)

- **Goal:** loading, offline and error states finished; Settings › Accounts working.
- **Items:** pulled in: ACC-5 (full), STA-1 (rest), STA-3 (indicator), STA-5 (open thread), PER-3 (rest).
- **Owns:** `src/ui/settings.rs`, `src/ui/status.rs` (after W2), `src/settings.rs`.
- **Shared, append-only:** `app.rs` (`render_toast()` tone, Quit flush).
- **Acceptance:**
  - Pulling the network shows Offline within about 10 s; actions queue; reconnecting drains the queue.
  - Settings persist across launches.
  - Sign Out from Settings works.
- **Unit tests:**
  - Status state machine (Syncing → Synced → Offline → Synced; AuthExpired).
  - Settings round trip.

---

## 4. Execution order

```
Phase 0  W1 foundation ───────────────► merge ─► T0 (demo)
Phase 1  W2 auth-http ─┐
         W3 gmail-read ├─ parallel ───► merge W2, W3, W4 ─► T1a (mail read), T1b (calendar read) ─► fixes
         W4 cal-read  ─┘
Phase 2  W8 contacts  ─┐
         W5 mail-acts  ├─ parallel ───► merge W8, W5, W6, W7 ─► T2a, T2b, T2c ─► fixes
         W6 compose    │
         W7 cal-writes ┘
Phase 3  W11 states   ─┐
         W9 search     ├─ parallel ───► merge W11, W9, W10 ─► T3a, T3b ─► fixes
         W10 reader-x ─┘
Phase 4  hardening: T4 full sweep (real + demo) ─► fixes ─► re-test ─► roadmap to 🟢
```

### 4.1 Agents per phase

| Phase | Implementers (Opus 5.5) | Tester runs (GPT-6.1 Sol) | Fix implementers |
| --- | --- | --- | --- |
| 0 | 1 (W1) | T0 | 0–1 |
| 1 | 3 (W2, W3, W4) | T1a, T1b | 1–2 |
| 2 | 4 (W5, W6, W7, W8) | T2a, T2b, T2c | 1–3 |
| 3 | 3 (W9, W10, W11) | T3a, T3b | 1–2 |
| 4 | 0 | T4a (real), T4b (demo + regressions) | 1–2 |

A fix implementer is always fresh. It gets the tester's report and owns only the files the report points to. A re-test runs only the failed items plus one smoke item. No agent implements, tests and fixes in a loop. Subagents never commit; the main agent reviews, merges and commits.

### 4.2 Worktrees, branches, merge order

- **Worktrees:** `/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/pigeon-bs1-<name>`, branch `bs1/<name>`, created from the current `main`.
- **Builds:** `CARGO_TARGET_DIR=/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/target-bs1-<name>`.

| Phase | Branch | Merge order | Reserved schema block |
| --- | --- | --- | --- |
| 0 | `bs1/foundation` | 1 | 001 (all tables) |
| 1 | `bs1/auth-http` | 1 | none |
| 1 | `bs1/gmail-read` | 2 | 010 |
| 1 | `bs1/calendar-read` | 3 | 020 |
| 2 | `bs1/contacts-identity` | 1 | 030 |
| 2 | `bs1/mail-actions` | 2 | 040 |
| 2 | `bs1/compose-drafts` | 3 | 050 |
| 2 | `bs1/calendar-writes` | 4 | 060 |
| 3 | `bs1/states-settings` | 1 | 070 |
| 3 | `bs1/search-paging` | 2 | 080 |
| 3 | `bs1/reader-extras` | 3 | 090 |
| fixes | `bs1/fix-<phase>-<n>` | after the phase | none |

**Before each merge,** the main agent runs `cargo fmt --check`, `cargo build`, `cargo clippy --all-targets` (no new warnings) and `cargo test` on the branch rebased onto `main`.

**After each merge,** the main agent:

- runs the same checks again on `main`
- updates `docs/ROADMAP.md`: the entries' text, statuses per 6.2 and the counts
- commits

**Testers** work on a detached snapshot: `git worktree add --detach …/pigeon-test-<phase> <main-sha>`, with their own target dir.

### 4.3 Conflict hot spots

| File | Risk | Rule |
| --- | --- | --- |
| `Cargo.toml` | Every workstream needs crates | W1 adds all of them (2.1). Later workstreams may not edit it; a missing crate goes through the main agent. |
| `src/actions.rs` | New commands | W1 adds every new action and `COMMANDS` row. Later workstreams only fill handlers in files they own. |
| `src/app.rs` | 2,000 lines, everything routes through it | W1 moves the compose, draft, send and undo handlers to `src/app/compose.rs` and adds the `BackendEvent` subscription with one arm per variant calling `on_mail_changed`, `on_calendar_changed`, … in other files. Later workstreams change only their own arm, or the toast and quit code in W11. |
| `src/backend/mod.rs` | Shared API | W1 writes every signature, with `todo!()`-free demo bodies. Later workstreams fill bodies of their functions only. |
| `src/backend/gmail.rs`, `calendar.rs`, `sync.rs` | Read and write workstreams in different phases | Different phases, so no parallel edits; within a phase, separate marked blocks. |
| `store/schema.sql` | Migrations | Reserved block numbers (4.2). |
| `src/ui/composer.rs` | ContactBook (W8) and compose (W6) in one file | W1 moves `ContactBook` to `src/contacts.rs`. |
| `docs/ROADMAP.md` | Every branch edits it | Implementers edit only their items' entries; only the main agent changes statuses and the counts table. |

---

## 5. Testing

### 5.1 Unit tests (`cargo test`, no network)

- **`FakeTransport`** (W1, `http.rs`) matches `(method, path, key query params)` to a fixture file and a status. It records requests so tests can assert bodies and headers, and can fail with a scripted status sequence (`[503, 503, 200]`).
- **Fixtures** live in `tests/fixtures/google/`, are written by hand from Google's documented shapes, and use example.com addresses. They are loaded with `include_str!` from the module tests; the crate is a binary, so tests stay in modules.
- **Stores** are `Store::open_in_memory()` with the demo seed or a minimal seed.
- **Each workstream** lists its tests in section 3. Every bug a tester finds gets a regression unit test where it can be expressed without the GUI.
- **Demo mode** stays covered by the existing tests (`bundled_calendar_loads`, composer and search tests), adapted to the new types, plus a seed-count test.

### 5.2 Safety rules for testing on the real account

These go at the top of every tester prompt.

1. Drive only Pigeon (its pid and window) with Cua Driver. Never touch the browser, the Keychain dialog, System Settings or other apps. Sign-in in the browser is done by the user (or skipped, using the stored grant). If a system dialog appears, stop and report.
2. Mail: send only to the user's own address, with subjects that start with `[Pigeon test]`. Archive, delete, label, star, spam, mark and draft only threads whose subject starts with `[Pigeon test]`. Never empty Trash; never permanently delete.
3. Labels: use only the `Pigeon Test` label (made by the user beforehand; Open question 6).
4. Calendar: create, edit and delete only on the `Pigeon Test` calendar, with titles that start with `[Pigeon test]`. Guests: only the user's own second address (Open question 6) or none. Never change or RSVP real events, except a test invite sent for that purpose.
5. Don't change Google settings, contacts or calendars' sharing. Don't sign out unless the checklist item says so, and the main agent has confirmed the user can sign back in.
6. Test dark mode only in a scratch copy, never the system setting (AGENTS.md).
7. At the end of each run, list every test thread, label change and event created, so the main agent can confirm cleanup.

### 5.3 Tester checklists

Each run is 5–7 items against a release build of the snapshot (`cargo run --release`), and reports pass or fail with repro steps and screenshots. "Verify in Gmail/Calendar" means the main agent checks through the API or asks the user. The tester only looks at Pigeon, including after a relaunch.

**T0: demo regression (after W1)**

1. `PIGEON_DEMO=1`: the Inbox lists the placeholder threads; opening one renders its body; `j`/`k` and `n`/`p` work.
2. Archive a thread with `e`, Undo with `z`, star with `s`, label with `l`: all as before.
3. Compose, type formatted text, Esc → "Saved to Drafts"; reopen from Drafts with its formatting; send and Undo.
4. Search `from:sarah in:all` and `has:attachment` give results; Esc clears.
5. Calendar: week view shows events; drag an event; delete it and Undo; a recurring event asks "This / All".
6. Change Appearance to Dark in Settings, quit, relaunch: the setting is kept (PER-3). Set it back.

**T1a: sign-in and mail read (after W2 + W3)**

1. Launch without `PIGEON_DEMO`: no sign-in screen; the sidebar shows the user's real name and email; the status goes "Syncing…" → "Synced just now". (ACC-1, ACC-5)
2. The Inbox shows real threads, newest first, with unread dots. The list header's unread count equals the sidebar's Inbox count. (LST-1/2/3/6, FOL-2)
3. Open the newest thread: every message shows, the newest is expanded, older ones are collapsed with snippets; the header shows From, To and date; HTML renders. (RDR-2/3/4/5)
4. `g s`, `g d`, `g a`, `g !`, `g t` and one user label view each show plausible content; Drafts shows server drafts. (FOL-1/3/4, DRF-2)
5. Ask the user to send a mail titled `[Pigeon test] inbound` to the account. Within 60 s it appears at the top without moving the current selection. (SYN-2, LST-8)
6. Quit and relaunch: the list appears at once from the cache, before the status turns "Synced". (PER-1, STA-1)
7. In the sidebar, the status line is visible and readable in light mode. (ACC-5)

**T1b: calendar read (after W4)**

1. `cmd-2`: the week shows the real calendars' events with their colours; the calendars menu lists the real calendars and their visibility. (CAL-1/2/3)
2. A known recurring meeting shows on every expected day; a known moved or cancelled occurrence shows that way. (CAL-5)
3. Month and Day views and `t`/`n`/`p` navigation show events for the next 2 months without gaps. (CAL-4)
4. Go to Date "next fri" lands there; the palette finds an event by title. (CAL-4a, CAL-19)
5. An event with a Meet link shows Join; an all-day event sits in the all-day lane. (CAL-17, CAL-8)
6. Ask the user to edit a `[Pigeon test]` event in Google Calendar; it changes in Pigeon within 2 min, or on refocus. (SYN-4)

**T2a: mail actions (after W5)**

1. On a `[Pigeon test]` thread: archive (`e`); the main agent verifies it left the Inbox in Gmail. Move it back to the Inbox. (ACT-1)
2. Delete a `[Pigeon test]` thread and Undo within 5 s; the main agent verifies Gmail never trashed it. Delete another and let it commit. (ACT-2, UND-3)
3. Star, mark unread, mark read, and apply and remove `Pigeon Test` on a test thread; each shows in Gmail. (ACT-3/4/5, FOL-5)
4. Select three `[Pigeon test]` threads with `x`/`shift-j` and archive all; all three are archived in Gmail. (ACT-6)
5. Move a test thread to Spam, then archive it out of Spam; it ends in the Inbox in Gmail. (FOL-7, ACT-1a)
6. Opening an unread test thread marks it read in Gmail. (RDR-1)

**T2b: compose and drafts (after W6, W8)**

1. New message to the user's own address, subject `[Pigeon test] new`, with bold, a list and a link, plus one small attachment; send. "Sending…" then "Message sent"; it arrives in the Inbox with formatting and attachment. (CMP-1/7/8, ATT-6)
2. Reply and Reply All to it; both land in the same thread in Pigeon, and the main agent confirms Gmail threads them. (CMP-2/3/11, CMP-13)
3. Forward it; it arrives as a new thread with the quote block. (CMP-4/10)
4. Send another and Undo within 5 s: it doesn't arrive (check after 60 s), and the draft reopens. (UND-1)
5. Write a draft, Esc; the main agent sees it in Gmail Drafts; reopen it in Pigeon with its formatting; discard and Undo; discard again. (DRF-1/3/4, UND-2)
6. In To, type the first letters of a frequent contact; they're suggested first; the Cc/Bcc toggle and validation errors work. (CON-1/2, CMP-5/6/9)

**T2c: calendar writes (after W7)**

1. In the `Pigeon Test` calendar, drag to create `[Pigeon test] A`, set location, reminder 30 min and Free; the main agent verifies it in Google. (CAL-9/21/15/16/11)
2. Drag it to another day and resize it; Google shows the new time. Toggle all-day and back. (CAL-21, CAL-8, CAL-12)
3. Create a weekly test series (from Google, Open question 6); edit "This event", then "All events"; delete "This and following"; Google matches. (CAL-6, CAL-5)
4. Move the test event to another calendar the user owns, then back. (CAL-40)
5. Add the user's second address as a guest and choose "Send"; the invite arrives at that address. Then remove the guest and choose "Don't send". (CAL-13)
6. Delete the test event and Undo; then delete it for real; Google shows it gone. A read-only event (holidays) can't be edited. (CAL-10, CAL-22)

**T3a: search and paging (after W9)**

1. Search a word from a mail older than the sync window: "Searching Gmail…", then it appears. (SRC-7, SRC-5)
2. `from:`, `to:`, `subject:`, `is:unread`, `has:attachment`, `label:"Pigeon Test"` each give Gmail-consistent results. (SRC-3)
3. `in:sent`, `in:anywhere`, the scope chip, and the default scope from Settings all behave. (SRC-2/4)
4. The count shows "about N" for large result sets; typing fast doesn't flicker old results. (SRC-6/8)
5. Scroll to the end of the Archive and a label view, then "Load older" adds older threads. (LST-7)
6. ⌘K finds a cached thread by subject. (PAL-2, PAL-1)

**T3b: states and reader extras (after W10, W11)**

1. Turn Wi-Fi off. That is a system change, so the user does it, or the main agent uses a firewall rule scoped to Pigeon (Open question 6). The status shows Offline; archive a test thread; it stays archived locally. Reconnect: it syncs, and the status returns to Synced. (STA-3)
2. Open a never-opened thread while offline: "isn't downloaded yet"; after reconnect, Retry loads it. (RDR-6, STA-1)
3. A real invitation in the Inbox shows its card; answering a `[Pigeon test]` invite sent from the second address reaches the organizer. (CAL-20, CAL-14)
4. A newsletter shows "Load images"; clicking it loads them; with Settings › Images = Never there's no bar. (ATT-1)
5. Settings › Accounts shows email, scopes and last sync; Sync Now works. (ACC-5, ACC-6)
6. A link in a message opens in the browser; the tooltip shows the URL. The tester doesn't interact with the browser. (LNK-1)

**T4a / T4b: hardening.** Pick the failed and flaky items from T0–T3, plus:

- a cold start with an empty cache (rename `pigeon.sqlite` in the snapshot's own App Support copy, or start on a fresh account directory set by `PIGEON_DATA_DIR`), timing until the Inbox shows
- demo mode end to end
- a Quit during the undo window, then a relaunch, to check the queued op

---

## 6. Done, roadmap rules, risks, open questions

### 6.1 Definition of done

- Every item in 1.1 is 🟢 per 6.2, or listed with a reason as staying 🟡, agreed with the user.
- Pulled-in items (1.2) are 🟢 or 🟡 to the extent listed.
- `PIGEON_DEMO=1` runs the full UI with no network, and `cargo test` passes with no network, Keychain or App Support access.
- `cargo fmt`, `cargo build`, `cargo clippy --all-targets` (no new warnings) and `cargo test` are clean on `main`.
- No secret in the repo, logs or test output (grep for `ya29.`, `1//`, `GOCSPX`, `client_secret` in tracked files and test logs).
- Cold start shows cached mail before the network answers. A first-ever sync shows the first Inbox rows within about 30 s.
- All test threads, labels and events are cleaned up, or listed for the user.

### 6.2 Roadmap update rules

- Implementers update their items' **Today**, **Needs** and **Notes** in the same branch, but don't change statuses or counts.
- The main agent sets an item to 🟢 only after:
  1. it is merged to `main`,
  2. a tester checklist item naming that ID passed on the real account, and
  3. demo mode still works for it.
  Items that need no backend (1.1) need only (1) and (2).
- Pulled-in ⚪ items go to 🟡 when merged and to 🟢 under the same rule.
- Every merge commit recomputes the Progress table counts and the "Current phase" paragraph in section 2. The paragraph changes to "Linked to Google; demo mode on placeholder data" when Phase 1 merges.
- New items for things learned in the sprint (for example a From picker, or prefetching bodies) are added as ⚪ with IDs in their area.

### 6.3 Risks

| Risk | Effect | Mitigation |
| --- | --- | --- |
| Keychain prompts on every rebuild: an unsigned or ad-hoc `cargo` binary changes its code signature each build, and the Keychain asks "Pigeon wants to use your confidential information". Testers may not click system dialogs. | Phase 1 testing blocked. | Open question 3: sign dev builds with one stable self-signed identity, or create the item with an access list that allows Pigeon's path. |
| Testing-mode OAuth client: refresh tokens expire after 7 days. | "Sign in again" weekly; the existing dev grant may already be stale. | STA-4 lands in Phase 1. The user re-signs when asked. Long term, ACC-8. |
| Gmail quota (6,000 units/min/user; `threads.get` = 40) | Slow first sync; `j`/`k` through many unread threads could exhaust the quota. | Limiter, first-page-first sync, 1,000-thread cap, 150 ms body debounce, batching. |
| Google billing for quota above thresholds "later in 2026" | Cost if we're wasteful | We stay far below the per-project threshold for one user. |
| The DAT-2 folder model doesn't match Gmail exactly | A thread with a reply but no `INBOX` shows as Archive; Sent only lists sent-only threads | Open question 5. |
| Local RRULE subset | Rare rules expand wrongly | Unsupported rules are flagged and use `events.instances`. |
| Wall-clock times | A series in another zone drifts by an hour across a DST change | Accepted for this sprint (Open question 8). |
| `ureq` blocking the background executor | Starved background tasks | ≤ 4 requests in flight. |
| Real-mailbox tests | Data loss or embarrassment | 5.2 rules; testers only touch `[Pigeon test]` items; never permanent delete. |
| `app.rs` merge conflicts | Lost time | W1 seams and moves (4.3). |

### 6.4 Open questions

Recommended option first in each.

1. **The dev sign-in.** Resolved: done on 2026-10-26 for `alex.rivera@example.com`, after this plan's first check. `accounts.json` and the Keychain item (`Pigeon Google`) exist with all 8 scopes; Gmail, Calendar and People all answer. W2 picks it up on first launch.
2. **The size of the first Gmail sync.** The quota changed in May 2026, and 90 days can mean several thousand threads (about 7 min per 1,000).
   - a. The Inbox first page first, then backfill up to 1,000 threads or 90 days, whichever is smaller. Older mail comes through "Load older" and server search.
   - b. 90 days with no cap.
   - c. 30 days.
3. **Keychain prompts during development.**
   - a. Sign every dev and test build with one stable self-signed code-signing identity (`codesign -s "Pigeon Dev"`, a post-build step in the run instructions), so the Keychain trusts it.
   - b. Create the Keychain item with an access list that allows any app (`-A`); less safe.
   - c. Accept prompts, and the user clicks "Always Allow" during tests.
4. **How contacts are fetched.** The decision says `searchContacts` / `otherContacts.search` with a warm-up. Google's search is prefix-only, needs the warm-up and costs a request per keystroke. Pigeon's `ContactBook` already ranks locally.
   - a. Sync `connections.list` + `otherContacts.list` with sync tokens into the cache, and search locally (SYN-6).
   - b. Live search with warm-up, as decided.
   - c. Both: the local cache, with live search only when it has no match.
5. **The Sent view and folders.**
   - a. Keep one folder per thread (2.4), but let the Sent view list every thread that holds a message the user sent, like Gmail (a small change in `nav_contains()`).
   - b. Strict one folder: Sent shows only threads where every message is from the user.
6. **Test fixtures in the real account.** Testers can't create these themselves.
   - a. The user creates, once, before Phase 2: a `Pigeon Test` Gmail label, a `Pigeon Test` calendar with one weekly `[Pigeon test] series` event, and provides a second address they own for guest, invite and inbound tests. For T3b, the user toggles Wi-Fi when asked.
   - b. The main agent creates the label, calendar and series through the API with the stored grant, and the user only provides the second address.
   - c. Skip the guest, invite and offline items, and leave CAL-13, CAL-14, CAL-20 and STA-3 at 🟡.
7. **Mark as unread.**
   - a. The whole thread (`threads.modify`), which matches the thread-level dot.
   - b. Only the last message (`messages.modify`), like Gmail web inside a conversation.
8. **Time zones.**
   - a. Keep wall-clock times in the local zone this sprint: read with `timeZone=<local>`, write with the event's zone, and accept the DST drift for series in other zones.
   - b. Add `chrono-tz` (a new, fairly large crate with the tz database) and expand series in their own zone.
9. **Editing video links (CAL-17).**
   - a. The Join link is read-only when Google has `conferenceData`/`hangoutLink`. For other events, the field saves into `location` when that's empty, otherwise into the top of `description`.
   - b. Make the field read-only everywhere until CAL-34 (Meet creation).
   - c. Save it in `extendedProperties.private`. Only Pigeon would see it.
10. **Message bodies.** The decision says `threads.get format=full`; this plan uses `messages.get format=raw`, batched, and parsed with `mail-parser`. That gives one parser for charsets, RFC 2047, attachments and `text/calendar`. It costs the same quota for 1–2 message threads and more for long ones.
    - a. `raw` with `mail-parser`.
    - b. `format=full` JSON, with charset decoding done by hand.
11. **The `directory.readonly` scope.** Resolved: keep it. It's already granted, so dropping it saves nothing now; no item in this sprint calls it.
12. **Remote images (ATT-1)** are the only 🟡 item that adds a new network surface, the user's IP to senders.
    - a. In this sprint, as W10, default "Ask".
    - b. Defer; ATT-1 stays 🟡.
13. **Calendar recurrence.** The decision says "singleEvents handling". Is `singleEvents=false` right?
    - a. `singleEvents=false`: store series with their RRULE and exceptions, and expand locally. This matches `CalendarView`'s series, exception and "This / Following / All" model. With `singleEvents=true` and a sync token there can be no `timeMax`, so endless series come back expanded far into the future.
    - b. `singleEvents=true` with a bounded window, and series edits through `recurringEventId`.

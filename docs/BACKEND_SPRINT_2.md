# Backend sprint 2: Pigeon as a daily driver

The plan as agreed with the user on 2026-10-27 (the working copy with progress is the OpenChamber plan of the same name), plus the contracts every workstream follows (section 8).

## 1. Goal

Sprint 1 linked Pigeon to Google (103 of 176 roadmap items 🟢). Sprint 2 makes it usable as the user's main mail and calendar app: it stays running with a top-class menu bar panel (today's events, recent mail, one-key meeting join) and notifies, opens and forwards attachments, sends from the right address with a signature, manages labels, sorts the Inbox like Gmail, and covers the calendar features people use every week.

Same constraints as sprint 1: one account, local-first, no server, light crates only, every new action in ⌘K with a shortcut.

**Roadmap target:** 27 existing items to 🟢 (NTF-1, NTF-2, NTF-3, ATT-2, ATT-4, ATT-7, RDR-8, FOL-6, FOL-9, FOL-10, FOL-11, FOL-12, SRC-9, CAL-22, CAL-23, CAL-29, CAL-34, CAL-39, MST-1, MST-2, DRF-5, LNK-3, ACC-2, CON-5, CMP-12, STA-2, STA-5), so 130 of 176. Narrowed on purpose, to 🟡 with their remaining Needs kept: CAL-33 (no editing of calendar defaults), CAL-41 (a subset of settings); CAL-12 stays 🟡. New items, added with W1/W7: **PER-4** app bundle and install, **PER-5** single instance, **NTF-4** menu bar panel, **NTF-5** quick join, **NTF-6** open at login. Items whose proof is native (notification delivery and clicks, the menu bar's focus and click-away, global shortcuts, open at login) stay 🟡 until the user's checks pass.

## 2. Scope by workstream

| Workstream | Items | Phase |
| --- | --- | --- |
| **W1 App shell and background** | NTF-1, NTF-2, NTF-3, PER-4, PER-5, NTF-6 | 1 |
| **W2 Attachments and reader** | ATT-2, ATT-4, ATT-7, RDR-8 | 1 |
| **W4a Categories backend** | FOL-11 (model and counts) | 1 |
| **W5 Calendar** | CAL-29, CAL-33, CAL-34, CAL-41, CAL-23, CAL-39, reminder notifications | 1 |
| **W7 Menu bar** | NTF-4, NTF-5 | 1 (design, spike), 1.5 (build) |
| **W3 Compose and identity** | MST-1, MST-2, DRF-5, LNK-3, ACC-2 | 2 |
| **W4b Labels and Inbox views** | FOL-6, FOL-9, FOL-10, FOL-11 (views), FOL-12, SRC-9 | 2 |
| **W6 Contact card and failure drills** | CON-5, CMP-12, STA-2, STA-5, CAL-22 | 2 |

### W1 App shell and background

- **Pigeon.app (PER-4):** `scripts/bundle.sh` builds `Pigeon.app` (Info.plist, bundle id `com.pigeon.dev`, `mailto:` URL type, a simple placeholder icon), signed with "Pigeon Dev" so notification permission and the default-mail-app choice stay tied to one stable identity (tokens go through `/usr/bin/security`, so the Keychain isn't affected). `scripts/install.sh` copies it to `/Applications`; the user's copy is the installed one, re-installed after each merge.
- **Single instance (PER-5):** a `flock` on `<data dir>/pigeon.lock`; a second launch on the same data dir activates the first and exits.
- **Test-instance mode:** `PIGEON_DEV=1` registers no global shortcuts and no login item, and prefixes notification tags, so a tester's copy can't clash with the installed one.
- **Log file:** stderr is lost when launched from the Dock, so log to `<data dir>/pigeon.log` (size-capped) too, including gpui's `log` output.
- **NTF-3 stay running:** remove the quit in `src/main.rs` `on_window_closed` (gpui's macOS default already keeps the app alive) and add `on_reopen`. One `ensure_main_window(cx)` that every entry point uses (Dock, notification click, menu bar, `mailto:`), with app-level handlers for Compose, New Event and Open Thread so they work with no window open. Settings become app-owned (today `Saved` is loaded once in `main()` and its subscriptions live on the window's `PigeonApp`), so a reopened window has the latest settings; window-owned timers stop on close. Closing the window saves open composers as drafts. ⌘Q quits and flushes the outbox as today. Tests: close and reopen keep appearance, layout and preferences.
- **Background sync:** with notifications on, poll Gmail every 60 s and Calendar every 2 min with the window closed (`history.list` is cheap); sync at once on wake (`NSWorkspaceDidWakeNotification`).
- **Open at login (NTF-6):** `SMAppService.mainApp` through small raw ServiceManagement bindings (no crate for it is in the lock file). The Settings toggle shows the service's real status, including "needs approval in System Settings" and errors; only offered when running from `/Applications`. Off by default.
- **NTF-1 notifications:** gpui's own API (`cx.show_system_notification`, `on_system_notification_response`, which wrap `UNUserNotificationCenter`; they post at once and report no delivery result). Every new Inbox mail, any category (decision 4):
  - history's `messagesAdded` ids are carried through the metadata fetch; each fetched message is judged on its current labels and `internalDate`: INBOX + UNREAD, not SPAM, TRASH or DRAFT, received by the existing rule (not SENT, or addressed to me or an alias, so self-mail counts), `internalDate` within 15 min of the poll;
  - notified message ids are kept in the store (bounded) and checkpointed with the history cursor only after processing, so a crash, relaunch or offline gap neither repeats nor drops one;
  - more than 3 at once become one summary notification under a fixed tag, replaced by the next;
  - a full sync or a history reset never notifies; nothing for the thread open in the frontmost window;
  - a notification is dismissed (by tag) once its thread is read anywhere; clicking opens the thread.
  - Settings: a toggle, and "Send test notification", which also brings up macOS's permission prompt.
  - Tests: retry and relaunch, self-mail and alias fixtures, the summary.
- **NTF-2 Dock badge:** Primary unread in the Inbox (from W4a's counts) via `dockTile.setBadgeLabel`.
- **Notification helper:** W1 owns `notify(tag, title, body, actions)` and the response routing; W5 and W7 use it.
- **objc:** `NSStatusItem` (W7), the Dock badge, wake and the login item, through direct `objc2` / `objc2-app-kit` dependencies (in the lock file; enable the features needed) plus the raw ServiceManagement bindings. No other new crates.

### W2 Attachments and reader

- **Where the bytes are:** bodies come from `messages.get format=raw`, so Pigeon has no `attachmentId`; the raw message already holds every part. Attachments are taken by `part_id` from the raw message (cached raw copy, else re-fetched), not via `attachments.get`.
- **Safe files:** written to a private cache with generated paths; the shown name is sanitised (no traversal, no collisions); files get the quarantine attribute; apps, scripts and other executables are revealed in Finder instead of launched. The cache is size-capped; bytes a queued forward or draft still needs are copied out of the cache until the op completes. Tests: duplicate names, traversal names, eviction during a queued forward.
- **ATT-4:** chips open the file with the default app and save it (save dialog, default Downloads). ⌘K: Open Attachment, Save Attachment.
- **ATT-2:** inline `cid:` parts go into the image cache when the body is parsed, no extra request, and render without the remote-image consent (they're part of the mail).
- **ATT-7:** Forward attaches the original's attachments (removable chips); over 25 MB is refused with the existing message.
- **RDR-8:** "Show Original" shows the first ~1 MB of the raw source in a read-only monospace view with Copy, and "Save as .eml" for the whole message.

### W4a Categories backend (Phase 1, so the badge and menu bar have it)

- **Per message, not per thread:** a message is Primary when it carries none of `CATEGORY_SOCIAL`, `_PROMOTIONS`, `_UPDATES`, `_FORUMS`. A thread is in an Inbox category view when any of its Inbox messages is in that category, which matches how Gmail's `category:` search matches threads. Today labels are unioned per thread (`src/backend/gmail.rs`), so per-message category labels are kept. Tests: mixed-category threads and uncategorised messages, checked against the server's answers.
- **Counts:** no Gmail label count matches "Primary in Inbox" (`labels.get CATEGORY_*` also counts archived mail). Unread counts come from `threads.list labelIds=INBOX,UNREAD q=category:<x>` with ids only, up to 500 (shown as "500+"), re-run only when history touched the Inbox.
- **Lists:** the Primary view and its "Load older" pages ask the server with `q=in:inbox category:primary`, so the first page isn't mostly Promotions.
- **Interface:** W4a owns `category_of(message)`, `in_category(thread, cat)` and the counts; W1, W4b and W7 use them.

### W5 Calendar

- **Event policy (shared):** one function, `next_joinable(now)`, used by the menu bar, quick join and reminders: events on selected calendars only, not cancelled, not declined by me, not all-day; overlaps are decided the same way everywhere (the one that started most recently, then the soonest), and the panel lists both. Reminders honour Google's `remindOnRespondedEventsOnly`. Tests.
- **CAL-29 repeat rules:** a Repeat row in the panel: presets (daily, weekdays, weekly on the day, monthly by date or weekday, yearly) plus custom (interval, weekdays, end by date or count), and "Does not repeat". Shows any RRULE from Google in words. Changing a series' rule offers only This and following / All events, as Google does.
  - Writes replace the whole RRULE (today `patch_body()` only compares COUNT/UNTIL and `recurrence_lines()` keeps the old frequency and weekdays); removing the repeat is handled; EXDATE and RDATE lines are kept on purpose.
  - The editor only produces rules the local expander supports (BYDAY with ordinals such as `-1FR`, COUNT/UNTIL).
  - **Split fix:** the new series of a "This and following" split is inserted only after the original series was shortened successfully (today it goes in anyway, and a 412 leaves duplicates). Tests with a failing first op.
- **CAL-33 reminders:** up to 5 per event, popup or email; every `calendarList.defaultReminders` entry kept (today only the earliest is). W5 owns `reminders_for(event)`. Editing a calendar's defaults stays out (🟡).
- **Reminder notifications:** popup reminders fire through W1's helper, with a Join action when the event has a video link. Best effort while the Mac is awake: deadlines are recomputed on wake and on clock changes; reminders already past at launch or wake never fire; fired (calendar id, event id, instance start, offset) keys are kept in the store. Tests with a fake clock. W5 merges after W1.
- **CAL-34 Google Meet:** "Add Google Meet" in the panel, only when the calendar's `allowedConferenceSolutionTypes` includes `hangoutsMeet`. `conferenceData.createRequest` with a request id kept across retries and `conferenceDataVersion=1`; bounded polling until `success`, else a retryable "Couldn't add Meet" state. Writes that don't touch it keep existing `conferenceData`.
- **CAL-41 Google Calendar settings:** read `settings.list`: `weekStart`, `format24HourTime`, `defaultEventLength`, `autoAddHangouts`, `remindOnRespondedEventsOnly`. They're defaults until the user changes them in Pigeon (24-hour time and default length are new Settings rows). Time zone: a notice when Google's differs from the Mac's; converting stays out (it needs `chrono-tz`).
- **CAL-23 (decision 7):** the calendar reopens on the last date viewed, saved with the other calendar state (PER-3); `t` still jumps to today.
- **CAL-39 quick add:** ⌘K "New event: Lunch with Sam Fri 1pm" parses with Pigeon's own parsers (no `events.quickAdd` round-trip) and opens the panel to confirm.

### W7 Menu bar (decision 2: very important, top-class UI)

- **W7a design (Phase 1, before any code):** the main agent renders 2–3 mockups of the panel in Pigeon's own look (theme tokens from `src/theme.rs`, glass, light and dark) for the user to pick from and comment on.
- **W7s native spike (Phase 1, in parallel with W7a):** a small throwaway branch proving the hard parts before the build: the panel takes keyboard focus while another app stays active; it hides and shows again without being rebuilt (gpui has no per-window hide/show, so a narrow native bridge on the panel's `NSPanel` is allowed); Esc and click-away close it; repeated status-item clicks toggle without flicker; positioning on a secondary screen and over a full-screen app. If something can't be done well, the main agent reports to the user before W7b starts.
- **Status item:** a monochrome Pigeon glyph (template image, follows the menu bar's light/dark) with a dot for new Primary mail since the panel was last opened. Optional (Settings): the next meeting's title and countdown next to it ("Standup in 12m"), turning into "Join" while it runs.
- **Panel:** click the item, or a global shortcut, opens a translucent panel under it, Raycast/Linear quality: keyboard first, under 50 ms to visible, closes on Esc or click-away.
  - **Today:** the day's remaining events as a compact timeline (time, calendar colour, title, location); the meeting from `next_joinable` on top with a countdown and a prominent Join button. Tomorrow's first event when the day is empty. Clicking an event opens it in the calendar.
  - **Recent mail:** the latest ~8 Primary threads (unread dot, sender, subject, time). Enter opens the thread in the main window; `e` archives and `shift-i` marks read from the panel (same outbox and undo as the main app).
  - **Footer:** Compose (`c`), New event, Open Pigeon (⌘O), sync status. Every panel action is also in the main ⌘K.
- **Quick join:** a global shortcut opens the video link (Meet, Zoom, Teams, Webex, found by CAL-17) of `next_joinable` within 10 min.
- **Shortcuts:** defaults chosen to avoid macOS's own (⌃⌥Space is the input-source switch): ⌃⌥⌘Space for the panel, ⌃⌥⌘J for quick join, both changeable in Settings; if registering fails, Settings shows the conflict. Carbon `RegisterEventHotKey` (no Accessibility permission).
- **For testers:** dev ⌘K commands Open Menu Bar Panel, and Quick Join as a dry run that logs the URL. These bypass the real status-item path, so native behaviour stays a user check.

### W3 Compose and identity

- **Model:** `SendAs` grows `displayName`, `replyToAddress`, `signature`, `isDefault` (all in `sendAs.list` with the current `gmail.modify` scope). A draft keeps its chosen From and whether it carries Pigeon's signature, locally and across the Gmail round trip.
- **MST-1 From picker:** the composer gets From when the user has aliases; defaults to the default alias, or for replies the alias the mail was sent to. Sent mail uses the alias's display name and reply-to. Closes ACC-2, together with the calendar's "is this me" check using the aliases.
- **MST-2 signatures:** Gmail's per-alias HTML signature is kept apart from the editable body: a read-only block under it, added to the HTML part at send and draft save (and a plain-text version to the text part), swapped with From. Drafts from Gmail web: the `gmail_signature` block is detected when the draft is parsed, before it becomes editor text, so no second signature is added. Editing stays in Gmail's web settings, so no new OAuth scope. A Settings toggle to turn it off. Tests: reopen, alias switch, Gmail-origin drafts.
- **DRF-5 save while typing:** saved locally 2 s after typing stops; the Gmail draft updated at most every 30 s and on close; "Saving…/Saved" in the composer footer. Each save carries a revision number of a stable local draft id: a stale save answer never overwrites newer text, and never brings back a sent or discarded draft. Tests: typing during a save in flight, send after save, discard after save, crash recovery.
- **LNK-3:** `mailto:` links in messages, and system `mailto:` opens via gpui `on_open_urls` once Pigeon is the default mail app, open Pigeon's composer (to, cc, subject, body).

### W4b Labels and Inbox views

- **FOL-6:** create, rename, recolour and delete user labels from ⌘K and the sidebar. Colours: both text and background from Gmail's allowed palette, offered as a few curated, readable pairs. Rename also renames every child label (Gmail's API doesn't cascade): check for name collisions first, and if a child rename fails part way, resync labels and say which ones weren't renamed. Delete asks first, also about children, and never touches mail.
- **FOL-9:** nested labels (`Work/Clients`) as a collapsible tree in the sidebar and the label picker.
- **FOL-10:** respect each user label's `labelListVisibility` in the sidebar and `messageListVisibility` for label chips on rows; change them from the sidebar.
- **FOL-11 views (decision 3):** the Inbox shows Primary. Social, Promotions, Updates and Forums are views under Inbox in the sidebar, with W4a's unread counts, `g` chords and ⌘K entries; unused categories are hidden. `in:inbox` search keeps all categories, as Gmail does.
- **FOL-12:** Important marker on rows and in the reader; toggle with a shortcut and ⌘K.
- **SRC-9:** chip `after:`/`before:`, `older_than:`/`newer_than:`, `larger:`/`smaller:`, `filename:`, `cc:`/`bcc:`, `list:`, `category:`, `is:important`, `-` negation, `OR` and grouping. A query with any of these goes to Gmail as typed, and its results are Gmail's only, without the local results merged in (they would show false matches). Offline it says such searches need a connection. Tests: `OR`, parentheses and negation with the view scope.

### W6 Contact card and failure drills

- **CON-5:** clicking a sender or guest (or ⌘K "Show Contact") opens a card from the local cache: the connections sync adds `phoneNumbers` and `organizations` (its stored sync token is dropped once, since People needs the same field mask to reuse it; Other Contacts keeps its own valid mask); the card shows name, initial, addresses, phone, organisation and recent threads with them. Non-contacts show name and address only. No live People calls.
- **Failure drills (dev only):** with `PIGEON_DEV=1`, files in the data dir make the next Google write fail: `simulate-refusal` (400) and `simulate-conflict` (412). Lets testers check send refusal (CMP-12), rollback (STA-2) and conflicts (STA-5) on real data. Inert without `PIGEON_DEV`.
- **CAL-22:** tester pass on a read-only calendar: dragging and editing are refused with a clear message (needs a read-only calendar, see section 5).

## 3. Out of scope, and why

- **ACC-4 multiple accounts:** large change to the store and UI; best as sprint 3, once the user can log in to the second address.
- **SYN-3 / SYN-5 push, ACC-8 verification:** need a server, Pub/Sub, or Google's paid security assessment. Polling stays.
- **FOL-8 Delete forever / Empty Trash:** needs the full `https://mail.google.com/` scope; Gmail empties Trash after 30 days itself.
- **CAL-12 time zone conversion:** needs `chrono-tz`; only a notice this sprint.
- **Reminders while the Mac sleeps or Pigeon is quit:** best effort while awake and running.
- **MST-3 … MST-10** (vacation, filters, forwarding, IMAP, language, delegates, S/MIME, encryption): rarely used; Gmail web covers them.
- **CON-3, CON-4, CON-6, CON-7, CON-8:** need the contacts write scope or Workspace.
- **CAL-24 … 28, 30 … 32, 35 … 38:** calendar admin, sharing, free/busy and special event types.
- **LNK-2, DAT-7, PLN-1a/1b (Vim), PLN-2 (AI reading view):** later sprints.

## 4. How it runs

- Same workflow as sprint 1: Opus 5.5 implementers in worktrees (`bs2/<name>`, own `CARGO_TARGET_DIR`), the main agent merges, one GUI tester at a time on the detached snapshot, fresh implementers fix, re-test.
- **Contracts before dispatch** (as sprint 1 did): the main agent writes, in `docs/BACKEND_SPRINT_2.md`, each stream's owned files, its reserved store migration number, which shared files it may touch and how, and the shared interfaces with their signatures: W1 `ensure_main_window`, `notify` and response routing; W4a `category_of`, `in_category`, counts; W5 `reminders_for`, `next_joinable`. A stream that depends on another rebases onto it before merging.
- **Merge order:** Phase 1: W1 first, then W4a, W2, W5. Phase 1.5: W7b on top of W1, W4a and W5. Phase 2: W4b before W3 (both touch the composer and sidebar edges), W6 any time.
- **Hot spots:** `src/main.rs`, `src/app.rs`, `src/actions.rs`, `src/ui/settings.rs`, `src/backend/sync.rs`, `src/backend/mime.rs`, `docs/ROADMAP.md`. W7b lives mostly in a new `src/ui/menu_bar.rs`.
- **Real-account safety (unchanged):** send only to `alex.rivera@example.com` with `[Pigeon test]` subjects; act only on `[Pigeon test]` threads; calendars only "Pigeon Test" and "Pigeon Test 2"; never sign out or permanently delete; auto-advance off during mail actions. New: label management only on labels the tester creates, named `Pigeon Test …`, deleted at the end; no Gmail settings writes.
- **Testers** run in test-instance mode with their own `PIGEON_DATA_DIR`. They can't see or click macOS notifications, system prompts or global shortcuts: they check that notifications were *submitted* through `<data dir>/pigeon.log`, the Dock badge with `lsappinfo`, and the panel and quick join through the dev ⌘K commands. Delivery, clicks and the native panel behaviour are user checks. Test arrivals are created by the main agent with `messages.insert` (INBOX, UNREAD), as `pigeon_inbound.py` does, including one self-addressed.

## 5. Before and during the sprint (user)

1. **Optional, before the next sign-in: publish the Google app** so sign-ins stop expiring every 7 days: Google Cloud Console → Google Auth Platform → Audience → **Publish app** → Confirm. It stays unverified (fine for personal use, up to 100 users). It only takes effect at the next sign-in.
2. **Sign in again when Pigeon asks** (the current sign-in expires around Nov 2; needs the user at the Mac). If published, choose Advanced → Go to Pigeon on Google's "hasn't verified this app" screen. While the sign-in is expired, real-account tests pause; building, unit tests and demo-mode tests go on.
3. **Pick a menu bar mockup** (W7a), from the phone is fine.
4. **Allow notifications** right after W1 merges: Settings › Notifications › "Send test notification", then Allow.

User checks at the end of the sprint: the menu bar panel's look, feel, focus and click-away, and the global shortcuts; a notification arriving and clicking it; making Pigeon the default mail app; open at login; attaching a file (ATT-6, from sprint 1); invites from the second address (CAL-14, CAL-20).

## 6. Decisions

Answered by the user, 2026-10-27:

1. **Theme:** daily driver.
2. **Closing the window:** keep running in the Dock and the menu bar; ⌘Q quits. The menu bar is very important and must be top class: today's events, recent mail, quick meeting join (W7).
3. **Categories:** Inbox shows Primary; the other categories are views under Inbox.
4. **Notifications:** every new Inbox mail, all categories.
5. **Sprint 1 leftovers:** keep all three as they are (Esc reverts a pending series change; approximate count only while Gmail has more pages; demo Sent at 34).
6. **Read-only calendar test:** the main agent may subscribe to a public holidays calendar for CAL-22 and remove it afterwards.
7. **CAL-23:** the calendar reopens on the last date viewed, not today.
8. **Publishing the Google app:** not required to start; it can wait until the user's next sign-in (section 5).
9. **Menu bar design (W7a):** design A, "Agenda first". From the top:
   - the date and the sync status;
   - a card for the next meeting: a countdown chip, the time, the title, the video service and guests, and Join (`j`) and Details buttons;
   - "Later today": time, calendar colour bar, title and location;
   - "Recent mail": about 4 Primary threads, each with an unread dot, sender, subject and time;
   - a footer with Compose (`c`), New event (`n`) and Open Pigeon (⌘O).

   The next meeting shows next to the icon by default ("Design review in 12m"). The mockup is in `docs/design/menubar.html`.

## 7. Done

- The roadmap target in section 1 is met, or each miss is listed with a reason, agreed with the user.
- `cargo fmt`, build, `clippy --all-targets` clean, `cargo test` (no network) passes; demo mode makes no network requests.
- With the window closed and nothing happening, Pigeon idles near 0% CPU with stable memory (measured in T3).
- `docs/ROADMAP.md` current (statuses, counts, the new PER-4, PER-5, NTF-4, NTF-5, NTF-6); `docs/BACKEND_SPRINT_2.md` records the plan, contracts and decisions.
- An Opus 5.5 agent builds the sprint summary artifact with the web-artifacts skill.

## 8. Contracts

Written by the main agent before Phase 1 is dispatched. A stream that needs to break one asks the main agent first.

### 8.1 Branches, worktrees, migrations

- **Worktree:** `/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/pigeon-bs2-<name>`, branch `bs2/<name>`, from the current `main`.
- **Builds:** `CARGO_TARGET_DIR=/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/target-bs2-<name>`.
- **Migrations:** a stream that changes the schema adds one block to `src/backend/store/schema.sql` with its reserved number. Settings are rows in the `settings` table and need no migration.

| Phase | Stream | Branch | Merge order | Schema block |
| --- | --- | --- | --- | --- |
| 1 | W1 App shell and background | `bs2/shell` | 1 | 100 |
| 1 | W4a Categories backend | `bs2/categories` | 2 | 110 |
| 1 | W2 Attachments and reader | `bs2/attachments` | 3 | 120 |
| 1 | W5 Calendar | `bs2/calendar` | 4 | 130 |
| 1 | W7s Native panel spike | `bs2/menubar-spike` | never merged | none |
| 1.5 | W7b Menu bar | `bs2/menubar` | 1 | 140 |
| 2 | W4b Labels and Inbox views | `bs2/labels` | 1 | 150 |
| 2 | W3 Compose and identity | `bs2/compose` | 2 | 160 |
| 2 | W6 Contact card and failure drills | `bs2/contacts-drills` | any | 170 |
| any | Fixes | `bs2/fix-<n>` | after its phase | 180 and up, given by the main agent |

### 8.2 Who wires what

A stream that depends on another one merging first codes against the interfaces in 8.3 and makes the hookup itself after rebasing onto `main`:

- **W4a** merges after W1 and switches W1's Dock badge from the Inbox's server unread count to Primary unread.
- **W5** merges after W1 and sends its reminders through `notify`. Until then its scheduler is unit-tested with a fake clock and a recording sink.
- **W7b** starts after W1, W4a and W5 are merged.
- **W3** merges after W4b and rebases onto it.

### 8.3 Shared interfaces

The names and shapes are fixed. Details such as extra fields or derives are the owner's call.

```rust
// src/notify.rs (W1). Wraps cx.show_system_notification / dismiss_system_notification
// and routes on_system_notification_response.
pub enum NoticeTarget {
    Thread { id: String },
    Event { calendar: String, id: String, start: NaiveDateTime },
    Inbox,
}
pub struct Notice {
    pub tag: String,                 // prefixed in test-instance mode
    pub title: String,
    pub body: String,
    pub target: NoticeTarget,        // where a click on the body goes
    pub join_url: Option<String>,    // adds a "Join" action that opens the URL
}
pub fn notify(notice: Notice, cx: &mut App);
pub fn dismiss(tag: &str, cx: &mut App);

// src/app/shell.rs (W1). The one way to reach the main window, whether or not
// one is open: Dock, notification clicks, the menu bar, mailto:.
pub enum Show {
    Main,
    Thread(String),
    Event { calendar: String, id: String, start: NaiveDateTime },
    Compose(Option<Mailto>),         // Mailto { to, cc, bcc, subject, body }; W3 fills parsing
    NewEvent,
}
pub fn ensure_main_window(cx: &mut App) -> WindowHandle<PigeonApp>;
pub fn show(what: Show, cx: &mut App);

// src/backend/categories.rs (W4a)
pub enum Category { Primary, Social, Promotions, Updates, Forums }
pub fn category_of(label_ids: &[String]) -> Category;            // one message's labels
pub fn in_category(thread: &Thread, category: Category) -> bool; // any Inbox message in it
pub struct CategoryCounts { /* unread per category; capped at 500 ("500+") */ }
impl Backend { pub fn category_counts(&self) -> &CategoryCounts; } // changes emit CountsChanged

// src/calendar/policy.rs (W5)
pub struct Reminder { pub minutes: u32, pub method: ReminderMethod } // Popup | Email
pub fn reminders_for(event: &Event, calendar: &Calendar) -> Vec<Reminder>;
pub fn next_joinable(data: &CalendarData, now: NaiveDateTime) -> Option<Occurrence>;
pub fn video_link(event: &Event) -> Option<&str>;                   // Meet, Zoom, Teams, Webex
```

### 8.4 File ownership in Phase 1

The owning stream is the only one that edits a file. Shared files are edited only as their row says.

| Stream | Owns |
| --- | --- |
| W1 | `src/main.rs`, new `src/notify.rs`, new `src/app/shell.rs`, new `src/native.rs` (objc: Dock badge, wake, login item), new `scripts/bundle.sh`, `scripts/install.sh`, `packaging/` (Info.plist, icon), `Cargo.toml`, `src/settings.rs` (moving `Saved` to the app), logging. In `src/backend/sync.rs`: poll timings and carrying `messagesAdded` through. |
| W2 | `src/backend/mime.rs`, new `src/backend/attachments.rs`, `src/ui/reader.rs`, `src/ui/html_view.rs`, `src/backend/images.rs`, the forward path in `src/app/compose.rs`. |
| W4a | new `src/backend/categories.rs`, the Primary list query in `src/backend/gmail.rs`. In `src/backend/sync.rs`: one count refresh function and its call. |
| W5 | `src/backend/calendar.rs`, `src/calendar/*`, new `src/calendar/policy.rs`, `src/ui/calendar/*`, quick add in `src/ui/command_palette.rs`. |
| W7s | its own throwaway branch; edits anything it needs and is never merged. |

**Shared files:**

| File | Rule |
| --- | --- |
| `src/actions.rs` | Append new actions, keybindings and `COMMANDS` rows at the end of each list, under a `// W<n>` comment. The main agent resolves conflicts at merge. |
| `src/app.rs` | Smallest possible edits: one call into the stream's own file per hook. New logic goes in `src/app/*.rs` files the stream owns. |
| `src/backend/mod.rs` | New public functions and `BackendEvent` variants in a block per stream. |
| `src/ui/settings.rs` | Each stream adds its own section or rows; `Saved` fields are appended. W1 adds the Notifications and General › Open at login rows; W5 the calendar rows. |
| `Cargo.toml` | W1 only, in Phase 1. It adds `objc2`, `objc2-foundation` and `objc2-app-kit` (the 0.6 / 0.3 versions already in the lock file) with the features W7b will need as well (`NSStatusBar`, `NSStatusItem`, `NSPanel`, `NSImage`, `NSDockTile`, `NSWorkspace`). Anything else goes through the main agent. |
| `docs/ROADMAP.md` | Implementers edit only their items' text, and add the new items they own (W1: PER-4, PER-5, NTF-6; W7b: NTF-4, NTF-5). Only the main agent changes statuses and the counts. |

### 8.5 Before each merge

The main agent rebases the branch onto `main` and runs `cargo fmt --check`, `cargo build`, `cargo clippy --all-targets` (no new warnings) and `cargo test`. After merging, it runs them again on `main`, updates the roadmap's statuses and counts, and commits.

## 9. Outcome

**Status:** done. Every workstream merged (W1, W4a, W2, W5, W7b, W4b, W3, W6), plus fix rounds fix-1 to fix-7. 345 tests pass; clippy is clean. The roadmap has 131 of 181 items 🟢: all of section 1's target except MST-1, LNK-3 and ACC-2, which wait on a user check or a second verified address, and NTF-4, NTF-5, NTF-6, which wait on user checks.

**Testing:** GUI testing moved from Cua Driver to Codex (`codex exec -m gpt-6.1-terra`, with its built-in computer use) during the sprint. The rounds:

- T1a and T1a-2, app shell: pass. ⌘O with no window open can't be tested without the Dock.
- T1b, attachments: pass.
- T1c, calendar: pass after fix-2 and fix-4.
- T1d, menu bar: pass after fix-3 (the New Event freeze).
- T2a, labels and Inbox views: pass after fix-5.
- T2b, compose: pass after fix-6 (signature fetch, discard on quit).
- T2c, contact card, drills and read-only calendars: pass after fix-7.
- R2 to R4, retests: R2 pass; R3 and R4 partial (the tester's screenshots went stale while Pigeon's window was covered), and the user checked the rest by hand.
- The user's own final check of the rollback, read-only drag and labels: pass.

**Bugs found and fixed:**

- Calendar-default reminders were refused with a 400 (CAL-33).
- After a split, the scope wasn't asked; then an edit was lost to the split's sync (CAL-29).
- Held forward copies were left behind (ATT-7).
- New Event from the menu bar panel looped and kept opening windows.
- Gmail-only search ignored the Inbox scope.
- New labels and visibility changes didn't show until relaunch.
- Send-as addresses and signatures were only read on a full sync.
- A discarded draft came back after quitting during Undo.
- The open event panel kept a rejected title after a rollback.
- Dragging a read-only all-day or Month event showed no toast.
- The read-only toast used the success style.
- ⌘K in the composer body opened the link dialog; links are now ⇧⌘K, so ⌘K is always the palette.

**Known limits:** gpui stops drawing while the window is fully covered. That's normal; testers working in the background saw stale pixels. The rest are the user checks in the roadmap.

**User checks left:**

- Allow notifications.
- The menu bar status item and the global shortcuts.
- Make Pigeon the default mail app and click a `mailto:` link in another app.
- Open at login.
- Sign in again around Nov 2, or publish the Google app first.
- Sprint 1 leftovers: ATT-6, CAL-14 and CAL-20, ACC-3 and ACC-7.

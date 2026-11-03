# Glossary

The words Pigeon uses, and the code or doc each one comes from. Where the UI says something different from the code, the entry says so.

| Term | Meaning in Pigeon | Where it lives |
| --- | --- | --- |
| Conversation | What the UI calls one mail thread. Code and docs say `Thread`; user-facing strings always say "conversation" ("Couldn't archive 2 conversations"). | `Thread` in `src/data.rs`; UI strings across `src/ui/` |
| Thread | The model for a conversation: its messages, one folder, its labels and Gmail's `threadId`. Never shown to the user as "thread". | `src/data.rs`, `threads` table in `src/backend/store/schema.sql` |
| Demo data | The bundled, generated mailbox and calendar Pigeon runs on with `PIGEON_DEMO=1`, or when picked at sign-in. In the UI always "demo data"; docs and code comments also say "placeholder data" for the same `assets/*.json`. "Mock data", "sample data" and "fake data" are wrong. | `src/backend/demo.rs`, `assets/mailbox.json`, `assets/calendar.json` |
| All Mail | The view over every conversation except Spam and Trash. A search scope ("Search All Mail", the scope chip), not a sidebar folder; Archive is Gmail's "All Mail minus Inbox". Always capitalized. | `Scope::All` in `src/search.rs`; `src/ui/thread_list.rs` |
| Folder | The one place a conversation sits: Inbox, Archive, Sent, Drafts, Spam or Trash. Derived from its messages' Gmail labels by `derive_folder()`, and a sidebar row. A folder is not a label. | `Folder` in `src/data.rs`, `src/backend/gmail.rs` |
| Label | A Gmail user label. Gmail stores labels per message; Pigeon stores the union on the conversation and keeps one folder per conversation (DAT-2). | `Thread::label_ids` / `labels` in `src/data.rs`, `src/backend/labels.rs` |
| Alias / send-as / "me" | An alias is another address the user sends as; Gmail's `sendAs` entries (`SendAs`) also carry name, reply-to and signature. `is_me()` treats the account address and every alias as the user, and `me()` is the user's own contact. | `Account::aliases`, `send_as`, `is_me()`, `MailData::me()` in `src/data.rs` |
| Outbox | The durable write queue: every action becomes a row in the `outbox` table until it commits, so it survives a quit. Queued ops are laid over the store when threads are read, and consecutive changes coalesce. | `src/backend/outbox.rs`, `outbox` table |
| Undo / rollback | Undo is the 5 s window (`UNDO_WINDOW`) before an op commits: undoing deletes the row, so nothing was ever sent. Rollback is the other path, when Google refuses an op: Pigeon re-reads the server's truth, drops the change and shows an error toast. | `UNDO_WINDOW` in `src/backend/mod.rs`; rollback in `src/backend/outbox.rs`, `src/app/undo.rs` |
| Sync | Keeping the store current with Gmail, Calendar and People. Two kinds for mail: full and partial. Also runs on focus and on Sync Now (⌘R). | `src/backend/sync.rs`, `docs/ROADMAP.md` SYN |
| Full sync | The first pass and the recovery pass: profile, labels, send-as, Inbox, drafts, Sent and Starred, then a backfill bounded to 1,000 conversations or 90 days. | SYN-1, `src/backend/sync.rs` |
| Partial sync | The normal pass: `history.list` from the stored `gmail_history_id`, refetching only touched conversations. Pigeon's word is "partial sync" (or "incremental sync" for Calendar); "delta" is not used here. | SYN-2, `HISTORY_KEY` in `src/backend/sync.rs` |
| Polling | How Pigeon learns about changes, since push needs a server: Gmail every 45 s while active, every 60 s with the window closed and notifications on, else every 5 min; Calendar every 2 min. | `POLL_ACTIVE` / `POLL_BACKGROUND` / `POLL_INACTIVE` and `INTERVAL` in `src/backend/sync.rs` |
| Store (cache) / migration | The SQLite file holding mail, calendars, contacts, settings, the outbox and AI results. It is a cache: Google holds the truth. Schema changes are numbered blocks in `schema.sql`, applied by `user_version`; a merged block is never edited. | `src/backend/store/`, `src/backend/store/schema.sql` |
| Roadmap status | ⚪ Not done: nothing built. 🟡 Partially done: built but not linked to the real backend yet, or waiting on a user check; its "Needs" line says what. 🟢 Fully linked: works end to end. | `docs/ROADMAP.md`, "Status legend" |
| Needs Reply | Jev's yes/no marker on Inbox conversations that wait for an answer, plus the Needs Reply view (`g r`) and the dismiss/restore commands. | AI-4, `src/ai/pass.rs`, `src/app.rs` |
| Best Matches | The AI section above search results: local candidates reranked by Jev, shown only for searches of 3+ plain words. | AI-13, `src/ai/search.rs`, `src/app.rs` |
| Ask Pigeon | The floating question panel (⇧⌘J): writing model turns the question into search terms, Jev reranks mail and events, the answer streams with citations. | AI-14, `src/ui/ask_panel.rs`, `src/ai/search.rs` |
| Background pass | The scheduled AI work: decides what is missing or stale, queues it at Background priority, and runs when AI is turned on, at startup, on new mail, on `BodiesReady` and every 5 minutes. | AI-3, `src/ai/pass.rs`, `src/ai/queue.rs` |
| AI command | A user-facing action gated on AI (`When::Ai`), offered in ⌘K and the keymap only while its feature is allowed. | `AiNeed` in `src/actions.rs`, `src/app/ai.rs` |
| Period | The span of days the calendar shows, moved by next/previous period. User-facing names are the Day, Week and Month views. | `src/ui/calendar/mod.rs`, `src/ui/calendar/header.rs` |
| All-day lane | The rows above the time grid holding all-day and multi-day events; extra lanes fold into "+N". | `src/ui/calendar/week_view.rs` |
| Quick join | Opening the video link of the meeting under way or starting within 10 minutes, from the menu bar panel, ⌃⌥⌘J or ⌘K. | NTF-5, `quick_join()` in `src/app/today.rs` |
| RSVP | The user's answer to an invite: accepted, tentative, declined or needs action, read from the guest list and written with `CalOp::Rsvp`. | `RsvpStatus` in `src/calendar/mod.rs` |
| Repeat rule | A series' RRULE, stored as `Rule` and expanded locally; Pigeon supports a subset and shows "Custom repeat" for the rest. | `src/calendar/recurrence.rs` |
| Sprint / workstream | A multi-stream push recorded in a `docs/*_SPRINT.md` plan; each workstream (W1, W2, …) owns a set of files and roadmap items. | `docs/BACKEND_SPRINT_1.md`, `docs/BACKEND_SPRINT_2.md`, `docs/AI_SPRINT.md` |
| Roadmap item ID | An area prefix plus a number, such as `ACC-1`, `CMP-7` or `CAL-4a`. IDs are stable and referenced from code comments and sprint docs. | `docs/ROADMAP.md` |
| Pigeon Test | The test-data convention: the `Pigeon Test` Gmail label, the `Pigeon Test` calendar, `[Pigeon test]` subjects and the `guest@example.org` second address. Testers touch only these. | `docs/BACKEND_SPRINT_1.md` §5.2, 6.4 item 6 |

## Naming rules

- User-facing text says "conversation", "demo data" and "All Mail"; code says `Thread`, `demo` and `Scope::All`.
- US spelling: "color", "canceled", "labeled".
- Title case for buttons, menus, commands and segments; sentence case for tooltips, settings rows, descriptions and toasts.
- Gmail methods drop the `users.` prefix: `threads.modify` is `users.threads.modify`.
- Roadmap IDs never change, and a new item takes the next number in its area.

# Pigeon: Backend Requirements

This file lists every front-end feature that needs backend support. It describes the code as it is today, so that when the backend is built, each item can be checked off and no part of the UI is left as a fake.

## Current state

- All mail data comes from `assets/mailbox.json`. It is loaded once at startup by `MailData::load()` in `src/data.rs`, called from `main()` in `src/main.rs`.
- All calendar data comes from `assets/calendar.json`. It is loaded once at startup by `CalendarData::load()` in `src/calendar/mod.rs`, also called from `main()`.
- There is no network, sync, sending or storage code in `src/`.
- Every mail change (archive, delete, star, read state, send, draft save, discard) is applied in memory to `PigeonApp::data` in `src/app.rs`. Every calendar change (create, edit, delete, RSVP, show or hide a calendar) is applied in memory to `CalendarView::data` in `src/ui/calendar/mod.rs`.
- Nothing is saved. Quitting (`Quit`, `cmd-q`) discards all changes, including drafts and sent messages.
- There is one account. It comes from the `account` object in the JSON.

## How to read each item

- **Feature:** what the user sees or does, and the shortcut if there is one.
- **Today:** how it works now (in-memory, placeholder JSON, display only, stub). Source is given as file and function.
- **Backend needs:** the real behavior required. Provider names (Gmail API, IMAP) are examples, not requirements.
- **Notes:** edge cases and gaps the backend must handle.

Items marked **UI not built yet** have no front-end code. They are here because the UI implies them or because the backend will need them.

Feature IDs use these area codes: ACC, FOL, LST, RDR, ACT, CMP, DRF, UND, SRC, CON, ATT, LNK, PAL, CAL, NTF, STA, PER, DAT. The checklist at the end uses the same IDs.

---

## 1. Account (ACC)

### ACC-1 Account card

- **Feature:** The sidebar shows the account name, email and an avatar with initials at the top. Clicking it does nothing.
- **Today:** Display only. `render_sidebar()` in `src/ui/sidebar.rs` reads `data.account` from the JSON. The card has no click handler.
- **Backend needs:** Fetch the signed-in profile (name, primary address). Gmail: `users.getProfile`. IMAP: from the login and the user's own identity.
- **Notes:** The avatar colour comes from the email address (`Theme::avatar_color`), so it is stable per account.

### ACC-2 Identity of "me"

- **Feature:** Messages from the user show as "me" in the reader. Replies skip the user's own address. Sent and Drafts rows show "To:".
- **Today:** `MailData::is_me()` and `MailData::me()` in `src/data.rs` compare against the single `account.email`, ignoring case. `contact_name()` in `src/ui/reader.rs` shows "me".
- **Backend needs:** The full set of addresses the user sends from (aliases, "send as" addresses, plus the primary). Each message's `from` must be checked against all of them.
- **Notes:** Only one address is checked today. Aliases would show as other people.

### ACC-3 Add, sign in to and remove accounts

- **Feature:** UI not built yet. There is no way to add an account, sign in or remove one.
- **Today:** Not implemented.
- **Backend needs:** OAuth sign-in (Gmail) or IMAP/SMTP credentials. Secure token storage (Keychain). Account removal, which deletes local data for that account.
- **Notes:** The data model holds exactly one account (`MailData::account`). See DAT-2.

### ACC-4 Account switcher and multiple accounts

- **Feature:** UI not built yet. The account card is the only place an account appears. There is no switcher and no unified inbox.
- **Today:** Not implemented. `PigeonApp::data` is a single `MailData`.
- **Backend needs:** Several accounts, each with its own threads, labels and sync state. Each thread and draft must know its account. The UI needs a switcher, or a unified view with account badges.
- **Notes:** Thread IDs such as `t-0001` are only unique inside one JSON file. They must be unique per account when there are several.

### ACC-5 Per-account sync status

- **Feature:** UI not built yet. There is no "last synced" time or sync indicator.
- **Today:** Not implemented.
- **Backend needs:** A sync state per account (last successful sync, current state, last error), exposed to the UI.
- **Notes:** Links to STA-1 and STA-3.

---

## 2. Folders and labels (FOL)

### FOL-1 System folders and go-to shortcuts

- **Feature:** The sidebar lists Inbox, Starred, Sent, Drafts, Archive, Spam and Trash. Shortcuts: `g i` Inbox, `g s` Starred, `g t` Sent, `g d` Drafts, `g a` Archive, `g !` Spam, and Trash via the palette.
- **Today:** Placeholder JSON. `Folder` enum in `src/data.rs` has six variants: Inbox, Archive, Sent, Drafts, Spam, Trash. Each thread has one `folder`. Starred is a flag, not a folder. `FOLDERS` in `src/ui/sidebar.rs` and `set_nav()` in `src/app.rs` handle navigation.
- **Backend needs:** Map each server folder or label to one of the six folders. Gmail: INBOX, SENT, DRAFT, SPAM, TRASH, and "not in INBOX" for Archive. IMAP: the special-use folders (`\Sent`, `\Drafts`, `\Junk`, `\Trash`, `\Archive`). Any server folder that does not map must be handled, because `Folder` deserialization fails on unknown values.
- **Notes:** See DAT-2 for the one-folder-per-thread limit.

### FOL-2 Sidebar counts

- **Feature:** Each sidebar row can show a number. Drafts shows the total number of drafts. Inbox, Spam and labels show the number of unread threads. Starred, Sent, Archive and Trash show nothing.
- **Today:** Computed from loaded threads. `nav_count()` in `src/ui/sidebar.rs`.
- **Backend needs:** Real counts from the server, not from loaded threads. Gmail: `labels.get` gives `messagesUnread` and `threadsUnread`. IMAP: `STATUS ... UNSEEN`. The count must update when sync changes it.
- **Notes:** The counts only cover what has been loaded. With paging (LST-7), the numbers would be wrong.

### FOL-3 Label list with colours

- **Feature:** Under "Labels" in the sidebar, each user label has a coloured dot and its name. Colours also appear on chips.
- **Today:** Placeholder JSON. `MailData::labels` from `assets/mailbox.json`. Colours are hex strings parsed by `hex_color()` in `src/data.rs`. Ten labels exist.
- **Backend needs:** List user labels with names and colours. Gmail: `labels.list` plus label colour fields. IMAP: folders or keywords, with colours kept locally, since IMAP has no colour.
- **Notes:** Label IDs are referenced by threads in `Thread::labels`. Renaming a label must keep its ID stable.

### FOL-4 Label views

- **Feature:** Selecting a label shows its threads. The thread list title is the label name. `g l` or the palette "Go to Label…" opens a label search.
- **Today:** Placeholder JSON. `Nav::Label(usize)` in `src/app.rs`. `nav_contains()` shows threads whose `labels` contain the ID and that are not in Spam or Trash.
- **Backend needs:** Query the threads for a label from the server. Gmail: `threads.list` with `labelIds`. IMAP: the folder or keyword search.
- **Notes:** Label views include threads in Sent, Drafts and Archive. Spam and Trash are excluded, as in Gmail.

### FOL-5 Apply and remove labels on a thread

- **Feature:** `l`, the palette's "Label…", Message › Label…, the toolbar's "…" menu or a row's right-click menu toggle a label on the selected thread, or on every checked thread (ACT-6). `l` opens the palette scoped to labels; applied ones say "Applied · ↵ removes it". In the list, labels show as up to three coloured dots (names in a tooltip); the reader shows them as chips.
- **Today:** In memory. `toggle_label()` and `open_label_palette()` in `src/app/mail.rs` change `Thread::labels`. If every target already has the label it is removed, otherwise it is added to all of them. There is no undo.
- **Backend needs:** Change labels on a thread and send the change to the server. Gmail: `threads.modify` (`addLabelIds`, `removeLabelIds`). IMAP: `UID COPY` or `UID MOVE` to a folder, or `STORE +X-GM-LABELS` on Gmail IMAP.
- **Notes:** Use the optimistic-update-and-rollback pattern (see STA-2). Labels are per thread here. Gmail is per message in some views, so the mapping must be chosen (see DAT-2).

### FOL-6 Create, rename, recolour and delete labels

- **Feature:** UI not built yet. Labels are fixed.
- **Today:** Not implemented. The `labels` list is read once from JSON.
- **Backend needs:** Label CRUD on the server. Gmail: `labels.create`, `labels.patch`, `labels.delete`. IMAP: `CREATE`, `RENAME`, `DELETE` for folders.
- **Notes:** Deleting a label removes it from all threads. The backend must decide what happens to threads that use it.

### FOL-7 Report spam and mark as not spam

- **Feature:** UI not built yet. Spam is only reachable by navigating to it. Nothing moves a thread to or from Spam.
- **Today:** Not implemented as a button or a shortcut. Archive (`e`) in Spam moves the thread to Inbox, which is the "not spam" move. Delete moves a Spam thread to Trash. Nothing moves a thread into Spam.
- **Backend needs:** A "report spam" and a "not spam" action. Gmail: `messages.modify` with `SPAM` added or removed. IMAP: move to the Junk folder.
- **Notes:** Mapping Gmail's spam label to the Spam folder is done at sync time (FOL-1).

### FOL-8 Empty Trash and permanent delete

- **Feature:** UI not built yet. Delete moves threads to Trash. Nothing removes them for good, and Trash has no bulk action.
- **Today:** Not implemented. `delete()` in `src/app.rs` moves threads to Trash and never removes them. Trash has no action, so Delete does nothing there.
- **Backend needs:** Permanent delete and empty Trash. Gmail: `threads.delete` or `threads.trash`, and the 30-day auto-purge. IMAP: `EXPUNGE` after `\Deleted`.
- **Notes:** Permanent delete is not reversible. It needs a confirmation step in the UI.

---

## 3. Thread list (LST)

### LST-1 Thread rows

- **Feature:** Each row shows: the sender's name (or "To: name" in Sent and Drafts), a message count if there is more than one, an attachment icon, a star, the date, the subject, label dots and the snippet of the last message. A compact density (View › Compact List, or the palette) puts sender, subject and date on one line.
- **Today:** Placeholder JSON. `render_thread_row()` in `src/ui/thread_list.rs` builds rows from `MailData::threads`. The snippet is `Message::snippet` from the JSON, cleaned by `clean_snippet()` in `src/data.rs` (a leading heading or bold first line is dropped). The density is in memory only (PER-3). Date text comes from `list_date()` in `src/data.rs`.
- **Backend needs:** Thread list with: subject, participants, message count, last date, snippet, unread flag, starred flag, attachment flag, labels. Gmail: `threads.list` plus `threads.get` with metadata format. IMAP: thread the messages (`THREAD` or by `References`) and fetch headers.
- **Notes:** The snippet is currently taken from the last message. The backend should send the same thing, or the row will change after sync.

### LST-2 Unread styling

- **Feature:** Unread rows have a blue dot, bold sender and subject, and brighter text.
- **Today:** Placeholder JSON. `Thread::unread` from JSON. Flipped by `select_thread()`, `mark_read()` and `mark_unread()` in `src/app.rs`.
- **Backend needs:** A server-backed read state. Gmail: the `UNREAD` label on the thread. IMAP: `\Seen` on each message, with thread-level state derived from it.
- **Notes:** See ACT-4, ACT-5 and DAT-3 for thread-level versus message-level state.

### LST-3 Sort order

- **Feature:** The list is newest first, by date of the last message.
- **Today:** In memory. `MailData::load()` sorts threads once. `update_threads()` in `src/app.rs` sorts after every change.
- **Backend needs:** Sorting done on the server, with a stable cursor for paging (see LST-7). Local ordering must match the server.
- **Notes:** `update_threads()` re-sorts all threads on each change. This will be slow with a large mailbox.

### LST-4 Archived section in the Inbox

- **Feature:** The Inbox list ends with a collapsible "Archived" header and a count. Expanding shows the archived threads. Shortcut: none (`ToggleArchivedSection` is in the palette as "Show / Hide Archived in Inbox").
- **Today:** Placeholder JSON. `Row::ArchivedHeader` built in `refresh_rows()` in `src/app.rs`. The expanded state `archived_expanded` is in memory only.
- **Backend needs:** The Inbox query must include archived threads, or the list must fetch them separately. The archived count must come from the server.
- **Notes:** This is a view choice. The backend only needs to provide threads with their folder. The count is per FOL-2.

### LST-5 Keyboard list navigation

- **Feature:** `j` and `k`, or `↓` and `↑`, move the selection. `Enter` or `o` opens the thread.
- **Today:** In memory. `select_next()`, `select_prev()` and `select_thread()` in `src/app.rs`.
- **Backend needs:** When a selected thread is not loaded yet (after paging or a sync gap), the backend must load its messages on demand. See RDR-6.
- **Notes:** Selecting a thread marks it read at once (RDR-1).

### LST-6 Unread count in the list header

- **Feature:** The header shows "N unread" for the threads visible in the current view.
- **Today:** Computed from loaded rows in `render_thread_list()` in `src/ui/thread_list.rs`.
- **Backend needs:** The count comes from the server (see FOL-2). It should match the sidebar.
- **Notes:** Right now the header and the sidebar count use the same loaded data, so they agree. They will drift apart once paging exists.

### LST-7 Pagination and lazy loading

- **Feature:** UI not built yet. The list is virtualised (only visible rows are drawn), but all threads are loaded at startup. There is no "load more" and no scroll-to-load.
- **Today:** All 124 threads from JSON are in memory. `ListState` in `src/app.rs` is virtualised, but the data is not paged.
- **Backend needs:** Cursor-based paging for each view (Gmail `pageToken`, IMAP ranges by UID). Pages are requested as the user scrolls. Only the visible and recently viewed threads are kept in memory.
- **Notes:** Search, counts, contacts (CON-1) and palette thread search (PAL-2) all assume full data today. Each needs a server query when paging is added.

### LST-8 Live refresh and new mail in the list

- **Feature:** UI not built yet. New mail never appears while the app is open.
- **Today:** Not implemented.
- **Backend needs:** A sync loop (push or polling). Gmail: `history.list` or Pub/Sub watch. IMAP: IDLE or polling. New threads must be inserted without losing the selection or scroll position. `refresh_rows(true)` in `src/app.rs` already splices rows and can be reused.
- **Notes:** Links to NTF-1 and STA-5.

### LST-9 Empty-result text

- **Feature:** The list shows "Nothing in <view>" when the view is empty. When searching, it shows "No results in <scope>" with a hint.
- **Today:** Display only. `render_thread_list()` in `src/ui/thread_list.rs` chooses the text from loaded rows.
- **Backend needs:** Empty text must not show while a fetch is still loading. The backend needs to say whether a list is loaded, loading or empty (see STA-1).
- **Notes:** With lazy loading, "Nothing" could show before data arrives.

---

## 4. Reading pane (RDR)

### RDR-1 Opening a thread marks it read

- **Feature:** Selecting a thread marks it as read at once. There is no delay and no setting.
- **Today:** In memory. `select_thread()` in `src/app.rs` sets `unread = false`.
- **Backend needs:** Send the read change to the server. Gmail: `threads.modify` removes `UNREAD`. IMAP: `STORE +FLAGS (\Seen)` on the thread's messages.
- **Notes:** Opening a Drafts thread also clears `unread` (in memory). The change must be sent or ignored for drafts. Keyboard navigation through the list marks threads read as well. Consider a delay, or a setting, as in many clients.

### RDR-2 Conversation view

- **Feature:** The reader shows the subject, a label chip row, the message count ("1 message" or "N messages") and every message in order, oldest first. The newest message is always expanded.
- **Today:** Placeholder JSON. `render_conversation()` in `src/ui/reader.rs` reads `Thread::messages`.
- **Backend needs:** All messages of a thread, with their order, read flags and headers. Gmail: `threads.get` with full format. IMAP: fetch by thread or by `References`.
- **Notes:** Messages are sorted by `date` after loading (`MailData::load()`). Order depends on the date header, which can be wrong. Decide the rule on the backend.

### RDR-3 Expand and collapse older messages

- **Feature:** Older messages show as one row (sender, snippet, date). Click to expand. Expanded state is kept until another thread is selected.
- **Today:** In memory. `expanded_messages` and `toggle_message()` in `src/app.rs`. Cleared in `select_thread()`.
- **Backend needs:** Nothing beyond RDR-2. Expanded state can stay in the UI. It could be saved per thread (see PER-3).
- **Notes:** Collapsed rows show `Message::snippet`. The backend must supply snippets for each message, not only for the thread.

### RDR-4 Message header

- **Feature:** Each message header shows the sender's name, email, a "to … · cc …" line, and the full date (for example "Sun, Oct 25, 9:12 AM").
- **Today:** Placeholder JSON. `render_message()` in `src/ui/reader.rs`. Dates come from `full_date()` in `src/data.rs`, converted to local time. Bcc is not shown.
- **Backend needs:** Header fields per message: From, To, Cc, Bcc (sent items only), Date with timezone, Subject. Gmail: `messages.get` with metadata headers. IMAP: `ENVELOPE` and `BODY[HEADER]`.
- **Notes:** Long recipient lists are shown in full. The backend does not need to truncate them.

### RDR-5 Message body rendering

- **Feature:** The HTML body is shown natively: headings, lists, quotes, tables, links, bold, italic, code and simple colours. Scripts, styles, iframes and form controls (inputs, buttons, selects) are dropped. Images show a placeholder with their alt text.
- **Today:** Placeholder JSON. `Message::html` from JSON. Parsed by `html::parse()` in `src/html.rs` and drawn by `src/ui/html_view.rs`. Parsed bodies are cached by hash in `HtmlCache` (`src/html.rs`).
- **Backend needs:** The full decoded HTML body (or plain text when there is no HTML), with charset decoded. For `multipart/alternative`, pick the HTML part. For `cid:` references, see ATT-2.
- **Notes:** Parsing covers only the tags listed in `html.rs`. Unknown tags keep their text. Keep this in mind when comparing with the email client's own rendering.

### RDR-6 Fetch a message body on demand

- **Feature:** UI not built yet. All bodies are in memory. A thread opened before its body is fetched would show nothing.
- **Today:** Not implemented. `Message::html` is always present in the JSON.
- **Backend needs:** Fetch the body of a message when it is opened or expanded. Show a loading state (see STA-1). Keep recently read bodies in the local store (see PER-1).
- **Notes:** Thread lists should not need bodies. Keep metadata and bodies separate, so large mailboxes stay fast.

### RDR-7 Message navigation and reader focus

- **Feature:** `n` and `p` move between messages in the open conversation and expand the one they reach. `o` or `Enter` moves focus into the reader. `u` or `Esc` returns focus to the list. `↓` and `↑` scroll the reader.
- **Today:** In memory. `step_message()`, `next_message()`, `prev_message()`, `open_thread()`, `back_to_list()`, `scroll_reader()` in `src/app.rs`. Focus is `reader_focused`.
- **Backend needs:** Nothing beyond RDR-2 and RDR-6.
- **Notes:** Pure UI. Listed so that message loading (RDR-6) is taken into account when `n` and `p` move onto a message that has not loaded.

---

## 5. Message and thread actions (ACT)

Each of these acts on the checked threads when any are checked (ACT-6), otherwise on the selected thread (`targets()` in `src/app/mail.rs`). Each one currently changes only `data.threads` in memory. Hovering a list row also shows Archive, Delete, Mark read/unread and Star buttons in place of its date, and right-clicking a row opens a native menu with the same actions plus Reply, Forward, Move to and Label. The reader toolbar, the keyboard shortcut and the palette entry all dispatch the same action (see `src/actions.rs`).

### ACT-1 Archive and move to Inbox

- **Feature:** Shortcut `e`. Toolbar button "Archive" (its tooltip changes to "Move to Inbox" for archived threads). Palette "Archive". Moves the selected thread to Archive, or back to Inbox if it is already in Archive. The selection moves to the next thread, or the previous one if there is none after it.
- **Today:** In memory. `archive()` and `move_selected()` in `src/app.rs`. The target comes from `Folder::archive_target()` in `src/data.rs`: Inbox goes to Archive; Archive, Spam and Trash go to Inbox; Sent and Drafts do nothing. The toolbar hides the Archive button in Sent and Drafts. The rule follows each thread's own folder, so it also applies in Starred and label views.
- **Backend needs:** Move the thread to Archive. Gmail: `threads.modify` with `removeLabelIds: ["INBOX"]`. IMAP: `UID MOVE` to the Archive folder. Moving back: `UID MOVE` to INBOX. Spam to Inbox is the "not spam" action (FOL-7), and Trash to Inbox is a restore.
- **Notes:** Rollback is needed if the server rejects the change (STA-2).

### ACT-2 Delete

- **Feature:** Shortcut `delete`, `backspace` or `#`. Toolbar button "Delete". Palette "Delete". Moves the thread to Trash, then selects the next thread. A toast offers Undo (UND-3).
- **Today:** In memory. `delete()` in `src/app.rs`. Delete moves a thread to Trash from every folder except Trash and Drafts. In Trash it does nothing. In Drafts it discards the draft through the same path as the Discard button (DRF-4), with its Undo toast, then moves to the neighbor.
- **Backend needs:** Move to Trash. Gmail: `threads.trash`. IMAP: `UID MOVE` to the Trash folder. Discarding a draft is DRF-4.
- **Notes:** Delete in Trash does nothing for now. The backend should decide whether it means a permanent delete (see FOL-8). The toolbar hides the Delete button in Trash.

### ACT-1a Move to a folder

- **Feature:** `v`, the palette's "Move to…", Message › Move to…, the toolbar's "…" menu or a row's right-click menu move the selected (or checked) threads to Inbox, Archive, Spam or Trash. `v` opens the palette scoped to folders, without the current one. A toast offers Undo.
- **Today:** In memory. `move_to()`, `move_threads()` and `open_move_palette()` in `src/app/mail.rs` set `Thread::folder`.
- **Backend needs:** The same moves as ACT-1 and ACT-2. Moving to Spam is "report spam" (FOL-7).
- **Notes:** Drafts are skipped when moving; Sent threads can be moved.

### ACT-3 Star and unstar

- **Feature:** Shortcut `s`. Toolbar button "Star" or "Unstar" (filled star when starred). Palette "Star / Unstar". A star icon appears on the list row. In the Starred view, unstarring removes the thread from the list at once.
- **Today:** In memory. `toggle_star()` in `src/app.rs`. The flag is `Thread::starred`.
- **Backend needs:** Gmail: `threads.modify` adds or removes `STARRED`. IMAP: `\Flagged` on messages, with thread-level star derived from it.
- **Notes:** Starring a thread in Spam or Trash is allowed. The Starred view hides those threads.

### ACT-4 Mark as unread

- **Feature:** Shortcut `shift-u`. Toolbar button "Mark as unread". Palette "Mark as Unread". The selected thread shows its unread dot. The selection stays on the thread.
- **Today:** In memory. `mark_unread()` in `src/app.rs`.
- **Backend needs:** Gmail: `threads.modify` adds `UNREAD`. IMAP: remove `\Seen` from the messages.
- **Notes:** Thread-level here. The backend must decide whether it marks the last message or every message. Opening the thread again marks it read (RDR-1).

### ACT-5 Mark as read

- **Feature:** Shortcut `shift-i`. Palette "Mark as Read". There is no toolbar button.
- **Today:** In memory. `mark_read()` in `src/app.rs`.
- **Backend needs:** Gmail: `threads.modify` removes `UNREAD`. IMAP: add `\Seen`.
- **Notes:** The shortcut is listed in the shortcuts sheet but has no toolbar button. The toolbar's "Mark as unread" button has no matching "Mark as read" button.

### ACT-6 Bulk select and bulk actions

- **Feature:** `x` checks the selected thread; `shift-j`/`shift-k` (or `shift-↓`/`shift-↑`) extend the check; `cmd`-click toggles a row and `shift`-click checks a range; the checkbox in a row's inset (shown on hover) toggles it. `Esc` clears. While threads are checked the reading pane shows "N conversations selected" with their subjects and Archive, Delete, Mark as Unread/Read, Star, Move to and Label buttons, and every action (keys, palette, menus) applies to all of them. There is no select-all.
- **Today:** In memory. `PigeonApp::checked` (thread IDs) and the selection functions in `src/app/mail.rs`; `render_selection_summary()` in `src/ui/reader.rs`. Each action loops over the targets.
- **Backend needs:** Batch operations. Gmail: `messages.batchModify` (up to 1000 message IDs per call). IMAP: `UID` sets in one command.
- **Notes:** With bulk actions, one failure can leave some threads changed and others not. The rollback rules (STA-2) must handle partial results.

---

## 6. Compose and sending (CMP)

Drafts are described in section 7. Undo of a send is described in section 8.

### CMP-1 New message

- **Feature:** Shortcut `c`. Toolbar: none on the reader toolbar. Palette "Compose New Message". Opens an empty draft in the reading pane, with the title "New Message".
- **Today:** Creates a `Composer` (`src/ui/composer.rs`) through `compose()` in `src/app.rs`. The draft is in memory until it is saved (DRF-1).
- **Backend needs:** A new draft on the server, created on first save (see DRF-6). Account choice when there are several (ACC-4).
- **Notes:** No signature is added. Signatures are not built.

### CMP-2 Reply

- **Feature:** Shortcut `r`. Toolbar button "Reply". Palette "Reply". Opens a draft under the conversation. Subject is "Re: " plus the thread subject, unless it already starts with "re:".
- **Today:** Recipients are derived from the thread's last message in `reply_content()` in `src/app.rs`. If the last message is from the user, the reply goes to its recipients. Otherwise it goes to its sender. The reply is marked with `reply_to`, and the sent reply is appended to that thread.
- **Backend needs:** Send with threading headers (`In-Reply-To`, `References`), so the reply joins the conversation on other clients. See DAT-4.
- **Notes:** Reply uses only the last message's sender. It ignores earlier participants.

### CMP-3 Reply all

- **Feature:** Shortcut `a`. Toolbar button "Reply all". Palette "Reply All". To: the sender and the recipients of the last message, minus the user's address. Cc: the last message's Cc, minus anyone already in To.
- **Today:** `reply_content()` in `src/app.rs`. Duplicates are removed by email (case-insensitive).
- **Backend needs:** The same as CMP-2. Also: `Reply-To` header support, if the sender sets one. Today it is ignored.
- **Notes:** Mailing list addresses and no-reply senders are included as they are.

### CMP-4 Forward

- **Feature:** Shortcut `f`. Toolbar button "Forward". Palette "Forward". Subject "Fwd: " plus the thread subject, unless it already starts with "fwd:" or "fw:". The original message is quoted under the note area, with From, Date, Subject and To lines.
- **Today:** `reply_content()` and `Quoted::new()` in `src/ui/composer.rs`. A forward has no `reply_to`, so the draft opens alone in the reading pane, and sending it creates a new thread in Sent with the "Fwd:" subject and the quoted original in the body (see CMP-11).
- **Backend needs:** A forward must be a new message with its own headers, not a reply to the original (no `In-Reply-To`). Include the original's attachments (ATT-7).
- **Notes:** The backend must not thread a forward with the original. Gmail: the new message gets no `threadId`. IMAP: no `In-Reply-To` or `References` from the original.

### CMP-5 Recipient fields

- **Feature:** To, Cc and Bcc fields show addresses as chips. Typing a comma, semicolon or Enter turns the text into a chip. An unfinished address becomes a chip when the draft is sent. Tab moves between fields. Backspace on an empty field removes the last chip. A chip with no `@` is shown in the danger colour.
- **Today:** `Recipients` and `commit()` in `src/ui/composer.rs`. Address parsing is `parse_address()`: `Name <addr>` or a bare address with an `@` and a dot in the domain.
- **Backend needs:** Server-side address validation at send time. The server's rejection must reach the user (CMP-12).
- **Notes:** Parsing is a local check only. A valid-looking address can still be rejected by the server.

### CMP-6 Show Cc and Bcc

- **Feature:** Shortcut `cmd-shift-c`. Link "Cc Bcc" next to the To field. Palette "Show Cc and Bcc". Shows the Cc and Bcc fields.
- **Today:** `show_cc_bcc()` in `src/ui/composer.rs`. Bcc content is in `DraftContent::bcc`.
- **Backend needs:** Bcc must be sent in the envelope only, not in the message headers. Gmail: the `raw` message is built without Bcc. SMTP: `RCPT TO` for Bcc recipients.
- **Notes:** The Bcc addresses are kept on the saved draft and on the sent message, but the reader does not show them (see RDR-4).

### CMP-7 Subject and formatted body

- **Feature:** Subject field (not shown for replies, which keep the thread's subject). The body is written in markdown and styled as you type (bold, italic, strikethrough, `<u>underline</u>`, code, headings, lists, quotes, links); the syntax stays visible, dimmed. A formatting bar above the send row and Gmail's shortcuts (`cmd-b`, `cmd-i`, `cmd-u`, `cmd-shift-x`, `cmd-e` code, `cmd-k` link, `cmd-shift-7` / `8` / `9` numbered list, bulleted list, quote, `cmd-\` clear formatting) wrap or unwrap the selection. Enter continues a list, Tab and Shift-Tab indent it. Elsewhere `Tab` moves focus between fields.
- **Today:** `Composer` in `src/ui/composer.rs`, with the editor in `src/ui/rich_editor.rs` and the markdown helpers in `src/markdown.rs`. On send and on saving a draft the body becomes HTML with `markdown::to_html()` (`DraftContent::body_html()`); forward quotes are added with `html()` in `Composer`. The message's `html` is that HTML, so the reader shows it formatted.
- **Backend needs:** Build a MIME message: `text/plain` and `text/html` in `multipart/alternative`. The HTML part is `Message::html`. The plain part should be the markdown as typed (readable as is) or the HTML's text (`html::plain_text`); pick one and use it for the snippet too. Set `Content-Type` and encode non-ASCII subjects and names correctly.
- **Notes:** Raw HTML typed into the body is sent as text, except `<u>` and `</u>`. Images can't be inserted. Subject is stored on the thread, so a reply that changes it does not change the thread's subject.

### CMP-8 Send

- **Feature:** Shortcut `cmd-enter`, from anywhere in the draft or the list. Send button in the composer footer. Palette "Send". The draft closes, the message goes to Sent (or is added to the thread for a reply), and a "Message sent" toast appears with Undo for 5 seconds (see UND-1).
- **Today:** In memory. `send_message()` in `src/app.rs` builds the message with `message_from()` and changes `data.threads`. Nothing leaves the app.
- **Backend needs:** Send through the account's provider. Gmail: `messages.send` with the `raw` message (or `drafts.send`). IMAP: SMTP `MAIL FROM`, `RCPT TO`, `DATA`, plus the sent copy (CMP-13). The UI must not show "sent" until the server accepts the message. See the notes below and UND-1.
- **Notes:** The undo window (5 seconds, `TOAST_DURATION` in `src/app.rs`) means the real send can be held for that time. Otherwise undo can only cancel a message that has already gone. `message_from()` sets `date` to the local clock and `snippet` to the first 140 characters of the body's text (its HTML without tags, so no markdown syntax). The server should set the real date, and the snippet should come from the same text.

### CMP-9 Send validation

- **Feature:** Sending with no recipients shows "Add at least one recipient" under the To field and keeps the draft open. An address that isn't valid ("zzz") shows "“zzz” isn't an email address" under its field. Text still being typed in a recipient field counts: it becomes the best-matching contact, or the address as typed. Subject and body are not checked. Sending without a subject or body is allowed with no warning.
- **Today:** `send_message()` in `src/app.rs` calls `Composer::validate()` (`src/ui/composer.rs`) and checks `DraftContent::has_recipients()`.
- **Backend needs:** Server rules: at least one recipient, sender address allowed, message size limit, and rejection reasons (for example, invalid recipient, over quota).
- **Notes:** A subject warning and an "empty body" warning are not built (UI not built yet). They are not claimed as features.

### CMP-10 Forward quote block

- **Feature:** Forwarded messages appear under the note area in a block with a "Forwarded message" label and From, Date, Subject and To lines, then the original body.
- **Today:** `Quoted` in `src/ui/composer.rs` (display) and `html()` (saved HTML wrapped in `<blockquote>`).
- **Backend needs:** The quoted original must be sent as HTML (or as attached message if the client chooses). The format must match what other clients expect.
- **Notes:** The quote is kept on the draft (`DraftOrigin::quoted` in `src/data.rs`), so a reopened forward draft shows the same quote block (see DRF-3).

### CMP-11 Reply appends to the thread

- **Feature:** A sent reply appears under the original conversation, not as a new thread. A sent forward starts a new thread in Sent (see CMP-4).
- **Today:** `send_message()` in `src/app.rs` pushes a reply into the thread found by `reply_to`. If that thread no longer exists, a new thread is made in Sent. A forward has no `reply_to`, so it always makes a new thread in Sent.
- **Backend needs:** Thread linking by server thread ID and headers. Gmail: `threadId` on the sent message. IMAP: `In-Reply-To` and `References`.
- **Notes:** A forward does not share the original thread. See CMP-4.

### CMP-12 Send failure feedback

- **Feature:** UI not built yet. When the server rejects a message, there is no feedback. A failed message must go back to the draft, with the error shown.
- **Today:** Not implemented. `Composer::set_error()` shows only the recipient error ("Add at least one recipient", or "“x” isn't an email address"). Nothing else can fail.
- **Backend needs:** Error results from send, with a reason the UI can show. A way to keep failed messages in an outbox (see PER-2). A retry action.
- **Notes:** The composer's `error` field and the footer error text already exist, so they can carry the server's message.

### CMP-13 Sent copy on the server

- **Feature:** Sent messages appear in the Sent folder, both here and on the server.
- **Today:** In memory only. The Sent thread is created in `send_message()`.
- **Backend needs:** Gmail: sent messages are added with the `SENT` label automatically. IMAP: `APPEND` the message to the Sent folder after SMTP, unless the server does it. Sync must then not show the same message twice.
- **Notes:** Deduplicate on `Message-ID` (DAT-4).

---

## 7. Drafts (DRF)

### DRF-1 Save on close or navigation

- **Feature:** A draft that has changed is saved to Drafts when it is closed (`Esc`, toolbar "Close and save draft", palette "Close Draft"), or when the user moves to another thread, folder or label. The toast "Saved to Drafts" appears without Undo. An unchanged draft is not saved.
- **Today:** In memory. `stash_composer()` and `save_draft()` in `src/app.rs`. Drafts are `Thread` values in `data.threads`, with one message each. The ID is `draft-local-N` (DAT-5).
- **Backend needs:** Save the draft to the server: Gmail `drafts.create` or `drafts.update`, IMAP `APPEND` to Drafts and delete the old copy. Store the server draft ID on the local draft.
- **Notes:** Saving happens on close, not while typing (DRF-5). Closing the app loses unsaved changes, because nothing is written to disk (PER-2).

### DRF-2 Drafts list

- **Feature:** The Drafts folder lists all saved drafts. Each row shows "To: <first recipient>". The sidebar shows the number of drafts.
- **Today:** Placeholder JSON for the starting drafts (6 in `mailbox.json`). New drafts are in memory.
- **Backend needs:** Drafts from the server, with recipients and subject. Gmail: `drafts.list`. IMAP: the Drafts folder.
- **Notes:** Drafts without recipients show "(no recipients)". Their subject can be empty and shows "(no subject)".

### DRF-3 Open and edit a draft

- **Feature:** Selecting a Drafts thread opens it in the composer, with the title "Draft". Edits follow DRF-1.
- **Today:** `open_draft()` in `src/app.rs` rebuilds the composer from the stored message. A draft saved here keeps its body as typed, in markdown, on `DraftOrigin::markdown` (`src/data.rs`), and reopens with it; the message's `html` is made from it. A draft from the JSON has no markdown, so its body is taken from its HTML as plain text (`html::plain_text`). The draft's kind, reply target and forwarded quote are kept in `DraftOrigin` on the thread (`src/data.rs`), so reopening restores the same kind and quote block. A forward draft reopens as a forward, and sending it creates a new thread (CMP-4).
- **Backend needs:** Fetch the draft's full content, including attachments and headers, when it is opened. Store the markdown with the draft so formatting survives reopening: for example as the `text/plain` part, or a custom header or part that other clients ignore. Drafts written in other clients have only HTML and reopen as plain text, losing their formatting, unless HTML is converted to markdown.
- **Notes:** Replying again to a thread that has a reply draft reopens that draft instead of starting a second one (`reply_draft_of()` in `src/app.rs`); the backend must keep the draft's link to its thread. Attachments on drafts keep only their metadata (ATT-6). The HTML of drafts written elsewhere is lost on reopening (see above). The forwarded quote is not in the draft's body, so draft search does not match text that only appears in the quote. The backend's draft storage should keep the quote as a separate part, as the app does now.

### DRF-4 Discard a draft

- **Feature:** Shortcut `cmd-shift-d`. Toolbar button "Discard draft" (trash icon). Palette "Discard Draft". The draft is removed and the toast "Draft discarded" appears with Undo. A new unchanged draft is discarded with no toast.
- **Today:** In memory. `discard_draft()` in `src/app.rs` removes the Drafts thread from `data.threads`.
- **Backend needs:** Delete the server draft. Gmail: `drafts.delete`. IMAP: `\Deleted` and `EXPUNGE` in Drafts. Undo must restore it, or recreate it, before the window ends (UND-2).
- **Notes:** The discard is sent at once. Undo may need the draft to be restored from a copy the app keeps for the undo window.

### DRF-5 Save while typing

- **Feature:** UI not built yet. Drafts are saved on close or navigation only. A long draft has no periodic save, so a crash loses everything typed since the last save.
- **Today:** Not implemented.
- **Backend needs:** Periodic or debounced saves to local store (PER-1) and to the server (DRF-6). Show a "Saving…" and "Saved" state.
- **Notes:** A debounce (for example 2 seconds after typing stops) is a common choice.

### DRF-6 Sync drafts with the server

- **Feature:** Drafts created on one device appear on others. Changes made elsewhere are seen here.
- **Today:** Not implemented. Drafts exist only in memory (DRF-1).
- **Backend needs:** Two-way sync of drafts. Conflicts (the same draft edited in two places) need a rule: last write wins, or keep both as copies.
- **Notes:** Use the server's draft ID to find drafts on sync. Local IDs (`draft-local-N`) must be replaced by server IDs after the first save (DAT-5).

---

## 8. Undo (UND)

### UND-1 Undo send

- **Feature:** After Send, a toast shows "Message sent" and an Undo button (shortcut `cmd-z`, `z`, or the palette's "Undo"). Undo takes the message back out of Sent (or removes the appended message from the reply thread), restores the draft it was sent from, and reopens the composer. The toast disappears after 5 seconds.
- **Today:** In memory. `send_message()` and `undo()` in `src/app.rs`. `UndoAction::Send` holds the thread ID, whether it was appended, the removed draft and the composer.
- **Backend needs:** Hold the message in an outbox for the undo window before it is sent. Undo cancels it before dispatch and restores the draft on the server. After the window, the message is sent and undo is no longer available.
- **Notes:** The 5-second window (`TOAST_DURATION`) is a UI choice. The backend's hold time must be at least as long, or the UI must not offer Undo. Once sent, undo cannot recall it (recall is not built).

### UND-2 Undo discard

- **Feature:** After Discard, a toast shows "Draft discarded" and Undo for 5 seconds. Undo puts the draft back in Drafts and reopens it.
- **Today:** In memory. `discard_draft()` and `undo()` in `src/app.rs`. `UndoAction::Discard`.
- **Backend needs:** The server delete (DRF-4) must be delayed until the undo window ends, or the draft must be recreated on undo.
- **Notes:** A new draft that was never saved has nothing on the server, so undo only restores the composer.

### UND-3 Undo for other actions

- **Feature:** Archive, Delete and Move to (one thread or many) show a toast ("Archived", "Moved 3 conversations to Trash") with Undo for 5 seconds. Undo is `z`, `cmd-z`, the toast's button or the palette's "Undo". It puts every thread back in its folder and reselects the thread that was selected. Star, Mark read or unread and labels have no undo.
- **Today:** In memory. `move_threads()` creates `UndoAction::Move { moves, selected }` and `undo_move()` reverses it, in `src/app/mail.rs`.
- **Backend needs:** A reversible action log: each change keeps its inverse until the undo window passes, and the server call is queued for the same delay.
- **Notes:** Multiple undos (undo stack) are not built.

---

## 9. Search (SRC)

Search is in memory. Each thread gets a `SearchIndex` from `Thread::reindex()` in `src/data.rs`. It holds the lowercased subject, sender names and addresses, recipient names and addresses (to, cc, bcc), snippets, the plain-text body of every message, and attachment file names. Matching is done in `src/search.rs` (`Query::matches`).

### SRC-1 Search bar and keys

- **Feature:** Shortcut `/` or `cmd-f`, or the search icon in the list header, opens a field at the top of the list. `Esc` clears the search and closes it. `Enter` moves focus into the results. While the field is empty, a dropdown lists the operators and the last 5 searches; while a `from:` or `to:` value is typed, it lists matching contacts. `↓`/`↑` move through it and `enter` takes one. A chip in the field shows the scope ("Inbox" or "All mail") and toggles it. Under the field, the recognized operators show as chips next to the result count.
- **Today:** In memory. `toggle_search()`, `close_search()`, `focus_results()` in `src/app.rs`; `search_suggestions()`, `remember_search()`, `toggle_search_scope()` in `src/app/mail.rs`. `render_search_bar()` and `render_search_suggestions()` in `src/ui/thread_list.rs`. Recent searches are kept in memory only (PER-3).
- **Backend needs:** Search requests to the server, or to the local index (PER-1), with the results shown in the list. The list needs a "searching" state (STA-1).
- **Notes:** Pure UI parts (focus, keys) need no backend.

### SRC-2 Default search scope

- **Feature:** Without `in:`, a search covers the current view. In the Inbox it also covers archived threads.
- **Today:** In memory. `refresh_rows()` in `src/app.rs` uses `nav_contains()` and the Inbox+Archive rule.
- **Backend needs:** Search scoped to a folder or label, with the Inbox scope including Archive.
- **Notes:** A search in Sent, Drafts, Spam or Trash uses that folder only.

### SRC-3 Search operators

- **Feature:** `from:`, `to:`, `subject:`, `label:`, `is:unread`, `is:starred`, `has:attachment` (or `has:attachments`). Quoted phrases such as `from:"Sarah Chen"` are supported. Terms are combined with AND.
- **Today:** In memory. `Query::parse()` and `Term` in `src/search.rs`. Values are compared as lowercase substrings of the index (`Query::matches`). `label:` matches on the label ID or name with letters and digits only.
- **Backend needs:** Translate the query to the server's search language. Gmail: `q` string (`from:`, `to:`, `subject:`, `label:`, `is:unread`, `is:starred`, `has:attachment`). IMAP: `SEARCH` with `FROM`, `TO`, `SUBJECT`, `UNSEEN`, `FLAGGED`. Keep result behaviour the same, or document the differences.
- **Notes:** Gmail matches words, not substrings. A substring match in local search will return different results from the server, so the same query can give two lists.

### SRC-4 Scope operator `in:`

- **Feature:** `in:inbox`, `in:archive` (or `in:archived`), `in:sent`, `in:drafts` (or `in:draft`), `in:spam`, `in:trash`, `in:anywhere`, `in:all`, `in:starred`, or `in:<label>`.
- **Today:** In memory. `Scope` in `src/search.rs`. `in:all` covers everything except Spam and Trash. `in:anywhere` covers everything. `in:starred` and `in:<label>` add a term and use scope All.
- **Backend needs:** Server scoping for each folder and for "all mail" (Gmail: no `in:` filter, plus the `-in:spam -in:trash` exclusion, or `in:anywhere`).
- **Notes:** The meaning of "all" must match the backend's definition. Currently it is "everything except Spam and Trash".

### SRC-5 Text matching

- **Feature:** A word that is not an operator matches the thread's text index: subject, names, addresses, snippets, body text and attachment names.
- **Today:** In memory. `Term::Text` in `src/search.rs`, against `SearchIndex::all`.
- **Backend needs:** Full-text search over message bodies (not only snippets) and attachment names. Gmail: the built-in index. IMAP: `TEXT` search (slow on large folders). A local index (PER-1) if offline search is needed.
- **Notes:** Body text is only in memory today. A server or local index is needed once bodies are not all loaded (RDR-6).

### SRC-6 Result count

- **Feature:** Above the results: "N results in <scope>" (or "N result" for one).
- **Today:** Count of `rows`, in memory. `render_thread_list()` in `src/ui/thread_list.rs`.
- **Backend needs:** Total count from the server. Gmail: `resultSizeEstimate` is an estimate and is not exact. IMAP: count of `SEARCH` results.
- **Notes:** Gmail's estimate can change between pages. Show "about N" or a loading state if it is used.

### SRC-7 Full-mailbox search

- **Feature:** `in:all` and `in:anywhere` search the whole mailbox. Today, the whole mailbox is loaded, so these are complete.
- **Today:** In memory, over all loaded threads (the full JSON).
- **Backend needs:** Server search over all folders, including mail that is not yet synced locally. Show when a result set is partial (STA-3).
- **Notes:** This is the main reason search must move to the server once paging (LST-7) exists.

### SRC-8 Live search while typing

- **Feature:** Results update on every keystroke. There is no debounce and no cancel.
- **Today:** In memory. `on_search_event()` in `src/app.rs` runs `Query::parse()` and `refresh_rows()` on each `InputEvent::Change`.
- **Backend needs:** Debounced requests, with stale responses ignored. A cancel on each new query.
- **Notes:** Local search is instant, so the current design works now. Server search needs the debounce.

---

## 10. Contacts and autocomplete (CON)

### CON-1 Recipient suggestions

- **Feature:** Typing in a recipient field, or in an event's "Add guests" field, shows up to six suggestions. Names and addresses that start with the query come first, then those with a word that starts with it ("ch" finds Sarah Chen), then, for queries of three letters or more, those that contain it. Only when nothing matches is the query matched fuzzily. Automated senders (no-reply, notification, newsletter, alerts, receipts and similar addresses) and spam senders are never suggested. Already-added recipients are excluded. Up and down arrows, `Ctrl-n` and `Ctrl-p` move through them. Enter or a click picks one. Escape closes the list.
- **Today:** Placeholder data. `ContactBook::new(data.contacts())` in `PigeonApp::new()` in `src/app.rs`. `MailData::contacts()` in `src/data.rs` collects every sender and recipient in the JSON outside Spam, except the user, sorted by how often they appear. `ContactBook::suggest()` and `is_automated()` in `src/ui/composer.rs` rank and filter; the composer and the event panel both call `suggest()`. The book is built once at startup.
- **Backend needs:** A contacts source. Gmail: People API (contacts and "other contacts"). CardDAV, or addresses taken from sent and received mail. The list must update when the user sends to a new address (today it does not, until restart).
- **Notes:** The book is `Rc<ContactBook>` and shared with every composer and the calendar. Changes must reach open composers. "Automated" is guessed from the address's local part; a real contacts source can say which addresses the user has written to, which is a better signal.

### CON-2 Typed name and address resolution

- **Feature:** A typed address is used as it is. A typed name followed by a comma or semicolon takes the best match from the book (`ContactBook::best()`, ranked as in CON-1). After a comma, a name with no match is dropped silently. On Enter, the highlighted suggestion is used when the text has no `@` or the user has moved through the list. Otherwise the typed address is used, or the field shows "“…” isn't an email address".
- **Today:** `resolve()` and `commit()` in `src/ui/composer.rs`. Names come from message headers in the JSON only.
- **Backend needs:** Lookup of a name in contacts (CON-1). Keep "dropped silently" out of the final behaviour: a name with no match should show an error or leave the text in the field.
- **Notes:** Address validation is local only (see CMP-5).

### CON-3 Contact management

- **Feature:** UI not built yet. Contacts cannot be viewed, added, edited or removed.
- **Today:** Not implemented.
- **Backend needs:** Contacts CRUD on the server (Gmail People API `createContact`, `updateContact`, `deleteContactPerson`).
- **Notes:** A contacts screen, or a "save contact" action, is needed for this to be useful.

---

## 11. Images and attachments (ATT)

### ATT-1 Remote images

- **Feature:** Images in email bodies show as a bordered box with an image icon and the alt text (or "Image" when there is no alt text). Images are never downloaded: GPUI here has no HTTP client (only `NullHttpClient`), so loading them needs a new dependency or the backend. Tiny tracking images (width or height 2 pixels or less) are dropped.
- **Today:** Placeholder. `Block::Image` in `src/html.rs` keeps only the `alt` text. The `src` attribute is ignored. `Renderer::block()` in `src/ui/html_view.rs` draws the box.
- **Backend needs:** A proxy or a fetch on user demand that loads remote images. Gmail and most clients have a "load remote images" setting. The fetch should go through the backend, so the user's IP and open tracking are not exposed. Cache the results.
- **Notes:** A setting to choose "always", "ask" or "never" is not built (UI not built yet). Tracking pixels are dropped by size only, which is a local rule that works today.

### ATT-2 Inline (`cid:`) images

- **Feature:** UI not built yet. Images embedded in the message (referenced as `cid:`) show as the placeholder box, the same as ATT-1.
- **Today:** Placeholder. `is_safe_url()` in `src/html.rs` does not allow `cid:`, and images are not drawn anyway.
- **Backend needs:** Read the MIME part with the matching `Content-ID`, and deliver it as a data source or a file. Gmail: the part is in `payload.parts`. IMAP: `BODYSTRUCTURE` and `BODY[part]`.
- **Notes:** The HTML renderer needs to know about the data source once images are drawn (see ATT-1).

### ATT-3 Attachment list in messages

- **Feature:** Under each message body, attachments appear as chips: an icon for the file type (image, PDF/document, spreadsheet, archive, other), name and size (for example "393 KB" or "5.6 MB").
- **Today:** Placeholder JSON. `Message::attachments` (`Attachment { name, size_bytes, mime }`) from `mailbox.json`. `render_message()` in `src/ui/reader.rs`. `file_size()` and `Attachment::kind()` (from the MIME type, then the extension) in `src/data.rs`; `attachment_icon()` in `src/ui/mod.rs`.
- **Backend needs:** Attachment metadata per message: name, size, MIME type, and an ID to fetch it (Gmail: `attachmentId`, IMAP: the part number).
- **Notes:** The MIME type must be real for the icon to be right; a missing one falls back to the extension.

### ATT-4 Open and download an attachment

- **Feature:** The attachment chip is not clickable. Nothing opens or saves the file, although the chip looks like a button.
- **Today:** Display only. No handler is attached in `render_message()`. The chip is a plain tag, with no hover style, border or pointer cursor.
- **Backend needs:** Download the bytes on demand (Gmail: `messages.attachments.get`, IMAP: `BODY[part]`). Cache them locally. Then open with the system viewer or save with a file dialog. Show progress for large files.
- **Notes:** The chip should not look clickable until this exists. Until then it is display only.

### ATT-5 Attachment icon in the list

- **Feature:** A paperclip icon appears on a thread row when any message in the thread has an attachment.
- **Today:** Placeholder data. `Thread::has_attachments()` in `src/data.rs`.
- **Backend needs:** A thread-level "has attachments" flag from the server. Gmail: the `filename` on message parts. IMAP: `BODYSTRUCTURE`.
- **Notes:** `has_attachments()` counts any entry in `Message::attachments`, so inline parts count too if the backend sends them. Decide whether they should.

### ATT-6 Attach files to a new message

- **Feature:** The composer's paperclip button opens a file picker (several files at once), and files dropped on the composer are attached. Each shows as a chip with its type icon, name and size, and an × to remove it. Attachments stay with the draft (DRF-1) and appear on the sent message.
- **Today:** In memory. `attach_paths()` and the picker in `src/ui/composer.rs` build `Attachment::from_file()` (name, size and a MIME type guessed from the extension). Only the path's metadata is read; the bytes are never read or stored, so the sent copy only lists the files. `DraftContent::attachments` carries them to `message_from()` in `src/app.rs`.
- **Backend needs:** Read the files and upload them for a draft or a sent message. Gmail: `messages.send` with multipart (or resumable upload for large files, above about 5 MB). IMAP: MIME part in the message.
- **Notes:** Show upload progress and size limits (Gmail: 25 MB per message). Drafts need the same storage (DRF-6).

### ATT-7 Forward keeps attachments

- **Feature:** UI not built yet. Forwarding a message does not include its attachments.
- **Today:** Not implemented. `reply_content()` and `Quoted` in `src/ui/composer.rs` carry only the quoted HTML and header lines.
- **Backend needs:** Copy the original's attachments into the new message on send (Gmail: re-attach the same parts, IMAP: re-fetch and attach).
- **Notes:** Depends on ATT-4 for the bytes.

---

## 12. Links (LNK)

### LNK-1 Open a link in the browser

- **Feature:** Clicking a link in an email opens it in the default browser. Hovering shows the URL as a tooltip. Only `http`, `https`, `mailto` and `tel` links are clickable. Other links show as plain text.
- **Today:** Works without a backend. `cx.open_url()` in `Renderer::text()` in `src/ui/html_view.rs`, on an `InteractiveText`. Allowed schemes are set in `is_safe_url()` in `src/html.rs`.
- **Backend needs:** None for the current behaviour. Link safety (LNK-2) and redirect handling are backend-side if they are built.
- **Notes:** Links are opened with the URL exactly as it appears in the email, including tracking parameters and redirect wrappers.

### LNK-2 Link safety checks

- **Feature:** UI not built yet. There is no warning for suspicious links, no check that the visible text matches the URL, and no removal of tracking parameters.
- **Today:** Not implemented.
- **Backend needs:** A safe-link check service or local rules (for example, a known phishing list). Optional: unwrap redirect links and strip tracking parameters before opening.
- **Notes:** Checks that need a network call must not block the click for long. Use a warning in the UI.

### LNK-3 mailto: links

- **Feature:** UI not built yet. A `mailto:` link opens in the system's default mail handler, not in Pigeon's composer.
- **Today:** Passed to the system through `cx.open_url()` in `src/ui/html_view.rs`. Not handled by the composer.
- **Backend needs:** The composer must accept the link's address and parameters (`subject`, `body`, `cc`). Mostly UI work, but the account chosen for the new message is a backend decision (ACC-4).
- **Notes:** Until this exists, `mailto:` links leave the app.

---

## 13. Command palette (PAL)

### PAL-1 Command list

- **Feature:** `cmd-k` or `cmd-shift-p` opens the palette. It lists every command in `COMMANDS` (`src/actions.rs`), each with its shortcut when there is one. Commands run the same action handlers as the keyboard and toolbar. The palette does not run its own logic.
- **Today:** Works in memory. `open_palette()` and `palette_items()` in `src/app.rs`. `on_palette_event()` dispatches the action.
- **Backend needs:** Each command inherits the backend need of its action. The commands that need backend work are: Archive (ACT-1), Delete (ACT-2), Star (ACT-3), Mark as Unread and Read (ACT-4, ACT-5), Compose, Reply, Reply All, Forward (CMP-1 to CMP-4), Send (CMP-8), Close Draft (DRF-1), Discard Draft (DRF-4), Undo (UND-1 to UND-3), Move to (ACT-1a), Label (FOL-5), New Event (CAL-9), Delete Event (CAL-10). Navigation, view and app commands need no backend.
- **Notes:** Commands that can't run now (Reply with no thread selected, Undo with nothing to undo, invite answers on a non-invite) are left out (`When` in `src/actions.rs`, `command_available()` in `src/app/mail.rs`). The last commands run come first while the query is empty (in memory, PER-3). `v` and `l` open the palette scoped to folders or labels (ACT-1a, FOL-5). Commands are scoped: each `Group` in `src/actions.rs` has a `scope()` (Mail, Calendar or Global), and the palette and the `?` sheet only list the current mode's commands plus the global ones.

### PAL-2 Thread search in the palette

- **Feature:** Typing in the palette also finds threads. A thread matches on its subject or on the name of its last non-self sender, as a substring. Up to 30 threads are shown after the commands.
- **Today:** In memory, over all loaded threads. `update_matches()` in `src/ui/command_palette.rs`. Items come from `palette_items()` in `src/app.rs`.
- **Backend needs:** A search query (SRC-1 to SRC-5) with a result cap. The palette should not load every thread to build its list.
- **Notes:** Body text is not searched in the palette, unlike the search bar. Keep the two consistent, or note the difference.

### PAL-3 Label jumps

- **Feature:** The palette lists "Go to Label: <name>" for every label. `g l` opens the palette already filtered to labels.
- **Today:** In memory. `palette_items()` builds the entries from `MailData::labels`. `go_to_label()` opens with the query `label: `.
- **Backend needs:** Label list (FOL-3).
- **Notes:** Labels are fixed today. Creating a label (FOL-6) would add new entries here.

---

## 14. Calendar (CAL)

The calendar shares the window with mail. Its state lives in `CalendarView` (`src/ui/calendar/mod.rs`), an entity owned by `PigeonApp`. The model and the pure logic are in `src/calendar/` (`mod.rs`, `time.rs`, `recurrence.rs`, `layout.rs`).

### CAL-1 Switch between Mail and Calendar

- **Feature:** The rail's Mail and Calendar icons, `cmd-1` and `cmd-2`, and the palette's "Go to Mail" and "Go to Calendar" switch modes. Mail state (folder, selection, open thread, composer) is kept across switches.
- **Today:** Works locally. `PigeonApp::set_mode()` in `src/app.rs` sets `mode` and moves focus. The rail reads the mode in `src/ui/rail.rs`.
- **Backend needs:** None. Account linking (ACC-3) must ask for calendar access as well as mail.
- **Notes:** An account without calendar access needs an empty state in the calendar. That is not built.

### CAL-2 Calendar list, colours and visibility

- **Feature:** The header's calendars button opens a menu that shows and hides each calendar. The palette lists "Show Calendar: X" and "Hide Calendar: X". Events take their calendar's colour.
- **Today:** Placeholder JSON (`calendars` in `assets/calendar.json`), in memory. `CalendarView::toggle_calendar()`. `render_calendars_button()` in `src/ui/calendar/header.rs`.
- **Backend needs:** The account's calendar list (Google `calendarList`, or CalDAV collections) with name, colour and the user's "shown" setting. Save visibility changes to the server or locally (CAL-23).
- **Notes:** Shared and subscribed calendars (holidays, birthdays, a colleague's) are often read-only (CAL-22).

### CAL-3 Load events for the visible range

- **Feature:** Each view shows the events in its range: one day, a Monday-first week, or the month grid in whole weeks.
- **Today:** All events are in memory. `CalendarView::visible_range()` and `visible_occurrences()` filter them on every render.
- **Backend needs:** A range query (`timeMin` and `timeMax`) with a local cache, so going to the next or previous period is instant. Incremental sync (sync tokens or CalDAV `sync-collection`) to pick up changes made elsewhere.
- **Notes:** Prefetch the ranges on either side of the visible one. Recurring events must be returned once as a series, not as copies (CAL-5).

### CAL-4 Day, week and month views and navigation

- **Feature:** Day, Week (the default) and Month views. `d`, `w`, `m` switch views. `t` goes to today. `n` and `p` go to the next or previous period. `←`/`→` move the selected day (marked in the Week and Month headers), and the selection to the nearest event on it. `↓`/`↑`, `j`/`k` and `tab`/`shift-tab` go to the next or previous event in time, past the period shown. Clicking a day number opens that day. The time grid scrolls to about 8am on open. Overlapping events sit side by side in equal columns, widening into free ones; columns narrower than 60pt overlap a little instead. The Day view caps its event column at 640pt and, when there's room, shows a month to jump from and the day's events as a list beside it. Week numbers can be shown (View menu, palette).
- **Today:** Works locally. `set_view()`, `open_day()`, `step()`, `step_day()`, `select_adjacent()` in `src/ui/calendar/mod.rs`. Drawing is in `week_view.rs` (time grid, all-day lane, columns from `layout::pack_columns()` and `layout::column_box()`), `agenda.rs` (the Day view's side area), `mini_month.rs` and `month_view.rs` (pills, multi-day bars from `layout::pack_lanes()`, "+N more").
- **Backend needs:** None beyond CAL-3. Next and previous event look up to 62 days past the period shown, so that range must be cached or fetched.
- **Notes:** The week start (Monday) is fixed in `time::week_start()`. A user setting would need to be stored (CAL-23).

### CAL-4a Go to a date

- **Feature:** Clicking the header's title, `g d`, `cmd-shift-t`, the palette's "Go to Date…" or Go › Go to Date… opens a field over a small month. The field reads typed dates ("nov 12", "next fri", "tomorrow", "in 2 weeks", "dec 2027") and shows the date it reads; enter or a click on a day goes there.
- **Today:** Works locally. `toggle_date_picker()`, `go_to_date()` in `mod.rs`; `render_title()` in `header.rs`; parsing in `time::parse_date()`.
- **Backend needs:** None beyond CAL-3.
- **Notes:** Typed dates are English only.

### CAL-5 Recurring events

- **Feature:** Repeating events show on every matching day. The panel shows the rule ("Every week").
- **Today:** Placeholder JSON. `recurrence` is one keyword (`daily`, `weekdays`, `weekly`, `biweekly`, `monthly`, `yearly`). A series can also carry `exceptions` (original starts it skips, like `EXDATE`) and `until` (its end), both set locally by CAL-6; one occurrence changed on its own is an event of its own with `recurring_event_id` and `original_start` (like `RECURRENCE-ID`). `recurrence::occurrences()` expands only the visible range and honors exceptions and the end.
- **Backend needs:** Full RRULE support (`INTERVAL`, `BYDAY`, `UNTIL`, `COUNT`), `EXDATE`, and changed single occurrences (`RECURRENCE-ID`). Either expand on the server (Google `singleEvents=true`) and keep the series ID, or expand locally from the RRULE.
- **Notes:** The UI has no way to set or change a rule. New events never repeat.

### CAL-6 This event or all events

- **Feature:** Changing, dragging or deleting one occurrence of a repeating event asks which ones it's for. In the panel, the first change shows "Apply this change to: This event / All events" (the change shows on the whole series until answered; closing the panel without answering keeps it on all). Dropping a dragged occurrence asks "Move repeating event: This event / All events" over the grid, with the event drawn at its new time; enter takes "This event", escape cancels. Deleting asks "This event / This and following / All events", in the panel when it's open, over the grid otherwise. Undo restores any of them.
- **Today:** In memory. "This event" copies the occurrence into an event of its own (`CalendarView::detach()`) and adds its original start to the series' `exceptions`; "This and following" sets the series' `until` and drops its changed occurrences from then on; "All events" changes or deletes the series and its changed occurrences. `SeriesPrompt`, `answer_prompt()`, `delete_occurrence()`, `restore()` in `mod.rs`; `choose_edit_scope()` in `event_panel.rs`.
- **Backend needs:** The matching writes: an exception instance for one occurrence (Google: patch the instance; CalDAV: a `RECURRENCE-ID` override, or an `EXDATE` for a delete), a series split for "following" (end the series with `UNTIL`, plus a new series if the change continues), a series update for "all". Changed occurrences must keep their link to the series.
- **Notes:** "This and following" is offered for deletes only. Edits to the series after an occurrence was changed on its own don't reach that occurrence, as in Google Calendar.

### CAL-7 Edit an event in the panel

- **Feature:** Clicking an event, or `e`/`enter` on the selected one, opens the right-side panel. Title, calendar, location, video link, guests, notes, reminder and busy/free can be edited in place. Esc closes it.
- **Today:** In memory. `open_event()`, `close_panel()` in `mod.rs`. Fields are owned by `EventPanel` in `event_panel.rs`. Text fields apply on every change (`on_field_event()`).
- **Backend needs:** An event update (Google `events.patch`, or a CalDAV `PUT` with `If-Match`). Batch the writes: save when the panel closes or after a short pause, not on every keystroke.
- **Notes:** The panel applies changes with no Save button, so the backend must handle failed writes without losing the user's text (STA-2). Conflicts with edits made elsewhere need an ETag check.

### CAL-8 Date, time and all-day editing

- **Feature:** Start and end date and time fields with a date picker and a time picker. Typed dates and times are parsed ("tomorrow", "Oct 27", "10/27", "3pm", "15:30", "noon"). Up and down change the field by a day or 15 minutes. The all-day switch turns an event into whole days and back. Tab reaches every field and control of the panel in reading order (calendar picker, title, dates and times, all-day switch, location, link, guests, notes, reminder); enter or space work the switch and open the pickers, whose rows up, down and enter choose.
- **Today:** In memory. `commit_field()`, `step_field()`, `set_date()`, `set_time()`, `toggle_all_day()` in `event_panel.rs`. Parsing is in `time::parse_date()` and `time::parse_time()`.
- **Backend needs:** Same update as CAL-7. All-day events use `date` values with an exclusive end date; timed events use `dateTime` with a time zone (CAL-12).
- **Notes:** Changing the start keeps the duration. An end time at or before the start moves to the next day. Ends are at least 15 minutes after the start.

### CAL-9 Create an event

- **Feature:** `c` or the header's "+" creates an event at the next half hour today (or on the selected day), opens it and focuses the title. Dragging on empty space in the Day or Week grid, or double-clicking it, also creates one (CAL-21).
- **Today:** In memory. `new_event()` and `CalendarView::create_event(start, end, all_day, ..)` in `mod.rs`. New events go to the first visible calendar, with a 10-minute reminder, busy, and the user as organizer.
- **Backend needs:** An event insert. Return the server ID and replace the local one (CAL-11). A new event left without a title is dropped when its panel closes (escape, close, or opening another event), with no toast, so don't insert it before it has a title.
- **Notes:** A default calendar setting is not built.

### CAL-10 Delete an event and undo

- **Feature:** `delete`/`backspace` or the panel's Delete button removes the event and shows a toast with Undo.
- **Today:** In memory. `delete_occurrence()` (and `delete_event()` for a whole event) emits `CalendarEvent::Deleted` with a `Removal`; `PigeonApp` shows the toast with `UndoAction::DeleteEvent`, and Undo calls `restore()`. Repeating events ask first (CAL-6).
- **Backend needs:** A delete that waits until the toast is gone, or a delete plus re-create on undo. If there are guests, the server should send them a cancellation; the user may want to be asked first.
- **Notes:** Deleting an event someone else organizes should mean "decline and hide", not delete (CAL-22).

### CAL-11 IDs for new events

- **Feature:** Not visible. New events get IDs like `e-local-1`.
- **Today:** `CalendarView::new_id()` uses a counter that starts at 0 on each launch.
- **Backend needs:** Same as DAT-5: server IDs, or a local UUID until the first save. The iCalendar `UID` should be stable so invite emails can find the event (CAL-20).
- **Notes:** None.

### CAL-12 Time zones

- **Feature:** Times are shown as written in the data.
- **Today:** Wall-clock times. `wall_time()` in `src/calendar/mod.rs` drops the JSON's UTC offset (`naive_local()`), and the model uses `NaiveDateTime`. The placeholder data uses Eastern offsets for San Francisco places, so converting to the Mac's zone would move every event.
- **Backend needs:** Store each event's time zone, convert to the user's zone for display, and write times back with a zone. All-day events have no zone. Recurrences must expand in the event's own zone so they stay put across daylight-saving changes.
- **Notes:** The UI has no zone picker and no "shown in another zone" hint. Both are needed for travel.

### CAL-13 Guests and invitations

- **Feature:** The panel lists the organizer and guests with RSVP icons and a summary counting everyone listed ("3 people · 2 yes · 1 awaiting"). Typing in "Add guests" suggests contacts. `enter` adds the guest; `backspace` on an empty field removes the last one.
- **Today:** In memory. `render_guests()`, `update_guest_suggestions()`, `add_guest()`, `remove_guest()` in `event_panel.rs`. Suggestions come from `ContactBook::suggest()` in `src/ui/composer.rs` (CON-1).
- **Backend needs:** Save the attendee list, and send invites and updates to guests (Google `sendUpdates`, or iMIP emails for CalDAV). Free/busy lookup for guests would help scheduling.
- **Notes:** Adding a guest sends nothing today. The user is not asked "send updates to guests?".

### CAL-14 My RSVP

- **Feature:** For events someone else organizes, the panel shows "Going? Yes / Maybe / No", and so does the invite card in the mail reader (CAL-20). Invites the user has not answered are drawn outlined; declined ones are struck through.
- **Today:** In memory. `CalendarView::set_rsvp()` in `mod.rs` sets the user's attendee status. `Event::my_status()` and `Look` in `week_view.rs` pick the style.
- **Backend needs:** Write the user's response to the server (Google attendee `responseStatus`, or an iMIP `REPLY`) so the organizer gets it. The same write serves the panel and the invite card.
- **Notes:** Changing an answer sends a new reply each time; consider sending once the user stops changing it. Answering for one occurrence of a series is not supported (CAL-6).

### CAL-15 Reminders

- **Feature:** The panel picks a reminder (none, at start, 5, 10, 15, 30 minutes, 1 hour, 1 day before).
- **Today:** In memory. `reminder_minutes` on `Event`. Nothing fires.
- **Backend needs:** Save the reminder. Schedule macOS notifications for upcoming reminders, including after a restart and while the window is closed (NTF-3).
- **Notes:** Servers allow several reminders per event; the UI has one.

### CAL-16 Busy and free

- **Feature:** The panel's Busy/Free choice. The grid draws busy and free events the same way.
- **Today:** In memory. `busy` on `Event`.
- **Backend needs:** Save it (Google `transparency`, iCalendar `TRANSP`). It affects what others see in free/busy lookups.
- **Notes:** None.

### CAL-17 Video links

- **Feature:** The panel shows and edits a video link, with a Join button that opens it in the browser.
- **Today:** Placeholder JSON (`video_link`), in memory. Opened through `cx.open_url()`.
- **Backend needs:** Read conference data (Google `conferenceData`, or a URL in the location or description). Creating a Meet link for a new event needs the conference API.
- **Notes:** None.

### CAL-18 Today and the now-line

- **Feature:** Today is highlighted and a red line marks the current time in Day and Week.
- **Today:** Works locally from the system clock: `today()` and `now()` in `src/ui/calendar/mod.rs`. Past events are faded.
- **Backend needs:** None.
- **Notes:** The line only moves when the view re-renders. A timer to redraw it each minute is not built.

### CAL-19 Event search in the palette

- **Feature:** In Calendar mode, typing in the palette finds events by title. Choosing one jumps to its next occurrence and opens it.
- **Today:** In memory, over every loaded event. `palette_items()` in `src/app.rs` adds `PaletteCommand::Event` items (search-only). `CalendarView::reveal_event()` jumps to it.
- **Backend needs:** An event search query (Google `events.list` with `q`) with a result cap, as for PAL-2.
- **Notes:** Location, guests and notes are not searched.

### CAL-20 Invite emails linked to events

- **Feature:** An invitation email shows an invite card above its body: title, date and time, repeat rule, location, video link with Join, organizer and head count, in the calendar's colour. A small timeline shows the user's other events that day around the invite's time, and names the busy events it overlaps ("Conflicts with …"). Yes / Maybe / No answer it (`shift-y`, `shift-m`, `shift-n`) and show the current answer. "Open in Calendar" (`shift-c`) switches to the calendar on the event's day with it open. A cancellation shows a muted card with "Remove from calendar" (`shift-x`), which deletes the event with an Undo toast. All are in the palette and the `?` sheet.
- **Today:** Placeholder JSON. `Message::invite` (`Invite { event_id, method }`, `method` is `request` or `cancel`) in `src/data.rs`. The card and its actions are in `src/ui/invite.rs`; it reads `CalendarView::data` and answers through `CalendarView::set_rsvp()`, so answers given in the card show in the grid and the panel, and the other way round.
- **Backend needs:** Parse `text/calendar` parts (iMIP `REQUEST`, `CANCEL`, `REPLY`) and match them to events by `UID`. An invite for an event not yet in the calendar should add it (as "needs action") when the mail arrives. Answering from the card sends the reply as in CAL-14. "Remove from calendar" deletes the user's copy only; it must not send a cancellation to anyone.
- **Notes:** The card shows the event's next occurrence from today. An invite whose event was deleted shows "This event is no longer in your calendar". Conflicts count only busy events the user hasn't declined; free/busy of other calendars (shared, delegated) is not considered.

### CAL-21 Drag to create, move and resize

- **Feature:** In the Day and Week grids: drag on empty space to create an event (a live preview follows the mouse, snapped to 15 minutes, with its times floating above it; on release the event opens with its title focused); double-click empty space for a 30-minute event; drag an event to move it in time and across days; drag its bottom edge to resize it (at least 15 minutes). Holding the mouse near the grid's top or bottom scrolls it. A press that moves less than 4 pixels is a click and opens the event. Escape cancels a drag.
- **Today:** In memory. `src/ui/calendar/drag.rs` (`GridDrag`, `press_grid()`, `press_occurrence()`, `drag_to()`, `finish_drag()`) calls `CalendarView::create_event()` and `retime_occurrence()`. The all-day lane and the Month view can't be dragged.
- **Backend needs:** Same as CAL-9 and CAL-7: one write on release, not one per mouse move. Moving an event with guests should ask whether to send them an update.
- **Notes:** Moving one occurrence of a series asks "This event / All events" (CAL-6). A failed write must put the event back where it was (STA-2).

### CAL-22 Organizer and read-only permissions

- **Feature:** Every event can be edited, including those organized by others and those on shared calendars.
- **Today:** No checks. Edits apply in memory to any event.
- **Backend needs:** The user's access role per calendar (Google `accessRole`) and whether guests may modify the event. The panel must turn read-only where the server would refuse the change.
- **Notes:** Guests may usually change only their own RSVP, reminder and (on their copy) busy/free.

### CAL-23 Persisted calendar UI state

- **Feature:** UI not built yet. The view (Day, Week, Month), the shown date, week numbers and calendar visibility reset on launch.
- **Today:** In memory only, in `CalendarView` (`view`, `date`) and `Calendar::visible`. All reset in `CalendarView::new()`.
- **Backend needs:** Save these with the other UI state (PER-3). Visibility may belong on the server (CAL-2).
- **Notes:** None.

---

## 15. Notifications and background work (NTF)

### NTF-1 New mail notifications

- **Feature:** UI not built yet. No system notifications are sent.
- **Today:** Not implemented.
- **Backend needs:** A sync loop that detects new messages (LST-8) and reports them. The UI shows a macOS notification with the sender and subject, and clicking it opens the thread.
- **Notes:** Respect "do not disturb" (macOS Focus), and mute per label if that is added.

### NTF-2 Unread badge

- **Feature:** UI not built yet. The app has no dock or menu bar badge.
- **Today:** Not implemented. Unread counts exist only in the sidebar (FOL-2).
- **Backend needs:** The total unread count for the inbox (FOL-2). The app updates the dock badge from it.
- **Notes:** The badge must use the server count, not the loaded count.

### NTF-3 Background sync when the window is closed

- **Feature:** UI not built yet. Quitting stops everything. Quitting also loses unsaved state (PER-2).
- **Today:** Not implemented. `main()` quits when the last window closes (`cx.on_window_closed`).
- **Backend needs:** A background process, or a login item, that keeps syncing and holds the outbox. Or a sync that runs on launch and catches up.
- **Notes:** Keep the window-close behaviour in mind: closing the window currently quits the app.

---

## 16. Loading, error and sync states (STA)

These are the states the UI must show once the backend exists. None exists today.

### STA-1 Initial load and loading states

- **Feature:** UI not built yet. Each view and each opened message should show loading, loaded, empty or error. Today the window opens with everything loaded, so no loading state exists.
- **Today:** Not implemented. `MailData::load()` runs synchronously in `main()` and calls `expect` if the JSON is bad.
- **Backend needs:** A per-view and per-message state (loading, loaded, error, stale). Skeleton rows for the list. A spinner or "Loading…" in the reader.
- **Notes:** The empty-state text (LST-9) must not show during loading. Launch time must not depend on network calls.

### STA-2 Action failure and rollback

- **Feature:** UI not built yet. Every action (archive, delete, star, read, send, draft save) changes the UI immediately and does not check the result. If the server fails, the change is kept on screen but not on the server.
- **Today:** Not implemented. Every handler in `src/app.rs` changes `data` and calls `cx.notify()`.
- **Backend needs:** Pending state for each change, confirmation from the server, and rollback on failure. Show an error toast. The existing toast (`show_toast()` in `src/app.rs`) can carry the message.
- **Notes:** Rollback must restore the thread's folder, flags and position in the list. Send failures are in CMP-12.

### STA-3 Offline mode and queued changes

- **Feature:** UI not built yet. The app behaves the same with no network. Changes are kept in memory only.
- **Today:** Not implemented.
- **Backend needs:** An offline mode with a visible state. Changes are queued in the local store (PER-2) and sent when the network returns. Reading uses the local cache (PER-1). Sending is held in the outbox.
- **Notes:** Show the queue size, and which messages are still pending.

### STA-4 Expired sign-in

- **Feature:** UI not built yet. There is no prompt when the sign-in or token has expired.
- **Today:** Not implemented.
- **Backend needs:** Detect an auth error on every call, refresh the token, and if that fails, show a "sign in again" prompt that keeps local changes.
- **Notes:** A failed refresh must not lose the outbox (PER-2).

### STA-5 Conflicts with server changes

- **Feature:** UI not built yet. If a thread changes on the server while it is open, or a draft is edited elsewhere, nothing happens here.
- **Today:** Not implemented.
- **Backend needs:** Conflict rules for threads and drafts. The reader must refresh the open thread. A draft conflict must not silently replace the user's text.
- **Notes:** Links to DRF-6 and LST-8.

---

## 17. Local storage and persistence (PER)

### PER-1 Local mailbox store

- **Feature:** UI not built yet. Mail is not stored. Every launch reads the bundled JSON, and every change is lost on quit.
- **Today:** Not implemented. `MailData` in `src/data.rs` is the only copy, held in memory.
- **Backend needs:** A local database (for example SQLite) with threads, messages, labels, drafts, the outbox and sync state. The search index (SRC-5) and the parsed bodies (`HtmlCache` in `src/html.rs`) can be kept alongside it.
- **Notes:** Bodies can be stored compressed. Keep the attachment bytes separate.

### PER-2 Outbox and drafts that survive quit

- **Feature:** UI not built yet. Quitting loses unsent messages and unsaved drafts. Pending changes are not kept.
- **Today:** Not implemented. `Quit` in `src/actions.rs` calls `cx.quit()` with no save step.
- **Backend needs:** A durable outbox (pending sends, draft saves, flag changes) written before the UI reports success. Flushed on the next launch.
- **Notes:** Quitting should wait for pending writes, or warn if they are not done.

### PER-3 Persisted UI state

- **Feature:** UI not built yet. Window and layout state is reset on each launch.
- **Today:** In memory only, in `PigeonApp`: `nav`, `sidebar_open`, `sidebar_width`, `list_width`, `archived_expanded`, `expanded_messages`, the list density, recent searches and recently run palette commands. All reset on launch (`PigeonApp::new()`).
- **Backend needs:** Save these to a local settings file. This is not server data, but it is needed to keep the same layout after restart.
- **Notes:** `expanded_messages` is per thread. Keep it, or accept the reset.

---

## 18. Data model (DAT)

### DAT-1 JSON as the data source

- **Feature:** Everything the UI shows comes from `assets/mailbox.json`, read by `MailData::load()` (`src/data.rs`) through `include_str!`. Messages are sorted by date and empty threads are dropped.
- **Today:** Placeholder JSON. Compiled into the binary.
- **Backend needs:** Replace with the backend's data. The JSON format can stay as a test fixture.
- **Notes:** The JSON contains message IDs (`"id": "m-0001"`) and thread IDs. Only thread IDs are read. Message IDs are dropped (see DAT-3).

### DAT-2 Thread model: one folder per thread

- **Feature:** Each thread has one `folder` (`Folder` enum: Inbox, Archive, Sent, Drafts, Spam, Trash) and a `labels` list of label IDs. Starred and unread are flags on the thread.
- **Today:** Placeholder JSON and in-memory changes. `Thread` in `src/data.rs`.
- **Backend needs:** A decision on the model. Gmail is label-based: a thread can be in several labels, and "Archive" means "not in INBOX". IMAP is folder-based: a message is in one folder. The UI assumes one folder. The backend must map each thread to one folder, and say what happens when a thread is in several (for example, a Gmail thread in INBOX and Work).
- **Notes:** This is the most important data-model decision. Changing it later affects all of sections 2, 3 and 5.

### DAT-3 Message model: missing fields

- **Feature:** Each message has: from, to, cc, bcc, date, snippet, html, attachments. Bcc is not in the JSON and defaults to empty.
- **Today:** Placeholder JSON. `Message` in `src/data.rs`.
- **Backend needs:** Add: server message ID, thread ID, read and starred flags per message, labels per message (if FOL-5 uses them), plain-text body, the MIME part tree (for ATT-2 and ATT-4), and the time the message was last changed on the server.
- **Notes:** Message read state is per message in IMAP, but the UI shows one state per thread. See ACT-4 and RDR-1.

### DAT-4 Threading headers

- **Feature:** Replies are placed in the right conversation on the local side only (CMP-11). Headers are not kept.
- **Today:** Not stored. `Message` has no `Message-ID`, `In-Reply-To`, `References` or `Reply-To` field. Replies are linked only by the `reply_to` thread ID held in memory, which is not sent.
- **Backend needs:** Store `Message-ID`, `In-Reply-To`, `References` and `Reply-To` for every message. Use them to build threads and to set headers on sent replies (CMP-2, CMP-3). Use `Message-ID` to deduplicate the sent copy (CMP-13).
- **Notes:** Without these, replies from Pigeon will not thread correctly in other clients.

### DAT-5 Local IDs for drafts and sent messages

- **Feature:** Not visible to the user. Drafts and new sent threads get IDs like `draft-local-1` and `sent-local-1`.
- **Today:** `new_id()` in `src/app.rs` makes these from a counter in `PigeonApp::next_id`, which starts at 0 on each launch.
- **Backend needs:** Stable IDs that come from the server, or from a local UUID stored in the database. Replace the `-local-` IDs after the first server save.
- **Notes:** Once persisted, the counter would reuse IDs after a restart. Use UUIDs, not a counter.

### DAT-6 Search index rebuild cost

- **Feature:** Not visible. Every change to a thread rebuilds its search index, including parsing the HTML of every message.
- **Today:** `Thread::reindex()` in `src/data.rs` calls `html::parse()` for each message's body. It runs at load for every thread, and on each save or send (`save_draft()`, `send_message()` in `src/app.rs`).
- **Backend needs:** Compute the index when a body is stored, not when a thread changes. Keep the index in the local store (PER-1).
- **Notes:** `update_threads()` also re-sorts all threads on each change.

---

## 19. Things with no backend need (for completeness)

These are real features that do not depend on the backend. They are listed so the list above is not read as complete UI:

- Pane resizing for sidebar and list (`PaneResize`, `resize_pane()` in `src/app.rs`).
- Sidebar show and hide (`cmd-\`, `toggle_sidebar()`).
- Keyboard shortcuts sheet (`?`), built from the live keymap in `src/ui/shortcuts_help.rs`. Like the palette, it shows only the current mode's commands plus the global ones.
- Calendar views, navigation and keyboard selection (`d`/`w`/`m`, `t`, `j`/`k`, arrows, `tab`), see CAL-4.
- Light and dark theme that follows the system (`Theme::sync()` in `src/theme.rs`).
- Keyboard focus between list, reader, composer and search.
- Window title bar and drag area (`window_drag_area()` in `src/ui/mod.rs`).
- Quit (`cmd-q`).

---

## 20. Not built yet, implied by the UI

These are gaps the backend will need to support once the UI is built. Each one has a short definition here, and is also in the checklist.

- **Account switching and multiple accounts**: see ACC-3 and ACC-4.
- **Real unread and label counts**: see FOL-2.
- **Loading, error and offline states**: see STA-1 to STA-5.
- **Pagination and lazy loading of large mailboxes**: see LST-7.
- **Remote image loading**: see ATT-1 and ATT-2.
- **Attachment open and download**: see ATT-4.
- **Creating, renaming and deleting labels**: see FOL-6.
- **Notifications**: see NTF-1 to NTF-3.
- **Offline cache**: see PER-1 to PER-3.
- **Report spam and not spam**: see FOL-7.
- **Permanent delete and empty Trash**: see FOL-8.
- **Undo for star, read state and labels**: see UND-3.
- **Uploading attached files**: see ATT-6.
- **Forwarding attachments**: see ATT-7.
- **Auto-saving drafts while typing**: see DRF-5.
- **Failure feedback for send**: see CMP-12.
- **Contact management**: see CON-3.
- **Calendar**: series edits (CAL-6), invite cards in mail (CAL-20), drag to create and move (CAL-21), permissions (CAL-22).

Signatures and a settings screen are not built and are not listed as features here, because nothing in the UI refers to them.

---

## Checklist

Backend status is "Not started" for every item, because no backend code exists. Update the column as work progresses.

| Feature | Area | Today | Backend status |
| --- | --- | --- | --- |
| ACC-1 Account card | Account | Display only | Not started |
| ACC-2 Identity of "me" | Account | Placeholder JSON (one address) | Not started |
| ACC-3 Add, sign in and remove accounts | Account | UI not built yet | Not started |
| ACC-4 Account switcher and multiple accounts | Account | UI not built yet | Not started |
| ACC-5 Per-account sync status | Account | UI not built yet | Not started |
| FOL-1 System folders and go-to shortcuts | Folders & labels | Placeholder JSON | Not started |
| FOL-2 Sidebar counts | Folders & labels | Derived from loaded data | Not started |
| FOL-3 Label list with colours | Folders & labels | Placeholder JSON | Not started |
| FOL-4 Label views | Folders & labels | Placeholder JSON, in memory | Not started |
| FOL-5 Apply and remove labels on a thread | Folders & labels | In memory | Not started |
| FOL-6 Create, rename, recolour and delete labels | Folders & labels | UI not built yet | Not started |
| FOL-7 Report spam and not spam | Folders & labels | UI not built yet | Not started |
| FOL-8 Empty Trash and permanent delete | Folders & labels | UI not built yet | Not started |
| LST-1 Thread rows | Thread list | Placeholder JSON | Not started |
| LST-2 Unread styling | Thread list | Placeholder JSON, in memory | Not started |
| LST-3 Sort order | Thread list | In memory | Not started |
| LST-4 Archived section in the Inbox | Thread list | In memory | Not started |
| LST-5 Keyboard list navigation | Thread list | In memory | Not started |
| LST-6 Unread count in list header | Thread list | Derived from loaded data | Not started |
| LST-7 Pagination and lazy loading | Thread list | UI not built yet | Not started |
| LST-8 Live refresh and new mail | Thread list | UI not built yet | Not started |
| LST-9 Empty-result text | Thread list | Display only | Not started |
| RDR-1 Opening a thread marks it read | Reading pane | In memory | Not started |
| RDR-2 Conversation view | Reading pane | Placeholder JSON | Not started |
| RDR-3 Expand and collapse older messages | Reading pane | In memory | Not started |
| RDR-4 Message header | Reading pane | Placeholder JSON | Not started |
| RDR-5 Message body rendering | Reading pane | Placeholder JSON | Not started |
| RDR-6 Fetch a message body on demand | Reading pane | UI not built yet | Not started |
| RDR-7 Message navigation and reader focus | Reading pane | In memory | Not started |
| ACT-1 Archive and move to Inbox | Message actions | In memory | Not started |
| ACT-1a Move to a folder | Message actions | In memory | Not started |
| ACT-2 Delete | Message actions | In memory | Not started |
| ACT-3 Star and unstar | Message actions | In memory | Not started |
| ACT-4 Mark as unread | Message actions | In memory | Not started |
| ACT-5 Mark as read | Message actions | In memory | Not started |
| ACT-6 Bulk select and bulk actions | Message actions | In memory | Not started |
| CMP-1 New message | Compose & sending | In memory | Not started |
| CMP-2 Reply | Compose & sending | In memory | Not started |
| CMP-3 Reply all | Compose & sending | In memory | Not started |
| CMP-4 Forward | Compose & sending | In memory | Not started |
| CMP-5 Recipient fields | Compose & sending | Local parsing only | Not started |
| CMP-6 Show Cc and Bcc | Compose & sending | In memory | Not started |
| CMP-7 Subject and formatted body | Compose & sending | In memory | Not started |
| CMP-8 Send | Compose & sending | In memory | Not started |
| CMP-9 Send validation | Compose & sending | In memory (recipients only) | Not started |
| CMP-10 Forward quote block | Compose & sending | Local HTML | Not started |
| CMP-11 Reply appends to the thread | Compose & sending | In memory | Not started |
| CMP-12 Send failure feedback | Compose & sending | UI not built yet | Not started |
| CMP-13 Sent copy on the server | Compose & sending | In memory | Not started |
| DRF-1 Save on close or navigation | Drafts | In memory | Not started |
| DRF-2 Drafts list | Drafts | Placeholder JSON (starter drafts), in memory | Not started |
| DRF-3 Open and edit a draft | Drafts | In memory | Not started |
| DRF-4 Discard a draft | Drafts | In memory | Not started |
| DRF-5 Save while typing | Drafts | UI not built yet | Not started |
| DRF-6 Sync drafts with the server | Drafts | Not implemented | Not started |
| UND-1 Undo send | Undo | In memory | Not started |
| UND-2 Undo discard | Undo | In memory | Not started |
| UND-3 Undo for other actions | Undo | In memory (moves only) | Not started |
| SRC-1 Search bar and keys | Search | In memory | Not started |
| SRC-2 Default search scope | Search | In memory | Not started |
| SRC-3 Search operators | Search | In memory | Not started |
| SRC-4 Scope operator `in:` | Search | In memory | Not started |
| SRC-5 Text matching | Search | In memory | Not started |
| SRC-6 Result count | Search | Derived from loaded data | Not started |
| SRC-7 Full-mailbox search | Search | In memory, loaded data only | Not started |
| SRC-8 Live search while typing | Search | In memory | Not started |
| CON-1 Recipient suggestions | Contacts | Derived from placeholder JSON | Not started |
| CON-2 Typed name and address resolution | Contacts | Local only | Not started |
| CON-3 Contact management | Contacts | UI not built yet | Not started |
| ATT-1 Remote images | Images & attachments | Placeholder (alt text) | Not started |
| ATT-2 Inline `cid:` images | Images & attachments | Placeholder (alt text) | Not started |
| ATT-3 Attachment list in messages | Images & attachments | Placeholder JSON, display only | Not started |
| ATT-4 Open and download attachment | Images & attachments | Display only, no handler | Not started |
| ATT-5 Attachment icon in list | Images & attachments | Placeholder JSON | Not started |
| ATT-6 Attach files to a new message | Images & attachments | In memory, metadata only | Not started |
| ATT-7 Forward keeps attachments | Images & attachments | UI not built yet | Not started |
| LNK-1 Open link in browser | Links | Works locally, no backend | Not started |
| LNK-2 Link safety checks | Links | UI not built yet | Not started |
| LNK-3 `mailto:` links | Links | UI not built yet | Not started |
| PAL-1 Command list | Command palette | In memory | Not started |
| PAL-2 Thread search in palette | Command palette | In memory | Not started |
| PAL-3 Label jumps | Command palette | Placeholder JSON | Not started |
| CAL-1 Switch between Mail and Calendar | Calendar | Works locally | Not started |
| CAL-2 Calendar list, colours and visibility | Calendar | Placeholder JSON, in memory | Not started |
| CAL-3 Load events for the visible range | Calendar | In memory, all loaded | Not started |
| CAL-4 Day, week and month views | Calendar | Works locally | Not started |
| CAL-5 Recurring events | Calendar | Placeholder JSON (simple rules) | Not started |
| CAL-6 Edits apply to the whole series | Calendar | In memory | Not started |
| CAL-7 Edit an event in the panel | Calendar | In memory | Not started |
| CAL-8 Date, time and all-day editing | Calendar | In memory | Not started |
| CAL-9 Create an event | Calendar | In memory | Not started |
| CAL-10 Delete an event and undo | Calendar | In memory | Not started |
| CAL-11 IDs for new events | Calendar | Local counter | Not started |
| CAL-12 Time zones | Calendar | Wall-clock, offsets dropped | Not started |
| CAL-13 Guests and invitations | Calendar | In memory, nothing sent | Not started |
| CAL-14 My RSVP | Calendar | In memory | Not started |
| CAL-15 Reminders | Calendar | In memory, nothing fires | Not started |
| CAL-16 Busy and free | Calendar | In memory | Not started |
| CAL-17 Video links | Calendar | Placeholder JSON, in memory | Not started |
| CAL-18 Today and the now-line | Calendar | Works locally | Not started |
| CAL-19 Event search in the palette | Calendar | In memory | Not started |
| CAL-20 Invite emails linked to events | Calendar | Placeholder JSON, in memory | Not started |
| CAL-21 Drag to create, move and resize | Calendar | In memory | Not started |
| CAL-22 Organizer and read-only permissions | Calendar | No checks | Not started |
| CAL-23 Persisted calendar UI state | Calendar | UI not built yet | Not started |
| NTF-1 New mail notifications | Notifications & background | UI not built yet | Not started |
| NTF-2 Unread badge | Notifications & background | UI not built yet | Not started |
| NTF-3 Background sync when closed | Notifications & background | UI not built yet | Not started |
| STA-1 Loading states | Loading, error & sync states | Not implemented | Not started |
| STA-2 Action failure and rollback | Loading, error & sync states | Not implemented | Not started |
| STA-3 Offline mode and queued changes | Loading, error & sync states | UI not built yet | Not started |
| STA-4 Expired sign-in | Loading, error & sync states | UI not built yet | Not started |
| STA-5 Conflicts with server changes | Loading, error & sync states | UI not built yet | Not started |
| PER-1 Local mailbox store | Storage & persistence | Not implemented | Not started |
| PER-2 Outbox and drafts that survive quit | Storage & persistence | Not implemented | Not started |
| PER-3 Persisted UI state | Storage & persistence | In memory | Not started |
| DAT-1 JSON as the data source | Data model | Placeholder JSON | Not started |
| DAT-2 Thread model: one folder per thread | Data model | Placeholder JSON, in memory | Not started |
| DAT-3 Message model: missing fields | Data model | Placeholder JSON | Not started |
| DAT-4 Threading headers | Data model | Not stored | Not started |
| DAT-5 Local IDs for drafts and sent | Data model | In memory counter | Not started |
| DAT-6 Search index rebuild cost | Data model | In memory | Not started |

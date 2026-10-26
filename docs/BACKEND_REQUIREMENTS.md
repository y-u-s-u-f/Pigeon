# Pigeon: Backend Requirements

This file lists every front-end feature that needs backend support. It describes the code as it is today, so that when the backend is built, each item can be checked off and no part of the UI is left as a fake.

## Current state

- All mail data comes from `assets/mailbox.json`. It is loaded once at startup by `MailData::load()` in `src/data.rs`, called from `main()` in `src/main.rs`.
- There is no network, sync, sending or storage code in `src/`.
- Every change (archive, delete, star, read state, send, draft save, discard) is applied in memory to `PigeonApp::data` in `src/app.rs`.
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

- **Feature:** UI not built yet. Labels appear as chips on threads, but there is no way to add or remove one.
- **Today:** Not implemented. `Thread::labels` is never changed in code.
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

- **Feature:** Each row shows: the sender's name (or "To: name" in Sent and Drafts), a message count if there is more than one, an attachment icon, a star, the date, the subject, label chips and the snippet of the last message.
- **Today:** Placeholder JSON. `render_thread_row()` in `src/ui/thread_list.rs` builds rows from `MailData::threads`. The snippet is `Message::snippet` from the JSON. Date text comes from `list_date()` in `src/data.rs`.
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

Each of these acts on the selected thread only. Each one currently changes only `data.threads` in memory. The reader toolbar, the keyboard shortcut and the palette entry all dispatch the same action (see `src/actions.rs`).

### ACT-1 Archive and move to Inbox

- **Feature:** Shortcut `e`. Toolbar button "Archive" (its tooltip changes to "Move to Inbox" for archived threads). Palette "Archive". Moves the selected thread to Archive, or back to Inbox if it is already in Archive. The selection moves to the next thread, or the previous one if there is none after it.
- **Today:** In memory. `archive()` and `move_selected()` in `src/app.rs`. The target comes from `Folder::archive_target()` in `src/data.rs`: Inbox goes to Archive; Archive, Spam and Trash go to Inbox; Sent and Drafts do nothing. The toolbar hides the Archive button in Sent and Drafts. The rule follows each thread's own folder, so it also applies in Starred and label views.
- **Backend needs:** Move the thread to Archive. Gmail: `threads.modify` with `removeLabelIds: ["INBOX"]`. IMAP: `UID MOVE` to the Archive folder. Moving back: `UID MOVE` to INBOX. Spam to Inbox is the "not spam" action (FOL-7), and Trash to Inbox is a restore.
- **Notes:** Rollback is needed if the server rejects the change (STA-2).

### ACT-2 Delete

- **Feature:** Shortcut `#`. Toolbar button "Delete". Palette "Delete". Moves the thread to Trash, then selects the next thread.
- **Today:** In memory. `delete()` in `src/app.rs`. Delete moves a thread to Trash from every folder except Trash and Drafts. In Trash it does nothing. In Drafts it discards the draft through the same path as the Discard button (DRF-4), with its Undo toast, then moves to the neighbor.
- **Backend needs:** Move to Trash. Gmail: `threads.trash`. IMAP: `UID MOVE` to the Trash folder. Discarding a draft is DRF-4.
- **Notes:** Delete in Trash does nothing for now. The backend should decide whether it means a permanent delete (see FOL-8). The toolbar hides the Delete button in Trash. Undo for a thread move is not available (UND-3).

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

- **Feature:** UI not built yet. Only one thread can be acted on at a time. There is no multi-select, no select-all and no bulk archive or delete.
- **Today:** Not implemented. Every action uses `PigeonApp::selected`, a single index.
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

### CMP-7 Subject and plain-text body

- **Feature:** Subject field (not shown for replies, which keep the thread's subject). Body is a plain text editor that grows as you type. Pressing `Tab` moves focus between fields.
- **Today:** `Composer` in `src/ui/composer.rs`. The body is saved as HTML built by `html::from_plain_text()`. Forward quotes are added with `html()` in `Composer`.
- **Backend needs:** Build a MIME message: `text/plain` and `text/html` in `multipart/alternative`. Set `Content-Type` and encode non-ASCII subjects and names correctly.
- **Notes:** There is no formatting toolbar. Rich text is not built (not listed as a feature). Subject is stored on the thread, so a reply that changes it does not change the thread's subject.

### CMP-8 Send

- **Feature:** Shortcut `cmd-enter`, from anywhere in the draft or the list. Send button in the composer footer. Palette "Send". The draft closes, the message goes to Sent (or is added to the thread for a reply), and a "Message sent" toast appears with Undo for 5 seconds (see UND-1).
- **Today:** In memory. `send_message()` in `src/app.rs` builds the message with `message_from()` and changes `data.threads`. Nothing leaves the app.
- **Backend needs:** Send through the account's provider. Gmail: `messages.send` with the `raw` message (or `drafts.send`). IMAP: SMTP `MAIL FROM`, `RCPT TO`, `DATA`, plus the sent copy (CMP-13). The UI must not show "sent" until the server accepts the message. See the notes below and UND-1.
- **Notes:** The undo window (5 seconds, `TOAST_DURATION` in `src/app.rs`) means the real send can be held for that time. Otherwise undo can only cancel a message that has already gone. `message_from()` sets `date` to the local clock and `snippet` to the first 140 characters of the plain body. The server should set the real date, and the snippet should come from the same text.

### CMP-9 Send validation

- **Feature:** Sending with no recipients shows "Add at least one recipient" in the footer and keeps the draft open. Subject and body are not checked. Sending without a subject or body is allowed with no warning.
- **Today:** `send_message()` in `src/app.rs` checks `DraftContent::has_recipients()`.
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
- **Today:** `open_draft()` in `src/app.rs` rebuilds the composer from the stored message. The body is taken from the draft's HTML as plain text (`html::plain_text`). The draft's kind, reply target and forwarded quote are kept in `DraftOrigin` on the thread (`src/data.rs`), so reopening restores the same kind and quote block. A forward draft reopens as a forward, and sending it creates a new thread (CMP-4).
- **Backend needs:** Fetch the draft's full content, including attachments and headers, when it is opened.
- **Notes:** Attachments on drafts are not kept, because `message_from()` always sets none. Rich HTML is lost on reopening. The forwarded quote is not in the draft's body, so draft search does not match text that only appears in the quote. The backend's draft storage should keep the quote as a separate part, as the app does now.

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

- **Feature:** After Send, a toast shows "Message sent" and an Undo button (shortcut `cmd-z`, or the palette "Undo Send / Discard"). Undo takes the message back out of Sent (or removes the appended message from the reply thread), restores the draft it was sent from, and reopens the composer. The toast disappears after 5 seconds.
- **Today:** In memory. `send_message()` and `undo()` in `src/app.rs`. `UndoAction::Send` holds the thread ID, whether it was appended, the removed draft and the composer.
- **Backend needs:** Hold the message in an outbox for the undo window before it is sent. Undo cancels it before dispatch and restores the draft on the server. After the window, the message is sent and undo is no longer available.
- **Notes:** The 5-second window (`TOAST_DURATION`) is a UI choice. The backend's hold time must be at least as long, or the UI must not offer Undo. Once sent, undo cannot recall it (recall is not built).

### UND-2 Undo discard

- **Feature:** After Discard, a toast shows "Draft discarded" and Undo for 5 seconds. Undo puts the draft back in Drafts and reopens it.
- **Today:** In memory. `discard_draft()` and `undo()` in `src/app.rs`. `UndoAction::Discard`.
- **Backend needs:** The server delete (DRF-4) must be delayed until the undo window ends, or the draft must be recreated on undo.
- **Notes:** A new draft that was never saved has nothing on the server, so undo only restores the composer.

### UND-3 Undo for other actions

- **Feature:** UI not built yet. Archive, Delete, Star, and Mark read or unread have no undo. Only send and discard can be undone.
- **Today:** Not implemented. `move_selected()` and the other action handlers in `src/app.rs` do not create a toast.
- **Backend needs:** A reversible action log: each change keeps its inverse until the undo window passes, and the server call is queued for the same delay.
- **Notes:** Multiple undos (undo stack) are not built.

---

## 9. Search (SRC)

Search is in memory. Each thread gets a `SearchIndex` from `Thread::reindex()` in `src/data.rs`. It holds the lowercased subject, sender names and addresses, recipient names and addresses (to, cc, bcc), snippets, the plain-text body of every message, and attachment file names. Matching is done in `src/search.rs` (`Query::matches`).

### SRC-1 Search bar and keys

- **Feature:** Shortcut `/`, or the search icon in the list header, opens a field at the top of the list. `Esc` clears the search and closes it. `Enter` or `↓` moves focus into the results.
- **Today:** In memory. `toggle_search()`, `close_search()`, `focus_results()` in `src/app.rs`. `render_search_bar()` in `src/ui/thread_list.rs`.
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

- **Feature:** Typing in a recipient field shows up to six suggestions. Matching is fuzzy on the name and address. Already-added recipients are excluded. Up and down arrows, `Ctrl-n` and `Ctrl-p` move through them. Enter or a click picks one. Escape closes the list.
- **Today:** Placeholder data. `ContactBook::new(data.contacts())` in `PigeonApp::new()` in `src/app.rs`. `MailData::contacts()` in `src/data.rs` collects every sender and recipient in the JSON, except the user, sorted by how often they appear. The book is built once at startup.
- **Backend needs:** A contacts source. Gmail: People API (contacts and "other contacts"). CardDAV, or addresses taken from sent and received mail. The list must update when the user sends to a new address (today it does not, until restart).
- **Notes:** The book is `Rc<ContactBook>` and shared with every composer. Changes must reach open composers.

### CON-2 Typed name and address resolution

- **Feature:** A typed address is used as it is. A typed name followed by a comma or semicolon takes the best fuzzy match from the book. After a comma, a name with no match is dropped silently. On Enter, the highlighted suggestion is used when the text has no `@` or the user has moved through the list. Otherwise the typed address is used, or the field shows "“…” isn't an email address".
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

- **Feature:** Images in email bodies show as a bordered box with the alt text (or "Image" when there is no alt text). Images are never downloaded. Tiny tracking images (width or height 2 pixels or less) are dropped.
- **Today:** Placeholder. `Block::Image` in `src/html.rs` keeps only the `alt` text. The `src` attribute is ignored. `Renderer::block()` in `src/ui/html_view.rs` draws the box.
- **Backend needs:** A proxy or a fetch on user demand that loads remote images. Gmail and most clients have a "load remote images" setting. The fetch should go through the backend, so the user's IP and open tracking are not exposed. Cache the results.
- **Notes:** A setting to choose "always", "ask" or "never" is not built (UI not built yet). Tracking pixels are dropped by size only, which is a local rule that works today.

### ATT-2 Inline (`cid:`) images

- **Feature:** UI not built yet. Images embedded in the message (referenced as `cid:`) show as the placeholder box, the same as ATT-1.
- **Today:** Placeholder. `is_safe_url()` in `src/html.rs` does not allow `cid:`, and images are not drawn anyway.
- **Backend needs:** Read the MIME part with the matching `Content-ID`, and deliver it as a data source or a file. Gmail: the part is in `payload.parts`. IMAP: `BODYSTRUCTURE` and `BODY[part]`.
- **Notes:** The HTML renderer needs to know about the data source once images are drawn (see ATT-1).

### ATT-3 Attachment list in messages

- **Feature:** Under each message body, attachments appear as chips: file icon, name and size (for example "393 KB" or "5.6 MB").
- **Today:** Placeholder JSON. `Message::attachments` (`Attachment { name, size_bytes, mime }`) from `mailbox.json`. `render_message()` in `src/ui/reader.rs`. `file_size()` in `src/data.rs`.
- **Backend needs:** Attachment metadata per message: name, size, MIME type, and an ID to fetch it (Gmail: `attachmentId`, IMAP: the part number).
- **Notes:** `Attachment::mime` is never read (`#[allow(dead_code)]` in `src/data.rs`). It is kept for ATT-4.

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

- **Feature:** UI not built yet. The composer has no attach button and no drag-and-drop.
- **Today:** Not implemented. `message_from()` in `src/app.rs` always sets `attachments: Vec::new()`.
- **Backend needs:** Upload a file for a draft or a sent message. Gmail: `messages.send` with multipart (or resumable upload for large files, above about 5 MB). IMAP: MIME part in the message.
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
- **Backend needs:** Each command inherits the backend need of its action. The commands that need backend work are: Archive (ACT-1), Delete (ACT-2), Star (ACT-3), Mark as Unread and Read (ACT-4, ACT-5), Compose, Reply, Reply All, Forward (CMP-1 to CMP-4), Send (CMP-8), Close Draft (DRF-1), Discard Draft (DRF-4), Undo Send / Discard (UND-1, UND-2). Navigation and app commands need no backend.
- **Notes:** The palette also shows "Undo Send / Discard" even when there is nothing to undo. It does nothing then.

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

## 14. Calendar placeholder (CAL)

### CAL-1 Calendar button in the rail

- **Feature:** The far-left rail has a calendar icon under the mail icon. Clicking it does nothing. The mail icon is always the active one.
- **Today:** Stub. `render()` in `src/ui/rail.rs` builds the item with no click handler. Only `on_mouse_down` is set, to stop the window drag.
- **Backend needs:** None for the stub. Remove or hide the button until CAL-2 exists, or the user will see a dead control.
- **Notes:** The rail comment says Calendar is a placeholder.

### CAL-2 Calendar view

- **Feature:** UI not built yet. There is no calendar view, no agenda and no event creation.
- **Today:** Not implemented.
- **Backend needs:** Google Calendar API (events list, create, update, delete, free/busy) or CalDAV. Account linking (ACC-3) must include calendar scope. Time zones and recurring events need care.
- **Notes:** Out of scope unless the product needs it. It is listed because the rail implies it.

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
- **Today:** In memory only, in `PigeonApp`: `nav`, `sidebar_open`, `sidebar_width`, `list_width`, `archived_expanded`, and `expanded_messages`. All reset on launch (`PigeonApp::new()`).
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
- Keyboard shortcuts sheet (`?`), built from the live keymap in `src/ui/shortcuts_help.rs`.
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
- **Applying and removing labels from a thread**: see FOL-5.
- **Bulk select and bulk actions**: see ACT-6.
- **Report spam and not spam**: see FOL-7.
- **Permanent delete and empty Trash**: see FOL-8.
- **Undo for actions other than send and discard**: see UND-3.
- **Attaching files to a message**: see ATT-6.
- **Forwarding attachments**: see ATT-7.
- **Auto-saving drafts while typing**: see DRF-5.
- **Failure feedback for send**: see CMP-12.
- **Contact management**: see CON-3.
- **Calendar**: see CAL-2.

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
| FOL-5 Apply and remove labels on a thread | Folders & labels | UI not built yet | Not started |
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
| ACT-2 Delete | Message actions | In memory | Not started |
| ACT-3 Star and unstar | Message actions | In memory | Not started |
| ACT-4 Mark as unread | Message actions | In memory | Not started |
| ACT-5 Mark as read | Message actions | In memory | Not started |
| ACT-6 Bulk select and bulk actions | Message actions | UI not built yet | Not started |
| CMP-1 New message | Compose & sending | In memory | Not started |
| CMP-2 Reply | Compose & sending | In memory | Not started |
| CMP-3 Reply all | Compose & sending | In memory | Not started |
| CMP-4 Forward | Compose & sending | In memory | Not started |
| CMP-5 Recipient fields | Compose & sending | Local parsing only | Not started |
| CMP-6 Show Cc and Bcc | Compose & sending | In memory | Not started |
| CMP-7 Subject and plain-text body | Compose & sending | In memory | Not started |
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
| UND-3 Undo for other actions | Undo | UI not built yet | Not started |
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
| ATT-6 Attach files to a new message | Images & attachments | UI not built yet | Not started |
| ATT-7 Forward keeps attachments | Images & attachments | UI not built yet | Not started |
| LNK-1 Open link in browser | Links | Works locally, no backend | Not started |
| LNK-2 Link safety checks | Links | UI not built yet | Not started |
| LNK-3 `mailto:` links | Links | UI not built yet | Not started |
| PAL-1 Command list | Command palette | In memory | Not started |
| PAL-2 Thread search in palette | Command palette | In memory | Not started |
| PAL-3 Label jumps | Command palette | Placeholder JSON | Not started |
| CAL-1 Calendar button in the rail | Calendar placeholder | Stub, no-op | Not started |
| CAL-2 Calendar view | Calendar placeholder | UI not built yet | Not started |
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

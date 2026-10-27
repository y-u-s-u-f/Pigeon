# Pigeon roadmap

One place for where the app stands and what comes next: every feature, its status, the Google API it maps to, what the front end does today and what the backend must provide. It replaces `docs/BACKEND_REQUIREMENTS.md`; the item IDs (ACC-1, CMP-7, CAL-4a, …) are unchanged, so references in code and docs still work.

## Status legend

| Status | Meaning |
| --- | --- |
| ⚪ Not done | Nothing built. |
| 🟡 Partially done | Built in the front end on placeholder data (or in memory), not linked to a real backend. Every feature that exists in the front end today has this status, including ones that need no backend, until the app runs end to end on real data. |
| 🟢 Fully linked | Works end to end with the real backend. Nothing has this status yet. |

To update: change the status cell in the area's table, then the counts below.

## Progress

| Section | ⚪ Not done | 🟡 Partially done | 🟢 Fully linked | Total |
| --- | --- | --- | --- | --- |
| [Planned features](#3-planned-features-pln) | 3 | 0 | 0 | 3 |
| [Accounts and sign-in](#4-accounts-and-sign-in-acc) | 2 | 5 | 1 | 8 |
| [Folders and labels](#51-folders-and-labels-fol) | 6 | 2 | 4 | 12 |
| [Thread list](#52-thread-list-lst) | 1 | 3 | 5 | 9 |
| [Reading pane](#53-reading-pane-rdr) | 1 | 4 | 3 | 8 |
| [Message actions](#54-message-and-thread-actions-act) | 0 | 7 | 0 | 7 |
| [Compose and sending](#55-compose-and-sending-cmp) | 0 | 3 | 10 | 13 |
| [Drafts](#56-drafts-drf) | 1 | 1 | 4 | 6 |
| [Undo](#57-undo-und) | 0 | 1 | 2 | 3 |
| [Search](#58-search-src) | 1 | 8 | 0 | 9 |
| [Contacts](#59-contacts-con) | 6 | 0 | 2 | 8 |
| [Images and attachments](#510-images-and-attachments-att) | 3 | 4 | 0 | 7 |
| [Links](#511-links-lnk) | 2 | 1 | 0 | 3 |
| [Mail settings](#512-mail-settings-mst) | 10 | 0 | 0 | 10 |
| [Calendar](#6-calendar-cal) | 18 | 7 | 17 | 42 |
| [Sync and push](#71-sync-and-push-syn) | 2 | 5 | 0 | 7 |
| [Notifications and background](#72-notifications-and-background-ntf) | 3 | 0 | 0 | 3 |
| [Loading, error and offline states](#73-loading-error-and-offline-states-sta) | 2 | 3 | 0 | 5 |
| [Storage and persistence](#81-storage-and-persistence-per) | 1 | 1 | 1 | 3 |
| [Command palette](#82-command-palette-pal) | 0 | 3 | 0 | 3 |
| [Data model](#83-data-model-dat) | 1 | 6 | 0 | 7 |
| **Overall** | **63** | **64** | **49** | **176** |

## Contents

1. [Intro](#pigeon-roadmap) (above)
2. [Principles and current phase](#2-principles-and-current-phase)
3. [Planned features (PLN)](#3-planned-features-pln)
4. [Accounts and sign-in (ACC)](#4-accounts-and-sign-in-acc)
5. Mail: [FOL](#51-folders-and-labels-fol) · [LST](#52-thread-list-lst) · [RDR](#53-reading-pane-rdr) · [ACT](#54-message-and-thread-actions-act) · [CMP](#55-compose-and-sending-cmp) · [DRF](#56-drafts-drf) · [UND](#57-undo-und) · [SRC](#58-search-src) · [CON](#59-contacts-con) · [ATT](#510-images-and-attachments-att) · [LNK](#511-links-lnk) · [MST](#512-mail-settings-mst)
6. [Calendar (CAL)](#6-calendar-cal)
7. Sync and states: [SYN](#71-sync-and-push-syn) · [NTF](#72-notifications-and-background-ntf) · [STA](#73-loading-error-and-offline-states-sta)
8. App-wide: [PER](#81-storage-and-persistence-per) · [PAL](#82-command-palette-pal) · [DAT](#83-data-model-dat) · [No backend need](#84-features-with-no-backend-need)
9. [Adjacent Google APIs (later)](#9-adjacent-google-apis-optional-later)
10. [Not planned or out of scope](#10-not-planned-or-out-of-scope)

---

## 2. Principles and current phase

Principles are in [`AGENTS.md`](../AGENTS.md): blazing fast and light (no webviews, no heavy dependencies), keyboard-centric (every action has a shortcut and is in ⌘K), minimal and flat on translucent glass, YAGNI.

**Current phase: linked to Google; demo mode on placeholder data.** Backend sprint 1 ([`BACKEND_SPRINT_1.md`](BACKEND_SPRINT_1.md)) is under way: Phases 0–1 (foundation, sign-in, Gmail read, Calendar read) are merged; write actions, compose, contacts, search and states come next. `PIGEON_DEMO=1` runs on the placeholder data.

- Mail comes from `assets/mailbox.json`, loaded once by `MailData::load()` (`src/data.rs`). Calendar comes from `assets/calendar.json`, loaded by `CalendarData::load()` (`src/calendar/mod.rs`). Both are called from `main()`.
- There is no network, sync, sending or storage code in `src/`.
- Mail changes apply in memory to `PigeonApp::data` (`src/app.rs`); calendar changes to `CalendarView::data` (`src/ui/calendar/mod.rs`). Nothing is saved: quitting loses everything, drafts and sent messages included.
- There is one account, from the `account` object in the JSON.

**Reading the entries.** Each area has a table (ID, feature, status, Google API), then one entry per ID: **Today** (how it works now, with file and function), **Needs** (what the backend must provide) and **Notes** (edge cases). Gmail methods are written without the `users.` prefix (`threads.modify` is `users.threads.modify`). IMAP/SMTP equivalents are kept where useful, as Gmail also speaks IMAP with XOAUTH2.

**Backend facts that shape everything** (from Google's docs):

- For installed apps, Google recommends polling `history.list` for mail changes rather than push. Gmail push (`watch`) goes through Cloud Pub/Sub and Calendar push needs an HTTPS webhook, so both need a server. See SYN.
- Most useful Gmail scopes (`gmail.modify`, `gmail.readonly`, `gmail.compose`, `gmail.settings.basic`) are *restricted*. They need Google's restricted-scope verification, and storing or sending that data through our own servers also triggers a security assessment. Keeping mail on the Mac avoids the assessment. See ACC-8.
- Gmail labels are per message; Pigeon's model is per thread with one folder. See DAT-2.

---

## 3. Planned features (PLN)

The user's own plans, not driven by Google's APIs.

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| PLN-1a | Vim mode: composing and editing | ⚪ Not done | None |
| PLN-1b | Vim mode: navigating the app | ⚪ Not done | None |
| PLN-2 | AI reading view | ⚪ Not done | None (an LLM API) |

#### PLN-1a Vim mode while composing

- **Idea:** Optional modal editing in the compose body: normal, insert and visual modes; motions; operators; text objects; registers; undo and redo; `:` commands such as `:w` to save the draft and, as a stretch, `:wq` to send. Off by default, toggled in Settings (a new row in `src/settings.rs`).
- **Open questions:** How it fits the live-styled markdown editor (`src/ui/rich_editor.rs`) and its formatting shortcuts; where the mode indicator goes; whether it also applies to the subject, recipient and event-panel fields.

#### PLN-1b Vim mode while navigating

- **Idea:** Vim-style keys across the list, reader, calendar and settings: `hjkl`, `gg`/`G`, counts (`5j`), `/` search, building on the existing Gmail-style `j`/`k`. Same Settings toggle as PLN-1a, or its own.
- **Open questions:** Conflicts with single-letter Gmail shortcuts (`e`, `s`, `l`, `v`, `x`, `a`, `r`, `c`, `n`/`p`, `g …` chords) in `src/actions.rs`; whether Vim mode replaces or layers over them; showing Vim bindings in ⌘K and the `?` sheet.

#### PLN-2 AI reading view

- **Idea:** A small model (Claude Haiku) turns each email into a clean, consistent reading format for Pigeon: messy HTML, newsletters and forwarded chains become tidy text with the useful structure kept. Later, not in backend sprint 1. During that sprint a Haiku agent may be used offline on sample mails to tune the native renderer (see `docs/BACKEND_SPRINT_1.md` 6.4).
- **Open questions:** On by default or per message; where the model runs and who pays for it; privacy (mail content leaves the Mac); caching results; always keeping the original one keystroke away.

> **More user features go here.** Add each as PLN-3, PLN-4, … with status ⚪ Not done.

---

## 4. Accounts and sign-in (ACC)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| ACC-1 | Account card | 🟢 Fully linked | Gmail `getProfile`; People `people.get` (`people/me`) |
| ACC-2 | Identity of "me" | 🟡 Partially done | Gmail `settings.sendAs.list` |
| ACC-3 | Add, sign in to and remove accounts | 🟡 Partially done | OAuth 2.0 for desktop apps (loopback + PKCE) |
| ACC-4 | Account switcher and multiple accounts | ⚪ Not done | One OAuth grant per account |
| ACC-5 | Per-account sync status | 🟡 Partially done | Sync state from SYN |
| ACC-6 | Scopes and consent | 🟡 Partially done | OAuth scopes, incremental and granular consent |
| ACC-7 | Token refresh, revoke and sign out | 🟡 Partially done | OAuth token and revoke endpoints |
| ACC-8 | Google Cloud project and app verification | ⚪ Not done | OAuth consent screen, scope verification |

#### ACC-1 Account card

- **Today:** The sidebar shows name, email and an initials avatar; clicking does nothing. `render_sidebar()` in `src/ui/sidebar.rs` reads `data.account`. Settings › Accounts shows `MailData::me()`, the granted features and the sync status. Avatar colour comes from the address (`Theme::avatar_color`). Signed in (W2), name and email come from `accounts.json` (the ID token's claims at sign-in), and every launch refreshes the name from People `people/me` (`Backend::check_profile()`), saving it back to `accounts.json`.
- **Needs:** Photo (`personFields=photos`) not fetched yet. The signed-in profile. Gmail `getProfile` gives `emailAddress`, `messagesTotal`, `threadsTotal` and `historyId` but no name; the name and photo come from People `people.get` with `resourceName=people/me` (`personFields=names,photos,emailAddresses`) or the OpenID userinfo.

#### ACC-2 Identity of "me"

- **Today:** `MailData::is_me()` and `me()` (`src/data.rs`) compare against `account.email` and `Account::aliases`, ignoring case (`Account::is_me()`). The aliases come from Gmail `settings.sendAs.list` at the start of each full sync, kept in `sync_state` (`gmail_aliases`) and loaded on launch (`src/backend/sync.rs`). Mail from an alias shows as "me" in the reader, replies and reply-all skip every alias, Sent and Drafts rows show "To:", and aliases are never suggested as recipients (`Store::contacts()`).
- **Needs:** Nothing more for "me". Replying from the alias a mail was sent to needs the From picker (see the sendAs item).
- **Notes:** The calendar's guest list still compares against the primary address only. See MST-1.

#### ACC-3 Add, sign in to and remove accounts

- **Today:** Built in W2 for one account. Launch without `PIGEON_DEMO` picks the source in `Backend::open()`: `accounts.json` plus the Keychain item (service `Pigeon Google`, read through `/usr/bin/security`) → Google mode with no sign-in screen; neither → the sign-in screen (`src/ui/onboarding.rs`: "Sign in with Google" (Enter), "Try with demo data" (D), waiting state with Cancel (Esc), errors in plain words). Sign-in (`Backend::sign_in()`, `src/backend/auth.rs`): loopback on `127.0.0.1:0`, PKCE S256 with a 64-byte `SecRandom` verifier, random `state`, `access_type=offline`, `prompt=consent`, `include_granted_scopes=true`, the 8 scopes, opened with `cx.open_url()`, 5 min timeout. The code is exchanged at `token_uri`; the refresh token goes to the Keychain through `security -i` (on stdin, never argv) and `{email, name, scopes}` to `accounts.json` (0600). A different account signed in before is forgotten. Sign In with Google from ⌘K; Use Demo Data switches a signed-out window to the placeholder data until relaunch. `PIGEON_DATA_DIR` moves `accounts.json`, `google_client.json` and the cache.
- **Needs:** Adding a second account (ACC-4). Google's desktop flow: open the system browser, loopback redirect (`http://127.0.0.1:<port>`) with PKCE, exchange the code, store tokens in the Keychain. Ask for mail, calendar and contacts scopes together (CAL-1). Removing an account revokes its token (ACC-7) and deletes its local data (PER-1).
- **Notes:** IMAP/SMTP credentials would be the non-Google path. See DAT-2 for the one-account model.

#### ACC-4 Account switcher and multiple accounts

- **Today:** Not built. `PigeonApp::data` is a single `MailData`.
- **Needs:** Several accounts, each with its own threads, labels, calendars, contacts and sync state. Every thread, draft and event knows its account. A switcher or a unified view with account badges.
- **Notes:** IDs like `t-0001` are only unique inside one JSON file; they must be unique per account.

#### ACC-5 Per-account sync status

- **Today:** Basic (W2). A status line under the sidebar's account card and in Settings › Accounts (`src/ui/status.rs`): "Synced just now" / "Synced 3 minutes ago" / "Syncing…" / "Syncing 340 of 1,000" / "Offline · 3 changes waiting" / "Sign in again" (a button that runs Sign In) / "Demo data". `Backend::set_status()` and `Backend::note_error()` are what sync (W3, W4) calls; until then the launch profile check sets "Synced just now".
- **Today (W11):** What the line says goes through `backend::health::Health` (`src/backend/health.rs`): sync's Syncing/Synced, with an expired sign-in over everything, then "Offline · N changes waiting", then "Waiting for Gmail…" while a request waits out a backoff of 4 s or more (`GoogleTransport::waiting_for_gmail()`). Times read "Synced just now / 4 min ago / 2 h ago / 3 days ago" (`status::ago()`) and refresh once a minute. At launch it shows the last sync from the store until this launch's first pass. Settings › Accounts: the account row (name, email, allowed features, status line), Access (every granted scope in plain words), Last sync ("3 min ago · Oct 27, 14:02", plus the state when not Synced), Sync Now and Sign Out.
- **Needs:** A tester pass (T3b item 5). Per account once there are several (ACC-4). Links to STA-1, STA-3.

#### ACC-6 Scopes and consent

- **Today:** W2 asks for `openid`, `userinfo.email`, `userinfo.profile`, `gmail.modify`, `calendar`, `contacts.readonly`, `contacts.other.readonly` and `directory.readonly` (`auth::SCOPES`), stores the granted `scope` in `accounts.json`, and exposes `Backend::has_scope()` / `granted_scopes()`. Settings › Accounts lists the allowed features ("Mail, Calendar and Contacts allowed", or what's not). Calendar without its scope shows "Calendar access wasn't granted. Sign in again to allow it." (CAL-1). A missing scope also makes Sign In available while signed in.
- **Needs:** W3 and W8 to skip Mail and Contacts sync without their scopes. Pick the narrowest scopes that cover the features. Likely set: `gmail.modify` (read, label, send, trash; no permanent delete), `gmail.settings.basic` (filters, vacation, send-as, signatures), `calendar` or the narrower `calendar.events` + `calendar.calendarlist` + `calendar.calendars` + `calendar.acls` + `calendar.settings.readonly` + `calendar.freebusy`, `contacts` and `contacts.other.readonly` (and `directory.readonly` for Workspace), plus `openid email profile`. Permanent delete (FOL-8) needs `https://mail.google.com/`.
- **Notes:** Users can untick scopes on the consent screen (granular consent). Read the granted scopes from the token response and turn off features that lack them. Ask for extra scopes later with incremental authorization (`include_granted_scopes=true`).

#### ACC-7 Token refresh, revoke and sign out

- **Today:** W2. `auth::Auth` keeps the access token in memory with its expiry and refreshes 60 s early; one refresh runs at a time and other callers wait for it. `GoogleTransport` refreshes once and retries on a 401. Sign Out (⌘K) revokes at `oauth2.googleapis.com/revoke` (best effort), deletes the Keychain item, the `accounts.json` entry and the account's cached rows (`Store::delete_account()`), then shows the sign-in screen. Settings › Accounts › Sign out (W11) asks first ("Sign out? <email> and its cached mail and calendar are removed from this Mac", Cancel / Sign Out; Enter confirms, Esc cancels), then runs the same Sign Out. ⌘K's Sign Out doesn't ask.
- **Needs:** Refresh the access token before it expires, retry once on 401, then show "sign in again" (STA-4). Sign out calls the revoke endpoint and deletes the Keychain item.
- **Notes:** While the OAuth app is in "Testing" status, refresh tokens expire after 7 days.

#### ACC-8 Google Cloud project and app verification

- **Needs:** A Google Cloud project with the Gmail, Calendar and People APIs enabled, a desktop OAuth client, and a consent screen. Publishing to the public needs verification: restricted Gmail scopes need restricted-scope review; a security assessment is also needed if restricted data is stored on or sent through our servers.
- **Notes:** This decides whether the backend is a local sync engine (data stays on the Mac) or a server. Push notifications (SYN-3, SYN-5) need a server.

---

## 5. Mail

### 5.1 Folders and labels (FOL)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| FOL-1 | System folders and go-to shortcuts | 🟢 Fully linked | System labels `INBOX`, `SENT`, `DRAFT`, `SPAM`, `TRASH`, `STARRED`, `UNREAD` |
| FOL-2 | Sidebar counts | 🟢 Fully linked | `labels.get` (`threadsUnread`, `messagesUnread`) |
| FOL-3 | Label list with colours | 🟢 Fully linked | `labels.list`, label `color` |
| FOL-4 | Label views | 🟢 Fully linked | `threads.list` with `labelIds` |
| FOL-5 | Apply and remove labels on a thread | 🟡 Partially done | `threads.modify` |
| FOL-6 | Create, rename, recolour and delete labels | ⚪ Not done | `labels.create`, `labels.patch`, `labels.update`, `labels.delete` |
| FOL-7 | Report spam and not spam | 🟡 Partially done | `threads.modify` (add or remove `SPAM`) |
| FOL-8 | Empty Trash and permanent delete | ⚪ Not done | `threads.delete`, `messages.delete`, `messages.batchDelete` |
| FOL-9 | Nested labels | ⚪ Not done | Label names with `/` |
| FOL-10 | Label visibility | ⚪ Not done | Label `labelListVisibility`, `messageListVisibility` |
| FOL-11 | Inbox categories | ⚪ Not done | `CATEGORY_PERSONAL`, `_SOCIAL`, `_PROMOTIONS`, `_UPDATES`, `_FORUMS` |
| FOL-12 | Important marker | ⚪ Not done | `IMPORTANT` label |

#### FOL-1 System folders and go-to shortcuts

- **Today:** The sidebar lists Inbox, Starred, Sent, Drafts, Archive, Spam, Trash. Shortcuts `g i`, `g s`, `g t`, `g d`, `g a`, `g !`; Trash via the palette. `Folder` (`src/data.rs`) has six variants; each thread has one, derived from its messages' Gmail labels by `gmail::derive_folder()` when synced (W3). Sent lists every conversation holding a message the user sent, like Gmail (`nav_contains()`, 6.4 item 5). Starred is a flag. `FOLDERS` in `src/ui/sidebar.rs`, `set_nav()` in `src/app.rs`.
- **Needs:** A tester pass on the real account (T1a). IMAP later: special-use `\Sent`, `\Drafts`, `\Junk`, `\Trash`, `\Archive`.
- **Notes:** See DAT-2 for the one-folder limit. Spam and Trash sync only their newest 25 threads; older ones need LST-7.

#### FOL-2 Sidebar counts

- **Today:** Inbox, Spam and labels show Gmail's `threadsUnread` from `labels.get` (synced with the labels and after every partial sync that changed something; stored in `labels.threads_unread`). Reading or marking unread here moves the number at once: `ServerCounts` keeps the local count from when Gmail's was read and adds the difference (`view_unread()`, `src/ui/sidebar.rs`). Drafts shows the number of Drafts threads; demo mode (no server counts) counts loaded threads as before.
- **Needs:** A tester pass comparing with Gmail (T1a). IMAP: `STATUS … UNSEEN`.
- **Notes:** Between a local change and the next sync the count is an estimate.

#### FOL-3 Label list with colours

- **Today:** From `labels.list` (W3): user labels the user hasn't hidden (`labelListVisibility != labelHide`), with `color.backgroundColor`, else Gmail's grey. Stored in `labels` (hidden ones as `type = 'hidden'`, system ones for their counts); labels gone from Gmail are dropped (`Store::replace_gmail_labels()`). Dots in the sidebar, chips in the reader. Demo mode: the ten placeholder labels.
- **Needs:** A tester pass (T1a). `textColor` isn't used. IMAP has no colours; keep them locally.
- **Notes:** `Thread::labels` holds label IDs; renames keep IDs stable. Nested labels (`a/b`) show their full name (FOL-9).

#### FOL-4 Label views

- **Today:** `Nav::Label(usize)` (`src/app.rs`); `nav_contains()` shows cached threads carrying the label, excluding Spam and Trash. The cache holds the Inbox's first 100 threads, the first pages of Sent, Starred, Spam and Trash, and a backfill of up to 1,000 threads or 90 days (W3), so a label view shows the labelled threads in that window. `g l` or "Go to Label…" opens a label search.
- **Needs:** Older labelled threads through `threads.list labelIds=…` with `pageToken` (LST-7, W9). IMAP: folder or keyword search.
- **Notes:** Label views include Sent, Drafts and Archive, as in Gmail.

#### FOL-5 Apply and remove labels on a thread

- **Today:** `l`, palette "Label…", Message › Label…, the toolbar "…" menu or a row's right-click toggle a label on the selected or checked threads (ACT-6). `l` opens the palette scoped to labels; applied ones say "Applied · ↵ removes it". The list shows up to three dots; the reader shows chips. `toggle_label()` and `open_label_palette()` in `src/app/mail.rs`: if every target has the label it's removed, otherwise added to all. No undo.
- **Needs:** `threads.modify` with `addLabelIds` / `removeLabelIds` (IMAP: `UID COPY`/`MOVE`, or `STORE +X-GM-LABELS`). Optimistic update with rollback (STA-2).
- **Notes:** Gmail labels are per message; `threads.modify` changes every message in the thread. See DAT-2.

#### FOL-6 Create, rename, recolour and delete labels

- **Today:** Not built; labels are read once from JSON.
- **Needs:** `labels.create`, `labels.patch` / `update`, `labels.delete` (IMAP: `CREATE`, `RENAME`, `DELETE`).
- **Notes:** Gmail only accepts colours from a fixed palette. Deleting a label removes it from every message. New labels add palette entries (PAL-3).

#### FOL-7 Report spam and not spam

- **Today:** Move to… › Spam (`v`, ACT-1a) puts a thread in Spam in memory; Archive (`e`) in Spam moves it to Inbox ("not spam"). No dedicated button or shortcut.
- **Needs:** `threads.modify` adding `SPAM` (and removing `INBOX`), or removing `SPAM` and adding `INBOX`. IMAP: move to Junk.
- **Notes:** Mapping `SPAM` to the Spam folder is done at sync (FOL-1).

#### FOL-8 Empty Trash and permanent delete

- **Today:** Not built. `delete()` (`src/app.rs`) moves to Trash and never removes; Delete does nothing in Trash.
- **Needs:** `threads.delete`, `messages.delete`, `messages.batchDelete` (all need the `https://mail.google.com/` scope, ACC-6). Gmail purges Trash after 30 days on its own. IMAP: `\Deleted` + `EXPUNGE`.
- **Notes:** Not reversible; needs a confirmation.

#### FOL-9 Nested labels

- **Needs:** Gmail nests labels by name (`Work/Clients`). Show them as a tree in the sidebar and the label palette.

#### FOL-10 Label visibility

- **Needs:** Respect `labelListVisibility` (`labelShow`, `labelShowIfUnread`, `labelHide`) in the sidebar and `messageListVisibility` on rows; allow changing them via `labels.patch`.

#### FOL-11 Inbox categories

- **Needs:** Gmail's category labels (`CATEGORY_PERSONAL`, `CATEGORY_SOCIAL`, `CATEGORY_PROMOTIONS`, `CATEGORY_UPDATES`, `CATEGORY_FORUMS`) as Inbox tabs or filters, and the `category:` search operator (SRC-9).

#### FOL-12 Important marker

- **Needs:** Show and toggle the `IMPORTANT` label (`threads.modify`); `is:important` in search (SRC-9).

### 5.2 Thread list (LST)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| LST-1 | Thread rows | 🟢 Fully linked | `threads.list` + `threads.get` (`format=metadata`) |
| LST-2 | Unread styling | 🟢 Fully linked | `UNREAD` label |
| LST-3 | Sort order | 🟢 Fully linked | `threads.list` order |
| LST-4 | Archived section in the Inbox | 🟡 Partially done | `threads.list` (`q=-in:inbox`) |
| LST-5 | Keyboard list navigation | 🟢 Fully linked | `threads.get` on demand |
| LST-6 | Unread count in the list header | 🟢 Fully linked | `labels.get` |
| LST-7 | Pagination and lazy loading | ⚪ Not done | `pageToken`, `maxResults` |
| LST-8 | Live refresh and new mail in the list | 🟡 Partially done | `history.list` (SYN-2) |
| LST-9 | Empty-result text | 🟡 Partially done | None |

#### LST-1 Thread rows

- **Today:** Sender (or "To: name" in Sent and Drafts), message count, attachment icon, star, date, subject, label dots, snippet of the last message. Compact density puts sender, subject and date on one line. `render_thread_row()` (`src/ui/thread_list.rs`). With Google (W3) rows come from `threads.list` + batched `threads.get format=metadata` (`gmail::thread_from_gmail()`): subject of the first message, per-message Gmail `snippet` (HTML entities decoded), headers parsed by `mail-parser` (RFC 2047, address lists), date from the Date header, sorted by `internalDate`. Demo: placeholder JSON.
- **Needs:** A tester pass (T1a). The attachment icon needs the body (ATT-5).
- **Notes:** The row's snippet is the last message's, not the thread's.

#### LST-2 Unread styling

- **Today:** Blue dot, bold sender and subject. `Thread::unread` is true when any message has `UNREAD` (W3 mapping); flipped locally by `select_thread()`, `mark_read()`, `mark_unread()`.
- **Needs:** A tester pass (T1a). IMAP: `\Seen` per message.
- **Notes:** See ACT-4, ACT-5, DAT-3.

#### LST-3 Sort order

- **Today:** Newest first by the last message's `internalDate` (`MailData::sort_threads()`); `update_threads()` re-sorts after every change and every sync.
- **Needs:** A tester pass (T1a). A stable cursor once paging exists (LST-7).

#### LST-4 Archived section in the Inbox

- **Today:** The Inbox ends with a collapsible "Archived" header counting the cached Archive threads (`Row::ArchivedHeader` in `refresh_rows()`, `src/app.rs`); palette "Show / Hide Archived in Inbox". With Google, the cache's archived threads come from the backfill (W3).
- **Needs:** "Load older" (LST-7) for archived mail past the window. Gmail has no cheap server count for "not in Inbox" (1.1).

#### LST-5 Keyboard list navigation

- **Today:** `j`/`k` or arrows move; `Enter`/`o` open. `select_next()`, `select_prev()`, `select_thread()` (`src/app.rs`). Selecting marks read (RDR-1) and asks for the bodies (RDR-6), which are fetched only once the selection has stayed 150 ms, so key repeat doesn't spend quota.
- **Needs:** A tester pass (T1a).

#### LST-6 Unread count in the list header

- **Today:** "N unread" from `view_unread()`, the same number as the sidebar: Gmail's count for Inbox, Spam and labels (FOL-2), else the loaded rows'.
- **Needs:** A tester pass (T1a).

#### LST-7 Pagination and lazy loading

- **Today:** Not built. The list is virtualised (`ListState`), but all 124 threads load at startup.
- **Needs:** Cursor paging per view: `threads.list` `pageToken` and `maxResults` (up to 500). IMAP: UID ranges. Keep only visible and recent threads in memory.
- **Notes:** Search, counts, contacts (CON-1) and palette search (PAL-2) assume full data and each need a server query once paging exists.

#### LST-8 Live refresh and new mail in the list

- **Today:** Partial sync (SYN-2, W3) refetches the threads `history.list` reports every 45 s while the window is active (5 min in the background), on focus and on Sync Now, and emits `ThreadsChanged` / `ThreadsRemoved`. `on_threads_changed()` (`src/app/backend_events.rs`) reloads them through `update_threads()` (`MailData::upsert_threads()`: changed threads replaced, new ones inserted newest first), which keeps the selection and splices rows so the list stays put. A list scrolled to its top stays at the top, so new mail shows instead of the splice scrolling it out of view, and a selected row that was showing is revealed again if it was pushed down. Threads moved elsewhere leave or join each view (Inbox, labels, Sent, open search) by the same filter; a selected thread that leaves stays open in the reader.
- **Needs:** A tester pass (T1a item 5), including new mail arriving with a thread selected. Sync 2 s after a committed write comes with W5. Links to NTF-1, STA-5.

#### LST-9 Empty-result text

- **Today:** "Nothing in <view>", or "No results in <scope>" with a hint while searching (`render_thread_list()`). With Google, an empty view shows skeleton rows until its first page is stored (`Backend::view_loaded()`), or "Couldn't load <view>" if the sync failed before then (W3).
- **Needs:** A tester pass (T1a).

### 5.3 Reading pane (RDR)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| RDR-1 | Opening a thread marks it read | 🟡 Partially done | `threads.modify` (remove `UNREAD`) |
| RDR-2 | Conversation view | 🟢 Fully linked | `threads.get` (`format=full`) |
| RDR-3 | Expand and collapse older messages | 🟡 Partially done | Per-message `snippet` |
| RDR-4 | Message header | 🟢 Fully linked | `messages.get` headers |
| RDR-5 | Message body rendering | 🟢 Fully linked | `messages.get` `payload` parts |
| RDR-6 | Fetch a message body on demand | 🟡 Partially done | `messages.get` (`format=full`) |
| RDR-7 | Message navigation and reader focus | 🟡 Partially done | None |
| RDR-8 | Show original (raw message) | ⚪ Not done | `messages.get` (`format=raw`) |

#### RDR-1 Opening a thread marks it read

- **Today:** `select_thread()` (`src/app.rs`) sets `unread = false` at once, also while moving through the list with keys and for Drafts.
- **Needs:** `threads.modify` removing `UNREAD` (IMAP: `STORE +FLAGS (\Seen)`). Skip for drafts.
- **Notes:** Consider a delay or a setting, as many clients do.

#### RDR-2 Conversation view

- **Today:** Subject, label chips, "N messages", every message oldest first (by `internalDate`), newest expanded. `render_conversation()` (`src/ui/reader.rs`). Draft messages are left out of the conversation (they're Drafts threads). A sync that changes the open thread reloads it in place.
- **Needs:** A tester pass (T1a).

#### RDR-3 Expand and collapse older messages

- **Today:** Collapsed rows show sender, snippet (Gmail's per-message `snippet`), date; click to expand. `expanded_messages`, `toggle_message()`, cleared in `select_thread()`.
- **Needs:** A tester pass (T1a). Expanded state could be saved (PER-3).

#### RDR-4 Message header

- **Today:** Sender name, email, "to … · cc …", full local date via `full_date()`. Bcc not shown. `render_message()`. With Google the headers come from `format=metadata` (From, To, Cc, Bcc, Reply-To, Subject, Date, Message-ID, In-Reply-To, References), parsed by `mail-parser` (W3); `message_id`/`in_reply_to`/`references` are stored without angle brackets.
- **Needs:** A tester pass (T1a).
- **Notes:** Long recipient lists are shown in full.

#### RDR-5 Message body rendering

- **Today:** HTML is drawn natively (headings, lists, quotes, tables, links, emphasis, code, simple colours); scripts, styles, iframes and form controls dropped; images become placeholders (ATT-1). `html::parse()` (`src/html.rs`), `src/ui/html_view.rs`, cached by hash in `HtmlCache`. Message text is selectable. With Google the body comes from `messages.get format=raw` parsed by `mail-parser` (`mime::parse_message()`, W3): charsets, transfer encodings, the `text/html` part of `multipart/alternative`, plain text converted to HTML when there's no HTML.
- **Needs:** A tester pass on real newsletters and replies (T1a). `cid:` parts per ATT-2.
- **Notes:** Unknown tags keep their text only.

#### RDR-6 Fetch a message body on demand

- **Today:** Built (W3). When a thread stays selected 150 ms, `Backend::fetch_body()` fetches every message without a body with `messages.get format=raw` in one HTTP batch (60 units each), parses it (`mime::parse_message()`) and stores html, attachments and invitation in `bodies` / `messages`, plus `search_text`. Messages show "Loading…" meanwhile; a failure shows "Couldn't load this message", the reason and Retry (palette "Retry Loading Message"). Bodies survive later metadata refetches. Demo mode has every body already.
- **Needs:** A tester pass (T1a). Eviction at 300 MB (`bodies.last_opened`) isn't built. Offline wording ("isn't downloaded yet") comes with W11.
- **Notes:** A Gmail draft opened in the composer before its body arrives starts empty; W6 (`drafts.get`) handles opening drafts.

#### RDR-7 Message navigation and reader focus

- **Today:** `n`/`p` move between messages and expand; `o`/`Enter` focus the reader; `u`/`Esc` return; arrows scroll. `step_message()`, `open_thread()`, `back_to_list()`, `scroll_reader()` (`src/app.rs`). A message whose body is loading shows "Loading…" when reached.
- **Needs:** A tester pass on real data.

#### RDR-8 Show original

- **Needs:** "Show original" view of the raw RFC 822 source (`messages.get` `format=raw`), useful for headers and debugging.

### 5.4 Message and thread actions (ACT)

Each action applies to the checked threads if any (ACT-6), otherwise the selected one (`targets()` in `src/app/mail.rs`). Hovering a row shows Archive, Delete, Mark read/unread and Star; right-click opens a native menu with those plus Reply, Forward, Move to, Label. Toolbar, shortcut and palette all dispatch the same action (`src/actions.rs`).

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| ACT-1 | Archive and move to Inbox | 🟡 Partially done | `threads.modify` (`INBOX`); `threads.untrash` |
| ACT-1a | Move to a folder | 🟡 Partially done | `threads.modify`, `threads.trash` |
| ACT-2 | Delete | 🟡 Partially done | `threads.trash` |
| ACT-3 | Star and unstar | 🟡 Partially done | `threads.modify` (`STARRED`) |
| ACT-4 | Mark as unread | 🟡 Partially done | `threads.modify` / `messages.modify` (`UNREAD`) |
| ACT-5 | Mark as read | 🟡 Partially done | `threads.modify` (`UNREAD`) |
| ACT-6 | Bulk select and bulk actions | 🟡 Partially done | `messages.batchModify`, HTTP batch |

#### ACT-1 Archive and move to Inbox

- **Today:** `e`, toolbar "Archive" (tooltip "Move to Inbox" for archived threads), palette. Selection advances (Settings › Mail › Auto-advance off clears it). `archive()`, `move_selected()` (`src/app.rs`); target from `Folder::archive_target()`: Inbox → Archive; Archive, Spam, Trash → Inbox; Sent and Drafts nothing (button hidden there). Follows each thread's own folder, also in Starred and label views.
- **Needs:** `threads.modify` removing `INBOX`; back to Inbox adds it. Spam → Inbox is "not spam" (FOL-7); Trash → Inbox is `threads.untrash` plus adding `INBOX`. IMAP: `UID MOVE`.
- **Notes:** Rollback on failure (STA-2).

#### ACT-1a Move to a folder

- **Today:** `v`, palette "Move to…", Message › Move to…, toolbar "…", right-click: Inbox, Archive, Spam or Trash (palette omits the current one). Undo toast. `move_to()`, `move_threads()`, `open_move_palette()` (`src/app/mail.rs`). Drafts are skipped; Sent can move.
- **Needs:** The moves of ACT-1, ACT-2 and FOL-7.

#### ACT-2 Delete

- **Today:** `delete`, `backspace`, `#`, toolbar, palette. Moves to Trash and selects the next thread, with Undo (UND-3). In Drafts it discards the draft (DRF-4); in Trash it does nothing (button hidden). `delete()` (`src/app.rs`).
- **Needs:** `threads.trash` (IMAP: `UID MOVE` to Trash).
- **Notes:** Whether Delete in Trash means permanent delete is open (FOL-8).

#### ACT-3 Star and unstar

- **Today:** `s`, toolbar, palette; star on the row. Unstarring in Starred removes the row at once. `toggle_star()`; `Thread::starred`. Allowed in Spam and Trash; Starred hides those.
- **Needs:** `threads.modify` adding or removing `STARRED` (IMAP: `\Flagged`).
- **Notes:** Gmail stars messages; removing `STARRED` from the thread unstars all of them.

#### ACT-4 Mark as unread

- **Today:** `shift-u`, toolbar, palette. Selection stays. `mark_unread()`.
- **Needs:** Add `UNREAD`: to every message (`threads.modify`) or only the last (`messages.modify`); decide which. IMAP: remove `\Seen`.
- **Notes:** Opening the thread again marks it read (RDR-1).

#### ACT-5 Mark as read

- **Today:** `shift-i`, palette; no toolbar button. `mark_read()`.
- **Needs:** `threads.modify` removing `UNREAD`.

#### ACT-6 Bulk select and bulk actions

- **Today:** `x` checks; `shift-j`/`shift-k` (or shift-arrows) extend; `cmd`-click toggles, `shift`-click ranges; row checkbox on hover; `Esc` clears. The reader shows "N conversations selected" with subjects and action buttons; all actions apply to all. No select-all. `PigeonApp::checked`; `render_selection_summary()` (`src/ui/reader.rs`).
- **Needs:** `messages.batchModify` takes message IDs (up to 1,000), so expand threads to messages, or send one `threads.modify` per thread in an HTTP batch (SYN-7). IMAP: UID sets.
- **Notes:** Partial failures must be handled by rollback (STA-2).

### 5.5 Compose and sending (CMP)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| CMP-1 | New message | 🟢 Fully linked | `drafts.create` |
| CMP-2 | Reply | 🟢 Fully linked | `messages.send` with `threadId` |
| CMP-3 | Reply all | 🟢 Fully linked | As CMP-2 |
| CMP-4 | Forward | 🟢 Fully linked | `messages.send` (no `threadId`) |
| CMP-5 | Recipient fields | 🟢 Fully linked | Server validation on send |
| CMP-6 | Show Cc and Bcc | 🟡 Partially done | `raw` MIME; Gmail delivers Bcc |
| CMP-7 | Subject and formatted body | 🟢 Fully linked | `raw` MIME (`multipart/alternative`) |
| CMP-8 | Send | 🟢 Fully linked | `messages.send`, `drafts.send` |
| CMP-9 | Send validation | 🟡 Partially done | Send errors |
| CMP-10 | Forward quote block | 🟢 Fully linked | `raw` MIME |
| CMP-11 | Reply appends to the thread | 🟢 Fully linked | `threadId`, `In-Reply-To`, `References` |
| CMP-12 | Send failure feedback | 🟡 Partially done | Send errors |
| CMP-13 | Sent copy on the server | 🟢 Fully linked | `SENT` label (automatic) |

#### CMP-1 New message

- **Today:** `c` or palette "Compose New Message" opens an empty draft in the reading pane ("New Message"). `compose()` (`src/app/compose.rs`), `Composer` (`src/ui/composer.rs`). In memory until saved (DRF-1).
- **Needs:** A tester pass on the real account (T2b); account choice when there are several (ACC-4); From alias choice (MST-1).
- **Notes:** With Google (W6), saving it creates a Gmail draft (DRF-1) and sending goes through `messages.send` (CMP-8). From is the primary address and the account's name.
- **Notes:** No signature (MST-2).

#### CMP-2 Reply

- **Today:** `r`, toolbar, palette. Draft opens under the conversation; subject "Re: …" unless it already starts with "re:". `reply_recipients()` (`src/app/compose.rs`): if the last message is from the user (or an alias), reply to its To, else to its sender; never to the user unless they're its only recipient (a mail to yourself replies to yourself), as Gmail does. Unit-tested. `reply_to` links the thread.
- **Needs:** A tester pass (T2b). Verified on the real account (W6): the reply lands in the same Gmail thread.
- **Notes:** Earlier participants are ignored. With Google (W6) the send carries `threadId` and `In-Reply-To`/`References` built from the thread's last stored message (`mime::reply_threading()`, `src/backend/mime.rs`); a reply goes to the last message's `Reply-To` when it has one.

#### CMP-3 Reply all

- **Today:** `a`, toolbar, palette. To: sender plus the last message's recipients, minus the user (and aliases); Cc: its Cc minus anyone in To. To the user's own message: its To and Cc, the sender left out. If only Cc remains it moves to To; if no one remains, it goes to the user. Dedupe by email, case-insensitive. `reply_recipients()`, unit-tested.
- **Needs:** A tester pass (T2b).
- **Notes:** `Reply-To` is honoured (W6): its addresses replace the sender's in To (`reply_recipients()`); not on the user's own messages.
- **Notes:** Mailing lists and no-reply senders are included as is.

#### CMP-4 Forward

- **Today:** `f`, toolbar, palette. Subject "Fwd: …" unless it starts with "fwd:"/"fw:". The original is quoted with From, Date, Subject, To. No `reply_to`, so it opens alone and sending creates a new Sent thread (CMP-11).
- **Needs:** A tester pass (T2b). Include the original's attachments (ATT-7).
- **Notes:** With Google (W6) a forward is a new message: no `threadId`, `In-Reply-To` or `References`; the quote goes as a `<blockquote>` in the HTML part (CMP-10).

#### CMP-5 Recipient fields

- **Today:** To, Cc, Bcc as chips. Comma, semicolon or Enter commits; an unfinished address commits on send; Tab moves between fields; Backspace on empty removes the last chip; a chip without `@` is red. `Recipients`, `commit()`, `parse_address()` (`Name <addr>` or a bare address with `@` and a dotted domain).
- **Needs:** A tester pass (T2b). Local parsing is only a first check; Gmail's rejection reaches the user as "Not sent: …" (CMP-12).

#### CMP-6 Show Cc and Bcc

- **Today:** `cmd-shift-c`, the "Cc Bcc" link, or palette. `show_cc_bcc()`; `DraftContent::bcc`.
- **Needs:** A tester pass (T2b). SMTP: `RCPT TO` only.
- **Notes (W6):** `Bcc` goes in the `raw` message; Gmail strips it on delivery.
- **Notes:** Bcc is kept on drafts and the sent copy, but the reader doesn't show it (RDR-4).

#### CMP-7 Subject and formatted body

- **Today:** Subject (hidden for replies, which keep the thread's). Markdown body styled as you type (bold, italic, strikethrough, `<u>underline</u>`, code, headings, lists, quotes, links); syntax stays visible, dimmed. Formatting bar and Gmail shortcuts (`cmd-b`, `cmd-i`, `cmd-u`, `cmd-shift-x`, `cmd-e`, `cmd-k`, `cmd-shift-7`/`8`/`9`, `cmd-\`). Enter continues lists; Tab/Shift-Tab indent; Page Up/Down move by page. `Composer` (`src/ui/composer.rs`), `src/ui/rich_editor.rs`, `src/markdown.rs`. On send and save, `markdown::to_html()` (`DraftContent::body_html()`) makes `Message::html`.
- **Needs:** A tester pass (T2b): formatting intact in Gmail web.
- **Notes:** With Google (W6), `mime::build_message()` (`src/backend/mime.rs`, `mail-builder`) makes `multipart/alternative`: `text/plain` is the markdown as typed, `text/html` is `markdown::to_html()`; non-ASCII subjects and names are RFC 2047-encoded; base64url into `raw`. Typed raw HTML is sent as text except `<u>`/`</u>`. No inline images. A reply that changes the subject doesn't change the thread's.

#### CMP-8 Send

- **Today:** `cmd-enter` anywhere in the draft or list, Send button, palette. The draft closes, the message goes to Sent (or the thread), toast "Sending…" with Undo for 5 s (UND-1), then "Message sent" once it went. `send_message()` (`src/app/compose.rs`), `Outgoing::to_message()` (`src/backend/mod.rs`). It queues `MailOp::Send`, held for the undo window. With Google (W6) `commit_compose()` (`src/backend/compose.rs`) sends it: `drafts.send` for a draft saved in Gmail, else `messages.send`, both through the upload endpoint (`uploadType=multipart`) above 5 MB; at the user's quota priority. Demo mode applies it to its in-memory store.
- **Needs:** A tester pass (T2b). Verified on the real account (W6): new, reply, draft send.
- **Notes:** `date` and `snippet` stay local until the next sync refetches the thread.

#### CMP-9 Send validation

- **Today:** No recipients: "Add at least one recipient" under To. Invalid address: "“zzz” isn't an email address". Typed text becomes the best contact or the address as typed. Subject and body aren't checked. `Composer::validate()`, `DraftContent::has_recipients()`.
- **Needs:** A tester pass (T2b).
- **Notes:** Subject and empty-body warnings aren't built. Attachments over 25 MB in all are refused: "Attachments are over Gmail’s 25 MB limit" under the attachments, when attaching and on send (W6). Gmail's own reasons come back as "Not sent: …" (CMP-12).

#### CMP-10 Forward quote block

- **Today:** "Forwarded message" block with From, Date, Subject, To, then the body. `Quoted` (display) and `html()` (`<blockquote>`) in `src/ui/composer.rs`. Kept on the draft (`DraftOrigin::quoted`, `src/data.rs`) so reopened forwards show it (DRF-3).
- **Needs:** A tester pass (T2b).
- **Notes (W6):** Sent as a `<blockquote>` in the HTML part; the plain part has only the note. A forward draft saved in Gmail keeps the quote only locally (`DraftOrigin::quoted`).

#### CMP-11 Reply appends to the thread

- **Today:** `send_message()` pushes a reply into the `reply_to` thread, or a new Sent thread if it's gone. Forwards always make a new Sent thread.
- **Needs:** A tester pass (T2b). IMAP: `In-Reply-To` and `References`.
- **Notes (W6):** The reply's thread gets the sent copy under Gmail's ids at once (CMP-13). A reply to a thread still `sent-local-*` (its own send not yet committed) goes without `threadId` and threads by its headers.

#### CMP-12 Send failure feedback

- **Today:** With Google (W6), a send Gmail turns down (after the transport's retries) goes back to Drafts as a draft (`draft-local-*`, or the draft it was sent from), leaves the conversation it was shown in, and shows the toast "Not sent: <Gmail's reason>" (for example "Not sent: Invalid To header", checked on the real account). `send()` in `src/backend/compose.rs`.
- **Needs:** A tester pass (T2b). No retry button; reopening the draft and sending again is the retry.

#### CMP-13 Sent copy on the server

- **Today:** The Sent thread is made in memory by `send_message()`. With Google (W6), once Gmail answers, the sent copy is stored under its message and thread ids with the Message-ID it went with and Gmail's labels; a new thread's `sent-local-*` id is swapped for Gmail's (`IdReplaced`), then `ThreadsChanged` reloads it, so the history poll finds the same message instead of a second one.
- **Needs:** A tester pass (T2b). IMAP: `APPEND` to Sent unless the server does.

### 5.6 Drafts (DRF)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| DRF-1 | Save on close or navigation | 🟢 Fully linked | `drafts.create`, `drafts.update` |
| DRF-2 | Drafts list | 🟢 Fully linked | `drafts.list` |
| DRF-3 | Open and edit a draft | 🟢 Fully linked | `drafts.get` |
| DRF-4 | Discard a draft | 🟢 Fully linked | `drafts.delete` |
| DRF-5 | Save while typing | ⚪ Not done | `drafts.update` (debounced) |
| DRF-6 | Sync drafts with the server | 🟡 Partially done | `drafts.list`, `history.list` |

#### DRF-1 Save on close or navigation

- **Today:** A changed draft is saved when closed (`Esc`, toolbar "Close and save draft", palette "Close Draft") or when the user moves elsewhere; toast "Saved to Drafts" (no Undo). Unchanged drafts aren't saved. `stash_composer()`, `save_draft()` (`src/app/compose.rs`), which also queues `MailOp::SaveDraft` (an `Outgoing`, `src/backend/mod.rs`); demo mode stores it in its in-memory store. Drafts are one-message `Thread`s with IDs `draft-local-<uuid>` (DAT-5).
- **Needs:** A tester pass (T2b). IMAP: `APPEND` to Drafts and delete the old copy.
- **Notes:** Quitting loses unsaved changes (PER-2). With Google (W6) `SaveDraft` stores the draft locally first, then `drafts.create` (or `drafts.update` with the stored draft id) with `X-Pigeon-Markdown: 1` and, for replies, `threadId` and threading headers; the local id stays, `drafts.draft_id` and the message id are stored so the drafts sync sees it unchanged. A failed save keeps the local text and toasts "Couldn't save the draft to Gmail: …"; there's no automatic retry yet.

#### DRF-2 Drafts list

- **Today:** Rows show "To: <first recipient>", "(no recipients)", "(no subject)". The sidebar shows the count. With Google (W3) the Drafts step lists `drafts.list` (up to 500) and fetches each changed draft's message with `messages.get format=metadata`: each draft is a Drafts thread `draft-<draft id>` (a reply draft keeps `origin.reply_to` = its thread), its draft id stored in `drafts.draft_id`; drafts gone from Gmail go. Re-read when history touches a draft. Demo: six starter drafts.
- **Needs:** A tester pass (T1a). Opening one with its body and markdown: DRF-3 (W6).

#### DRF-3 Open and edit a draft

- **Today:** Selecting a Drafts thread opens it in the composer ("Draft") via `open_draft()`. Drafts saved here keep their markdown in `DraftOrigin::markdown` and reopen with it; JSON drafts reopen from their HTML as plain text (`html::plain_text`). Kind, reply target and forward quote are kept in `DraftOrigin`. Replying again to a thread with a reply draft reopens that draft (`reply_draft_of()`).
- **Needs:** A tester pass (T2b).
- **Notes:** With Google (W6), opening a Gmail draft whose text isn't here yet fetches it with `drafts.get format=raw` (`Backend::fetch_draft()`, `src/backend/compose.rs`) instead of the message-body fetch; a draft written here (`X-Pigeon-Markdown: 1`) gets its markdown back from the `text/plain` part (`mime::parse_draft()`), one written in Gmail opens as its plain text. The composer fills in when it arrives, unless the user has already typed (`Composer::load()`). Formatting of drafts written elsewhere is lost (no HTML-to-markdown). Draft search doesn't match text only in the quote. Attachments of a Gmail draft are sent again from the draft itself.

#### DRF-4 Discard a draft

- **Today:** `cmd-shift-d`, toolbar trash, palette. Removed with toast "Draft discarded" and Undo; an unchanged new draft is dropped silently. `discard_draft()`.
- **Needs:** A tester pass (T2b).
- **Notes (W6):** `DeleteDraft` removes the local copy, then `drafts.delete` (permanent) after the undo window (UND-2).

#### DRF-5 Save while typing

- **Today:** Not built; a crash loses everything since the last close.
- **Needs:** Debounced saves (for example 2 s after typing stops) to the local store (PER-1) and `drafts.update`; a "Saving…/Saved" state.

#### DRF-6 Sync drafts with the server

- **Today:** Drafts from Gmail come with the mail sync (DRF-2, W3). Drafts saved here go to Gmail (DRF-1) and keep their `draft-local-*` id, linked to the Gmail draft by `drafts.draft_id` (W6). Last write wins: a draft edited elsewhere reloads, but an open draft's text is never replaced once the user has typed (DRF-3).
- **Needs:** A tester pass (T2b): a draft written in Gmail web appears, edits sync.
- **Notes:** `drafts.update` replaces the draft's message, so its message ID changes on every save; track the draft ID, not the message ID.

### 5.7 Undo (UND)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| UND-1 | Undo send | 🟢 Fully linked | Hold `messages.send` for the window |
| UND-2 | Undo discard | 🟢 Fully linked | Delay `drafts.delete` |
| UND-3 | Undo for other actions | 🟡 Partially done | Delay or invert `threads.modify` |

#### UND-1 Undo send

- **Today:** Toast "Sending…" with Undo (`cmd-z`, `z`, palette) for 5 s (`TOAST_DURATION`), then "Message sent". Undo removes the message from Sent or the thread, restores the draft and reopens the composer, focused where the caret was (`Composer::remember_focus()` / `restore_focus()`), so its shortcuts work at once. `UndoAction::Send` (`src/app.rs`), `undo()` (`src/app/compose.rs`). The send is a `MailOp::Send` the outbox holds for the 5 s (PER-2); Undo cancels it, or says "Already sent" if it went. With Google the held send is committed by W6 (CMP-8).
- **Needs:** A tester pass (T2b). Gmail's own "undo send" isn't in the API, so the hold is ours.
- **Notes:** Once sent it can't be recalled.

#### UND-2 Undo discard

- **Today:** Toast "Draft discarded" with Undo for 5 s; Undo puts the draft back and reopens it, focused in the field it was in (so `cmd-shift-d` works again at once). `UndoAction::Discard`. Discarding a saved draft queues a delayed `MailOp::DeleteDraft`; Undo cancels it, or saves the draft again if it already went.
- **Needs:** A tester pass (T2b). A never-saved draft only needs the composer back.
- **Notes (W6):** The delayed `drafts.delete` is committed to Gmail; Undo after it went creates the draft again.

#### UND-3 Undo for other actions

- **Today:** Archive, Delete and Move to (one or many) show "Archived" / "Moved 3 conversations to Trash" with Undo (`z`, `cmd-z`, button, palette) for 5 s; Undo restores folders and the selection. `UndoAction::Move`, `undo_move()` (`src/app/mail.rs`). Each move queues Gmail ops (`MailOp::moves()`) that wait out the window; Undo cancels them, or queues the moves back once they went. Star, read state and labels go at once and have no undo; there's no undo stack.
- **Needs:** The ops committed to Gmail, with the inverse request after a commit (W5).

### 5.8 Search (SRC)

Search is in memory. `Thread::reindex()` (`src/data.rs`) builds a `SearchIndex` with lowercased subject, sender and recipient names and addresses (to, cc, bcc), snippets, plain-text bodies and attachment names. Matching is `Query::matches` in `src/search.rs`.

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| SRC-1 | Search bar and keys | 🟡 Partially done | `threads.list` with `q` |
| SRC-2 | Default search scope | 🟡 Partially done | `labelIds`, `q` |
| SRC-3 | Search operators | 🟡 Partially done | Gmail `q` operators |
| SRC-4 | Scope operator `in:` | 🟡 Partially done | `in:`, `includeSpamTrash` |
| SRC-5 | Text matching | 🟡 Partially done | Gmail full-text index |
| SRC-6 | Result count | 🟡 Partially done | `resultSizeEstimate` |
| SRC-7 | Full-mailbox search | 🟡 Partially done | `threads.list` with `q` |
| SRC-8 | Live search while typing | 🟡 Partially done | Debounced `threads.list` |
| SRC-9 | More Gmail operators | ⚪ Not done | Gmail `q` operators |

#### SRC-1 Search bar and keys

- **Today:** `/`, `cmd-f` or the list header icon opens the field; `Esc` clears and closes; `Enter` focuses results. Empty field: dropdown of operators and the last 5 searches; while typing `from:`/`to:`: matching contacts; arrows and Enter pick. A chip toggles scope ("Inbox"/"All mail"); recognised operators show as chips beside the count. `toggle_search()`, `close_search()`, `focus_results()` (`src/app.rs`); `search_suggestions()`, `remember_search()`, `toggle_search_scope()` (`src/app/mail.rs`); `render_search_bar()`, `render_search_suggestions()` (`src/ui/thread_list.rs`). Recent searches in memory (PER-3).
- **Needs:** Server search (`threads.list` with `q`) or the local index (PER-1), and a "searching" state (STA-1).

#### SRC-2 Default search scope

- **Today:** Without `in:`, the current view (or all mail per Settings › Mail › Search scope); Inbox also covers Archive. `refresh_rows()` with `nav_contains()`.
- **Needs:** Scope by `labelIds` or `q` (`in:inbox OR -in:inbox…`); Sent, Drafts, Spam and Trash search only themselves.

#### SRC-3 Search operators

- **Today:** `from:`, `to:`, `subject:`, `label:`, `is:unread`, `is:starred`, `has:attachment(s)`, quoted phrases (`from:"Sarah Chen"`), AND only. `Query::parse()`, `Term` (`src/search.rs`); substring matches; `label:` matches ID or alphanumeric name.
- **Needs:** Pass the same operators in Gmail's `q` (IMAP: `SEARCH FROM/TO/SUBJECT/UNSEEN/FLAGGED`).
- **Notes:** Gmail matches words, not substrings, so local and server results can differ for the same query.

#### SRC-4 Scope operator `in:`

- **Today:** `in:inbox`, `in:archive(d)`, `in:sent`, `in:draft(s)`, `in:spam`, `in:trash`, `in:anywhere`, `in:all`, `in:starred`, `in:<label>`. `Scope` (`src/search.rs`). `in:all` = everything but Spam and Trash; `in:anywhere` = everything.
- **Needs:** The same meanings server-side: Gmail excludes Spam and Trash unless `includeSpamTrash=true` or `in:anywhere`.

#### SRC-5 Text matching

- **Today:** Plain words match `SearchIndex::all` (`Term::Text`).
- **Needs:** Full-text over bodies and attachment names: Gmail's index; IMAP `TEXT` (slow); a local index (PER-1) for offline search.
- **Notes:** Bodies are only in memory today; needed once RDR-6 lands.

#### SRC-6 Result count

- **Today:** "N results in <scope>" from `rows`.
- **Needs:** Gmail's `resultSizeEstimate` is an estimate and can change between pages; show "about N" or a loading state.

#### SRC-7 Full-mailbox search

- **Today:** `in:all`/`in:anywhere` are complete because everything is loaded.
- **Needs:** Server search over mail not yet synced; mark partial results (STA-3). The main reason search must move to the server once LST-7 exists.

#### SRC-8 Live search while typing

- **Today:** Every keystroke re-runs `Query::parse()` and `refresh_rows()` (`on_search_event()`), no debounce.
- **Needs:** Debounced server requests; cancel or ignore stale responses.

#### SRC-9 More Gmail operators

- **Needs:** Parse and suggest more of Gmail's operators: `after:`/`before:`, `older_than:`/`newer_than:`, `larger:`/`smaller:`, `filename:`, `cc:`/`bcc:`, `list:`, `category:` (FOL-11), `is:important` (FOL-12), `has:drive`, `-` negation, `OR` and grouping. With server search most of these can pass through `q` untouched.

### 5.9 Contacts (CON)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| CON-1 | Recipient suggestions | 🟢 Fully linked | `people.searchContacts`, `otherContacts.search`, `people.searchDirectoryPeople` |
| CON-2 | Typed name and address resolution | 🟢 Fully linked | As CON-1 |
| CON-3 | Contact management | ⚪ Not done | `people.createContact`, `updateContact`, `deleteContact`, batch variants |
| CON-4 | Contact photos | ⚪ Not done | `photos` person field; `updateContactPhoto`, `deleteContactPhoto` |
| CON-5 | Contact card | ⚪ Not done | `people.get`, `people.getBatchGet` |
| CON-6 | Contact groups | ⚪ Not done | `contactGroups.*`, `contactGroups.members.modify` |
| CON-7 | Workspace directory | ⚪ Not done | `people.listDirectoryPeople`, `searchDirectoryPeople` |
| CON-8 | Save a frequent address as a contact | ⚪ Not done | `otherContacts.copyOtherContactToMyContactsGroup` |

#### CON-1 Recipient suggestions

- **Today:** Recipient fields and the event panel's "Add guests" show up to six suggestions: prefix matches first, then word-prefix ("ch" finds Sarah Chen), then substring for 3+ letters, fuzzy only when nothing else matches. Automated senders (no-reply, notification, newsletter, alerts, receipts…) and spam senders are never suggested; added recipients are excluded. Arrows, `Ctrl-n`/`Ctrl-p`, Enter or click, Escape. The book comes from the store: My Contacts first, then Other Contacts (both synced from People, SYN-6) and people sent to from here, each group by how often the address appears in the cached mail; then every other non-Spam sender and recipient in the mail by frequency (`Backend::contacts()`, `Store::contacts()` and `rank()` in `src/backend/store/contacts.rs`). Ranking in `ContactBook::suggest()` and `is_automated()` (`src/contacts.rs`). One `SharedContactBook` (`Rc<RefCell<ContactBook>>`) is shared with every composer and the calendar; `BackendEvent::ContactsChanged` updates it in place (`ContactBook::update()`, contacts keep their index so open suggestion lists stay right), so open composers see new contacts on the next keystroke. Recipients of a sent message join at once (`Backend::note_recipients()`, stored as `source = 'sent'` rows), in demo mode too.
- **Needs:** Nothing for this sprint (6.4 item 4: no live People search). Later, maybe `people.searchContacts` / `otherContacts.search` or `searchDirectoryPeople` for addresses not in the cache.
- **Notes:** Other Contacts tells us who the user has written to, a better signal than guessing "automated" from the local part.

#### CON-2 Typed name and address resolution

- **Today:** A typed address is used as is. A name followed by comma or semicolon takes `ContactBook::best()` over the synced book (CON-1). A name that matches no one stays in the field as text, with "No contact named “x”" under it (an invalid address: "“x” isn’t an email address"); `on_recipient_event()` in `src/ui/composer.rs`. On Enter, the highlighted suggestion is used if the text has no `@` or the user moved through the list; otherwise the typed address, or the error. `resolve()`, `commit()`.
- **Needs:** Nothing more from the backend.

#### CON-3 Contact management

- **Today:** Not built.
- **Needs:** People `createContact`, `updateContact` (needs the contact's `etag`), `deleteContact`; `batchCreateContacts`, `batchUpdateContacts`, `batchDeleteContacts` for bulk. A contacts screen or a "save contact" action.

#### CON-4 Contact photos

- **Today:** Initials avatars coloured by address (`avatar()` in `src/ui/mod.rs`) in the reader and the guest list.
- **Needs:** Photos from the `photos` person field, cached locally; fall back to initials. Optional editing with `updateContactPhoto`, `deleteContactPhoto`.

#### CON-5 Contact card

- **Needs:** Clicking a sender or guest shows name, addresses, phone, organisation and recent threads. `people.get` / `getBatchGet` for details; recent threads from search (`from:` / `to:`).

#### CON-6 Contact groups

- **Needs:** List groups (`contactGroups.list`, `batchGet`), suggest a group in recipient fields and expand it to its members (`contactGroups.get` with `maxMembers`, then `people.getBatchGet`). Optional: create, rename, delete groups and change members (`contactGroups.create`, `update`, `delete`, `contactGroups.members.modify`).

#### CON-7 Workspace directory

- **Needs:** For Workspace accounts, colleagues from the domain directory: `people.searchDirectoryPeople` (live) and `people.listDirectoryPeople` (cache, with sync token). Needs `directory.readonly`.

#### CON-8 Save a frequent address as a contact

- **Needs:** "Add to contacts" on an Other Contact: `otherContacts.copyOtherContactToMyContactsGroup`.

### 5.10 Images and attachments (ATT)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| ATT-1 | Remote images | 🟡 Partially done | None (HTTP fetch or proxy) |
| ATT-2 | Inline (`cid:`) images | ⚪ Not done | `payload.parts` `Content-ID`, `messages.attachments.get` |
| ATT-3 | Attachment list in messages | 🟡 Partially done | `payload.parts` (`filename`, `body.attachmentId`) |
| ATT-4 | Open and download an attachment | ⚪ Not done | `messages.attachments.get` |
| ATT-5 | Attachment icon in the list | 🟡 Partially done | `has:attachment`, `payload.parts` |
| ATT-6 | Attach files to a new message | 🟡 Partially done | `messages.send` / `drafts.*` upload (`multipart` or `resumable`) |
| ATT-7 | Forward keeps attachments | ⚪ Not done | `messages.attachments.get` + re-attach |

#### ATT-1 Remote images

- **Today:** Images show as a bordered box with an icon and the alt text (or "Image"); never downloaded, since GPUI here only has `NullHttpClient`. Tracking pixels (≤ 2 px) are dropped. `Block::Image` (`src/html.rs`) keeps only `alt`; `Renderer::block()` (`src/ui/html_view.rs`) draws the box.
- **Needs:** Load on demand or through a proxy so the user's IP and opens aren't exposed; cache results. The setting is in: Settings › Mail › Load remote images, Ask / Always / Never (W11, saved as `remote_images`).
- **Notes:** Needs an HTTP client dependency or the backend.

#### ATT-2 Inline (`cid:`) images

- **Today:** Shown as ATT-1 placeholders; `is_safe_url()` rejects `cid:`.
- **Needs:** Find the part whose `Content-ID` matches, fetch it (`messages.attachments.get` if it has an `attachmentId`), hand it to the renderer. IMAP: `BODYSTRUCTURE` + `BODY[part]`.

#### ATT-3 Attachment list in messages

- **Today:** Chips under each body: type icon, name, size. `Message::attachments`, `render_message()`, `file_size()`, `Attachment::kind()`, `attachment_icon()`. With Google (W3) they come from the raw message once the body loads (`mime::parse_message()`): RFC 2231/2047 names, decoded size, MIME type, `part_id` = the parser's part id. `.ics` parts and inline `cid:` images aren't listed.
- **Needs:** A tester pass. Gmail's `attachmentId` (for ATT-4) isn't in `format=raw`; opening needs `messages.get format=full` or the raw part.

#### ATT-4 Open and download an attachment

- **Today:** Chips are plain tags with no handler, hover or pointer.
- **Needs:** `messages.attachments.get` (base64url data), cache, open with Quick Look or the default app, save via a file dialog, progress for large files.

#### ATT-5 Attachment icon in the list

- **Today:** Paperclip when any message has an attachment (`Thread::has_attachments()`). With Google that's known only once the thread's bodies are loaded: `format=metadata` carries no parts.
- **Needs:** A thread-level flag without fetching bodies (for example `q=has:attachment` ids, or `payload.mimeType == multipart/mixed` as a hint).
- **Notes:** Inline `cid:` images don't count.

#### ATT-6 Attach files to a new message

- **Today:** The paperclip opens a file picker (multi-select); dropping files also attaches. Chips with icon, name, size and ×; kept on the draft and shown on the sent copy. `attach_paths()`, `Attachment::from_file()` (name, size, MIME from extension), `Attachment::path`. More than 25 MB in all is refused ("Attachments are over Gmail’s 25 MB limit"). With Google (W6) the files are read from disk when the draft is saved or the message sent and go in a `multipart/mixed` message, through `uploadType=multipart` above 5 MB; files of a draft opened from Gmail are taken from that draft.
- **Needs:** A tester pass (T2b). Progress for big uploads; `resumable` uploads; larger files as Drive links (see §9).
- **Notes:** A file moved or deleted before sending fails the send ("Not sent: couldn't read “x”"). Demo mode reads nothing.

#### ATT-7 Forward keeps attachments

- **Today:** Not built; `reply_content()` and `Quoted` carry only the quote.
- **Needs:** Fetch the original's attachments (ATT-4) and attach them on send.

### 5.11 Links (LNK)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| LNK-1 | Open a link in the browser | 🟡 Partially done | None |
| LNK-2 | Link safety checks | ⚪ Not done | None (optional Safe Browsing API) |
| LNK-3 | `mailto:` links | ⚪ Not done | None |

#### LNK-1 Open a link in the browser

- **Today:** Works without a backend: `cx.open_url()` in `Renderer::text()` on an `InteractiveText`; tooltip shows the URL. Only `http`, `https`, `mailto`, `tel` are clickable (`is_safe_url()`). URLs open exactly as written, tracking and redirect wrappers included.

#### LNK-2 Link safety checks

- **Needs:** Warn on suspicious links and text/URL mismatches; optionally unwrap redirects and strip tracking parameters. Network checks must not block the click for long.

#### LNK-3 `mailto:` links

- **Today:** Passed to the system mail handler, not Pigeon's composer.
- **Needs:** Open the composer with the address and `subject`, `body`, `cc`; pick the account (ACC-4). Optionally register Pigeon as the default mail app.

### 5.12 Mail settings (MST)

Gmail's `users.settings` resources. All need `gmail.settings.basic` unless noted.

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| MST-1 | Send-as aliases and From picker | ⚪ Not done | `settings.sendAs.list`, `get`, `patch`, `update` |
| MST-2 | Signatures | ⚪ Not done | `sendAs.signature` via `settings.sendAs.patch` |
| MST-3 | Vacation responder | ⚪ Not done | `settings.getVacation`, `updateVacation` |
| MST-4 | Filters | ⚪ Not done | `settings.filters.list`, `get`, `create`, `delete` |
| MST-5 | Forwarding and auto-forwarding | ⚪ Not done | `settings.forwardingAddresses.list`, `get`; `settings.getAutoForwarding` |
| MST-6 | IMAP and POP settings | ⚪ Not done | `settings.getImap`, `updateImap`, `getPop`, `updatePop` |
| MST-7 | Gmail display language | ⚪ Not done | `settings.getLanguage`, `updateLanguage` |
| MST-8 | Delegates | ⚪ Not done | `settings.delegates.*` (service accounts only) |
| MST-9 | S/MIME certificates | ⚪ Not done | `settings.sendAs.smimeInfo.*` |
| MST-10 | Client-side encryption | ⚪ Not done | `settings.cse.identities.*`, `settings.cse.keypairs.*` |

#### MST-1 Send-as aliases and From picker

- **Needs:** `sendAs.list` for every address the user may send from (feeds ACC-2), a From picker in the composer that defaults to `isDefault` (or, for replies, the alias the mail was sent to), and the alias's `displayName` and `replyToAddress` on sent mail. Users can edit those with `patch`/`update`.
- **Notes:** Creating, deleting and verifying aliases (`create`, `delete`, `verify`) are only available to Workspace service accounts with domain-wide delegation; send users to Gmail settings for that.

#### MST-2 Signatures

- **Needs:** Read each alias's `signature` (HTML) and insert it into new messages and replies; edit it in Settings and save with `sendAs.patch`. Gmail sanitises the HTML.
- **Notes:** The composer writes markdown; the signature is HTML, so it needs its own rendering in the composer.

#### MST-3 Vacation responder

- **Needs:** Settings page for `VacationSettings`: on/off, subject, plain or HTML body, start and end time, contacts-only and domain-only.

#### MST-4 Filters

- **Needs:** List, create and delete filters: `criteria` (from, to, subject, query, negatedQuery, hasAttachment, size) and `action` (addLabelIds, removeLabelIds, forward). "Filter messages like this" from a thread.
- **Notes:** The API has no filter update; edit = delete + create.

#### MST-5 Forwarding and auto-forwarding

- **Needs:** Show forwarding addresses (`forwardingAddresses.list`, `get`) and the auto-forwarding state (`getAutoForwarding`) read-only, with a link to Gmail to change them.
- **Notes:** `forwardingAddresses.create`/`delete` and `updateAutoForwarding` are service-account only (`gmail.settings.sharing`).

#### MST-6 IMAP and POP settings

- **Needs:** Low priority: these configure other clients. Show and toggle with `getImap`/`updateImap`, `getPop`/`updatePop` if wanted.

#### MST-7 Gmail display language

- **Needs:** Low priority: Pigeon follows macOS. `getLanguage` could help pick reply prefixes or date formats.

#### MST-8 Delegates

- **Needs:** Every `delegates` method (list, get, create, delete) is only available to service accounts with domain-wide delegation, so a consumer app can't manage delegates. Reading a mailbox someone delegated to you isn't in the Gmail API either. Tracked only so it isn't forgotten; see §10.

#### MST-9 S/MIME certificates

- **Needs:** Workspace with hosted S/MIME: list, upload (PKCS#12), set default and delete certificates per alias (`smimeInfo.list`, `insert`, `setDefault`, `delete`). Show signed/encrypted state on received mail.

#### MST-10 Client-side encryption

- **Needs:** Workspace Enterprise feature with an external key service and hardware keys for user-managed identities. The client must encrypt and decrypt mail itself. Later, if ever; see §10.

---

## 6. Calendar (CAL)

The calendar shares the window with mail. State lives in `CalendarView` (`src/ui/calendar/mod.rs`), owned by `PigeonApp`; model and pure logic in `src/calendar/` (`mod.rs`, `time.rs`, `recurrence.rs`, `layout.rs`).

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| **Views and navigation** | | | |
| CAL-1 | Switch between Mail and Calendar | 🟢 Fully linked | None (calendar scopes at sign-in) |
| CAL-2 | Calendar list, colours and visibility | 🟢 Fully linked | `calendarList.list`, `calendarList.patch` (`selected`), `colors.get` |
| CAL-3 | Load events for the visible range | 🟢 Fully linked | `events.list` (`timeMin`, `timeMax`, `syncToken`) |
| CAL-4 | Day, week and month views and navigation | 🟢 Fully linked | As CAL-3 |
| CAL-4a | Go to a date | 🟡 Partially done | None |
| CAL-18 | Today and the now-line | 🟢 Fully linked | None |
| CAL-19 | Event search in the palette | 🟢 Fully linked | `events.list` with `q` |
| CAL-23 | Persisted calendar UI state | ⚪ Not done | Local; `calendarList.selected` |
| **Events** | | | |
| CAL-7 | Edit an event in the panel | 🟢 Fully linked | `events.patch` / `events.update` with ETag |
| CAL-8 | Date, time and all-day editing | 🟢 Fully linked | `start`/`end` (`date` or `dateTime` + `timeZone`) |
| CAL-9 | Create an event | 🟢 Fully linked | `events.insert` |
| CAL-10 | Delete an event and undo | 🟢 Fully linked | `events.delete` (`sendUpdates`) |
| CAL-11 | IDs for new events | 🟢 Fully linked | Event `id`, `iCalUID` |
| CAL-12 | Time zones | 🟡 Partially done | `timeZone` fields; `settings` `timezone` |
| CAL-15 | Reminders | 🟢 Fully linked | `reminders` (`useDefault`, `overrides`) |
| CAL-16 | Busy and free | 🟢 Fully linked | `transparency` |
| CAL-17 | Video links | 🟡 Partially done | `conferenceData.entryPoints`, `hangoutLink` |
| CAL-21 | Drag to create, move and resize | 🟢 Fully linked | `events.insert`, `events.patch` |
| CAL-22 | Organizer and read-only permissions | 🟡 Partially done | `accessRole`, `guestsCanModify`, `locked`, `organizer.self` |
| CAL-28 | Event colours and labels | ⚪ Not done | `colorId` + `colors.get`; `eventLabelId` + `calendars` `labelProperties` |
| CAL-30 | Event attachments | ⚪ Not done | `attachments` (`supportsAttachments=true`) |
| CAL-31 | Event visibility and guest permissions | ⚪ Not done | `visibility`, `guestsCanModify`, `guestsCanInviteOthers`, `guestsCanSeeOtherGuests` |
| CAL-33 | Several reminders and default reminders | ⚪ Not done | `reminders.overrides` (`popup`, `email`), `calendarList.defaultReminders` |
| CAL-34 | Create Google Meet links | ⚪ Not done | `conferenceData.createRequest` (`conferenceDataVersion=1`) |
| CAL-39 | Quick add from text | ⚪ Not done | `events.quickAdd` |
| CAL-40 | Move an event to another calendar | 🟡 Partially done | `events.move` |
| **Recurrence** | | | |
| CAL-5 | Recurring events | 🟢 Fully linked | `recurrence` (RRULE, EXDATE), `events.instances`, `singleEvents` |
| CAL-6 | This event or all events | 🟢 Fully linked | Instance `events.patch`; series split with `UNTIL` |
| CAL-29 | Set and edit repeat rules | ⚪ Not done | `recurrence` |
| **Invites and RSVP** | | | |
| CAL-13 | Guests and invitations | 🟢 Fully linked | `attendees`, `sendUpdates` |
| CAL-14 | My RSVP | 🟡 Partially done | `attendees[].responseStatus` (`self`) |
| CAL-20 | Invite emails linked to events | 🟡 Partially done | `events.list` with `iCalUID`; iMIP parts in Gmail |
| CAL-32 | Optional guests, RSVP notes, extra guests and rooms | ⚪ Not done | `attendees[].optional`, `comment`, `additionalGuests`, `resource` |
| **Event types** | | | |
| CAL-35 | Focus time | ⚪ Not done | `eventType=focusTime`, `focusTimeProperties` |
| CAL-36 | Out of office | ⚪ Not done | `eventType=outOfOffice`, `outOfOfficeProperties` |
| CAL-37 | Working location | ⚪ Not done | `eventType=workingLocation`, `workingLocationProperties` |
| CAL-38 | Birthdays and special dates | ⚪ Not done | `eventType=birthday`, `birthdayProperties` |
| **Calendars and sharing** | | | |
| CAL-24 | Create, rename and delete calendars | ⚪ Not done | `calendars.insert`, `patch`, `update`, `delete` |
| CAL-25 | Subscribe to other calendars | ⚪ Not done | `calendarList.insert`, `calendarList.delete` |
| CAL-26 | Calendar sharing | ⚪ Not done | `acl.list`, `insert`, `patch`, `update`, `delete` |
| **Free/busy** | | | |
| CAL-27 | Free/busy and finding a time | ⚪ Not done | `freebusy.query` |
| **Settings** | | | |
| CAL-41 | Calendar settings from Google | ⚪ Not done | `settings.list`, `settings.get` |

### Views and navigation

#### CAL-1 Switch between Mail and Calendar

- **Today:** Rail icons, `cmd-1`/`cmd-2`, palette "Go to Mail/Calendar". Mail state is kept across switches. `PigeonApp::set_mode()`; `src/ui/rail.rs`. Sign-in asks for `calendar` (W2); without it the calendar pane shows "Calendar access wasn't granted. Sign in again to allow it." (`ui::status::missing_scope()`).
- **Needs:** A tester to confirm on the real account.

#### CAL-2 Calendar list, colours and visibility

- **Today:** The header's calendars button shows and hides calendars; palette "Show/Hide Calendar: X". Events take their calendar's colour. The menu groups "My calendars" (owner) and "Other calendars", marks reader calendars "View only", truncates long names and scrolls when long; rows are checkboxes for VoiceOver. `toggle_calendar()`; `render_calendars_button()` (`header.rs`). From Google (W4, `backend/calendar.rs`): `calendarList.list` (with `showHidden`, then its sync token) gives the name (`summaryOverride` else `summary`), colour (`backgroundColor`, else `colorId` through `colors.get`, fetched once and cached in `sync_state`), `selected` → visible, `hidden` (left out of the menu and not synced), `accessRole`, `primary`, `timeZone`, `defaultReminders` (`Store::put_google_calendar()`). Changes reach the view as `CalendarsChanged`.
- **Today (writes, W7):** Showing or hiding a calendar commits at once with `calendarList.patch` `{selected}` (`backend::calendar::commit()`); a failure reloads the list from the store, which puts the switch back.
- **Needs:** A tester pass on the real account (T2c).
- **Notes:** Shared and subscribed calendars are often read-only (CAL-22).

#### CAL-3 Load events for the visible range

- **Today:** All events in memory; `visible_range()` and `visible_occurrences()` filter on every render. From Google (W4): every calendar that isn't hidden or free/busy-only is synced whole, not by visible range: `events.list singleEvents=false timeZone=<local>` from 180 days back with no end, then by sync token (SYN-4), cached in the `events` table with Google's JSON beside the model. On the test account that is about 2,100 rows in 9 calendars, synced in 6–12 s the first time.
- **Needs:** Wiring `backend::calendar::sync_pass()` into the sync scheduler (merge of W3 and W4). Events older than 180 days aren't fetched (Load older isn't built for the calendar).

#### CAL-4 Day, week and month views and navigation

- **Today:** Day, Week (default), Month. `d`/`w`/`m`; `t` today; `n`/`p` period; `←`/`→` move the selected day; `↓`/`↑`, `j`/`k`, `tab`/`shift-tab` step through events past the period. Clicking a day number opens it. The grid scrolls to ~8am. Overlaps sit in equal columns, widening into free ones; columns narrower than 60pt overlap a little. Day view caps the event column at 640pt and shows a mini month and the day's list beside it. Week numbers via View menu or palette. `set_view()`, `open_day()`, `step()`, `step_day()`, `select_adjacent()` (`mod.rs`); `week_view.rs` (`layout::pack_columns()`, `column_box()`), `agenda.rs`, `mini_month.rs`, `month_view.rs` (`layout::pack_lanes()`, "+N more").
- **Needs:** Nothing beyond CAL-3: the whole future is cached, so next/previous event (62 days ahead) always finds what's there.
- **Notes:** Week start is fixed to Monday (`time::week_start()`); Google's `weekStart` setting could drive it (CAL-41).

#### CAL-4a Go to a date

- **Today:** Clicking the header title, `g d`, `cmd-shift-t`, palette or Go › Go to Date… opens a field over a small month. Reads typed dates ("nov 12", "next fri", "tomorrow", "in 2 weeks", "dec 2027"), English only. `toggle_date_picker()`, `go_to_date()`, `render_title()`, `time::parse_date()`.
- **Needs:** Nothing beyond CAL-3.

#### CAL-18 Today and the now-line

- **Today:** Today highlighted; a red now-line in Day and Week; past events faded. `today()`, `now()` from the system clock; Google's times are converted to the same local wall clock (CAL-12).
- **Needs:** None. A per-minute redraw timer isn't built; the line moves only on re-render.

#### CAL-19 Event search in the palette

- **Today:** In Calendar mode the palette finds events by title and jumps to the next occurrence. `PaletteCommand::Event` items in `palette_items()`; `reveal_event()`.
- **Needs:** `events.list` with `q` (matches title, description, location, attendees) and a cap, as PAL-2.
- **Notes:** Locally only titles are searched. With Google, the master of a series Pigeon can't expand also matches, and jumps to its first start (CAL-5).

#### CAL-23 Persisted calendar UI state

- **Today:** View and week numbers are saved with PER-3. Visibility changes queue `CalOp::SetSelected` (`calendarList.selected`), kept in demo mode's store until quit. The date resets on launch.
- **Needs:** Nothing for visibility (`calendarList.patch`, W7; CAL-2). The date still resets on launch.

### Events

#### CAL-7 Edit an event in the panel

- **Today:** Click, or `e`/`enter` on the selection, opens the right panel. Title, calendar, location, video link, guests, notes, reminder and busy/free edit in place; changes apply on every keystroke (`on_field_event()`); Esc closes. `open_event()`, `close_panel()`; `EventPanel` (`event_panel.rs`).
- **Today (writes, W7):** Edits settle, then go as one `events.patch` per event: when the panel leaves the event or after 1.5 s idle (`EDIT_DEBOUNCE`; `patched()`, `settle_edit()`, `flush_edit()` in `mod.rs`). The body carries only the changed fields (`backend::calendar::patch_body()`), with `If-Match` on the stored ETag; Google's answer is stored, so the next patch has the new ETag. A failure reloads Google's version (rollback) with a toast; a 412 says "Changed elsewhere; reloaded" (STA-5). A calendar sync runs 2 s after each committed write (`Backend::sync_calendar_soon()`). Read-only events (CAL-22) show their fields read-only.
- **Needs:** A tester pass on the real account (T2c). Edits still settling are lost if the app quits within 1.5 s. CalDAV: `PUT` with `If-Match`.
- **Notes:** No Save button, so failed writes must keep the user's text (STA-2). A sync that reloads the open event can replace fields being edited; the title, location and notes are read back from the panel's fields before each patch (`read_panel_text()`), the rest isn't.

#### CAL-8 Date, time and all-day editing

- **Today:** Date and time fields with pickers; typed values ("tomorrow", "Oct 27", "10/27", "3pm", "15:30", "noon"); up/down by a day or 15 minutes; all-day switch. Tab reaches every control in reading order; enter/space work switch and pickers. `commit_field()`, `step_field()`, `set_date()`, `set_time()`, `toggle_all_day()`; `time::parse_date()`, `parse_time()`. Changing the start keeps the duration; an end at or before the start moves to the next day; ends are ≥ 15 minutes after the start.
- **Today (writes, W7):** All-day writes `start.date`/`end.date` (exclusive end) and clears `dateTime`; timed writes the instant (UTC) in `dateTime` with `timeZone` (CAL-12) and clears `date`. Checked on the real account: all-day and back.
- **Needs:** A tester pass on the real account (T2c).

#### CAL-9 Create an event

- **Today:** `c` or the header "+" creates one at the next half hour (today or the selected day), opens it, focuses the title. Dragging or double-clicking the grid also creates (CAL-21). Goes to the first visible calendar, 10-minute reminder, busy, user as organizer. `new_event()`, `create_event()`. An untitled new event is dropped when its panel closes, without a toast.
- **Today (writes, W7):** `events.insert` once the new event has a title (on leaving the panel or after 1.5 s idle; `CalendarView::unsent`); an untitled one never reaches Google. With guests it asks "Send invitations to guests?" first (CAL-13).
- **Needs:** A tester pass on the real account (T2c).
- **Notes:** A default-calendar setting isn't built.

#### CAL-10 Delete an event and undo

- **Today:** `delete`/`backspace` or the panel's Delete; toast with Undo. `delete_occurrence()` / `delete_event()` queue a delayed `CalOp::Delete` and emit `CalendarEvent::Deleted`; `UndoAction::DeleteEvent` → `restore()`, which cancels the op, or re-inserts the events once it went. Series ask first (CAL-6). Every other calendar change queues a `CalOp` too (`CalendarView::queue()`, `patched()`).
- **Today (writes, W7):** The delete waits out the toast; Undo inside it cancels the op, so Google never sees it. Then `events.delete` (a 404/410 counts as done). With guests it asks "Send cancellation to guests?" first (`sendUpdates`). Undo after it went re-inserts with the same id; Google answers 409, and Pigeon restores the cancelled event with `status: confirmed` (checked on the real account). Someone else's event is declined (`responseStatus`, organizer told) and then deleted from the user's calendar, with the toast "Declined “…” and removed it" (CAL-22).
- **Needs:** A tester pass on the real account (T2c).
- **Notes:** Undo of "This event" after the delete went doesn't bring the occurrence back in Google (rare: the toast lasts as long as the undo window).

#### CAL-11 IDs for new events

- **Today:** A UUID's 32 hex digits (`CalendarView::new_id()`), valid as a client-chosen Google event id.
- **Today (writes, W7):** The UUID is the id `events.insert` sends, so the event keeps it in Google (checked on the real account). An occurrence taken out of its series ("This event") takes Google's instance id, `<series>_<UTC start>` or `<series>_<date>` (`backend::calendar::instance_id()`); the commit looks the instance up with `events.instances` and, if Google's id differs, the app swaps the event for Google's.
- **Needs:** A tester pass (T2c). `iCalUID` comes from Google once synced (CAL-20).

#### CAL-12 Time zones

- **Today:** Wall-clock times: `wall_time()` drops the JSON's UTC offset (`naive_local()`), and also reads the offset-less times the store writes; the model uses `NaiveDateTime`. The placeholder data uses Eastern offsets for San Francisco places, so converting would move every event. From Google (W4): events are read with `timeZone=<macOS zone>` and every `dateTime` is converted to the local wall clock (`time::LocalZone`, `iana-time-zone` + `chrono::Local`; no tz database). `Event::time_zone` keeps the event's own `start.timeZone` and `Calendar::time_zone` the calendar's, for writes. A timed series kept in another zone isn't expanded locally (its wall time moves across DST and its days can land on other dates here); Google's own occurrences are stored instead (CAL-5).
- **Today (writes, W7):** Writes send the instant with the event's own `timeZone`, else the macOS zone (`time_json()`), so a series keeps its zone.
- **Needs:** A tester pass (T2c). Google's `timezone` setting isn't read; the macOS zone is used.
- **Notes:** No zone picker or "shown in another zone" hint yet.

#### CAL-15 Reminders

- **Today:** One reminder (none, at start, 5, 10, 15, 30 min, 1 h, 1 day). `reminder_minutes`. Nothing fires.
- **Today (writes, W7):** Saved as `reminders` `{useDefault: false, overrides: [{popup, m}]}`; "No reminder" is `useDefault: false` with no overrides. Written only when changed, so a calendar default stays in use until the user picks one.
- **Needs:** A tester pass (T2c). Schedule macOS notifications, also after restart and with the window closed (NTF-3). Several reminders: CAL-33.

#### CAL-16 Busy and free

- **Today:** Busy/Free choice; drawn the same. `busy` on `Event`.
- **Today (writes, W7):** Saved as `transparency` (`opaque`/`transparent`). Busy/free and reminder changes alone don't ask about guests.
- **Needs:** A tester pass (T2c). It affects free/busy for others (CAL-27).

#### CAL-17 Video links

- **Today:** Shows and edits a link with a Join button (`cx.open_url()`). `video_link` in JSON. From Google (W4): `conferenceData.entryPoints` (the `video` one) or else `hangoutLink` fills `Event::conference_uri` and `video_link`, so Join works.
- **Today (writes, W7):** A link from `conferenceData`/`hangoutLink` is read-only in the panel and never written. A link typed in Pigeon is saved into the location when that's empty, else at the top of the description (6.4 item 9; `place()`); after the next sync it shows there, not in the video field.
- **Needs:** A tester pass (T2c). Phone and PIN entry points aren't shown, nor a URL found in location or description. Creating Meet links: CAL-34.

#### CAL-21 Drag to create, move and resize

- **Today:** Day and Week grids: drag empty space to create (live preview snapped to 15 minutes, times floating above; opens with title focused on release); double-click for 30 minutes; drag to move across time and days; drag the bottom edge to resize (≥ 15 min). Auto-scroll near edges; < 4 px is a click; Escape cancels. `src/ui/calendar/drag.rs` (`GridDrag`, `press_grid()`, `press_occurrence()`, `drag_to()`, `finish_drag()`) calls `create_event()` and `retime_occurrence()`. The all-day lane and Month view can't be dragged.
- **Today (writes, W7):** A move or resize writes once on release (`flush_edit()` in `finish_drag()`), asking about guests when the event has any; a drag-create inserts once titled (CAL-9). Series ask "This event / All events" first (CAL-6). Failures reload Google's version, which snaps the event back (STA-2). Read-only events (CAL-22) don't move and have no resize edge.
- **Needs:** A tester pass on the real account (T2c).

#### CAL-22 Organizer and read-only permissions

- **Today (W7):** `backend::calendar::access()` gives Edit, Rsvp or ReadOnly: a reader or free/busy calendar is read-only; on a writer/owner calendar the organizer (`organizer.self`, or the user's address, as in demo mode) or a guest with `guestsCanModify` edits; birthdays and events from Gmail don't; an invited user only answers. Read-only events show their fields read-only with a line saying why, no guest field, no resize edge, and can't be dragged; Delete is hidden. For an invite, Delete reads "Decline and remove" and declines it, then deletes the user's copy (CAL-10). The calendar picker (CAL-40) is only for the organizer of a whole event, and lists calendars the user can't write to faded. Applies in demo mode too.
- **Needs:** A tester pass (T2c). `locked` and `privateCopy` aren't read.
- **Notes:** Guests can usually change only their RSVP, reminders and, on their copy, busy/free.

#### CAL-28 Event colours and labels

- **Needs:** Per-event colour override: legacy `colorId` from the `colors.get` event palette, or the newer labels: `eventLabelId` pointing at the calendar's `labelProperties.eventLabels` (custom name and colour, up to 200 per calendar; requires `eventLabelVersion=1`, which makes `colorId` ignored). Managing labels needs owner access (`calendars.update`).

#### CAL-30 Event attachments

- **Needs:** Show `attachments` (Drive `fileUrl`, `title`, `mimeType`, `iconLink`; up to 25) and open them in the browser; adding them needs `supportsAttachments=true` and a Drive file (§9).

#### CAL-31 Event visibility and guest permissions

- **Needs:** `visibility` (`default`, `public`, `private`) and the guest switches `guestsCanModify`, `guestsCanInviteOthers`, `guestsCanSeeOtherGuests` in the panel.
- **Notes:** On a recurring event, making one instance more private applies to the whole series.

#### CAL-33 Several reminders and default reminders

- **Needs:** More than one reminder per event, `email` as well as `popup` (`reminders.overrides`, up to 5), and the calendar's `defaultReminders` from `calendarList` (edited with `calendarList.patch`). `notificationSettings` on `calendarList` controls Google's own emails.

#### CAL-34 Create Google Meet links

- **Needs:** "Add Google Meet" in the panel: `conferenceData.createRequest` with a `requestId` and `conferenceSolutionKey.type=hangoutsMeet`, sent with `conferenceDataVersion=1`; poll until `status.statusCode` is `success`. Google's `autoAddHangouts` setting (CAL-41) says whether to add one by default.

#### CAL-39 Quick add from text

- **Needs:** Optional: `events.quickAdd` creates an event from text ("Lunch with Sam Friday 1pm"). Pigeon's own parsers (`time::parse_date`, `parse_time`) may be enough; quickAdd needs a network round-trip before the event shows.

#### CAL-40 Move an event to another calendar

- **Today:** The panel's calendar picker (`render_calendar_picker()` in `event_panel.rs`) changes the event's calendar. W7: it goes as `events.move` with `destination` (`CalOp::Move`, before any patch of the same edit); Google's answer is stored under the new calendar. Only the organizer of a whole event (not one occurrence) gets the picker; calendars the user can't write to are faded and can't be picked.
- **Needs:** A tester pass on the real account (T2c; not checked yet, as it touches a second calendar). Birthdays, focus time, out of office and working location events can't move.

### Recurrence

#### CAL-5 Recurring events

- **Today:** `recurrence` is a `Rule` (`src/calendar/recurrence.rs`): `freq`, `interval`, `by_day` (with optional ordinals), `by_month_day`, `count`, `until` (the series' end, exclusive; RRULE's inclusive UNTIL is converted both ways), plus `unsupported` and `custom` flags. The placeholder data's keywords become constructors; `Rule::label()` says "Every weekday", "Every 2 weeks", "Every week on Mon, Wed", "Every month on the last Fri", or "Custom repeat". `Rule::parse()` / `to_rrule()` read and write RRULE; `parse_recurrence()` reads Google's `recurrence` lines (RRULE, EXDATE in the local zone, a date, or UTC). Expansion (`recurrence::occurrences()`, visible range only) covers `FREQ` daily to yearly, `INTERVAL`, `BYDAY` (ordinals for monthly), `BYMONTHDAY` (negative too), `COUNT`, `UNTIL` and exceptions. With Google (W4), series come once with their rule (`singleEvents=false`); every instance Google lists on its own, moved or cancelled, is stored (cancelled ones as hidden rows) and the series' `exceptions` are recomputed as its EXDATEs plus those instances' `original_start`. Rules outside the subset (BYSETPOS, BYMONTH, BYWEEKNO, hourly…), RDATE/EXRULE, EXDATEs in another zone, and timed series kept in another zone are flagged `unsupported`: not expanded, and `events.instances` (180 days back to 400 ahead) is stored as concrete occurrences instead. Checked against Google's own expansion (`singleEvents=true`) on the test account: the same 913 occurrences over 70 days, across the DST change.
- **Needs:** Nothing more for reading. Writes of series scopes are CAL-6 (W7).
- **Notes:** Instances of an unsupported series show as occurrences "changed separately" in the panel's repeat line (W7 could say "part of a series" instead). Their window doesn't slide until the series changes or the calendar fully resyncs.
- **Notes:** The UI can't set a rule (CAL-29); new events never repeat.

#### CAL-6 This event or all events

- **Today:** Changing, dragging or deleting one occurrence asks which ones. In the panel, the first change shows "Apply this change to: This event / All events" (shown on the whole series until answered; closing the panel, escape, clicking an empty grid slot, opening another event, dragging or deleting before answering takes the change back, like Google Calendar's Cancel, and nothing is written: `EditScope`, `drop_unscoped_edit()`). Dropping a drag asks over the grid; enter = "This event", escape cancels. Delete asks "This event / This and following / All events" (escape cancels), then "Send cancellation to guests?" when there are guests (Cancel keeps the event); a panel change answered "All events" then asks about guests, and Cancel there puts the series back. Undo restores any. "This event" copies the occurrence into its own event (`detach()`) and adds an exception; "This and following" sets `until` and drops later changed occurrences; "All" changes or deletes the series and its exceptions. `SeriesPrompt`, `answer_prompt()`, `delete_occurrence()`, `restore()` (`mod.rs`); `choose_edit_scope()` (`event_panel.rs`).
- **Today (writes, W7):** "This event": the occurrence takes Google's instance id (CAL-11); the commit finds the instance with `events.instances` around its original start and patches only what differs from the series (`If-Match` on the instance's ETag); a delete deletes the instance. "This and following": the series' RRULE gets `UNTIL` just before the occurrence (`rewrite_rrule()`: COUNT dropped, other parts and EXDATE lines kept as Google wrote them), with `If-Match`. "All": patches the series; a retime that moves the series' end rewrites `UNTIL` the same way. Changing an instance changes its series' ETag, so the series is read again after each instance write. The panel holds its edits back until "This / All" is answered, so a "This event" change never touches the series in Google: the series' own patch on detaching (only its new exception, so an empty body, not sent) doesn't read the panel's text, which is the occurrence's (`send_edit()`; before, it carried the new title to the series and every occurrence). All three checked on the real account's `[Pigeon test] series`; the panel's "This event" re-checked live after that fix (only the one instance changed).
- **Needs:** A tester pass (T2c). "This and following" for edits (series split) isn't offered. CalDAV: `RECURRENCE-ID` overrides or `EXDATE`.
- **Notes:** "This and following" is offered for deletes only. Series edits don't reach occurrences changed on their own, as in Google Calendar.

#### CAL-29 Set and edit repeat rules

- **Needs:** A repeat picker in the panel (presets plus custom: interval, weekdays, monthly by date or weekday, end by date or count) writing `recurrence` RRULE strings; show the rule in words for any RRULE Google returns.

### Invites and RSVP

#### CAL-13 Guests and invitations

- **Today:** The panel lists organizer and guests with RSVP icons and a summary ("3 people · 2 yes · 1 awaiting"). "Add guests" suggests contacts (CON-1); `enter` adds, `backspace` on empty removes the last. `render_guests()`, `update_guest_suggestions()`, `add_guest()`, `remove_guest()`. Nothing is sent and the user isn't asked.
- **Today (writes, W7):** Guest changes save `attendees`, keeping Google's entries for the guests who stay (rooms included, though not shown). When a save or delete touches an event with guests other than the user, a card over the grid asks "Send updates to guests?" (or "Send invitations…", "Send cancellation…"): Send (enter) → `sendUpdates=all`, Don't send → `none`, Cancel (esc) puts the event back (`GuestPrompt`, `answer_guests()`). Changes only to the user's reminder or busy/free don't ask (`asks_guests()`). A new question while one is open answers the old one "Don't send".
- **Needs:** A tester pass on the real account (T2c); whether invites arrive at the second address is checked by the user later (6.4 item 6). `externalOnly` isn't offered. CalDAV: iMIP. Free/busy for guests: CAL-27.
- **Notes:** With `attendeesOmitted` the guest list is never written.

#### CAL-14 My RSVP

- **Today:** For others' events the panel shows "Going? Yes / Maybe / No", as does the invite card (CAL-20). Unanswered invites are outlined; declined ones struck through. `set_rsvp()`, `Event::my_status()`, `Look` (`week_view.rs`).
- **Today (writes, W7):** The last answer goes after 1.5 s (or when the panel leaves the event) as an `events.patch` of `attendees` where only the user's own entry (`self`) changes `responseStatus`, with `sendUpdates=all` so the organizer is told. One write serves the panel and the card.
- **Needs:** A tester pass with a real invitation (T3b item 3).
- **Notes:** Answering for one occurrence of a series isn't supported (CAL-6).

#### CAL-20 Invite emails linked to events

- **Today:** An invitation mail shows a card above its body: title, time, repeat rule, location, video link with Join, organizer and head count, in the calendar's colour, plus a mini timeline of the user's other events that day naming busy overlaps ("Conflicts with …"). Yes / Maybe / No (`shift-y`, `shift-m`, `shift-n`) answer it; "Open in Calendar" (`shift-c`). A cancellation shows a muted card with "Remove from calendar" (`shift-x`, with Undo). In the palette and `?` sheet. `Message::invite` (`Invite { event_id, method }`, `request`/`cancel`) in `src/data.rs`; `src/ui/invite.rs` reads `CalendarView::data` and answers via `set_rsvp()`, so card and grid stay in step.
- **Needs:** Find the event by `iCalUID` (`events.list` with `iCalUID`); Google usually adds invitations sent to Gmail to the calendar itself. For non-Google invites, parse `text/calendar` parts (iMIP `REQUEST`, `CANCEL`, `REPLY`) and `events.import` a private copy. Answers write as CAL-14. "Remove from calendar" deletes only the user's copy and sends nothing.
- **Notes:** The card shows the next occurrence from today; an invite whose event is gone says "This event is no longer in your calendar". Conflicts count only the user's own busy, non-declined events, not shared calendars.

#### CAL-32 Optional guests, RSVP notes, extra guests and rooms

- **Needs:** Mark guests `optional`; add a note to an RSVP (`comment`) and a `+N` count (`additionalGuests`); show room and resource attendees (`resource=true`) apart from people.

### Event types

#### CAL-35 Focus time

- **Needs:** Show and create `focusTime` events (`focusTimeProperties`: `autoDeclineMode`, `declineMessage`, `chatStatus`). Timed only, primary calendar only, not available to every account.

#### CAL-36 Out of office

- **Needs:** Show and create `outOfOffice` events (`outOfOfficeProperties`: `autoDeclineMode`, `declineMessage`). Timed only, primary calendar only, not available to every account. Distinct from the Gmail vacation responder (MST-3).

#### CAL-37 Working location

- **Needs:** Show the user's working location per day (`workingLocationProperties`: `homeOffice`, `officeLocation`, `customLocation`), for example as an all-day chip; create and edit it. Primary calendar only, mostly Workspace.

#### CAL-38 Birthdays and special dates

- **Needs:** Show `birthday` events (annual, all-day) with `birthdayProperties` (`type`: birthday, anniversary, custom, other, self; `contact` links to People as `people/c…`). The API reads them and creates only `type=birthday`; they can't be moved to another calendar.

### Calendars and sharing

#### CAL-24 Create, rename and delete calendars

- **Needs:** Secondary calendars: `calendars.insert` (summary, description, time zone), `calendars.patch`/`update`, `calendars.delete`. Per-user colour and name override via `calendarList.patch` (`backgroundColor` with `colorRgbFormat=true`, `summaryOverride`).

#### CAL-25 Subscribe to other calendars

- **Needs:** Add an existing calendar (a colleague's, holidays) by ID with `calendarList.insert`; remove it from the list with `calendarList.delete` (doesn't delete the calendar).

#### CAL-26 Calendar sharing

- **Needs:** Show and change who can see a calendar: `acl.list`, `insert`, `patch`/`update`, `delete`. Roles `none`, `freeBusyReader`, `reader`, `writer`, `owner`; scopes `default` (public), `user`, `group`, `domain`. Owner access needed.

### Free/busy

#### CAL-27 Free/busy and finding a time

- **Today:** The invite card's conflict check uses only the user's own loaded events.
- **Needs:** `freebusy.query` (`timeMin`, `timeMax`, `items` of calendar IDs or guest emails) to show guests' busy blocks in the panel or grid and suggest a free slot; include shared calendars in conflict checks.

### Settings

#### CAL-41 Calendar settings from Google

- **Needs:** Read the user's Google Calendar settings with `settings.list` (read-only in the API; `settings.watch` for changes): `weekStart`, `timezone`, `format24HourTime`, `hideWeekends`, `defaultEventLength`, `showDeclinedEvents`, `hideInvitations`, `remindOnRespondedEventsOnly`, `locale`, `dateFieldOrder`, `autoAddHangouts`, `useKeyboardShortcuts`. Use them as defaults: week start (CAL-4), zone (CAL-12), new-event length (CAL-9), Meet by default (CAL-34), declined events.

---

## 7. Sync, notifications and states

### 7.1 Sync and push (SYN)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| SYN-1 | Gmail full sync | 🟡 Partially done | `threads.list` / `messages.list` + batched `get`; store `historyId` |
| SYN-2 | Gmail partial sync | 🟡 Partially done | `history.list` (`startHistoryId`, `historyTypes`) |
| SYN-3 | Gmail push | ⚪ Not done | `watch`, `stop`, Cloud Pub/Sub |
| SYN-4 | Calendar incremental sync | 🟡 Partially done | `events.list` / `calendarList.list` with `syncToken` |
| SYN-5 | Calendar push | ⚪ Not done | `events.watch`, `calendarList.watch`, `acl.watch`, `settings.watch`, `channels.stop` |
| SYN-6 | Contacts sync | 🟡 Partially done | `people.connections.list`, `otherContacts.list` with `syncToken` |
| SYN-7 | Batching, quotas and backoff | 🟡 Partially done | HTTP batch; `429` / `403 rateLimitExceeded` |

#### SYN-1 Gmail full sync

- **Today:** Built (W3, `src/backend/sync.rs`), as a plan of steps each stored and reported before the next: `getProfile` (history id H0 kept first), `labels.list` + batched `labels.get` (counts), `settings.sendAs` (aliases), Inbox (`threads.list labelIds=INBOX maxResults=100`, then `threads.get format=metadata` in batches of 20), drafts, the first 50 of Sent and Starred, 25 of Spam and Trash, then a backfill (`q=newer_than:90d`) to 1,000 threads. Threads whose stored `history_id` matches the listed one are skipped. On completion partial sync continues from H0. A step that fails (after the transport's own retries) doesn't restart the sync: the next pass resumes from that step, and a failed thread fetch retries only the threads that failed. The status shows "Syncing done of total". Every request declares its quota cost.
- **Needs:** W2's authenticated transport wired into `Backend::start_mail_sync()`; the quota limiter (W2). A tester pass (T1a).

#### SYN-2 Gmail partial sync

- **Today:** Built (W3): `history.list` from the stored `gmail_history_id`, paged; touched threads refetched in batches (404 or drafts-only → removed), drafts re-read when a draft was touched, label counts refreshed when anything changed, then the new history id stored. A 404 runs the full sync again, skipping unchanged threads. Every 45 s while active, 5 min in the background, on focus and on Sync Now; two network failures in a row → Offline with backoff 15 s → 2 min.
- **Needs:** A tester pass (T1a item 5). Sync 2 s after a committed write (W5). The pending-op overlay (2.7) is W5's.

#### SYN-3 Gmail push

- **Needs:** `watch` on the mailbox (optionally filtered by `labelIds`) publishing to a Cloud Pub/Sub topic that `gmail-api-push@system.gserviceaccount.com` may publish to; renew before it expires (at most 7 days; Google suggests daily); `stop` on sign-out. Each notification carries a `historyId` that triggers SYN-2.
- **Notes:** Needs our own Cloud project and a subscriber (a server or a pull subscription), so it depends on the ACC-8 decision. Polling SYN-2 works without it.

#### SYN-4 Calendar incremental sync

- **Today:** `backend::calendar::sync_pass(transport, store, account)` (W4) does one pass, blocking, off the main thread: `colors.get` once (cached), `calendarList.list` (full with `showHidden`, then by sync token; calendars gone from a full list or marked `deleted` lose their events), then per calendar `events.list singleEvents=false maxResults=2500 timeZone=<local>` with `timeMin=now−180d` on a full sync, paged to the end, storing `nextSyncToken` in `calendars.sync_token`; later passes send only `syncToken`. Cancelled items delete the event (and its instances) or, for an instance, become an exception of its series. A 410 wipes that calendar's events and syncs it in full again; a full listing replaces what was stored. A calendar Google refuses (403/404) is skipped and named in `CalendarChanges::errors`; a network failure stops the pass with an error. It returns `CalendarChanges { calendars, changed, removed }`, and `backend_events()` turns them into `CalendarsChanged` / `EventsChanged` / `EventsRemoved`; the app reloads just those ids in place (an event shared into several calendars is replaced copy by copy), so the view's date and selection stay.
- **Needs:** The scheduler (W3's `sync.rs`) calling it every 2 min while the window is active, on focus, and ~2 s after a committed calendar write, then emitting `backend_events()` (merge). Status line wiring for errors (ACC-5).

#### SYN-5 Calendar push

- **Needs:** `events.watch` (and `calendarList`, `acl`, `settings` watch) to an HTTPS webhook; channels expire and must be renewed; `channels.stop` to cancel. Each notification triggers SYN-4.
- **Notes:** Needs a public HTTPS server; a desktop-only backend polls SYN-4 instead.

#### SYN-6 Contacts sync

- **Today:** `src/backend/people.rs`: `people.connections.list personFields=names,emailAddresses` and `otherContacts.list readMask=names,emailAddresses`, both `requestSyncToken=true`, paged by 1,000, then by `syncToken` (deleted people go, changed ones are replaced). An expired token (400 `EXPIRED_SYNC_TOKEN` or 410) lists everything again. Stored per address in `contacts` with the tokens in `sync_state` (`people_sync_token`, `other_contacts_sync_token`), in one transaction (`Store::put_people()`). Run on launch, then every 30 minutes after a mail pass (`sync_contacts()` in `src/backend/sync.rs`); each list only when its scope (`contacts.readonly`, `contacts.other.readonly`) was granted. Emits `ContactsChanged` when anything changed. Read-only.
- **Needs:** Nothing more for this sprint.

#### SYN-7 Batching, quotas and backoff

- **Today:** W2, in `src/backend/http.rs`. `GoogleTransport` (what `Backend::transport()` hands sync) adds the access token, waits on a token-bucket limiter in Gmail quota units (6,000/min, 1,000 kept for user actions: `send_user()`; every `Request` declares `cost`), keeps at most 4 calls in flight, and retries network errors, 429, 5xx and 403 `rateLimitExceeded`/`userRateLimitExceeded` with truncated exponential backoff (1, 2, 4 … 64 s plus up to 1 s jitter, 8 tries). `UreqTransport` logs each call as `METHOD host/path status ms`, query redacted. HTTP batch encode/decode is from W1. `Gmail::batch` (`gmail.rs`) sends at most 20 calls per batch (Gmail runs a batch's calls in parallel and answers 429 "Too many concurrent requests for user" at 50), one batch at a time across sync and body fetches, and retries just the calls a batch answered with 429, 5xx or a rate-limit 403, on the same backoff schedule; other per-call errors (404 etc.) are answered as before.
- **Needs:** W3/W5 to batch their gets and modifies; `fields=` partial responses; a "Waiting for Gmail…" status during long backoffs (2.9). Group calls into HTTP batch requests (up to 100 calls; Gmail advises at most 50) for list-then-get and bulk actions (ACT-6). Respect per-user rate limits with exponential backoff on `429` and `403 rateLimitExceeded`/`userRateLimitExceeded`. Ask for partial responses (`fields`) to cut payloads.

### 7.2 Notifications and background (NTF)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| NTF-1 | New mail notifications | ⚪ Not done | From SYN-2 / SYN-3 |
| NTF-2 | Unread badge | ⚪ Not done | `labels.get` (`INBOX`) |
| NTF-3 | Background sync when the window is closed | ⚪ Not done | SYN loop |

#### NTF-1 New mail notifications

- **Needs:** A macOS notification with sender and subject for new Inbox mail; clicking opens the thread. Respect Focus; per-label mute if added.

#### NTF-2 Unread badge

- **Needs:** Dock badge from the server's Inbox unread count (FOL-2), not loaded data.

#### NTF-3 Background sync when the window is closed

- **Today:** `main()` quits when the last window closes (`cx.on_window_closed`); quitting stops everything and loses state (PER-2).
- **Needs:** Keep syncing and holding the outbox in the background (stay running, a login item, or a helper), or catch up on launch. Calendar reminders (CAL-15) need this too.

### 7.3 Loading, error and offline states (STA)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| STA-1 | Initial load and loading states | 🟡 Partially done | None |
| STA-2 | Action failure and rollback | ⚪ Not done | API error responses |
| STA-3 | Offline mode and queued changes | ⚪ Not done | None |
| STA-4 | Expired sign-in | 🟡 Partially done | OAuth `invalid_grant`, 401 |
| STA-5 | Conflicts with server changes | 🟡 Partially done | ETags, `historyId`, `syncToken` |

#### STA-1 Initial load and loading states

- **Today:** Launch reads the cache (`Backend::load_mail()`) and never waits on the network. List: skeleton rows while a view's first page loads, "Couldn't load <view>" if the first sync failed, the empty text only once loaded (`view_loaded()`, W3). Reader: "Loading…" per message, "Couldn't load this message" with Retry. The sidebar status line is W2's.
- **Today (W11):** The status line shows Offline and "Waiting for Gmail…" over sync's state (ACC-5, STA-3).
- **Needs:** A tester pass (T3b items 1–2): an unloaded body while offline reads "isn't downloaded yet", Retry after reconnecting.

#### STA-2 Action failure and rollback

- **Today:** Every handler changes `data` and calls `cx.notify()` without checking anything.
- **Today (W11):** `OpFailed` shows an error toast (`PigeonApp::show_error_toast()`): the danger tone from `theme.rs` with an alert icon, no Undo, `Role::Alert`. Success and Undo toasts are unchanged.
- **Needs:** Pending state per change, server confirmation, rollback of folder, flags and list position on failure (W5). Send failures: CMP-12.

#### STA-3 Offline mode and queued changes

- **Today (indicator, W11):** Offline shows within about 10 s of the network going away: a probe (TCP connect to `gmail.googleapis.com:443`, no request) runs every 8 s while the window is active (60 s in the background); two failures in a row are Offline, as are two network failures in a row in the transport. Offline, probes back off 15 s → 2 min; the first that connects, or a sync that succeeds, ends it and runs a sync and the queue at once. Once the transport is offline, a network error fails at once instead of retrying 8 times (`GoogleTransport::send_with()`); polls do the retrying. The line reads "Offline · N changes waiting" from the outbox's count, recounted when a change is queued.
- **Today (dev switch, 6.4 item 6):** With `PIGEON_DEV=1`, every request fails as a network error while `<data dir>/simulate-offline` exists (`touch` it / `rm` it; `UreqTransport::send()` in `src/backend/http.rs`, and the probe). Never active without `PIGEON_DEV=1`.
- **Needs:** A visible offline state; changes queued in the local store (PER-2) and sent when back online; reading from the cache (PER-1); sends held in the outbox; queue size and pending items shown.

#### STA-4 Expired sign-in

- **Today:** W2. `invalid_grant` on refresh marks the sign-in expired: requests fail with `TransportError::AuthExpired`, the status line shows a "Sign in again" button, and signing in again (same browser flow) keeps the cache and the outbox. A missing `google_client.json` with a stored account shows the same state.
- **Needs:** Detect auth errors on every call, refresh (ACC-7), and if refresh fails (`invalid_grant`) show "sign in again" while keeping local changes and the outbox.

#### STA-5 Conflicts with server changes

- **Today:** A sync that changes the open thread reloads it in place, keeping the selection (W3); drafts open in the composer aren't replaced. W11: the reader's expanded and focused messages are kept by message id across the reload (`open_messages()` / `restore_open_messages()` in `src/app/backend_events.rs`), so a new message doesn't shift them, and the message that was last stays open when a newer one arrives.
- **Today (events, W7):** Event patches carry `If-Match`; a 412 reloads Google's version into the store and the view, drops the op, and shows "Changed elsewhere; reloaded" (checked on the real account with a stale ETag).
- **Needs:** Rules for drafts changed elsewhere. Links to DRF-6, LST-8.

---

## 8. App-wide

### 8.1 Storage and persistence (PER)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| PER-1 | Local mailbox store | 🟡 Partially done | Holds SYN data |
| PER-2 | Outbox and drafts that survive quit | ⚪ Not done | None |
| PER-3 | Persisted UI state and settings | 🟢 Fully linked | None |

#### PER-1 Local mailbox store

- **Today:** SQLite (`rusqlite`, bundled) in `src/backend/store/`: `Store` (one connection behind a mutex, WAL, file 0600 in a 0700 folder), the full schema as migration block 001 in `schema.sql` (settings, labels, threads, messages, bodies, drafts, calendars, events, contacts, sync_state, outbox, images; account-scoped tables keyed by `account`), and reads and writes for mail (`store/mail.rs`), calendar (`store/calendar.rs`, events kept whole as JSON in `model`), contacts (`store/contacts.rs`) and the outbox (`store/outbox.rs`). Demo mode uses an in-memory store seeded from the JSON (`backend/demo.rs`); the UI's `MailData` and `CalendarData` are built from the store at launch (`Backend::load_mail()`, `load_calendar()`).
- **Needs:** Google mode on the file store at `~/Library/Application Support/Pigeon/pigeon.sqlite` (or `$PIGEON_DATA_DIR`) once W2 signs in; syncs writing to it (SYN-1/2/4/6); body eviction (300 MB LRU) and the image cache (ATT-1). Later migrations go in their reserved blocks (`docs/BACKEND_SPRINT_1.md`, 4.2).

#### PER-2 Outbox and drafts that survive quit

- **Today:** Every `MailOp` and `CalOp` is a row in the store's `outbox` table (`src/backend/outbox.rs`) before it commits. Sends, discards, moves, deletes and event deletes wait out the 5 s undo window (`UNDO_WINDOW`); Undo inside it deletes the row (`Backend::cancel()`), past it the UI queues the inverse op. Consecutive label changes to the same threads merge; a newer save of a draft replaces an older one; ops on one thread, event or draft go one at a time. The worker (`Backend::schedule()`) commits due rows; demo mode commits by applying the op to its in-memory store (`Store::apply_mail_op()`, `apply_cal_op()`), so nothing survives a quit there. Quit (⌘Q, the menu, closing the last window) runs `Backend::flush_on_quit()` first (W11, `src/main.rs` `quit()`): sends still in their undo window become due at once (`Outbox::send_now()`), and Quit waits until nothing due is left to commit, at most 5 s (`health::QUIT_WAIT`); other queued rows stay for the next launch.
- **Needs:** The Google commit, retries, rollback and pending-op overlay (W5); rows left at quit flushed on the next launch (the worker already picks them up). A tester pass of Quit inside a send's undo window (T4). Deviation from the plan: the store takes an op's change when it commits, not when it's queued, so Undo never has to revert the store; reads during the window come from the UI's view model and, after a sync, W5's overlay.

#### PER-3 Persisted UI state and settings

- **Today:** Saved to the `settings` table (one row per key, JSON values) of the on-disk store, in demo mode too: appearance, Reduce motion, auto-advance, default search scope, list density, text size, sidebar shown, sidebar and list widths, Archived expanded, calendar view, week numbers, and remote images (Ask/Always/Never, for ATT-1). `Saved` (`src/settings.rs`) is the snapshot; `PigeonApp::save_settings()` hands it to `Backend::save_settings()` whenever the app or the calendar notifies and it changed (written 300 ms after it settles, and on quit); `main()` loads it and `PigeonApp::apply_settings()` applies it before the first frame. Unknown keys are ignored and bad values take their default. The file is `~/Library/Application Support/Pigeon/pigeon.sqlite`, or `$PIGEON_DATA_DIR/pigeon.sqlite`. Still in memory: `nav`, `expanded_messages`, recent searches, recent palette commands, the window's size.
- **Today (W11):** Settings › Mail › Load remote images (Ask / Always / Never) changes and saves `saved.remote_images`, which the reader reads (ATT-1). The plan's list (appearance, motion, auto-advance, search scope, density, text size, pane widths, images) is all saved.
- **Needs:** Nothing from the server.
- **Notes:** `expanded_messages` is per thread; keep it or accept the reset.

### 8.2 Command palette (PAL)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| PAL-1 | Command list | 🟡 Partially done | Each command's own |
| PAL-2 | Thread search in the palette | 🟡 Partially done | `threads.list` with `q` |
| PAL-3 | Label jumps | 🟡 Partially done | `labels.list` |

#### PAL-1 Command list

- **Today:** `cmd-k` or `cmd-shift-p` lists every command in `COMMANDS` (`src/actions.rs`) with its shortcut and runs the same handlers. `open_palette()`, `palette_items()`, `on_palette_event()` (`src/app.rs`). Unavailable commands are left out (`When`, `command_available()` in `src/app/mail.rs`). Recent commands first on an empty query (in memory, PER-3). Each `Group` has a `scope()` (Mail, Calendar, Global); the palette and `?` sheet list the current mode's commands plus global ones. `v` and `l` open it scoped to folders or labels.
- **Needs:** Each command inherits its action's needs: ACT-1–6, CMP-1–4, CMP-8, DRF-1, DRF-4, UND-1–3, ACT-1a, FOL-5, CAL-9, CAL-10. Navigation, view and app commands need none.
- **Notes:** Sign In with Google, Sign Out and Use Demo Data work (W2: ACC-3, ACC-7). The backend's other commands are in already, doing nothing (or saying so in a toast) until their workstream lands: Sync Now (`cmd-r`, also File › Sync Now; SYN-2), Retry Loading Message (RDR-6), Load Images (`cmd-shift-i`, ATT-1), Load Older Conversations (LST-7).

#### PAL-2 Thread search in the palette

- **Today:** Matches subject or last non-self sender name, as substring, up to 30 threads after commands. `update_matches()` (`src/ui/command_palette.rs`). No body text, unlike the search bar.
- **Needs:** A capped server query (SRC-1–5); don't load every thread to build the list.

#### PAL-3 Label jumps

- **Today:** "Go to Label: <name>" per label; `g l` opens the palette with `label: `. `go_to_label()`. With Google the labels are Gmail's (FOL-3) and refresh with each label sync.
- **Needs:** A tester pass; new labels (FOL-6) add entries.

### 8.3 Data model (DAT)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| DAT-1 | JSON as the data source | 🟡 Partially done | Replaced by SYN data |
| DAT-2 | Thread model: one folder per thread | 🟡 Partially done | Gmail per-message `labelIds` |
| DAT-3 | Message model: missing fields | 🟡 Partially done | `Message` resource |
| DAT-4 | Threading headers | 🟡 Partially done | `payload.headers` |
| DAT-5 | Local IDs for drafts and sent messages | 🟡 Partially done | Draft `id`, message `id`, `threadId` |
| DAT-6 | Search index rebuild cost | 🟡 Partially done | None |
| DAT-7 | Import and insert messages | ⚪ Not done | `messages.import`, `messages.insert` |

#### DAT-1 JSON as the data source

- **Today:** The UI reads only from the backend's store. The JSON (`include_str!` in `backend::demo`) seeds the in-memory store of demo mode, which every launch is until sign-in lands (W2); `PIGEON_DEMO=1` will keep forcing it. `demo::mail_data()` makes it look like a Gmail sync: message ids `<thread>-m<N>` (the JSON's own message ids are still dropped), `internal_date` from the date, Gmail label ids per message from the thread's folder and flags, snippets cleaned. `MailData::load()` and `CalendarData::load()` are gone.
- **Needs:** Google data in the store (W3, W4); the JSON stays as demo data and test fixture.

#### DAT-2 Thread model: one folder per thread

- **Today:** Decided and implemented: `gmail::derive_folder()` (`src/backend/gmail.rs`) takes each message's labels (drafts left out): any `TRASH` → Trash, any `SPAM` → Spam, any `INBOX` → Inbox, only `SENT` messages → Sent, else Archive. Drafts are one-message threads with `folder = Drafts`. `Thread::label_ids` is the union over messages; `labels` (user labels), `unread` (any `UNREAD`) and `starred` (any `STARRED`) are derived from it when the store loads a thread. The store applies `MailOp::Modify`/`Trash`/`Untrash` to every message like `threads.modify` and derives the folder again when a folder label changed. UI moves map to Gmail ops in `MailOp::moves()`.
- **Needs:** Decision 6.4/5 of the sprint plan: the Sent view lists every thread with a message the user sent (W3, `nav_contains`).
- **Notes:** The most important data-model decision; it touches FOL, LST and ACT.

#### DAT-3 Message model: missing fields

- **Today:** `Message` (`src/data.rs`) has `id`, `label_ids`, from/to/cc/bcc, `date` (shown) and `internal_date` (the sort key), snippet, `body: Body` (`NotLoaded | Loading | Loaded { html } | Failed`; `Message::html()` is empty until loaded), attachments (now with `attachment_id`, `part_id`), invite (now with `ical_uid`), `message_id`, `in_reply_to`, `references`, `reply_to`, `size_estimate`. `Thread` adds `label_ids`, `history_id`, `loaded` (`Metadata | Full`). `Account` adds `aliases`; `is_me()` checks them (ACC-2). `Event` and `Calendar` gained Google's fields (see CAL-12, CAL-22); `Recurrence` became `Rule` (CAL-5).
- **Needs:** Filling them from Gmail (W3); the plain-text body and the part tree (ATT-2, ATT-4).

#### DAT-4 Threading headers

- **Today:** Stored for every synced message (W3, without angle brackets). With Google (W6) every message sent or saved gets a new `Message-ID` on the sender's domain (`mime::new_message_id()`); replies carry `In-Reply-To` = the parent's id and `References` = its chain plus its id (`mime::reply_threading()`), checked on the real account; the sent copy is stored with its `Message-ID` (CMP-13).
- **Needs:** A tester pass (T2b).

#### DAT-5 Local IDs for drafts and sent messages

- **Today:** `draft-local-<uuid>` / `sent-local-<uuid>` (`new_id()` in `src/app/compose.rs`), messages `local-<uuid>`, new events a bare UUID (32 hex digits, valid as a client-chosen Google event id, CAL-11). `BackendEvent::IdReplaced` swaps a thread's id in the view model, the draft origin, the selection and an open composer (`on_id_replaced()`, `src/app/backend_events.rs`); `Store::replace_thread_id()` does it in the store.
- **Needs:** The server answering with ids: `drafts.create`, `messages.send` (W6).

#### DAT-6 Search index rebuild cost

- **Today:** The index is built once when a thread is stored (`Thread::search_text()`) and kept in `threads.search_text`; loading takes it from there (`Thread::set_search()`), so launch parses no HTML for search. Saves and sends still rebuild that thread's index; `update_threads()` re-sorts everything.
- **Needs:** Bodies fetched later (RDR-6) re-index their thread when stored (W3).

#### DAT-7 Import and insert messages

- **Needs:** Later, for migration: `messages.import` (delivery-like scanning, up to 150 MB) or `messages.insert` (like IMAP `APPEND`, no scanning) to bring mail from another account into Gmail. Neither sends mail.

### 8.4 Features with no backend need

Real features that don't depend on the backend, listed so the tables aren't read as the whole UI:

- Pane resizing for sidebar and list (`PaneResize`, `resize_pane()` in `src/app.rs`); sidebar show/hide (`cmd-\`, `toggle_sidebar()`).
- Keyboard shortcuts sheet (`?`, `src/ui/shortcuts_help.rs`), built from the live keymap, scoped like the palette.
- Calendar views, navigation and keyboard selection (CAL-4).
- Light and dark following the system (`Theme::sync()` in `src/theme.rs`) or the Appearance setting.
- The settings page (⌘,, `src/ui/settings.rs`) with search and keyboard control, apart from saving values (PER-3).
- Keyboard focus between list, reader, composer and search; selectable message text.
- Window title bar and drag area (`window_drag_area()` in `src/ui/mod.rs`); Quit (`cmd-q`).

---

## 9. Adjacent Google APIs (optional, later)

Not catalogued in depth; each would be its own item when picked up.

- **Google Drive API:** send large files as Drive links instead of attachments (ATT-6), save attachments to Drive, attach Drive files to events (CAL-30). Needs a Drive scope such as `drive.file`.
- **Google Tasks API:** a task list beside the calendar, or "add to Tasks" from a thread.
- **Google Meet REST API:** meeting details (participants, recordings) beyond the join link that Calendar's `conferenceData` already gives (CAL-17, CAL-34).
- **Calendar resources (Admin SDK Directory):** room and resource lists for booking (CAL-32); Workspace only.
- **Google Safe Browsing API:** link checks (LNK-2).

---

## 10. Not planned or out of scope

- **Delegates, alias creation, forwarding-address creation and auto-forwarding changes** (`settings.delegates.*`, `sendAs.create`/`delete`/`verify`, `forwardingAddresses.create`/`delete`, `updateAutoForwarding`): only usable by Workspace service accounts with domain-wide delegation. Pigeon shows what it can read (MST-1, MST-5, MST-8) and links to Gmail for the rest.
- **Client-side encryption** (`settings.cse.*`, MST-10): Workspace Enterprise with an external key service; the client must do the crypto. Not before everything else.
- **Postmaster Tools API:** domain-level delivery stats for bulk senders; not a mail-client feature.
- **Email markup, AMP for Gmail:** AMP needs a web runtime, against the no-webview principle. Sender-side markup (actions, highlights) is ignored.
- **Gmail inbox feed (Atom)** and **Gmail and Calendar add-on scopes** (`gmail.addons.*`, `calendar.addons.*`): for legacy readers and Workspace add-ons, not standalone clients.
- **`calendars.clear`, `calendars.transferOwnership`, event `gadget`:** destructive or admin-only, or deprecated.
- **Not in Google's APIs at all:** snooze, scheduled send and server-side undo send. If wanted they must be app-side (local timers plus a label, or a server), which is a new decision, not an API mapping.

# Pigeon roadmap

One place for where the app stands and what comes next: every feature, its status, the Google API it maps to, what the front end does today and what the backend must provide. It replaces `docs/BACKEND_REQUIREMENTS.md`; the item IDs (ACC-1, CMP-7, CAL-4a, …) are unchanged, so references in code and docs still work.

## Status legend

| Status | Meaning |
| --- | --- |
| ⚪ Not done | Nothing built. |
| 🟡 Partially done | Built, but part of it isn't linked to the real backend yet or still waits on a check (a user check, or an account with a second address); its "Needs" line says what. |
| 🟢 Fully linked | Works end to end with the real backend. |

To update: change the status cell in the area's table, then the counts below.

## Progress

| Section | ⚪ Not done | 🟡 Partially done | 🟢 Fully linked | Total |
| --- | --- | --- | --- | --- |
| [Planned features](#3-planned-features-pln) | 3 | 0 | 0 | 3 |
| [Accounts and sign-in](#4-accounts-and-sign-in-acc) | 2 | 4 | 3 | 9 |
| [Folders and labels](#51-folders-and-labels-fol) | 1 | 2 | 11 | 14 |
| [Thread list](#52-thread-list-lst) | 0 | 1 | 9 | 10 |
| [Reading pane](#53-reading-pane-rdr) | 0 | 2 | 8 | 10 |
| [Message actions](#54-message-and-thread-actions-act) | 1 | 3 | 7 | 11 |
| [Compose and sending](#55-compose-and-sending-cmp) | 0 | 4 | 13 | 17 |
| [Drafts](#56-drafts-drf) | 0 | 0 | 6 | 6 |
| [Undo](#57-undo-und) | 0 | 1 | 3 | 4 |
| [Search](#58-search-src) | 0 | 0 | 9 | 9 |
| [Contacts](#59-contacts-con) | 5 | 0 | 3 | 8 |
| [Images and attachments](#510-images-and-attachments-att) | 0 | 2 | 6 | 8 |
| [Links](#511-links-lnk) | 1 | 2 | 1 | 4 |
| [Mail settings](#512-mail-settings-mst) | 8 | 1 | 1 | 10 |
| [Calendar](#6-calendar-cal) | 13 | 5 | 29 | 47 |
| [Sync and push](#71-sync-and-push-syn) | 2 | 0 | 5 | 7 |
| [Notifications and background](#72-notifications-and-background-ntf) | 0 | 3 | 3 | 6 |
| [Loading, error and offline states](#73-loading-error-and-offline-states-sta) | 0 | 0 | 5 | 5 |
| [Storage and persistence](#81-storage-and-persistence-per) | 0 | 0 | 5 | 5 |
| [Command palette](#82-command-palette-pal) | 0 | 0 | 3 | 3 |
| [Data model](#83-data-model-dat) | 1 | 3 | 3 | 7 |
| [Accessibility](#85-accessibility-a11y) | 0 | 9 | 1 | 10 |
| [System integration](#86-system-integration-sys) | 3 | 1 | 0 | 4 |
| **Overall** | **40** | **43** | **134** | **217** |

## Contents

1. [Intro](#pigeon-roadmap) (above)
2. [Principles and current phase](#2-principles-and-current-phase)
3. [Planned features (PLN)](#3-planned-features-pln)
4. [Accounts and sign-in (ACC)](#4-accounts-and-sign-in-acc)
5. Mail: [FOL](#51-folders-and-labels-fol) · [LST](#52-thread-list-lst) · [RDR](#53-reading-pane-rdr) · [ACT](#54-message-and-thread-actions-act) · [CMP](#55-compose-and-sending-cmp) · [DRF](#56-drafts-drf) · [UND](#57-undo-und) · [SRC](#58-search-src) · [CON](#59-contacts-con) · [ATT](#510-images-and-attachments-att) · [LNK](#511-links-lnk) · [MST](#512-mail-settings-mst)
6. [Calendar (CAL)](#6-calendar-cal)
7. Sync and states: [SYN](#71-sync-and-push-syn) · [NTF](#72-notifications-and-background-ntf) · [STA](#73-loading-error-and-offline-states-sta)
8. App-wide: [PER](#81-storage-and-persistence-per) · [PAL](#82-command-palette-pal) · [DAT](#83-data-model-dat) · [No backend need](#84-features-with-no-backend-need) · [A11Y](#85-accessibility-a11y) · [SYS](#86-system-integration-sys)
9. [Adjacent Google APIs (later)](#9-adjacent-google-apis-optional-later)
10. [Not planned or out of scope](#10-not-planned-or-out-of-scope)

---

## 2. Principles and current phase

Principles are in [`AGENTS.md`](../AGENTS.md): blazing fast and light (no webviews, no heavy dependencies), keyboard-centric (every action has a shortcut and is in ⌘K), minimal and flat on translucent glass, YAGNI.

**Current phase: linked to Google; demo mode on placeholder data.** Backend sprint 1 ([`BACKEND_SPRINT_1.md`](BACKEND_SPRINT_1.md)) linked sign-in, Gmail read and write, compose and drafts, Calendar read and write, contacts, search and paging, remote images, offline queueing and status. Backend sprint 2, "Pigeon as a daily driver" ([`BACKEND_SPRINT_2.md`](BACKEND_SPRINT_2.md)), is complete: the app stays open with a menu bar panel, notifications and a Dock badge; attachments; Gmail categories; labels; aliases and signatures; repeat rules, reminders and Meet in the calendar. Everything a tester can drive passed real-account test rounds. `PIGEON_DEMO=1` runs on the placeholder data.

**What's left:** items still 🟡 wait on checks only the user can do (allowing notifications, the menu bar status item and global shortcuts, making Pigeon the default mail app, open at login, the sign-in flow, attaching a file), an account with a second verified address (MST-1, ACC-2), or a part not built yet. The user signs in again around Nov 2, unless the Google app is published first. **Candidate next work:** multiple accounts (ACC-4) and the items still ⚪.

- Data lives in a local SQLite cache (`src/backend/store/`), synced with Gmail, Calendar and People by polling (`src/backend/sync.rs`). Writes go through the outbox (`src/backend/outbox.rs`) with undo and rollback.
- Demo mode seeds an in-memory store from `assets/mailbox.json` and `assets/calendar.json`; its changes are not sent anywhere.
- There is one account.

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
- **Needs (HIG, Generative AI):** Off until the user opts in, with a one-time explanation of what is sent and where; a visible "AI view" label on every message shown this way and one key to toggle back to the original; the original renderer as the fallback whenever the model fails or is slow; Retry; Report a Bad Result. Check against Google's Limited Use policy for Gmail data (ACC-8) before any mail leaves the Mac.

> **More user features go here.** Add each as PLN-3, PLN-4, … with status ⚪ Not done.

---

## 4. Accounts and sign-in (ACC)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| ACC-1 | Account card | 🟢 Fully linked | Gmail `getProfile`; People `people.get` (`people/me`) |
| ACC-2 | Identity of "me" | 🟡 Partially done | Gmail `settings.sendAs.list` |
| ACC-3 | Add, sign in to and remove accounts | 🟡 Partially done | OAuth 2.0 for desktop apps (loopback + PKCE) |
| ACC-4 | Account switcher and multiple accounts | ⚪ Not done | One OAuth grant per account |
| ACC-5 | Per-account sync status | 🟢 Fully linked | Sync state from SYN |
| ACC-6 | Scopes and consent | 🟢 Fully linked | OAuth scopes, incremental and granular consent |
| ACC-7 | Token refresh, revoke and sign out | 🟡 Partially done | OAuth token and revoke endpoints |
| ACC-8 | Google Cloud project and app verification | ⚪ Not done | OAuth consent screen, scope verification |
| ACC-9 | Profile photo | 🟡 Partially done | People `people.get` (`people/me`, `personFields=photos`) |

#### ACC-1 Account card

- **Today:** The sidebar shows name, email and an initials avatar; clicking does nothing. `render_sidebar()` in `src/ui/sidebar.rs` reads `data.account`. Settings › Accounts shows `MailData::me()`, the granted features and the sync status. Avatar colour comes from the address (`Theme::avatar_color`). Signed in (W2), name and email come from `accounts.json` (the ID token's claims at sign-in), and every launch refreshes the name from People `people/me` (`Backend::check_profile()`), saving it back to `accounts.json`.
- **Needs:** The photo is ACC-9. Gmail `getProfile` gives `emailAddress`, `messagesTotal`, `threadsTotal` and `historyId` but no name; the name comes from People `people.get` with `resourceName=people/me` (`personFields=names,emailAddresses,photos`).

#### ACC-2 Identity of "me"

- **Today:** `MailData::is_me()` and `me()` (`src/data.rs`) compare against `account.email` and `Account::aliases`, ignoring case (`Account::is_me()`). The aliases come from Gmail `settings.sendAs.list` at the start of each full sync, kept in `sync_state` (`gmail_aliases`) and loaded on launch (`src/backend/sync.rs`). Mail from an alias shows as "me" in the reader, replies and reply-all skip every alias, Sent and Drafts rows show "To:", and aliases are never suggested as recipients (`Store::contacts()`).
- **Needs:** Nothing more for "me". A tester pass on an account with a second send-as address.
- **Notes (sprint 2 W3):** `is_me` also checks every `sendAs` address (`Account::send_as`). The calendar uses all of them too: `Event::my_status(&Account)` (the event colours, the RSVP row), answering an invite (`set_rsvp`) and "You" in the guest list. Replies go from the alias the mail was sent to (MST-1).

#### ACC-3 Add, sign in to and remove accounts

- **Today:** Built in W2 for one account. Launch without `PIGEON_DEMO` picks the source in `Backend::open()`: `accounts.json` plus the Keychain item (service `Pigeon Google`, read through `/usr/bin/security`) → Google mode with no sign-in screen; neither → the sign-in screen (`src/ui/onboarding.rs`: "Sign in with Google" (Enter), "Try with demo data" (D), waiting state with Cancel (Esc), errors in plain words). Sign-in (`Backend::sign_in()`, `src/backend/auth.rs`): loopback on `127.0.0.1:0`, PKCE S256 with a 64-byte `SecRandom` verifier, random `state`, `access_type=offline`, `prompt=consent`, `include_granted_scopes=true`, the 8 scopes, opened with `cx.open_url()`, 5 min timeout. The code is exchanged at `token_uri`; the refresh token goes to the Keychain through `security -i` (on stdin, never argv) and `{email, name, scopes}` to `accounts.json` (0600). Signing in (or "Sign in again") as a different Google account than the one set up is refused with "You signed in as b@…. Pigeon is set up for a@…. Sign out first to switch accounts.": the new grant is revoked and the old account, its token, cache and queue stay (MB-056, polish W1); an old entry whose Keychain token is already gone is forgotten. Sign In with Google from ⌘K; Use Demo Data switches a signed-out window to the placeholder data. Polish W2 (ME-050): demo data picked on the sign-in screen goes back to it with Sign In (then the browser opens) or Sign Out (`Backend::demo_by_choice()`, `leave_demo()`), and ⌘K offers both there (`When::CanSignIn`, `When::CanSignOut`); only `PIGEON_DEMO` demo data says "Pigeon was opened with demo data. Quit and open it again to sign in." The sign-in screen's ↩, D and Esc are keymap bindings in the `Onboarding` context with `on_action` handlers; ↩ and D (`onboarding::Confirm`, `onboarding::TryDemo`, so the Pigeon menu's Sign In and Use Demo Data carry no bare key) skip a focused button, which Return presses itself, and do nothing while the browser is open (MC-089). Polish W2 (MA-072): `Backend::open()` only reads `accounts.json`; the Keychain token is read in the background after (`read_token()`), so the window doesn't wait on `/usr/bin/security`; no token there flips to the sign-in screen. `PIGEON_DATA_DIR` moves `accounts.json`, `google_client.json` and the cache.
- **Today (polish W3, MB-057, MB-008, MB-Q1):** The sign-in screen shows the app icon at 64 pt (the bundle's `AppIcon.png`; the envelope glyph under `cargo run`), says "Opens Google in your browser. Pigeon asks for your mail, calendar and contacts; they stay on this Mac and go only to Google.", and has a "Show Pigeon in the menu bar" checkbox (on by default, the same `MenuBarPrefs::icon` as Settings › Menu Bar). While waiting, "Open Again" starts a fresh sign-in in a new tab (cancels the waiting one; a new PKCE pair and port) for a tab closed by mistake. Once Google says yes, Pigeon comes back in front of the browser (`cx.activate(true)` in `app::today` on `SignedIn` from Google), and the main window says "Signed in as <email>" (a Done toast, `on_signed_in`, `src/app/backend_events.rs`).
- **Needs:** Adding a second account (ACC-4). Google's desktop flow: open the system browser, loopback redirect (`http://127.0.0.1:<port>`) with PKCE, exchange the code, store tokens in the Keychain. Ask for mail, calendar and contacts scopes together (CAL-1). Removing an account revokes its token (ACC-7) and deletes its local data (PER-1).
- **Notes:** IMAP/SMTP credentials would be the non-Google path. See DAT-2 for the one-account model. With ACC-4 the refused switch becomes "Add account".

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

- **Today:** W2. `auth::Auth` keeps the access token in memory with its expiry and refreshes 60 s early; one refresh runs at a time and other callers wait for it. `GoogleTransport` refreshes once and retries on a 401. Sign Out (⌘K) revokes at `oauth2.googleapis.com/revoke` (best effort), deletes the Keychain item, the `accounts.json` entry and the account's cached rows (`Store::delete_account()`), then shows the sign-in screen. Settings › Accounts › Sign out (W11) asks first ("Sign out? <email> and its cached mail and calendar are removed from this Mac", Cancel / Sign Out; Enter confirms, Esc cancels), then runs the same Sign Out. ⌘K's and the menu's Sign Out open Settings › Accounts with that confirmation showing (`ask_sign_out()` in `src/ui/settings.rs`); in demo mode it still just toasts.
- **Needs:** Refresh the access token before it expires, retry once on 401, then show "sign in again" (STA-4). Sign out calls the revoke endpoint and deletes the Keychain item.
- **Notes:** While the OAuth app is in "Testing" status, refresh tokens expire after 7 days.

#### ACC-8 Google Cloud project and app verification

- **Needs:** A Google Cloud project with the Gmail, Calendar and People APIs enabled, a desktop OAuth client, and a consent screen. Publishing to the public needs verification: restricted Gmail scopes need restricted-scope review; a security assessment is also needed if restricted data is stored on or sent through our servers.
- **Notes:** This decides whether the backend is a local sync engine (data stays on the Mac) or a server. Push notifications (SYN-3, SYN-5) need a server. The MCP server (SYS-4) hands Gmail data to AI agents the user connects; verification must cover it under Limited Use.

#### ACC-9 Profile photo

- **Today:** The user's Google profile photo replaces the initials circle in the sidebar's account card and Settings › Accounts (`account_avatar()`, `src/ui/mod.rs`; GPUI `img()`, circular, cropped square). The launch profile check (`Backend::check_profile()`) asks `people/me` for `photos` too (the `userinfo.profile` scope already granted, so no new sign-in); `photo::pick()` (`src/backend/photo.rs`) takes the primary photo and skips Google's placeholder (`default: true`), so those accounts keep initials. It's fetched at 128 px (`=s128-c`) over a plain HTTPS GET without credentials, checked to be a PNG/JPEG/GIF/WebP, and kept at `<data dir>/profile/<email>.img`; it's fetched again at launch when it's over a day old, in the background. Offline or on failure the cached file stays (or the initials); the initials show while it loads. A user with no photo loses the cached one. Sign out deletes it. Demo mode keeps initials and makes no request.
- **Needs:** A user check with the real account. A changed photo shows after a restart (GPUI keeps the decoded image by path for the session). Other people's photos (senders, guests, contact card) are not done: People `connections` / `otherContacts` with `photos` in the field masks, cached per address (CON).

---

## 5. Mail

### 5.1 Folders and labels (FOL)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| FOL-1 | System folders and go-to shortcuts | 🟢 Fully linked | System labels `INBOX`, `SENT`, `DRAFT`, `SPAM`, `TRASH`, `STARRED`, `UNREAD` |
| FOL-2 | Sidebar counts | 🟢 Fully linked | `labels.get` (`threadsUnread`, `messagesUnread`) |
| FOL-3 | Label list with colours | 🟢 Fully linked | `labels.list`, label `color` |
| FOL-4 | Label views | 🟢 Fully linked | `threads.list` with `labelIds` |
| FOL-5 | Apply and remove labels on a thread | 🟢 Fully linked | `threads.modify` |
| FOL-6 | Create, rename, recolour and delete labels | 🟢 Fully linked | `labels.create`, `labels.patch`, `labels.update`, `labels.delete` |
| FOL-7 | Report spam and not spam | 🟢 Fully linked | `threads.modify` (add or remove `SPAM`) |
| FOL-8 | Empty Trash and permanent delete | ⚪ Not done | `threads.delete`, `messages.delete`, `messages.batchDelete` |
| FOL-9 | Nested labels | 🟢 Fully linked | Label names with `/` |
| FOL-10 | Label visibility | 🟢 Fully linked | Label `labelListVisibility`, `messageListVisibility` |
| FOL-11 | Inbox categories | 🟢 Fully linked | Category labels (`CATEGORY_*`): the Inbox shows Primary; Social, Promotions, Updates, Forums under it |
| FOL-12 | Important marker | 🟢 Fully linked | `IMPORTANT` label |
| FOL-13 | Rearrange labels | 🟡 Partially done | None: Gmail has no label order |
| FOL-14 | Move to Category | 🟡 Partially done | `threads.modify` (`CATEGORY_*`) |

#### FOL-1 System folders and go-to shortcuts

- **Today:** The sidebar lists Inbox, Starred, Sent, Drafts, Archive, Spam, Trash. Shortcuts `g i`, `g s`, `g t`, `g d`, `g a`, `g !`; Trash via the palette. `Folder` (`src/data.rs`) has six variants; each thread has one, derived from its messages' Gmail labels by `gmail::derive_folder()` when synced (W3). Sent lists every conversation holding a message the user sent, like Gmail (`Thread::in_sent()`, also for `in:sent`; 6.4 item 5). Archive is Gmail's "All Mail minus Inbox": any thread out of Inbox/Spam/Trash holding mail the user received, including mail they sent to themselves or an alias, so such a thread shows in Sent and Archive both. Starred is a flag. `FOLDERS` in `src/ui/sidebar.rs`, `set_nav()` in `src/app.rs`.
- **Needs:** A tester pass on the real account (T1a). IMAP later: special-use `\Sent`, `\Drafts`, `\Junk`, `\Trash`, `\Archive`.
- **Notes:** See DAT-2 for the one-folder limit. Spam and Trash sync only their newest 25 threads; older ones load with "Load older" (LST-7, W9).
- **Today (polish W2):** Folder and category rows have tooltips that teach their go-to keys (ME-033) and a right-click menu with "Mark All as Read" (ACT-9, MB-010). Conversations dragged onto a folder, Starred, a category or a label move, star, label or recategorize them; only rows where that changes something light up (MA-087, `accepts_drop` / `drop_threads`). The account row opens Settings › Accounts (ME-029); in demo data a "Demo data · Sign in…" row replaces the status line (ME-050). The sidebar's hide button sits at the header's leading edge (MA-062) and the sidebar scrolls with a scroll bar (MB-068).

#### FOL-2 Sidebar counts

- **Today:** Inbox, Spam and labels show Gmail's `threadsUnread` from `labels.get` (synced with the labels and after every partial sync that changed something; stored in `labels.threads_unread`). Reading or marking unread here moves the number at once: `ServerCounts` keeps the local count from when Gmail's was read and adds the difference (`view_unread()`, `src/ui/sidebar.rs`). Drafts shows the number of Drafts threads; demo mode (no server counts) counts loaded threads as before.
- **Needs:** A tester pass comparing with Gmail (T1a). IMAP: `STATUS … UNSEEN`.
- **Today (polish W2):** Every view's count comes from one pass over the threads per frame (`NavCounts`, `PigeonApp::nav_counts()`, MC-037), shared by the sidebar, list header and rail. Counts read "1,234", capped at "9,999+" (ME-028); VoiceOver reads "Inbox, 12 unread".
- **Notes:** Between a local change and the next sync the count is an estimate.

#### FOL-3 Label list with colours

- **Today:** From `labels.list` (W3): user labels the user hasn't hidden (`labelListVisibility != labelHide`), with `color.backgroundColor`, else Gmail's grey. Stored in `labels` (hidden ones as `type = 'hidden'`, system ones for their counts); labels gone from Gmail are dropped (`Store::replace_gmail_labels()`). Dots in the sidebar, chips in the reader. Demo mode: the ten placeholder labels.
- **Needs:** A tester pass (T1a). `textColor` isn't used. IMAP has no colours; keep them locally.
- **Notes:** `Thread::labels` holds label IDs; renames keep IDs stable. Nested labels (`a/b`) show their full name (FOL-9).

#### FOL-4 Label views

- **Today:** `Nav::Label(usize)` (`src/app.rs`); `nav_contains()` shows cached threads carrying the label, excluding Spam and Trash. The cache holds the Inbox's first 100 threads, the first pages of Sent, Starred, Spam and Trash, and a backfill of up to 1,000 threads or 90 days (W3), so a label view shows the labelled threads in that window. `g l` or "Go to Label…" opens a label search.
- **Needs:** A tester pass (T3a) of "Load older" at the end of a label view (LST-7, W9). IMAP: folder or keyword search.
- **Notes:** Label views include Sent, Drafts and Archive, as in Gmail.

#### FOL-5 Apply and remove labels on a thread

- **Today:** `l`, palette "Label…", Message › Label…, the toolbar "…" menu or a row's right-click toggle a label on the selected or checked threads (ACT-6). `l` opens the palette scoped to labels; applied ones say "Applied · ↵ removes it". The list shows up to three dots; the reader shows chips. `toggle_label()` and `open_label_palette()` in `src/app/mail.rs`: if every target has the label it's removed, otherwise added to all. The toast has Undo (UND-4).
- **Needs:** Done for Gmail (W5): a `MailOp::Modify` that commits at once as `threads.modify` with `addLabelIds` / `removeLabelIds`, several threads in one HTTP batch, with rollback on failure (STA-2). IMAP: `UID COPY`/`MOVE`, or `STORE +X-GM-LABELS`.
- **Notes:** Gmail labels are per message; `threads.modify` changes every message in the thread. See DAT-2.

#### FOL-6 Create, rename, recolour and delete labels

- **Today (sprint 2 W4b):** ⌘K "New Label…" (`L N`, also the sidebar's "+"), "Rename Label…" (`L R`), "Change Label Color…" (`L C`), "Delete Label…" (`L D`), also from a label's right-click menu in the sidebar. They act on the open label, else ask which one; from the right-click menu, on the clicked label, without opening it (`LabelsUi::context_label`, polish W2 MB-017). The palette turns into a name field for new and renamed labels. Colours: nine curated pairs (`backend::labels::COLORS`), background and text both from Gmail's allowed palette (tested). Rename also renames every child (`Work/…`): names are checked for collisions first (`plan_rename`), the parent goes first; if a child fails, labels are read again (`labels.list`) and the toast names the ones not renamed. Delete asks first, offering "with its N sublabels" or "only this one", and never touches mail (Gmail takes the label off; the store does the same). The sidebar changes at once; on failure the labels reload from the store, which only changes once Gmail answered. A new label is stored as a shown user label even when Gmail's answer leaves out `type` and the visibilities; renames and other changes store what was asked, not Gmail's (possibly partial) answer. Demo mode changes the in-memory store. `src/backend/labels.rs`, `src/app/labels.rs`.
- **Needs:** Done; passed on the real account (sprint 2 T2a, after fix-5). IMAP: `CREATE`, `RENAME`, `DELETE`.
- **Notes:** Label changes go to Gmail at once (`labels.create`, `labels.patch`, `labels.delete`, at the user's priority) rather than through the outbox: a new label needs Gmail's id first and a rename goes parent, then children. Offline they're refused with "Label changes need a connection" and nothing changes. Needs `gmail.modify`. New labels add palette entries (PAL-3).

#### FOL-7 Report spam and not spam

- **Today:** Move to… › Spam (`v`, ACT-1a) puts a thread in Spam in memory; Archive (`e`) in Spam moves it to Inbox ("not spam"). No dedicated button or shortcut.
- **Needs:** Done for Gmail (W5): `threads.modify` adding `SPAM` and removing `INBOX`; out of Spam removes `SPAM` and adds `INBOX`. Both wait out the undo window. IMAP: move to Junk.
- **Notes:** Mapping `SPAM` to the Spam folder is done at sync (FOL-1).

#### FOL-8 Empty Trash and permanent delete

- **Today:** Not built. `delete()` (`src/app.rs`) moves to Trash and never removes; Delete does nothing in Trash.
- **Needs:** `threads.delete`, `messages.delete`, `messages.batchDelete` (all need the `https://mail.google.com/` scope, ACC-6). Gmail purges Trash after 30 days on its own. IMAP: `\Deleted` + `EXPUNGE`.
- **Notes:** Not reversible; needs a confirmation.

#### FOL-9 Nested labels

- **Today (sprint 2 W4b):** Labels nest by name (`Work/Clients` under `Work`, when `Work` exists; otherwise the whole name shows at the top), sorted by name at each level unless the user rearranged them (FOL-13; `labels::label_tree`). The sidebar indents children and folds a parent with its chevron, the context menu, or ⌘K "Fold / Unfold Sublabels" (`L F`). The `l` picker lists the tree indented; a folded parent's children show once you type. Folded labels are kept with the settings (`Saved::collapsed_labels`, the same `collapsed_labels` key), demo mode too, written by the debounced settings save (polish W2, MC-048). The fold triangle sits in the indent gutter before the dot, a DisclosureTriangle with its state (polish W2, MB-032). The whole Labels section folds from its header (a button with its state, chevron on hover; saved with the folded labels), and keyboard label commands skip a folded section (polish W2, MB-074).
- **Needs:** Done; passed on the real account (sprint 2 T2a).

#### FOL-10 Label visibility

- **Today (sprint 2 W4b):** The sidebar follows `labelListVisibility`: "show if unread" labels show while they have unread mail, hidden ones (and their children) don't, except the open one (unless it was just hidden while open, e.g. from its right-click menu: then it goes at once); a "N hidden" row (`L H`) lists them too, as their own group right under it (flat, by full name) and leaves the visible labels where they are. Rows show label dots only for labels whose `messageListVisibility` is "show"; the reader shows all. Change both from a label's right-click menu ("In Label List", "In Conversation List") or ⌘K "Label Visibility…" (`L V`), through `labels.patch`. Stored in the `labels` table (schema block 150); hidden labels are now loaded too, so the label picker and `label:` search see them.
- **Needs:** Done; passed on the real account (sprint 2 T2a, after fix-5).

#### FOL-11 Inbox categories

- **Needs:** Gmail's category labels (`CATEGORY_PERSONAL`, `CATEGORY_SOCIAL`, `CATEGORY_PROMOTIONS`, `CATEGORY_UPDATES`, `CATEGORY_FORUMS`) as Inbox tabs or filters, and the `category:` search operator (SRC-9).
- **Built (backend, sprint 2 W4a):** `src/backend/categories.rs`. A message is Primary unless it carries `CATEGORY_SOCIAL`, `_PROMOTIONS`, `_UPDATES` or `_FORUMS` (`category_of`); a thread is in a category when any of its Inbox messages is (`in_category`). Unread Inbox threads per category (`Backend::category_counts`, `CountsChanged`) come from `threads.list labelIds=INBOX,UNREAD q=category:<x>` with ids only, up to 500 ("500+"): at launch and after a sync whose history touched the Inbox, 50 quota units per refresh; kept in `sync_state` so launch shows the last ones at once. Demo mode counts the store (some demo senders get categories in code). `ViewKey::Category` asks Gmail for `in:inbox category:<x>` for "Load older", and the full sync also lists Primary's first page that way.
- **Views (sprint 2 W4b):** The Inbox shows Primary (`app::nav_holds`), its count and the rail's are Primary's unread (the Dock badge's number, moved at once by reads here until the next count). Social (`g o`), Promotions (`g p`), Updates (`g u`) and Forums (`g f`) are views under Inbox in the sidebar with their unread counts, and in ⌘K ("Go to …"); a category with no Inbox threads and no unread is left out. Their "Load older" pages ask `in:inbox category:<x>`, the Inbox's `category:primary`. Searching the Inbox, and `in:inbox`, still covers every category, as Gmail does.
- **Needs:** Done; passed on the real account (sprint 2 T2a).

#### FOL-12 Important marker

- **Today (sprint 2 W4b):** `Thread::important` comes from the `IMPORTANT` label. Rows show an amber marker; the reader's toolbar has a toggle (an outline glyph while off, polish W1). "Mark as Important / Not Important" in ⌘K, the thread menu and `=` (or `+`, as in Gmail) adds or takes off `IMPORTANT` with `threads.modify` through the outbox, at once like a star. `is:important` searches Gmail (SRC-9).
- **Needs:** Done; passed on the real account (sprint 2 T2a).

#### FOL-13 Rearrange labels

- **Today:** Drag a label in the sidebar onto a sibling (same parent) and it takes that place; a line under or over the target shows where (under when it comes from above, else over). Keyboard: ⌘K "Move Label Up" / "Move Label Down", `L ↑` / `L ↓` and the label's right-click menu move the open label one place among the siblings the sidebar shows (hidden ones are skipped). The arrangement is a list of label ids in memory (`LabelsUi::order`, `labels::label_tree(labels, order)`, `labels::drop_on()`, `labels::nudge()`); labels it doesn't list (new ones) follow their siblings by name; the `l` picker and "which label?" lists follow it too. Labels under "N hidden" are listed flat and can't be dragged. The sidebar scrolls as a whole under the account (rows no longer shrink).
- **Needs:** Persistence: the order is lost at quit. Gmail has no label order or API for it (the web app sorts by name), so keep it in the local store (a settings row like `collapsed_labels`, per account) and drop ids of deleted labels. IMAP: same, local only.
- **Notes:** Nesting follows the names (FOL-9), so a label can't be dragged under another parent without renaming it.

#### FOL-14 Move to Category

- **Today (polish W2, MB-062):** "Move to Category" in a row's menu and the toolbar "…" (a submenu with a check on the current one), ⌘K "Move to Category…" (a palette of the five) and dragging onto a category in the sidebar (LST-10) move Inbox conversations to Primary, Social, Promotions, Updates or Forums. Every message gets the target's `CATEGORY_*` label (Primary: `CATEGORY_PERSONAL`) and loses the others, here at once (`Thread::set_category()`, `src/data.rs`) and in Gmail through a `MailOp::Modify` (`move_to_category()`, `src/app/mail.rs`). A toast says so.
- **Today (polish W5 fix):** Undo and Redo (the toast, ⌘Z, ⇧⌘Z) put each conversation back in the category it came from (`UndoAction::Category`, `undo_category()`), with its own `MailOp::Modify` per thread.
- **Needs:** A check on the real account. Gmail's API docs list every `CATEGORY_*` label as one an app can add and remove; they don't say whether it trains Gmail's filter.

### 5.2 Thread list (LST)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| LST-1 | Thread rows | 🟢 Fully linked | `threads.list` + `threads.get` (`format=metadata`) |
| LST-2 | Unread styling | 🟢 Fully linked | `UNREAD` label |
| LST-3 | Sort order | 🟢 Fully linked | `threads.list` order |
| LST-4 | Archived section in the Inbox | 🟢 Fully linked | `threads.list` (`q=-in:inbox`) |
| LST-5 | Keyboard list navigation | 🟢 Fully linked | `threads.get` on demand |
| LST-6 | Unread count in the list header | 🟢 Fully linked | `labels.get` |
| LST-7 | Pagination and lazy loading | 🟢 Fully linked | `pageToken`, `maxResults` |
| LST-8 | Live refresh and new mail in the list | 🟢 Fully linked | `history.list` (SYN-2) |
| LST-9 | Empty-result text | 🟢 Fully linked | None |
| LST-10 | Drag conversations onto the sidebar | 🟡 Partially done | The moves of ACT-1a, ACT-3, FOL-5, FOL-14 |

#### LST-1 Thread rows

- **Today:** Sender (or "To: name" in Sent and Drafts), message count, attachment icon, star, date, subject, label dots, snippet of the last message. Compact density puts sender, subject and date on one line. `render_thread_row()` (`src/ui/thread_list.rs`). With Google (W3) rows come from `threads.list` + batched `threads.get format=metadata` (`gmail::thread_from_gmail()`): subject of the first message, per-message Gmail `snippet` (HTML entities decoded), headers parsed by `mail-parser` (RFC 2047, address lists), date from the Date header, sorted by `internalDate`. Demo: placeholder JSON.
- **Needs:** A tester pass (T1a). The attachment icon needs the body (ATT-5).
- **Today (polish W2):** The sender line reads like Gmail's: "Ana, me, Bob (4)", first names past two people and "Ana … Cy, Dan" past three (`Thread::senders_line()`, `src/data.rs`); Sent and Drafts say "To: Ana, Bob +3" (`recipients_line()`); a red "Draft" marks a conversation with a reply draft (`PigeonApp::draft_replies`). An empty subject shows "(no subject)" faint and italic (`subject_or_placeholder()`). A clock marks a send still waiting in the outbox ("Waiting to send", `Backend::queued_sends()`). With Differentiate Without Color on, label dots become chips with the label's first letter. Hover swaps the date for the quick actions and shows the checkbox through a GPUI hover group, so hovering no longer re-renders the app (ME-004); each action's tooltip shows its key. Dates put the day first ("3 Oct") where the region does (`data::day_first()`). Selected rows go grey while the window is in the background. Each row's VoiceOver label says what it shows (unread, people, count, draft, subject, date, starred, important, attachments, labels).
- **Notes:** The row's snippet is the last message's, not the thread's.

#### LST-2 Unread styling

- **Today:** Blue dot, bold sender and subject. `Thread::unread` is true when any message has `UNREAD` (W3 mapping); flipped locally by `select_thread()`, `mark_read()`, `mark_unread()`.
- **Needs:** A tester pass (T1a). IMAP: `\Seen` per message.
- **Notes:** See ACT-4, ACT-5, DAT-3.

#### LST-3 Sort order

- **Today:** Newest first by the last message's `internalDate` (`MailData::sort_threads()`); `update_threads()` re-sorts after every change and every sync.
- **Needs:** A tester pass (T1a). A stable cursor once paging exists (LST-7).

#### LST-4 Archived section in the Inbox

- **Today:** The Inbox ends with a collapsible "Archived" header counting the cached Archive threads (`Row::ArchivedHeader` in `refresh_rows()`, `src/app.rs`); palette "Show / Hide Archived in Inbox". With Google, the cache's archived threads come from the backfill (W3). Polish W2: its disclosure chevron leads, in the rows' inset, and the count has thousands separators; with nothing left in the Inbox it sits under "You're all caught up" (LST-9).
- **Needs:** Gmail has no cheap server count for "not in Inbox" (1.1), so the header counts cached threads only. Archived mail past the window comes from "Load older" in the Archive view (LST-7, W9).

#### LST-5 Keyboard list navigation

- **Today:** `j`/`k` or arrows move; `Enter`/`o` open; Home/End go to the first and last conversation, Page Up/Down move the selection a screenful (`page_list()`, `src/app/mail.rs`, polish W2). `select_next()`, `select_prev()`, `select_thread()` (`src/app.rs`). Selecting marks read (RDR-1) and asks for the bodies (RDR-6), which are fetched only once the selection has stayed 150 ms, so key repeat doesn't spend quota.
- **Needs:** A tester pass (T1a).

#### LST-6 Unread count in the list header

- **Today:** "N unread" from `view_unread()`, the same number as the sidebar: Gmail's count for Inbox, Spam and labels (FOL-2), else the loaded rows'. Polish W2: with thousands separators; a nested label's title reads "Work › Clients › Acme" with the parents muted. The header also has a New Message button beside Search, and while conversations are picked a select-all checkbox (mixed while only some are; ACT-6).
- **Needs:** A tester pass (T1a).

#### LST-7 Pagination and lazy loading

- **Today:** W9: signed in, every view but Drafts ends in a "Load older conversations" row (`Row::LoadOlder`, `render_load_older()` in `src/ui/thread_list.rs`). A click, Enter on the focused row, `j` past the last thread, or ⌘K "Load Older Conversations" runs `activate_load_older()` (`src/app/mail.rs`), which calls `Backend::load_older(view, oldest)`: `threads.list` (`maxResults=50`) with the view's `labelIds` (Archive: `q=-in:inbox -in:spam -in:trash -in:drafts`) and `before:<oldest cached thread in the view>`, then the `pageToken` on later clicks. Threads the store lacks are fetched with batched `threads.get format=metadata` and stored (`gmail::older_pages()`, `list_and_store()`); up to 5 pages per click when a page is all cached. The row says "Loading older conversations…" and, once Gmail has none, "No older conversations". Demo mode: no row; the command says "That's all the demo mail".
- **Needs:** A tester pass (T3a). Paging state is per launch (in memory); the threads loaded stay in the cache. IMAP: UID ranges. Keep only visible and recent threads in memory.
- **Notes:** Gmail's `before:` matches threads with any message before the date, so a page can repeat cached threads; they're skipped without a fetch.

#### LST-8 Live refresh and new mail in the list

- **Today:** Partial sync (SYN-2, W3) refetches the threads `history.list` reports every 45 s while the window is active (5 min in the background), on focus and on Sync Now, and emits `ThreadsChanged` / `ThreadsRemoved`. `on_threads_changed()` (`src/app/backend_events.rs`) reloads them through `update_threads()` (`MailData::upsert_threads()`: changed threads replaced, new ones inserted newest first), which keeps the selection and splices rows so the list stays put. A list scrolled to its top stays at the top, so new mail shows instead of the splice scrolling it out of view, and a selected row that was showing is revealed again if it was pushed down; one scrolled off screen is left there (the check compares the row with the viewport, since the list keeps bounds for measured rows below it). Threads moved elsewhere leave or join each view (Inbox, labels, Sent, open search) by the same filter; a selected thread that leaves stays open in the reader.
- **Needs:** A tester pass (T1a item 5), including new mail arriving with a thread selected. Sync 2 s after a committed write comes with W5. Links to NTF-1, STA-5.

#### LST-9 Empty-result text

- **Today:** "Nothing in <view>", or "No results in <scope>" with a hint while searching (`render_thread_list()`). With Google, an empty view shows skeleton rows until its first page is stored (`Backend::view_loaded()`), or "Couldn't load <view>" if the sync failed before then (W3).
- **Today (polish W2, ME-001):** A view counts as empty when it has no conversation rows, whatever headers it has. An empty Inbox says "You're all caught up" with the next event ("Next: Design review at 2:00 PM", or tomorrow's), above the Archived section. Every other view names itself and its next step, with the live key: Drafts and Sent "Press C to write a message", Starred "Press S on a conversation to star it", Archive (E), labels (L); Spam and Trash say Gmail empties them after 30 days. No search results offers a "Search All Mail" button; "Couldn't load" has "Try Now" (Sync Now). The skeleton's pulse runs at 30 fps.
- **Needs:** A tester pass (T1a).

#### LST-10 Drag conversations onto the sidebar

- **Today (polish W2, MA-087):** A row drags (with every picked row when it's one of them) onto a folder (move, with Undo), Starred (star), a label (add it) or an Inbox category (FOL-14). The ghost shows the subject, or the count. Only rows the drop would change light up (`accepts_drop()`, `drop_threads()` in `src/app/mail.rs`; `DraggedThreads` in `src/ui/thread_list.rs`; the sidebar's drop targets in `src/ui/sidebar.rs`).
- **Needs:** A user check. Nothing new from Gmail.

### 5.3 Reading pane (RDR)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| RDR-1 | Opening a thread marks it read | 🟢 Fully linked | `threads.modify` (remove `UNREAD`) |
| RDR-2 | Conversation view | 🟢 Fully linked | `threads.get` (`format=full`) |
| RDR-3 | Expand and collapse older messages | 🟢 Fully linked | Per-message `snippet` |
| RDR-4 | Message header | 🟢 Fully linked | `messages.get` headers |
| RDR-5 | Message body rendering | 🟢 Fully linked | `messages.get` `payload` parts |
| RDR-6 | Fetch a message body on demand | 🟢 Fully linked | `messages.get` (`format=full`) |
| RDR-7 | Message navigation and reader focus | 🟢 Fully linked | None |
| RDR-8 | Show original (raw message) | 🟢 Fully linked | `messages.get` (`format=raw`) |
| RDR-9 | Fold quoted history and long threads | 🟡 Partially done | None |
| RDR-10 | Context menus in the reader | 🟡 Partially done | None |

#### RDR-1 Opening a thread marks it read

- **Today:** `select_thread()` (`src/app.rs`) sets `unread = false` at once, also while moving through the list with keys and for Drafts.
- **Needs:** Done for Gmail (W5): `select_thread()` queues a `Modify` removing `UNREAD`, committed at once as `threads.modify` at user priority. IMAP: `STORE +FLAGS (\Seen)`. Skip for drafts.
- **Notes:** Consider a delay or a setting, as many clients do.

#### RDR-2 Conversation view

- **Today:** Subject, label chips, "N messages", every message oldest first (by `internalDate`), newest expanded. `render_conversation()` (`src/ui/reader.rs`). Polish W2: the conversation is at most 760pt wide, left-aligned (`READER_MAX`, MA-061), and bodies drop their indent when the reader is under 520pt; "(no subject)" shows faint and italic; label chips show the label's own name with the whole path as a tooltip and open the label on click (ME-022); the focus bar turns gray while another window has the keys (MB-042); with nothing selected the reader shows "J K to move · ↵ to open", or "Nothing to read here" when the list is empty (ME-001, ME-030). Draft messages are left out of the conversation (they're Drafts threads). A sync that changes the open thread reloads it in place.
- **Needs:** A tester pass (T1a).

#### RDR-3 Expand and collapse older messages

- **Today:** Collapsed rows show sender, snippet (Gmail's per-message `snippet`), date; click to expand. `expanded_messages`, `toggle_message()`, cleared in `select_thread()`. Polish W2: collapsed rows are tab stops (disclosure role, `aria_expanded`), the name truncates at half the row and the date is the list's short date with the full one as a tooltip (ME-013, ME-014); over four messages the middle folds into "N more messages" (RDR-9).
- **Needs:** A tester pass (T1a). Expanded state could be saved (PER-3).

#### RDR-4 Message header

- **Today:** Sender name, email, "to … · cc …", full local date via `full_date()`, on the app's 12- or 24-hour clock (§8.4), with "3 hr ago" under it for the last day (`relative_age()`). `render_message()`. Polish W2: three names a group, then "+N others", which opens the whole list (ME-012); Bcc on the user's own mail; long sender names truncate before the date (ME-013); the sender's address shows on hover and while the message has the keys (MA-047); a hover Reply and "…" (Reply All, Forward, Show Original, Save as .eml…) reply to that message, not the newest (ME-010; the reply itself is the composer's, CMP-15). With Google the headers come from `format=metadata` (From, To, Cc, Bcc, Reply-To, Subject, Date, Message-ID, In-Reply-To, References), parsed by `mail-parser` (W3); `message_id`/`in_reply_to`/`references` are stored without angle brackets.
- **Needs:** A tester pass (T1a).
- **Notes:** "+N others" stays open per message for the session (`ReaderUi`).

#### RDR-5 Message body rendering

- **Today:** HTML is drawn natively (headings, lists, quotes, tables, links, emphasis, code, simple colours); scripts, styles, iframes and form controls dropped; images load when the user asks (ATT-1). `html::parse()` (`src/html.rs`), `src/ui/html_view.rs`, cached in `HtmlCache` by message id and a fingerprint of the HTML, at most 300 documents (MC-049); bodies of 64 KB or more are parsed off the main thread (MC-054). Message text is selectable. Polish W2: the remote and inline images are listed once at parse time (`Document::remote_images`, `cid_images`); email colors are corrected once per background (`Style::readable`, MC-050); `dir="rtl"` and Hebrew or Arabic paragraphs align right (MA-042); images are dimmed slightly in dark mode (MA-033); placeholders take the image's size and say loading (pulsing), blocked or "didn't load" (MA-075); decorative images (`alt=""`) are silent to VoiceOver (MA-044); headings use `TEXT_TITLE2`/`LG`/`MD`; ⌥Tab rings the focused message's next link, button or linked image and ↵ opens it (MA-003; body links aren't Tab stops). With Google the body comes from `messages.get format=raw` parsed by `mail-parser` (`mime::parse_message()`, W3): charsets, transfer encodings, the `text/html` part of `multipart/alternative`, plain text converted to HTML when there's no HTML.
- **Needs:** A tester pass on real newsletters and replies (T1a). `cid:` parts per ATT-2.
- **Notes:** Unknown tags keep their text only.

#### RDR-6 Fetch a message body on demand

- **Today:** Built (W3). When a thread stays selected 150 ms, `Backend::fetch_body()` fetches every message without a body with `messages.get format=raw` in HTTP batches sized by Gmail's size estimate (under ~40 MB of base64 each; a bigger message alone, so the 64 MB read limit applies per batch, `body_chunks()` in `sync.rs`, polish W1 MC-012) (60 units each), parses it (`mime::parse_message()`) and stores html and attachments in `bodies` / `messages`, plus `search_text`. Messages show pulsing lines meanwhile (polish W2, ME-030), after 300 ms, with "Downloading message…" under them past a second; a body whose placeholder showed fades in over it, a fast one appears at once (polish W5, MA-066); a failure shows "Couldn't load this message", the reason and Retry (palette "Retry Loading Message"). An answer over the read limit is `TransportError::TooLarge` ("This message is too large to download"), which never counts toward Offline. Bodies survive later metadata refetches, and label changes and metadata refetches no longer rewrite them (polish W1 MC-003). `Backend::load_mail_heads()` / `Store::load_all_thread_heads()` read threads without bodies; the app starts on them (polish W2, MA-072), so the first frame doesn't wait on bodies, and keeps the ids it holds that way (`PigeonApp::heads`): selecting one reads its stored bodies with `load_threads()` (`ensure_bodies()`), then `fetch_body()` asks Gmail for the rest. The time to the first frame is logged (`window: first frame N ms after launch`). Demo mode has every body already.
- **Needs:** A tester pass (T1a). Eviction at 300 MB (`bodies.last_opened`) isn't built. Offline wording ("isn't downloaded yet") comes with W11.
- **Notes:** A Gmail draft opened in the composer before its body arrives starts empty; W6 (`drafts.get`) handles opening drafts.

#### RDR-7 Message navigation and reader focus

- **Today:** `n`/`p` move between messages and expand; `o`/`Enter` focus the reader; `u`/`Esc` return; arrows scroll. `step_message()`, `open_thread()`, `back_to_list()`, `scroll_reader()` (`src/app.rs`). A message whose body is loading shows "Loading…" when reached.
- **Needs:** A tester pass on real data.

#### RDR-8 Show original

- **Today (W2, sprint 2):** ⌘K "Show Original" (`cmd-alt-u`, again to close) shows the focused message's source (the newest unless `n`/`p` picked one) in place of the conversation: a read-only monospace view of its first 1 MB (`ORIGINAL_SHOWN`; a note says when there's more), long lines broken at 100 characters, in a `uniform_list` so big messages scroll smoothly. Toolbar: Copy (`cmd-c`, the shown text), Save as .eml (`cmd-alt-s`, also from ⌘K "Save Message as .eml…" without opening it; the whole message), Close (`Esc`). The source is fetched each time (`messages.get format=raw`, `Backend::raw_message()`); raw messages aren't kept. Demo mode makes one up from the placeholder message (`demo_raw()`). `Original`, `render_original()` (`src/ui/reader.rs`), `src/app/attachments.rs`, `src/backend/attachments.rs`. The source is decoded and split into lines off the main thread (polish W2, MC-054).
- **Needs:** Done; passed on the real account (sprint 2 T1b).
- **Notes:** Saving uses the save panel (starting where the last save went, else Downloads); the file gets the quarantine attribute. With `PIGEON_DEV=1` and `PIGEON_SAVE_DIR` set, saves go straight to that folder (numbered when the name is taken), so testers can check them.

#### RDR-9 Fold quoted history and long threads

- **Today (polish W2):** A message's trailing quoted history (a last `blockquote`, with the "On … wrote:" line before it, Gmail's `gmail_quote` div, or an Outlook "-----Original Message-----" block) folds behind a "•••" button that shows and hides it per message (ME-003; `html::trailing_quote_start()`, `html_view::Fold`); a forward's `gmail_quote` isn't folded. Over four messages, the ones between the first and the last two fold into "N more messages" rows, except expanded and focused ones (ME-008, `fold_slots()`); `n`/`p` unfold the message they reach. Folded messages stay children of the scroll view, so scrolling to a message by index still works. What was opened is kept for the session (`ReaderUi`).
- **Needs:** A user check on real replies (Gmail, Apple Mail, Outlook). Expanding unread messages when a thread opens is in `select_thread()` (polish W2-SHELL).

#### RDR-10 Context menus in the reader

- **Today (polish W2, MB-010):** Right-click or Control-click: a message (Reply, Reply All, Forward, Show Original, Save as .eml…; Copy while text is selected), an attachment (Open, Quick Look, Save…, Show in Finder), a sender or recipient (Copy Address, New Message, Show Contact), a link, button or linked image (Open Link, Copy Link, or Copy Address for `mailto:`). The toolbar's "…" and the attachment picker open under their button and the first attachment (MB-016). The toolbar groups Reply, Reply All and Forward (MB-020); in Spam the archive button reads "Not spam" (ME-027).
- **Needs:** A user check. Copying an attachment's file isn't offered.

### 5.4 Message and thread actions (ACT)

Each action applies to the checked threads if any (ACT-6), otherwise the selected one (`targets()` in `src/app/mail.rs`). Hovering a row shows Archive, Delete, Mark read/unread and Star; right-click opens a native menu with those plus Reply, Forward, Move to, Label. Right-click doesn't select, open or mark read: the menu acts on the right-clicked row (outlined while the menu is open; `context_target` in `src/app.rs`, preferred by `targets()`), or on all picked threads when the row is one of them. Reply, Reply All and Forward open that conversation first. Toolbar, shortcut and palette all dispatch the same action (`src/actions.rs`).

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| ACT-1 | Archive and move to Inbox | 🟢 Fully linked | `threads.modify` (`INBOX`); `threads.untrash` |
| ACT-1a | Move to a folder | 🟢 Fully linked | `threads.modify`, `threads.trash` |
| ACT-2 | Delete | 🟢 Fully linked | `threads.trash` |
| ACT-3 | Star and unstar | 🟢 Fully linked | `threads.modify` (`STARRED`) |
| ACT-4 | Mark as unread | 🟢 Fully linked | `threads.modify` / `messages.modify` (`UNREAD`) |
| ACT-5 | Mark as read | 🟢 Fully linked | `threads.modify` (`UNREAD`) |
| ACT-6 | Bulk select and bulk actions | 🟢 Fully linked | `messages.batchModify`, HTTP batch |
| ACT-7 | Open in Gmail | 🟡 Partially done | None: a web link with `authuser` |
| ACT-8 | Share… | ⚪ Not done | None (`NSSharingServicePicker`) |
| ACT-9 | Mark all as read in a view | 🟡 Partially done | `threads.list` (`labelIds` + `UNREAD`), `threads.batchModify` |
| ACT-11 | Snooze conversations | 🟡 Partially done | `threads.modify` (a hidden `Pigeon/Snoozed` label, `INBOX`, `UNREAD`); `labels.create` |

#### ACT-1 Archive and move to Inbox

- **Today:** `e`, toolbar "Archive" (tooltip "Move to Inbox" for archived threads), palette. Selection advances (Settings › Mail › Auto-advance off clears it). `archive()`, `move_selected()` (`src/app.rs`); target from `Folder::archive_target()`: Inbox → Archive; Archive, Spam, Trash → Inbox; Sent and Drafts nothing (button hidden there). Follows each thread's own folder, also in Starred and label views. Archiving mail the user sent to themselves (or an alias) leaves only `SENT` on it; it still lands in Archive, at once and after sync (`derive_folder()` with the account's addresses from `Store::me()`).
- **Needs:** Done for Gmail (W5): `threads.modify` removing `INBOX` (and `SPAM`); back to Inbox adds `INBOX`; Spam → Inbox is "not spam" (FOL-7); Trash → Inbox is `threads.untrash`, then the modify. Commits after the 5 s undo window. IMAP: `UID MOVE`.
- **Notes:** Rollback on failure (STA-2).

#### ACT-1a Move to a folder

- **Today:** `v`, palette "Move to…", Message › Move to…, toolbar "…", right-click: Inbox, Archive, Spam or Trash (palette and menus omit the current one). In Spam the archive slot reads "Not Spam" (`Folder::archive_title()`). Undo toast. `move_to()`, `move_threads()`, `open_move_palette()` (`src/app/mail.rs`). Drafts are skipped; Sent can move.
- **Needs:** Done for Gmail (W5): the moves of ACT-1, ACT-2 and FOL-7 (`MailOp::moves()`), committed by the outbox.

#### ACT-2 Delete

- **Today:** `delete`, `backspace`, `#`, toolbar, palette. Moves to Trash and selects the next thread, with Undo (UND-3). In Drafts it discards the draft (DRF-4); in Trash it does nothing (button hidden). `delete()` (`src/app.rs`).
- **Needs:** Done for Gmail (W5): `threads.trash` after the undo window; never a permanent delete. IMAP: `UID MOVE` to Trash.
- **Notes:** Whether Delete in Trash means permanent delete is open (FOL-8).

#### ACT-3 Star and unstar

- **Today:** `s`, toolbar, palette; star on the row. Unstarring in Starred removes the row at once. `toggle_star()`; `Thread::starred`. Allowed in Spam and Trash; Starred hides those.
- **Needs:** Done for Gmail (W5): `threads.modify` adding or removing `STARRED`, at once. IMAP: `\Flagged`.
- **Notes:** Gmail stars messages; removing `STARRED` from the thread unstars all of them.

#### ACT-4 Mark as unread

- **Today:** `shift-u`, toolbar, palette. Selection stays. `mark_unread()`.
- **Needs:** Done for Gmail (W5): `threads.modify` adding `UNREAD` to the whole thread (decision 6.4 item 7), at once. IMAP: remove `\Seen`.
- **Notes:** Opening the thread again marks it read (RDR-1).

#### ACT-5 Mark as read

- **Today:** `shift-i`, palette; no toolbar button. `mark_read()`.
- **Needs:** Done for Gmail (W5): `threads.modify` removing `UNREAD`, at once.

#### ACT-6 Bulk select and bulk actions

- **Today:** `x` checks; `shift-j`/`shift-k` (or shift-arrows) extend; `cmd`-click toggles, `shift`-click ranges; row checkbox on hover; `Esc` clears. ⌘A (and Edit › Select All in the list) checks every loaded conversation, keeping the anchor (`select_all_threads()`, polish W1, MA-078); ⇧⌘A clears. The reader shows "N conversations selected" with subjects and action buttons; all actions apply to all. `PigeonApp::checked`; `render_selection_summary()` (`src/ui/reader.rs`).
- **Needs:** Done for Gmail (W5): one op per pair of folders, sent as one `threads.modify` / `trash` / `untrash` per thread in HTTP batches of 20 (`Gmail::batch`, which retries rate-limited calls). IMAP: UID sets.
- **Notes:** A partial failure rolls back only the threads Gmail refused (STA-2).
- **Today (polish W2):** While any conversation is picked, the list header has a select-all checkbox: mixed while only some are, a click picks all or lets go (MB-Q12). A menu for several conversations starts with "N Conversations"; the toolbar's "…" leaves out what sits beside it; with no labels, the menu offers "New Label…" instead of an empty Label submenu.

#### ACT-7 Open in Gmail

- **Today (polish W2, MB-061):** ⌘K "Open in Gmail", Message › Open in Gmail and a row's menu open the conversation in the browser at `https://mail.google.com/mail/?authuser=<account>#all/<thread id>` (`Thread::gmail_link()`), so it opens in this account even with several signed in. A conversation not in Gmail yet, and demo data, say so instead.
- **Needs:** A check on the real account.

#### ACT-8 Share…

- **Idea (MB-061 step 2):** "Share…" for a conversation (its `.eml`) and an event (its link), through `NSSharingServicePicker`. An `.ics` file needs an ICS writer.

#### ACT-9 Mark all as read in a view

- **Today (polish W2, MB-010):** Right-clicking a folder, category or label in the sidebar offers "Mark All as Read" (left out when nothing there is unread); the label stays outlined while its menu is open (MB-017). It marks the view's cached unread conversations read as one undoable change (`PigeonApp::mark_all_read()`, `sidebar::MarkAllRead(Nav)`, `set_unread()`), through the outbox as usual.
- **Needs:** Gmail's count can hold more than the cache (older mail): a server-side pass listing the view's unread threads (`threads.list` with `labelIds` + `UNREAD`, paged) and taking `UNREAD` off them in batches (`threads.batchModify` or `messages.batchModify`), with the undo covering those too. IMAP: `STORE 1:* +FLAGS \Seen`.

#### ACT-11 Snooze conversations

- **Today (polish Stretch B):** `b`, the reader's alarm-clock button, ⌘K "Snooze…" (or "Change Snooze…" on a snoozed one) and Message › Snooze… open the shared time picker (`src/ui/later_picker.rs`): Later Today (+3 h, rounded), This Evening (6 PM), Tomorrow (8 AM), This Weekend (Sat 8 AM), Next Week (Mon 8 AM), skipping the ones already past, plus a typed field on the quick-add parser with a live read-back ("Fri, Oct 30, 9:00 AM"); presets and parsing in `src/later.rs`. Works on a multi-selection, with Undo. The conversation leaves the Inbox: Pigeon adds a hidden `Pigeon/Snoozed` label and removes `INBOX` through the outbox (`later::resolve_snoozed` finds the label, or creates it hidden with `labels.create`), and keeps the wake time in SQLite (`snoozes` table, `src/backend/store/snoozes.rs`). The label is left out of Pigeon's label list. Mailbox › Snoozed (`g b`) lists them soonest first, with the wake time in place of the date (no count in the sidebar, like Gmail's); Unsnooze (`⇧B`, the reader's bar, ⌘K) brings one back now, with Undo. On wake (a timer in `src/backend/later.rs`, also caught up at launch, on wake from sleep and on a time zone change, and never waiting more than 15 minutes, so a clock changed by hand is seen) the label comes off, `INBOX` and `UNREAD` go back, and the conversation sorts at the top (`Thread::sort_time()`, from the wake time). A new reply wakes it early (`sync.rs`). Moved back to the Inbox or unlabeled in another client (Gmail web's "Move to Inbox"), it stops being snoozed at the next sync, without the unread wake later (a leftover label comes off); trashed or marked spam there, it stays asleep and is dropped at its wake time. A snooze Gmail refuses (the label can't be made) rolls back and isn't snoozed. The picker re-reads its presets if one passes while it's open, and ↩ on typed text that isn't a time ahead picks nothing. After a catch-up, one notification says "3 conversations are back" (Settings › Notifications on). Demo mode keeps it all in memory.
- **Needs:** A user check on the real account: the label gets made hidden, the snooze survives a relaunch, it wakes on time and after sleep, a reply wakes it, and "Move to Inbox" in Gmail web ends it. Snoozes are app-side: nothing wakes while Pigeon isn't running (it catches up at the next launch), and other clients (Gmail web, phones) see the conversation in `Pigeon/Snoozed`, out of the Inbox, and after the wake at its original date, not at the top. A server-side waker would need a backend of our own.

### 5.5 Compose and sending (CMP)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| CMP-1 | New message | 🟢 Fully linked | `drafts.create` |
| CMP-2 | Reply | 🟢 Fully linked | `messages.send` with `threadId` |
| CMP-3 | Reply all | 🟢 Fully linked | As CMP-2 |
| CMP-4 | Forward | 🟢 Fully linked | `messages.send` (no `threadId`) |
| CMP-5 | Recipient fields | 🟢 Fully linked | Server validation on send |
| CMP-6 | Show Cc and Bcc | 🟢 Fully linked | `raw` MIME; Gmail delivers Bcc |
| CMP-7 | Subject and formatted body | 🟢 Fully linked | `raw` MIME (`multipart/alternative`) |
| CMP-8 | Send | 🟢 Fully linked | `messages.send`, `drafts.send` |
| CMP-9 | Send validation | 🟢 Fully linked | Send errors |
| CMP-10 | Forward quote block | 🟢 Fully linked | `raw` MIME |
| CMP-11 | Reply appends to the thread | 🟢 Fully linked | `threadId`, `In-Reply-To`, `References` |
| CMP-12 | Send failure feedback | 🟢 Fully linked | Send errors |
| CMP-13 | Sent copy on the server | 🟢 Fully linked | `SENT` label (automatic) |
| CMP-14 | Replies quote the answered message | 🟡 Partially done | `raw` MIME (HTML part) |
| CMP-15 | Reply to any message in a conversation | 🟡 Partially done | `In-Reply-To`, `References` |
| CMP-16 | Open a draft in its own window | 🟡 Partially done | None |
| CMP-17 | Send later (scheduled send) | 🟡 Partially done | `drafts.update`, `drafts.send` at the chosen time (app-side) |

#### CMP-1 New message

- **Today:** `c` or palette "Compose New Message" opens an empty draft in the reading pane ("New Message"). `compose()` (`src/app/compose.rs`), `Composer` (`src/ui/composer.rs`). In memory until saved (DRF-1).
- **Needs:** A tester pass on the real account (T2b); account choice when there are several (ACC-4).
- **Notes:** With Google (W6), saving it creates a Gmail draft (DRF-1) and sending goes through `messages.send` (CMP-8). From is the default send-as address, changeable with the From picker (MST-1); it carries that address's signature (MST-2) and saves while typing (DRF-5).

#### CMP-2 Reply

- **Today:** `r`, toolbar, palette. Draft opens under the conversation; subject "Re: …" unless it already starts with "re:". `reply_recipients()` (`src/app/compose.rs`): if the last message is from the user (or an alias), reply to its To, else to its sender; never to the user unless they're its only recipient (a mail to yourself replies to yourself), as Gmail does. Unit-tested. `reply_to` links the thread.
- **Needs:** A tester pass (T2b). Verified on the real account (W6): the reply lands in the same Gmail thread.
- **Notes (polish W2):** A reply answers the focused message while the reader has focus, else the newest (CMP-15), and quotes it (CMP-14). Its title is "Reply", or "Reply (Draft)" when reopened from Drafts; `Composer::title()` is shared with the reader's toolbar, so it no longer flips to "Draft" after the first autosave.
- **Notes:** Earlier participants are ignored. With Google (W6) the send carries `threadId` and `In-Reply-To`/`References` built from the thread's last stored message (`mime::reply_threading()`, `src/backend/mime.rs`); a reply goes to the last message's `Reply-To` when it has one.

#### CMP-3 Reply all

- **Today:** `a`, toolbar, palette. To: sender plus the last message's recipients, minus the user (and aliases); Cc: its Cc minus anyone in To. To the user's own message: its To and Cc, the sender left out. If only Cc remains it moves to To; if no one remains, it goes to the user. Dedupe by email, case-insensitive. `reply_recipients()`, unit-tested.
- **Needs:** A tester pass (T2b).
- **Notes:** `Reply-To` is honoured (W6): its addresses replace the sender's in To (`reply_recipients()`); not on the user's own messages.
- **Notes:** Mailing lists and no-reply senders are included as is.

#### CMP-4 Forward

- **Today:** `f`, toolbar, palette. Subject "Fwd: …" unless it starts with "fwd:"/"fw:". The original is quoted with From, Date, Subject, To. No `reply_to`, so it opens alone and sending creates a new Sent thread (CMP-11).
- **Needs:** A tester pass (T2b).
- **Notes:** The original's attachments come along as removable chips (ATT-7).
- **Notes:** With Google (W6) a forward is a new message: no `threadId`, `In-Reply-To` or `References`; the quote goes as a `<blockquote>` in the HTML part (CMP-10).

#### CMP-5 Recipient fields

- **Today:** To, Cc, Bcc as chips. Comma, semicolon or Enter commits; an unfinished address commits on send; Tab moves between fields; a chip without `@` is red. `Recipients`, `commit()`, `parse_address()` (`Name <addr>` or a bare address with `@` and a dotted domain).
- **Today (polish W2, MB-024):** Each chip's tooltip shows its address (an invalid one says "“x” isn’t an email address" and shows an alert glyph instead of the avatar, MA-024). Backspace in an empty field first picks the last chip, a second Backspace removes it (`RemoveLastRecipient`); Escape or typing unpicks it. Clicking a chip picks it; double-clicking puts it back in the field to edit. Right-click (or Control-click): Copy Address, Show Contact Card, Move to To/Cc/Bcc, Remove. Out of focus, a field with more than six chips shows five and "+N more". Suggestions use the shared person rows. Labels sit in one right-aligned column, so every input and the body start at the same x (ME-005). Contact suggestions split the query once and keep only the top six (`ContactBook::suggest`); `find` uses an address index (MC-066).
- **Needs:** A tester pass (T2b). Local parsing is only a first check; Gmail's rejection reaches the user as "Not sent: …" (CMP-12).

#### CMP-6 Show Cc and Bcc

- **Today:** `cmd-shift-c`, the "Cc Bcc" link, or palette. `show_cc_bcc()`; `DraftContent::bcc`.
- **Needs:** A tester pass (T2b). SMTP: `RCPT TO` only.
- **Notes (W6):** `Bcc` goes in the `raw` message; Gmail strips it on delivery.
- **Notes:** Bcc is kept on drafts and the sent copy; the reader shows it on the user's own mail (RDR-4, ME-012).

#### CMP-7 Subject and formatted body

- **Today (polish W2):** The format bar shows the styles at the caret (selected tint, `aria_toggled`), in groups: text styles, link, lines, clear (MB-037). Its buttons, Send and "Cc Bcc" are Tab stops (MA-005). Bold and headings are Semibold, as in the reader; list bullets are muted (MA-029, MA-031). Right-clicking the body shows Cut, Copy, Paste, Select All, Bold, Italic, Insert Link… instead of the code editor's menu (MB-002). Links are sent only for http(s), mailto and tel; others go as their text (`html::is_safe_url`, MC-083).
- **Today:** Subject (hidden for replies, which keep the thread's). Markdown body styled as you type (bold, italic, strikethrough, `<u>underline</u>`, code, headings, lists, quotes, links); syntax stays visible, dimmed. Formatting bar and Gmail shortcuts (`cmd-b`, `cmd-i`, `cmd-u`, `cmd-shift-x`, `cmd-e`, `cmd-shift-k` for a link so `cmd-k` stays the palette, `cmd-shift-7`/`8`/`9`, `cmd-\`). Enter continues lists; Tab/Shift-Tab indent; Page Up/Down move by page. `Composer` (`src/ui/composer.rs`), `src/ui/rich_editor.rs`, `src/markdown.rs`. On send and save, `markdown::to_html()` (`DraftContent::body_html()`) makes `Message::html`.
- **Needs:** A tester pass (T2b): formatting intact in Gmail web.
- **Notes:** With Google (W6), `mime::build_message()` (`src/backend/mime.rs`, `mail-builder`) makes `multipart/alternative`: `text/plain` is the markdown as typed, `text/html` is `markdown::to_html()`; non-ASCII subjects and names are RFC 2047-encoded; base64url into `raw`. Typed raw HTML is sent as text except `<u>`/`</u>`. No inline images. A reply that changes the subject doesn't change the thread's.

#### CMP-8 Send

- **Today:** `cmd-enter` anywhere in the draft or list, Send button, palette. The draft closes, the message goes to Sent (or the thread), toast "Sending…" with Undo for 5 s (UND-1), then "Message sent" once it went. `send_message()` (`src/app/compose.rs`), `Outgoing::to_message()` (`src/backend/mod.rs`). It queues `MailOp::Send`, held for the undo window. With Google (W6) `commit_compose()` (`src/backend/compose.rs`) sends it: `drafts.send` for a draft saved in Gmail, else `messages.send`, both through the upload endpoint (`uploadType=multipart`) above 5 MB; at the user's quota priority. Demo mode applies it to its in-memory store.
- **Needs:** A tester pass (T2b). Verified on the real account (W6): new, reply, draft send.
- **Notes:** `date` and `snippet` stay local until the next sync refetches the thread.
- **Notes (polish W2, MA-069):** A send still queued 3 s after its undo window (offline) replaces "Sending…" with "Will send when Pigeon reconnects". The clock glyph and "Waiting to send" on the row and in the reader come from `Backend::queued_sends()`, which the app reads only when the outbox changes (a send, its undo, a commit or failure, a new source) and keeps in `PigeonApp::queued_sends`.

#### CMP-9 Send validation

- **Today:** No recipients: "Add at least one recipient" under To. Invalid address: "“zzz” isn't an email address". Typed text becomes the best contact or the address as typed. `Composer::validate()`, `DraftContent::has_recipients()`.
- **Today (polish W2, ME-019):** A new message or forward without a subject says "No subject. Send again to send it anyway." under Subject; a body that mentions an attachment ("attach…", "enclosed", outside `>` lines) with none says so under the body. Each asks once; any edit asks again (`Composer::confirm_send()`). A Gmail draft whose text is still on its way shows "Loading draft…" and won't send ("The draft is still loading", ME-007).
- **Needs:** A tester pass (T2b).
- **Notes:** No empty-body warning. Attachments over 25 MB in all are refused: "Attachments are over Gmail’s 25 MB limit" under the attachments, when attaching and on send (W6). Gmail's own reasons come back as "Not sent: …" (CMP-12).

#### CMP-10 Forward quote block

- **Today:** "Forwarded message" block with From, Date, Subject, To, then the body. `Quoted` (display) and `html()` (`<blockquote>`) in `src/ui/composer.rs`. Kept on the draft (`DraftOrigin::quoted`, `src/data.rs`) so reopened forwards show it (DRF-3).
- **Needs:** A tester pass (T2b).
- **Notes (W6):** Sent as a `<blockquote>` in the HTML part; the plain part has only the note. A forward draft saved in Gmail keeps the quote only locally (`DraftOrigin::quoted`).

#### CMP-11 Reply appends to the thread

- **Today:** `send_message()` pushes a reply into the `reply_to` thread, or a new Sent thread if it's gone. Forwards always make a new Sent thread.
- **Needs:** A tester pass (T2b). IMAP: `In-Reply-To` and `References`.
- **Notes (W6):** The reply's thread gets the sent copy under Gmail's ids at once (CMP-13). A reply to a thread still `sent-local-*` (its own send not yet committed) goes without `threadId` and threads by its headers.

#### CMP-14 Replies quote the answered message

- **Today (polish W2, ME-009):** Reply and Reply All quote the answered message under "On {date}, {name} <{email}> wrote:", folded behind a "•••" button in the composer (Gmail style). Sent as `<div class="gmail_quote"><div class="gmail_attr">…</div><blockquote class="gmail_quote">…</blockquote></div>` in the HTML part (`quote_html()`, `src/ui/composer.rs`); kept on the draft in `DraftOrigin::quoted`, so a reopened reply keeps it. No quote when the answered message's body isn't loaded.
- **Needs:** A tester pass on the real account: the quote folds in Gmail web and Apple Mail. The plain-text part has only the note.

#### CMP-15 Reply to any message in a conversation

- **Today (polish W2, ME-010):** Reply, Reply All and Forward answer the focused message while the reader has focus, else the newest (drafts never): recipients, From, quote and held files come from it (`PigeonApp::answered()`, `src/app/compose.rs`). A per-message Reply cluster in the reader is the reader's part.
- **Today (polish W3, backend):** `Outgoing::answers` and `DraftOrigin::answers` carry the answered message's id; `threading()` (`src/backend/compose.rs`) builds `In-Reply-To`/`References` from that message, and from the thread's last when it's empty or gone. Looked up once per send (MC-029).
- **Today (polish W3 fix step):** The composer keeps the answered message's id (`Composer::answers`, set from `PigeonApp::answered()` in `compose()`, from `DraftOrigin::answers` when a draft reopens, and carried into a draft window); `outgoing()` hands it to drafts and sends, so a reply threads under the message it answers.
- **Needs:** A tester pass on the real account: a reply to an older message threads under it in Gmail web and Apple Mail.

#### CMP-12 Send failure feedback

- **Today:** With Google (W6), a send Gmail turns down (after the transport's retries) goes back to Drafts as a draft (`draft-local-*`, or the draft it was sent from), leaves the conversation it was shown in, and shows the toast "Not sent: <Gmail's reason>" (for example "Not sent: Invalid To header", checked on the real account). `send()` in `src/backend/compose.rs`.
- **Today (drills, sprint 2 W6):** With `PIGEON_DEV=1`, `touch <data dir>/simulate-refusal` makes the next Google write answer 400 with a Gmail-like body, so a send shows "Not sent: Simulated refusal (drill)" and goes back to Drafts on the real account (see STA-2).
- **Needs:** Done; passed with the refusal drill (sprint 2 T2c). No retry button; reopening the draft and sending again is the retry.

#### CMP-13 Sent copy on the server

- **Today:** The Sent thread is made in memory by `send_message()`. With Google (W6), once Gmail answers, the sent copy is stored under its message and thread ids with the Message-ID it went with and Gmail's labels; a new thread's `sent-local-*` id is swapped for Gmail's (`IdReplaced`), then `ThreadsChanged` reloads it, so the history poll finds the same message instead of a second one.
- **Needs:** A tester pass (T2b). IMAP: `APPEND` to Sent unless the server does.

#### CMP-16 Open a draft in its own window

- **Today (polish W3, MB-047):** ⌘K "Open Draft in New Window" and File › Open Draft in New Window move the open draft out of the reading pane into a window of its own (`src/app/draft_window.rs`), as it is: saved if changed, then made again there from the same state (text fields belong to the window they're made in). The window is titled with the subject (the draft's kind until it has one, updated as it autosaves), has the reading pane's draft toolbar (Send, Discard) past the traffic lights, and takes the draft's keys (send, ⌘S, Escape, From, Cc/Bcc, signature, formatting, the Edit menu). Escape, ⌘W and the close button put it in Drafts like closing it in the pane ("Saved to Drafts"). Sending or discarding from it closes the window; toasts and Undo show in the main window, and Undo Send / Undo Discard bring the draft back in the reading pane, also after its window closed. Selecting the draft in Drafts, or replying again to its conversation, brings its window forward instead of opening it twice. Closing the main window saves and closes the draft windows. Inline stays the default.
- **Today (polish W3 fix step):** The reading pane's draft toolbar has an "Open in new window" button. A Gmail draft moved to a window while its text is still loading gets it there (`refresh_open_draft`). The window follows Light / Dark while open.
- **Needs:** No backend. A user check of the two windows (not tested live this sprint). The contact card from a chip in the window opens in the main window, centered.

#### CMP-17 Send later (scheduled send)

- **Today (polish Stretch B):** ⇧⌘⏎ in the composer, its Send Later button, ⌘K "Send Later…" and Message › Send Later… open the shared time picker (ACT-11). The draft is saved, then queued in the outbox as `Op::Later { at, op: Send }` (kind `mail.send_later`, `not_before` = the time), which commits through the normal send path when due (`src/backend/later.rs`, `scheduled_sends`). "Scheduled for Fri 8:00 AM" with Undo for 5 s. Mailbox › Scheduled (`g ⇧T`, shown in the sidebar only while something is scheduled) lists them with the send time; the draft leaves Drafts meanwhile. Selecting one shows a bar with Edit (cancels the schedule and reopens the draft) and Cancel Send (back to Drafts), also in ⌘K ("Edit Scheduled Message", "Cancel Scheduled Send"). Each scheduled send waits only for its own draft's saves, so it never holds back other sends, scheduled or not; it isn't hurried by Quit and doesn't count as "waiting to send" until due. Its timer is re-checked on wake from sleep, a time zone change and at least every 15 minutes. What goes is Pigeon's saved copy (`drafts.send` with its `raw`), so edits made to the draft in Gmail web meanwhile are overwritten. A draft deleted in another client meanwhile (the drafts sync drops it) isn't sent: at its time it fails with "Not sent: the draft was deleted". Sends that go out more than a minute late post "A scheduled message went out late" with when it was due and when it went (the reason isn't known: not running, asleep or offline); a click opens Sent. A scheduled send that fails posts "A scheduled message wasn’t sent" (opens Drafts when the draft is back there), and its toast offers Open Drafts like a send's. Deleting a scheduled draft cancels it; one already going stays ("Already sending"). Not from a separate draft window: the Send Later button shows only in the main window's composer.
- **Needs:** A user check on the real account: it sends on time, after a relaunch and after sleep, and a draft deleted in Gmail web isn't sent. App-side only: Pigeon must be running (or it sends at the next launch), and Gmail shows the message in Drafts until then, not in Gmail's own Scheduled. A server-side scheduler would need a backend of our own.

### 5.6 Drafts (DRF)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| DRF-1 | Save on close or navigation | 🟢 Fully linked | `drafts.create`, `drafts.update` |
| DRF-2 | Drafts list | 🟢 Fully linked | `drafts.list` |
| DRF-3 | Open and edit a draft | 🟢 Fully linked | `drafts.get` |
| DRF-4 | Discard a draft | 🟢 Fully linked | `drafts.delete` |
| DRF-5 | Save while typing | 🟢 Fully linked | `drafts.update` (debounced) |
| DRF-6 | Sync drafts with the server | 🟢 Fully linked | `drafts.list`, `history.list` |

#### DRF-1 Save on close or navigation

- **Today:** A changed draft is saved when closed (`Esc`, toolbar "Close and save draft", palette "Close Draft") or when the user moves elsewhere; toast "Saved to Drafts" (no Undo). Unchanged drafts aren't saved. `stash_composer()`, `save_draft()` (`src/app/compose.rs`), which also queues `MailOp::SaveDraft` (an `Outgoing`, `src/backend/mod.rs`); demo mode stores it in its in-memory store. Drafts are one-message `Thread`s with IDs `draft-local-<uuid>` (DAT-5).
- **Needs:** A tester pass (T2b). IMAP: `APPEND` to Drafts and delete the old copy.
- **Notes:** Quitting loses unsaved changes (PER-2). With Google (W6) `SaveDraft` stores the draft locally first, then `drafts.create` (or `drafts.update` with the stored draft id) with `X-Pigeon-Markdown: 1` and, for replies, `threadId` and threading headers; the local id stays, `drafts.draft_id` and the message id are stored so the drafts sync sees it unchanged. A save Gmail refuses keeps the local text and toasts "Couldn't save the draft to Gmail: …"; one that can't reach Gmail stays queued and goes again (STA-3).

#### DRF-2 Drafts list

- **Today:** Rows show "To: <first recipient>", "(no recipients)", "(no subject)". The sidebar shows the count. With Google (W3) the Drafts step lists `drafts.list` (up to 500) and fetches each changed draft's message with `messages.get format=metadata`: each draft is a Drafts thread `draft-<draft id>` (a reply draft keeps `origin.reply_to` = its thread), its draft id stored in `drafts.draft_id`; drafts gone from Gmail go. Re-read when history touches a draft. Demo: six starter drafts.
- **Needs:** A tester pass (T1a). Opening one with its body and markdown: DRF-3 (W6).

#### DRF-3 Open and edit a draft

- **Today:** Selecting a Drafts thread opens it in the composer ("Draft") via `open_draft()`. Drafts saved here keep their markdown in `DraftOrigin::markdown` and reopen with it; JSON drafts reopen from their HTML as plain text (`html::plain_text`). Kind, reply target and forward quote are kept in `DraftOrigin`. Replying again to a thread with a reply draft reopens that draft (`reply_draft_of()`).
- **Needs:** A tester pass (T2b).
- **Notes:** With Google (W6), opening a Gmail draft whose text isn't here yet fetches it with `drafts.get format=raw` (`Backend::fetch_draft()`, `src/backend/compose.rs`) instead of the message-body fetch; a draft written here (`X-Pigeon-Markdown: 1`) gets its markdown back from the `text/plain` part (`mime::parse_draft()`), one written in Gmail opens as its plain text. The composer fills in when it arrives, unless the user has already typed (`Composer::load()`). Formatting of drafts written elsewhere is lost (no HTML-to-markdown). Draft search doesn't match text only in the quote. Attachments of a Gmail draft are sent again from the draft itself.

#### DRF-4 Discard a draft

- **Today:** `cmd-shift-d` (Gmail shortcuts preset) or `cmd-shift-backspace` (Apple Mail preset, where ⇧⌘D sends, MA-038), toolbar trash, palette, File › Discard Draft. Removed with toast "Draft discarded" and Undo; an unchanged new draft is dropped silently. `discard_draft()`.
- **Needs:** A tester pass (T2b).
- **Notes (W6):** `DeleteDraft` removes the local copy, then `drafts.delete` (permanent) after the undo window (UND-2).

#### DRF-5 Save while typing

- **Today (sprint 2 W3):** 2 s after typing stops (`AUTOSAVE_DELAY`, `src/ui/composer.rs`) the draft is saved to the local store at once (`Backend::save_draft`, `compose::save_locally`) and a `SaveDraft` op is queued for Gmail: the first right away, then at most every 30 s (`REMOTE_SAVE_EVERY_MS`; a newer save replaces the waiting one), and at once when the draft closes. Footer (polish W2, ME-006): nothing while edits wait, "Saving…" between the autosave and the store, then "Saved 2:14 PM" for 3 s, fading in and out (`SaveState::Dirty`, `motion::fade`). A new message gets its stable `draft-local-*` id at its first save and shows in Drafts. Each save carries a revision (milliseconds, rising) of that id, stored in `drafts.revision` (schema 160): a save, or a Gmail answer, older than what's stored never overwrites newer text (it only records Gmail's draft and message ids). A send or a discard that commits leaves a tombstone at the last revision (`draft_tombstones`), so a late save never brings the draft back; a newer save (Undo after the window) does. A send waits behind its draft's saves (resource `send,<local id>`), and a save waiting out its 30 s goes at once when the draft is sent or discarded (`Outbox::hurry`). Crash recovery: the local copy reloads into Drafts at launch, and its queued save goes again.
- **Needs:** Done; passed on the real account (sprint 2 T2b, after fix-6).
- **Notes:** Typing never sends more than one `drafts.update` per 30 s per draft.

#### DRF-6 Sync drafts with the server

- **Today:** Drafts from Gmail come with the mail sync (DRF-2, W3). Drafts saved here go to Gmail (DRF-1) and keep their `draft-local-*` id, linked to the Gmail draft by `drafts.draft_id` (W6). Last write wins: a draft edited elsewhere reloads, but an open draft's text is never replaced once the user has typed (DRF-3).
- **Needs:** A tester pass (T2b): a draft written in Gmail web appears, edits sync.
- **Notes:** `drafts.update` replaces the draft's message, so its message ID changes on every save; track the draft ID, not the message ID.

### 5.7 Undo (UND)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| UND-1 | Undo send | 🟢 Fully linked | Hold `messages.send` for the window |
| UND-2 | Undo discard | 🟢 Fully linked | Delay `drafts.delete` |
| UND-3 | Undo for other actions | 🟢 Fully linked | Delay or invert `threads.modify` |
| UND-4 | Undo history, redo and the Edit menu | 🟡 Partially done | Inverse ops (existing) |

#### UND-1 Undo send

- **Today:** Toast "Sending…" (a working toast) with Undo (`cmd-z`, `z`, palette) for 5 s, then "Message sent". Once the send went it leaves the undo history (UND-4); a send is never redone. Undo removes the message from Sent or the thread, restores the draft and reopens the composer, focused where the caret was (`Composer::remember_focus()` / `restore_focus()`), so its shortcuts work at once. `UndoAction::Send` (`src/app/undo.rs`), `undo_compose()` (`src/app/compose.rs`). The send is a `MailOp::Send` the outbox holds for the 5 s (PER-2); Undo cancels it while it hasn't started, even past the 5 s (a send queued offline), or says "Already sent" if it went. With Google the held send is committed by W6 (CMP-8).
- **Needs:** A tester pass (T2b). Gmail's own "undo send" isn't in the API, so the hold is ours.
- **Notes:** Once sent it can't be recalled.

#### UND-2 Undo discard

- **Today:** Toast "Draft discarded" with Undo; the discard stays in the undo history (UND-4), its held copies (ATT-7) until it falls off. Undo puts the draft back and reopens it, focused in the field it was in (so `cmd-shift-d` works again at once). `UndoAction::Discard`. Discarding a saved draft queues a delayed `MailOp::DeleteDraft`; Undo cancels it, or saves the draft again if it already went.
- **Needs:** A tester pass (T2b). A never-saved draft only needs the composer back.
- **Notes (W6):** The delayed `drafts.delete` is committed to Gmail; Undo after it went creates the draft again. Quitting inside the window commits the discard first (`flush_on_quit()`); if it can't go (offline), it stays queued and the draft stays hidden until it does.

#### UND-4 Undo history, redo and the Edit menu

- **Today (polish W1, MA-070, MA-071):** A session history of the last 20 changes (`UndoStack`, `src/app/undo.rs`), apart from the toast: `cmd-z` / `z` walks back after the toast went, `cmd-shift-z` (Edit › Redo, ⌘K "Redo") redoes moves, label changes and calendar retimes; "Nothing to undo" / "Nothing to redo" say so. Inside the 5 s window an undo cancels the held op; older entries use the inverse op, never a cancel. A new change clears redo; signing in or out clears both. The Edit menu runs `EditUndo`, `EditRedo`, `EditCopy`, `EditSelectAll`: a focused text field gets its own, else Pigeon's undo, the reader's selected text (or the source, or the contact card's address), or the reader's message text / every conversation in the list (⌘A, MA-078). Edit › Undo and ⌘K are titled from the history ("Undo Archive", "Redo Mark as Read"). A mail change undone or redone from the calendar keeps focus and the view there and says so in a toast ("Undid Archive").
- **Needs:** No backend work. Calendar drags and panel time edits go on the history as "Move Event" / "Resize Event" (`ui::calendar::Retimed`, subscribed in `PigeonApp::new`, polish W2). Deleted and discarded events, sends and discards aren't redone.

#### UND-3 Undo for other actions

- **Today:** Archive, Delete and Move to (one or many) show "Archived" / "Moved 3 conversations to Trash" with Undo (`z`, `cmd-z`, button, palette) for 5 s; Undo restores folders and the selection. `UndoAction::Move`, `undo_move()` (`src/app/mail.rs`). Each move queues Gmail ops (`MailOp::moves()`) that wait out the window; Undo cancels them, or queues the moves back once they went. Label, Important, star and read changes are undoable too (`UndoAction::Labels`, polish W1): on several conversations they toast ("Starred 3 conversations") with Undo; one star or read change stays silent but `cmd-z` still takes it back. Labeling and Important always toast with Undo.
- **Needs:** Done for Gmail (W5): Undo inside the window means no request at all; Undo after the commit (or while it's in flight) sends the inverse at once (`Backend::mail_now()`), e.g. `threads.untrash` + `addLabelIds INBOX` after a delete.

### 5.8 Search (SRC)

Search is in memory. `Thread::reindex()` (`src/data.rs`) builds a `SearchIndex` with lowercased subject, sender and recipient names and addresses (to, cc, bcc), snippets, plain-text bodies and attachment names. Matching is `Query::matches` in `src/search.rs`. Signed in, Gmail's search runs alongside it (W9, SRC-7) and its results join the list.

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| SRC-1 | Search bar and keys | 🟢 Fully linked | `threads.list` with `q` |
| SRC-2 | Default search scope | 🟢 Fully linked | `labelIds`, `q` |
| SRC-3 | Search operators | 🟢 Fully linked | Gmail `q` operators |
| SRC-4 | Scope operator `in:` | 🟢 Fully linked | `in:`, `includeSpamTrash` |
| SRC-5 | Text matching | 🟢 Fully linked | Gmail full-text index |
| SRC-6 | Result count | 🟢 Fully linked | `resultSizeEstimate` |
| SRC-7 | Full-mailbox search | 🟢 Fully linked | `threads.list` with `q` |
| SRC-8 | Live search while typing | 🟢 Fully linked | Debounced `threads.list` |
| SRC-9 | More Gmail operators | 🟢 Fully linked | Gmail `q` operators |

#### SRC-1 Search bar and keys

- **Today:** `/`, `cmd-f` or the list header icon opens the field; `Esc` clears and closes; `Enter` focuses results. Empty field: dropdown of operators and the last 5 searches; while typing `from:`/`to:`: matching contacts; arrows and Enter pick. A chip toggles scope ("Inbox"/"All mail"); recognised operators show as chips beside the count.
- **Today (polish W2):** The field's border turns accent while it has the keyboard; its ✕ clears the text and keeps the field ("Clear search"), and closes an empty search ("Close search"). The dropdown lists recent searches first, with "Clear Recent Searches" (also in ⌘K while searching), then six common operators; typing the start of an operator ("has") offers the ones it starts. It scrolls past about 360 pt and keeps the highlighted row in view. With an `in:` the scope chip names the scope ("All Mail", "Trash"), without a chevron. The operator chips are grey, not accent. "Couldn't search Gmail" has Retry. `toggle_search()`, `close_search()`, `focus_results()` (`src/app.rs`); `search_suggestions()`, `remember_search()`, `toggle_search_scope()` (`src/app/mail.rs`); `render_search_bar()`, `render_search_suggestions()` (`src/ui/thread_list.rs`). Recent searches in memory (PER-3).
- **Needs:** A tester pass (T3a). W9 adds Gmail's results to the local ones (SRC-7) and a "Searching Gmail…" note.

#### SRC-2 Default search scope

- **Today:** Without `in:`, the current view (or all mail per Settings › Mail › Search scope); Inbox also covers Archive. `refresh_rows()` with `nav_contains()`. W9: Gmail gets the same scope through `Query::to_gmail_q()` (`src/search.rs`): Inbox → all mail (a Gmail-only query: `labelIds=INBOX`, every category, and only the Inbox's threads show), Archive → `-in:inbox -in:spam -in:trash -in:drafts`, Sent/Spam/Trash/Starred/a label → its `labelIds` (Spam and Trash with `includeSpamTrash=true`). Drafts aren't sent to Gmail (they sync whole). Gmail's results are filtered by the same local scope.
- **Needs:** A tester pass (T3a).

#### SRC-3 Search operators

- **Today:** `from:`, `to:`, `subject:`, `label:`, `is:unread`, `is:starred`, `has:attachment(s)`, quoted phrases (`from:"Sarah Chen"`), AND only. `Query::parse()`, `Term` (`src/search.rs`); substring matches; `label:` matches ID or alphanumeric name.
- **Needs:** A tester pass (T3a). IMAP: `SEARCH FROM/TO/SUBJECT/UNSEEN/FLAGGED`.
- **Notes:** W9: `Query::to_gmail_q()` passes every operator to Gmail's `q`: phrases quoted, `has:attachments` as `has:attachment`, `label:`/`in:<label>` as `label:"<Gmail name>"` (several prefix matches as `{label:"A" label:"B"}`; checked on the real account: quoted names with spaces match the label id's threads). Gmail matches words, not substrings, so the list shows the union (SRC-5).

#### SRC-4 Scope operator `in:`

- **Today:** `in:inbox`, `in:archive(d)`, `in:sent`, `in:draft(s)`, `in:spam`, `in:trash`, `in:anywhere`, `in:all`, `in:starred`, `in:<label>`. `Scope` (`src/search.rs`). `in:all` = everything but Spam and Trash; `in:anywhere` = everything. A thread is in scope the way Gmail finds it (`Query::in_scope`): `in:sent` = any message the user sent (`Thread::in_sent`, so self-sent mail in the Inbox counts), `in:archive` = in no folder (sent-only conversations too); labels and stars match the thread's own, so Gmail's results aren't dropped for having one other derived folder.
- **Needs:** A tester pass (T3a).
- **Notes:** W9: `in:all` → no restriction (Gmail leaves out Spam and Trash), `in:anywhere` → `includeSpamTrash=true`, `in:archive` → `-in:inbox -in:spam -in:trash -in:drafts`, `in:inbox/sent/spam/trash` → their `labelIds`, `in:starred` → `is:starred`, `in:drafts` → local only.

#### SRC-5 Text matching

- **Today:** Plain words match `SearchIndex::all` (`Term::Text`).
- **Needs:** IMAP `TEXT` (slow). Offline, only cached text is searched.
- **Notes:** W9: the list shows the union of local substring matches and Gmail's whole-word matches (`ServerSearch::merge()`), newest first, each once.

#### SRC-6 Result count

- **Today:** "N results in <scope>" from the thread rows. W9: while Gmail has another page, the count is approximate: "About N results" when Gmail's `resultSizeEstimate` is larger than what's shown, else "N+ results"; exact once every page is in (`result_count()` in `src/ui/thread_list.rs`).
- **Needs:** A tester pass (T3a).
- **Notes:** Gmail's estimate is rough (often 201 for anything past a page).

#### SRC-7 Full-mailbox search

- **Today:** W9: signed in, every search also asks Gmail (`PigeonApp::search_gmail()` in `src/app/mail.rs` → `Backend::search()`): `threads.list q=… maxResults=25` at the user's priority, then batched `threads.get format=metadata` for threads the store lacks, which are stored and join the list. "Searching Gmail…" shows beside the count (or as the empty state) while waiting; a "More results from Gmail" row at the end fetches the next `pageToken`. Offline: local results with "Offline: downloaded mail only". Checked on the real account: a word with `older_than:2y` finds 25 threads all older than the sync window.
- **Needs:** A tester pass (T3a).
- **Notes:** Threads found stay in the cache, so they also show in their folders afterwards.

#### SRC-8 Live search while typing

- **Today:** Every keystroke re-runs `Query::parse()` and `refresh_rows()` (`on_search_event()`) on the cache at once. W9: Gmail is asked 300 ms after typing settles (`SEARCH_DEBOUNCE`); a new query drops the waiting request and its results (`ServerSearch::ask()`/`answer()` by serial), and the same query again (a refresh, the scope chip back) asks nothing.
- **Needs:** A tester pass (T3a).

#### SRC-9 More Gmail operators

- **Today (sprint 2 W4b):** A query with `after:`/`before:`, `older_than:`/`newer_than:`, `larger:`/`smaller:`, `filename:`, `cc:`/`bcc:`, `list:`, `category:`, `is:important`, a `-` negation, `OR`, or parentheses/braces goes to Gmail as typed (`Query::gmail_only`); an `in:` folder still sets the scope, and a view's own terms follow the query in parentheses (a label, Starred, Sent, Spam, Trash and the Inbox by `labelIds`; the Inbox then means all its categories but not its archived threads; All mail adds nothing). Its results are Gmail's only, without local matches (they'd be false ones). Each operator shows as a chip ("not from:bob", "OR", "( … )"). Offline, or in demo mode, it says such searches need a connection. The suggestions offer `after:`, `older_than:`, `larger:`, `is:important`, `category:`.
- **Needs:** Done; passed on the real account (sprint 2 T2a, after fix-5). `has:drive` and other operators pass through only alongside one of the above; alone they're matched locally as text.

### 5.9 Contacts (CON)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| CON-1 | Recipient suggestions | 🟢 Fully linked | `people.searchContacts`, `otherContacts.search`, `people.searchDirectoryPeople` |
| CON-2 | Typed name and address resolution | 🟢 Fully linked | As CON-1 |
| CON-3 | Contact management | ⚪ Not done | `people.createContact`, `updateContact`, `deleteContact`, batch variants |
| CON-4 | Contact photos | ⚪ Not done | `photos` person field; `updateContactPhoto`, `deleteContactPhoto` |
| CON-5 | Contact card | 🟢 Fully linked | `people.get`, `people.getBatchGet` |
| CON-6 | Contact groups | ⚪ Not done | `contactGroups.*`, `contactGroups.members.modify` |
| CON-7 | Workspace directory | ⚪ Not done | `people.listDirectoryPeople`, `searchDirectoryPeople` |
| CON-8 | Save a frequent address as a contact | ⚪ Not done | `otherContacts.copyOtherContactToMyContactsGroup` |

#### CON-1 Recipient suggestions

- **Today:** Recipient fields and the event panel's "Add guests" show up to six suggestions: prefix matches first, then word-prefix ("ch" finds Sarah Chen), then substring for 3+ letters, fuzzy only when nothing else matches. Automated senders (no-reply, notification, newsletter, alerts, receipts…) and spam senders are never suggested; added recipients are excluded. Arrows, `Ctrl-n`/`Ctrl-p`, Enter or click, Escape. The book comes from the store: My Contacts first, then Other Contacts (both synced from People, SYN-6) and people sent to from here, each group by how often the address appears in the cached mail; then every other non-Spam sender and recipient in the mail by frequency (`Backend::contacts()`, `Store::contacts()` and `rank()` in `src/backend/store/contacts.rs`). Ranking in `ContactBook::suggest()` and `is_automated()` (`src/contacts.rs`). One `SharedContactBook` (`Rc<RefCell<ContactBook>>`) is shared with every composer and the calendar; `BackendEvent::ContactsChanged` updates it in place (`ContactBook::update()`, contacts keep their index so open suggestion lists stay right), so open composers see new contacts on the next keystroke. Recipients of a sent message join at once (`Backend::note_recipients()`, stored as `source = 'sent'` rows), in demo mode too. Ranking is linear in the number of addresses (hash maps, polish W1 MC-007).
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

- **Today (sprint 2 W6):** Clicking a sender (name or avatar) or a recipient in the reader, a guest or organizer in the event panel, or ⌘K "Show Contact" (`⇧P`, the focused message's sender, or the first other person on the user's own mail) opens a card over the panes (`src/ui/contact_card.rs`, `src/app/contact_card.rs`): initial avatar, name, title · organisation, every address of the contact, phone numbers with their type, Compose (`C`, a new message to them) and Copy Address (`⌘C`, with a toast), and up to 8 recent conversations with any of their addresses, newest first, Spam, Trash and Drafts left out (`↑`/`↓`, `↵` opens one). Esc or a click outside closes it. All from the store (`Store::contact_card()`, `Backend::contact_card()`); no People call. People who aren't contacts show the name the mail gives, the address, "Not in your contacts" and the conversations. The connections sync now asks `personFields=names,emailAddresses,phoneNumbers,organizations` and stores them per contact (`contacts.details`, schema 170); 170 also drops the connections' sync token once, since People only takes a token back with the field mask it was made with, so the next sync lists the contacts again. Other contacts keep `readMask=names,emailAddresses`. In demo mode the card works on the placeholder mail (no phones or organisations there).
- **Today (polish W2-NAV, W2-FIX):** "Search Mail" (`S`) closes the card, switches to Mail and searches all mail for `from:` them (ME-033, `SearchFromContact`, `PigeonApp::search_from`). Opened by a click on a name in the reader (or its "Show Contact" menu item), a guest in the event panel or a recipient chip's menu in the composer, the card hangs at the click as a popover with no backdrop, closing on a click outside (`show_contact(contact, Some(position), ..)`, `ContactCardView::anchored_at`, MB-069); ⇧P, ⌘K and keyboard presses keep the centered card. Return opens the highlighted conversation unless a card button has focus. Buttons have a press state and keyboard focus; the highlighted conversation is the list's active descendant; with no conversations it says what to do next ("Write to them C", ME-001).
- **Needs:** Done; passed on the real account (sprint 2 T2c, after fix-7). Photos (CON-4). A live refresh (`people.get`) isn't planned while the cache is kept current by sync.

#### CON-6 Contact groups

- **Needs:** List groups (`contactGroups.list`, `batchGet`), suggest a group in recipient fields and expand it to its members (`contactGroups.get` with `maxMembers`, then `people.getBatchGet`). Optional: create, rename, delete groups and change members (`contactGroups.create`, `update`, `delete`, `contactGroups.members.modify`).

#### CON-7 Workspace directory

- **Needs:** For Workspace accounts, colleagues from the domain directory: `people.searchDirectoryPeople` (live) and `people.listDirectoryPeople` (cache, with sync token). Needs `directory.readonly`.

#### CON-8 Save a frequent address as a contact

- **Needs:** "Add to contacts" on an Other Contact: `otherContacts.copyOtherContactToMyContactsGroup`.

### 5.10 Images and attachments (ATT)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| ATT-1 | Remote images | 🟢 Fully linked | None (HTTP fetch or proxy) |
| ATT-2 | Inline (`cid:`) images | 🟢 Fully linked | `payload.parts` `Content-ID`, `messages.attachments.get` |
| ATT-3 | Attachment list in messages | 🟢 Fully linked | `payload.parts` (`filename`, `body.attachmentId`) |
| ATT-4 | Open and download an attachment | 🟢 Fully linked | `messages.attachments.get` |
| ATT-5 | Attachment icon in the list | 🟢 Fully linked | `has:attachment`, `payload.parts` |
| ATT-6 | Attach files to a new message | 🟡 Partially done | `messages.send` / `drafts.*` upload (`multipart` or `resumable`) |
| ATT-7 | Forward keeps attachments | 🟢 Fully linked | `messages.attachments.get` + re-attach |
| ATT-8 | Quick Look, Save All and drag to Finder | 🟡 Partially done | `messages.get` (`format=raw`), as ATT-4 |

#### ATT-1 Remote images

- **Today:** `Block::Image` (`src/html.rs`) keeps `src` (only safe `http(s)` addresses, or `cid:`), the `width`/`height` attributes, the link around it and its alignment; tracking pixels (≤ 2 px, by attribute or inline style) and `display:none` images are dropped. Images load only when asked: a "Load images" bar above each message with remote images (Settings › Images = Ask, the default), the `cmd-shift-i` "Load Images" command (the whole conversation), or Always. Never shows no bar (the command still loads). Consent is per message: a message shows its images only once it's allowed (or Always), even ones another message already loaded, so the bar's count is always what's hidden (`image_consent()` in `src/ui/reader.rs`). `html::is_safe_image_url()` allows `http(s)` to named public hosts only (no credentials, `localhost`, `.local`/`.internal`, private, loopback or link-local IPv4, IPv6 literals). `backend/images.rs` fetches with its own `ureq` agent: HTTPS only (`http://` is upgraded), no cookies, no referer, `Accept` image types, 10 MB cap, 30 s timeout, at most 4 at a time, off the main thread, each GET logged as one `Pigeon: GET host/path?… status ms` line (query redacted; cache hits log nothing); the type and size are sniffed from the bytes (PNG, JPEG, GIF, WebP; SVG and the rest aren't shown). Kept in the store's `images` table, newest first within 64 MB (`store/images.rs`), cleared on sign-out. Drawn with GPUI's `img()` at the attribute or natural width, scaled down to fit (`LoadedImages`, `Renderer::block()` in `src/ui/html_view.rs`; the bar in `src/ui/reader.rs`). Not in demo mode (no network; the command says so).
- **Needs:** Nothing more from Google; a proxy would hide the user's IP from senders (later). The setting is Settings › Mail › Load remote images, Ask / Always / Never (saved as `remote_images`).
- **Notes:** Which messages were allowed is kept in memory only, so the bar asks again after a relaunch (images come from the cache then). An image that fails shows "didn't load" (`LoadedImages::failed`, polish W2) and isn't retried until relaunch. `cid:` images need no consent (ATT-2).

#### ATT-2 Inline (`cid:`) images

- **Today (W2, sprint 2):** When a body is parsed (`mime::parse_message()`), every image part with a `Content-ID` (PNG, JPEG, GIF, WebP by sniffing; 10 MB each; polish W3, MB-078: HEIC/HEIF, by type or `ftyp` box, converted to JPEG through `NSBitmapImageRep`, `images::heif_to_jpeg()`) goes into the ATT-1 image cache under `cid:pigeon-<hash of its bytes>`, and the HTML's `cid:` references (percent-encoded or any case) are rewritten to that address, so no request fetches them and messages never share an id by accident (`inline_parts()`, `rewrite_cids()`, `Store::put_bodies()`). The reader shows them without the remote-image bar or consent, in demo mode too (`Document::cid_images`, `ShowImages`, `load_inline_images()`). One evicted from the cache, or in a body stored before this, comes back from the raw message once (`Backend::inline_images()`), which stores the body again.
- **Needs:** Done; passed on real mail (sprint 2 T1b). IMAP: `BODYSTRUCTURE` + `BODY[part]`.
- **Notes:** An inline image that's also an attachment (not marked inline) shows both in the body and as a chip. SVG and other types stay placeholders. HEIC attachments (not inline) aren't converted (MB-Q19 a); they get the image icon.

#### ATT-3 Attachment list in messages

- **Today:** Chips under each body: type icon, name, size (1,000-based like Finder: "1.5 MB", polish W2 MA-043). `Message::attachments`, `render_message()`, `file_size()`, `Attachment::kind()`, `attachment_icon()`. With Google (W3) they come from the raw message once the body loads (`mime::parse_message()`): RFC 2231/2047 names, decoded size, MIME type, `part_id` = the parser's part id. Inline `cid:` images and an invitation's unnamed `text/calendar` body part aren't listed; a named `.ics` file is, like any other. Videos and `.ics` files get a video and a calendar icon (polish W5, MB-079); audio and Wallet passes keep the plain file icon.
- **Needs:** A tester pass.
- **Notes:** Gmail's `attachmentId` isn't in `format=raw`; ATT-4 takes the bytes from the raw message by `part_id` instead.

#### ATT-4 Open and download an attachment

- **Today (W2, sprint 2):** Clicking a chip (or Enter on it, as it's a tab stop) opens the file with the default app; its download button saves it through the save panel, which starts in Downloads. ⌘K "Open Attachment" (`shift-o`) and "Save Attachment…" (`cmd-shift-s`) act on the conversation's attachment, or the focused message's while the reader has focus; with several, a menu lists them (`OpenAttachmentAt`, `SaveAttachmentAt`). The bytes come from the raw message (`messages.get format=raw`) by the attachment's `part_id`, since there's no `attachmentId` (`mime::part_bytes()`); a draft read back from Gmail finds its files by name. Raw messages aren't kept; the file is, in a private cache: `<data dir>/attachments/cache/<generated key>/<sanitised name>` (no directories, control or direction-changing characters, leading dots; 200 bytes at most), with the `com.apple.quarantine` attribute, 256 MB in all, least recently used out first. Apps, installers, scripts and binaries (by extension, MIME type and magic bytes) are revealed in Finder instead of opened. Saved files are quarantined too. `src/backend/attachments.rs`, `src/app/attachments.rs`, chips in `render_message()`. Demo attachments have no bytes: opening or saving one says so in a toast.
- **Needs:** Done; passed on real attachments (sprint 2 T1b). Polish W2: no "Getting…" toast; the chip pulses while its file comes, and clicking it again, or Esc in the reader, cancels (MA-067); a save toast reads "Saved “name”" with Show in Finder, and the next save panel starts in that folder (MB-060). Still open: determinate progress for big files, which needs a streamed download (a 25 MB file arrives in one request), and Esc to cancel.
- **Notes:** With `PIGEON_DEV=1` and `PIGEON_SAVE_DIR` set, saves go straight to that folder without the panel (dev and testing only). The quarantine attribute is set with `/usr/bin/xattr`.

#### ATT-5 Attachment icon in the list

- **Today:** Paperclip when any message has an attachment (`Thread::has_attachments()`). With Google that's known only once the thread's bodies are loaded: `format=metadata` carries no parts.
- **Needs:** A thread-level flag without fetching bodies (for example `q=has:attachment` ids, or `payload.mimeType == multipart/mixed` as a hint).
- **Notes:** Inline `cid:` images don't count.

#### ATT-6 Attach files to a new message

- **Today:** The paperclip opens a file picker (multi-select); dropping files also attaches. Chips with icon, name, size and ×; kept on the draft and shown on the sent copy. `attach_paths()`, `Attachment::from_file()` (name, size, MIME from extension), `Attachment::path`. More than 25 MB in all is refused ("Attachments are over Gmail’s 25 MB limit"). With Google (W6) the files are read from disk when the draft is saved or the message sent and go in a `multipart/mixed` message, through `uploadType=multipart` above 5 MB; files of a draft opened from Gmail are taken from that draft.
- **Needs:** A tester pass (T2b). Progress for big uploads; `resumable` uploads; larger files as Drive links (see §9).
- **Notes:** A file moved or deleted before sending fails the send ("Not sent: couldn't read “x”"). Demo mode reads nothing.

#### ATT-7 Forward keeps attachments

- **Today (W2, sprint 2):** Forward starts with the original's attachments as removable chips (`forwarded_attachments()`, `src/app/compose.rs`): each points at the original by `Attachment::message_id` and `part_id`, and its bytes are taken from the original's raw message when the forward is saved or sent (one fetch per original, `read_files()` in `src/backend/compose.rs`), so they go in the outgoing `multipart/mixed` message. Files already in the ATT-4 cache are copied out to `<data dir>/attachments/held/` when the forward opens, so evicting the cache can't take them from a queued send; held copies go once the send went (those of chips removed from the composer too, `Outgoing::dropped_files`), when a discard can no longer be undone or a never-saved forward closes unchanged, or after 30 days. Over 25 MB in all shows "Attachments are over Gmail’s 25 MB limit" at once, and Send refuses until chips are removed. Offline, the send waits in the outbox as before.
- **Needs:** Done; passed on the real account (sprint 2 T1b, T2b; held copies fixed and retested).
- **Notes:** Inline images of the original aren't attached, so the quote shows them as placeholders. Only the quoted (newest) message's files come along.

#### ATT-8 Quick Look, Save All and drag to Finder

- **Today (polish W2):** Quick Look (MB-044): Space on a focused chip, ⌘Y or ⌘K "Quick Look Attachment" (a menu with several) fetches the file as ATT-4 does and shows it in the system Quick Look panel (`native::quick_look()`, `QLPreviewPanel` through the runtime); again closes it. Anything that could run code is revealed in Finder instead. Save All (MB-059): ⌘K "Save All Attachments…" asks for a folder once, then saves every attachment the shortcuts act on there, numbering taken names; the toast offers Show in Finder. Drag to Finder (MA-086): a chip drags its cached file out of the window (`external_drag_payload`); the drag starts fetching it, so a file not fetched yet can be dragged again once it's here. Not for demo attachments or executables.
- **Needs:** A user check: the Quick Look panel giving the keys back to the window, and ← → between files (one file per preview today). Pigeon sets the panel's data source without being its responder-chain controller, which AppKit logs a warning for.

### 5.11 Links (LNK)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| LNK-1 | Open a link in the browser | 🟢 Fully linked | None |
| LNK-2 | Link safety checks | ⚪ Not done | None (optional Safe Browsing API) |
| LNK-3 | `mailto:` links | 🟡 Partially done | None |
| LNK-4 | `pigeon://` links for automation | 🟡 Partially done | None |

#### LNK-1 Open a link in the browser

- **Today:** Works without a backend: `cx.open_url()` in `Renderer::text()` on an `InteractiveText`; tooltip shows the URL. Only `http`, `https`, `mailto`, `tel` are clickable (`is_safe_url()`, now `pub(crate)` for the composer, MC-083). Right-click has Open Link and Copy Link (RDR-10); ⌥Tab moves through a message's links and ↵ opens one (MA-003). URLs open exactly as written, tracking and redirect wrappers included.

#### LNK-2 Link safety checks

- **Needs:** Warn on suspicious links and text/URL mismatches; optionally unwrap redirects and strip tracking parameters. Network checks must not block the click for long.

#### LNK-3 `mailto:` links

- **Today (sprint 2 W3):** `mailto:` links in messages (`html_view::open_link`) and from other apps (gpui `on_open_urls`, `Show::Compose(Some(Mailto))`) open Pigeon's composer as a new message with To, Cc, Bcc, Subject and Body filled in. `Mailto::parse` (`src/app/shell.rs`) follows RFC 6068: addresses before `?` and in `to=`, several per field, percent-decoded (UTF-8), `+` kept, CRLF in the body made newlines, unknown fields ignored. Settings › General › "Default mail app" shows whether Pigeon opens `mailto:` and offers "Make Default" (Launch Services `LSSetDefaultHandlerForURLScheme`, `src/native.rs`; only when running as the bundled app); also ⌘K "Make Pigeon the Default Mail App". The bundle's Info.plist declares the `mailto` scheme.
- **Today (polish W3):** When "Make Default" fails, its error stays in the Settings row until it works and shows as an error toast while the main window is open (ME-074, `shell::default_mail_app_error()`). The login item and the mail handler are asked of macOS once, after a change and each time Pigeon comes forward, not on every frame (MC-038, `shell::login_item()` / `shell::mail_handler()`). One `percent_decode`, in `backend/http.rs` (MC-017).
- **Needs:** `mailto:` links in messages passed (sprint 2 T2b). A user check left: making Pigeon the default mail app, then clicking a `mailto:` link in another app. Picking the account once there are several (ACC-4).

#### LNK-4 `pigeon://` links for automation

- **Today (polish W3, MB-063):** Links for Shortcuts, Raycast, Alfred and scripts (`open "pigeon://…"`), routed with `mailto:` in `open_urls` (`parse_pigeon_url()`, `src/app/shell.rs`, unit-tested): `pigeon://compose[/addresses][?to=&cc=&bcc=&subject=&body=]` opens a new message filled in as a `mailto:` link would; `pigeon://thread/<id>` opens a conversation by Gmail thread id; `pigeon://search?q=<query>` opens Mail's search with the query; `pigeon://event/new` opens a new event; `pigeon://today` shows Calendar on today. Any web page can open these, so each only opens or fills in something: nothing is sent, saved, deleted or changed, and anything else is ignored. The log names only the scheme, never the link.
- **Needs:** No backend. The scheme is declared in `packaging/Info.plist` (`CFBundleURLTypes`, polish W3); a user check from Shortcuts. App Intents and Siri need a Swift target (later).

### 5.12 Mail settings (MST)

Gmail's `users.settings` resources. All need `gmail.settings.basic` unless noted.

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| MST-1 | Send-as aliases and From picker | 🟡 Partially done | `settings.sendAs.list`, `get`, `patch`, `update` |
| MST-2 | Signatures | 🟢 Fully linked | `sendAs.signature` via `settings.sendAs.patch` |
| MST-3 | Vacation responder | ⚪ Not done | `settings.getVacation`, `updateVacation` |
| MST-4 | Filters | ⚪ Not done | `settings.filters.list`, `get`, `create`, `delete` |
| MST-5 | Forwarding and auto-forwarding | ⚪ Not done | `settings.forwardingAddresses.list`, `get`; `settings.getAutoForwarding` |
| MST-6 | IMAP and POP settings | ⚪ Not done | `settings.getImap`, `updateImap`, `getPop`, `updatePop` |
| MST-7 | Gmail display language | ⚪ Not done | `settings.getLanguage`, `updateLanguage` |
| MST-8 | Delegates | ⚪ Not done | `settings.delegates.*` (service accounts only) |
| MST-9 | S/MIME certificates | ⚪ Not done | `settings.sendAs.smimeInfo.*` |
| MST-10 | Client-side encryption | ⚪ Not done | `settings.cse.identities.*`, `settings.cse.keypairs.*` |

#### MST-1 Send-as aliases and From picker

- **Today (sprint 2 W3):** `sendAs.list` (current `gmail.modify` scope) is read at each full sync's start, at the first partial sync of each launch (so accounts synced before W3 get it) and every 6 hours after (`Step::SendAs`, `SEND_AS_EVERY`), and kept whole: email, `displayName`, `replyToAddress`, `signature`, `isDefault`, `isPrimary`, pending verification (`data::SendAs`, sync state `gmail_send_as`, loaded on launch). A change reaches the app as `BackendEvent::AccountChanged`, and an open draft's From and signature follow it (`Composer::set_identities`). With two or more verified addresses the composer shows a From row above To (`Composer::render_from`): a button with a menu of the addresses (↑/↓, Enter or Space, Esc; first in the Tab order), ⌘⇧F or ⌘K "Change From…". It defaults to the `isDefault` address; replies, reply-all and forwards go from the address I sent the mail from, else the first of mine in its To, Cc or Bcc (`pick_from`). Sent mail and drafts use the alias's display name (or the account's name when it has none) and its Reply-To (`Outgoing::reply_to_address`, `mime::build_message`). A draft keeps its From locally and in Gmail (the message's From header). Demo mode has a second address, `alex@riverastudio.example`, with a name, a Reply-To and a signature.
- **Needs:** The From picker passed in demo mode. The real account has one address (no From row), so sending from an alias needs an account with a second verified address. Editing names and Reply-To stays in Gmail's settings (`patch`/`update` need `gmail.settings.basic`).
- **Notes:** Creating, deleting and verifying aliases (`create`, `delete`, `verify`) are only available to Workspace service accounts with domain-wide delegation; send users to Gmail settings for that.

#### MST-2 Signatures

- **Today (sprint 2 W3):** The From address's Gmail signature (HTML from `sendAs.list`) shows read-only under the body ("--", then the signature, with a remove button), apart from the markdown. It goes into the HTML part at send and draft save as Gmail web writes it (`<div class="gmail_signature">`, `mime::signature_block`) and as text after `-- ` in the plain-text part; drafts written here also carry `X-Pigeon-Signature: 1`. Changing From swaps it; an address without one shows none. ⌘⇧E or ⌘K "Add / Remove Signature" toggles it on one draft; Settings › Mail › Signatures turns it off for new drafts. Whether a draft carries it is kept (`DraftOrigin::signature`) here and across Gmail: reopening a draft written here takes the signature out of its text, and a draft written in Gmail web has its `gmail_signature` block removed before it becomes editor text (`mime::parse_draft`, `strip_signature`), so it never gets a second one.
- **Needs:** Done; passed on the real account (sprint 2 T2b, after fix-6). Editing signatures stays in Gmail's web settings (`sendAs.patch` needs `gmail.settings.basic`).
- **Notes:** Signatures with images from the web show them only as the reader would (ATT-1).

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
| CAL-4a | Go to a date | 🟢 Fully linked | None |
| CAL-18 | Today and the now-line | 🟢 Fully linked | None |
| CAL-19 | Event search in the palette | 🟢 Fully linked | `events.list` with `q` |
| CAL-23 | Persisted calendar UI state | 🟢 Fully linked | Local; `calendarList.selected` |
| **Events** | | | |
| CAL-7 | Edit an event in the panel | 🟢 Fully linked | `events.patch` / `events.update` with ETag |
| CAL-8 | Date, time and all-day editing | 🟢 Fully linked | `start`/`end` (`date` or `dateTime` + `timeZone`) |
| CAL-9 | Create an event | 🟢 Fully linked | `events.insert` |
| CAL-10 | Delete an event and undo | 🟢 Fully linked | `events.delete` (`sendUpdates`) |
| CAL-11 | IDs for new events | 🟢 Fully linked | Event `id`, `iCalUID` |
| CAL-12 | Time zones | 🟡 Partially done | `timeZone` fields; `settings` `timezone` |
| CAL-15 | Reminders | 🟢 Fully linked | `reminders` (`useDefault`, `overrides`) |
| CAL-16 | Busy and free | 🟢 Fully linked | `transparency` |
| CAL-17 | Video links | 🟢 Fully linked | `conferenceData.entryPoints`, `hangoutLink` |
| CAL-21 | Drag to create, move and resize | 🟢 Fully linked | `events.insert`, `events.patch` |
| CAL-22 | Organizer and read-only permissions | 🟢 Fully linked | `accessRole`, `guestsCanModify`, `locked`, `organizer.self` |
| CAL-28 | Event colours and labels | ⚪ Not done | `colorId` + `colors.get`; `eventLabelId` + `calendars` `labelProperties` |
| CAL-30 | Event attachments | ⚪ Not done | `attachments` (`supportsAttachments=true`) |
| CAL-31 | Event visibility and guest permissions | ⚪ Not done | `visibility`, `guestsCanModify`, `guestsCanInviteOthers`, `guestsCanSeeOtherGuests` |
| CAL-33 | Several reminders and default reminders | 🟢 Fully linked | `reminders.overrides` (`popup`, `email`), `calendarList.defaultReminders` |
| CAL-34 | Create Google Meet links | 🟢 Fully linked | `conferenceData.createRequest` (`conferenceDataVersion=1`) |
| CAL-39 | Quick add from text | 🟢 Fully linked | `events.quickAdd` |
| CAL-40 | Move an event to another calendar | 🟢 Fully linked | `events.move` |
| CAL-42 | Open the location in Maps | 🟢 Fully linked | `location` |
| CAL-43 | Event context menu | 🟢 Fully linked | As the actions it offers |
| CAL-44 | Nudge an event with the keyboard | 🟡 Partially done | `events.patch` (as CAL-21) |
| CAL-45 | Duplicate an event | 🟡 Partially done | `events.insert` (as CAL-9) |
| CAL-46 | Email guests | 🟡 Partially done | None (sends through Gmail, CMP-8) |
| **Recurrence** | | | |
| CAL-5 | Recurring events | 🟢 Fully linked | `recurrence` (RRULE, EXDATE), `events.instances`, `singleEvents` |
| CAL-6 | This event or all events | 🟢 Fully linked | Instance `events.patch`; series split with `UNTIL` |
| CAL-29 | Set and edit repeat rules | 🟢 Fully linked | `recurrence` |
| **Invites and RSVP** | | | |
| CAL-13 | Guests and invitations | 🟢 Fully linked | `attendees`, `sendUpdates` |
| CAL-14 | My RSVP | 🟡 Partially done | `attendees[].responseStatus` (`self`) |
| CAL-20 | Invite emails linked to events | ⚪ Not done | `events.list` with `iCalUID`; iMIP parts in Gmail |
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
| CAL-41 | Calendar settings from Google | 🟢 Fully linked | `settings.list`, `settings.get` |

### Views and navigation

#### CAL-1 Switch between Mail and Calendar

- **Today:** Rail icons, `cmd-1`/`cmd-2`, palette "Go to Mail/Calendar". Mail state is kept across switches. `PigeonApp::set_mode()`; `src/ui/rail.rs`. Sign-in asks for `calendar` (W2); without it the calendar pane shows "Calendar access wasn't granted. Sign in again to allow it." (`ui::status::missing_scope()`).
- **Needs:** A tester to confirm on the real account.

#### CAL-2 Calendar list, colours and visibility

- **Today:** The header's calendars button shows and hides calendars; palette "Show/Hide Calendar: X". Events take their calendar's colour. The menu groups "My calendars" (owner) and "Other calendars", marks reader calendars "View only", truncates long names and scrolls when long; rows are checkboxes for VoiceOver. `toggle_calendar()`; `render_calendars_button()` (`header.rs`). From Google (W4, `backend/calendar.rs`): `calendarList.list` (with `showHidden`, then its sync token) gives the name (`summaryOverride` else `summary`), colour (`backgroundColor`, else `colorId` through `colors.get`, fetched once and cached in `sync_state`), `selected` → visible, `hidden` (left out of the menu and not synced), `accessRole`, `primary`, `timeZone`, `defaultReminders` (`Store::put_google_calendar()`). Changes reach the view as `CalendarsChanged`.
- **Today (writes, W7):** Showing or hiding a calendar commits at once with `calendarList.patch` `{selected}` (`backend::calendar::commit()`); a failure reloads the list from the store, which puts the switch back.
- **Today (polish W2):** The calendars button is a keyboard stop: ↓/↑ open the menu and move a highlight, Space or Enter shows or hides the highlighted calendar, Esc closes the menu and stays on the button (ME-045). ⌥-click on a calendar shows only it; ⌥-click it again shows them all. Section headers read "My Calendars" / "Other Calendars" without capitals (MA-027).
- **Needs:** A tester pass on the real account (T2c).
- **Notes:** Shared and subscribed calendars are often read-only (CAL-22).

#### CAL-3 Load events for the visible range

- **Today:** All events in memory; `visible_range()` and `visible_occurrences()` filter on every render. From Google (W4): every calendar that isn't hidden or free/busy-only is synced whole, not by visible range: `events.list singleEvents=false timeZone=<local>` from 180 days back with no end, then by sync token (SYN-4), cached in the `events` table with Google's JSON beside the model. On the test account that is about 2,100 rows in 9 calendars, synced in 6–12 s the first time.
- **Needs:** Wiring `backend::calendar::sync_pass()` into the sync scheduler (merge of W3 and W4). Events older than 180 days aren't fetched (Load older isn't built for the calendar).

#### CAL-4 Day, week and month views and navigation

- **Today:** Day, Week (default), Month. `d`/`w`/`m`; `t` today; `n`/`p` period; `←`/`→` move the selected day; `↓`/`↑`, `j`/`k` step through events past the period. Clicking a day number opens it. The grid scrolls to ~8am. Overlaps sit in equal columns, widening into free ones; columns narrower than 60pt overlap a little. Day view caps the event column at 640pt and shows a mini month and the day's list beside it. Week numbers via View menu or palette. `set_view()`, `open_day()`, `step()`, `step_day()`, `select_adjacent()` (`mod.rs`); `week_view.rs` (`layout::pack_columns()`, `column_box()`), `agenda.rs`, `mini_month.rs`, `month_view.rs` (`layout::pack_lanes()`, "+N more").
- **Today (polish W2):** Tab and ⇧Tab move through the header's controls (title, calendars, view switch, arrows, Today, New); events stay on ↑/↓ and `j`/`k` (MA-005, MA-Q2). The view switch is the shared segmented control; with focus, ←/→ change the view. The title is 22 pt (MA-030); the Day view's title, the day labels and the mini month follow the region's day/month order (MA-043, `data::day_first()`). Declined events are drawn faded and struck through behind the others and take no column (ME-042, ME-Q5); bars, Month bars and the Day list show the user's answer like blocks do (ME-041). Each day keeps a free strip on its right for drag-creating beside a full-width event (ME-053). Block text fits by the window's text size, not a fixed 12 pt (ME-039; the hour stays 50 pt, ME-Q8). The all-day lane shows three lanes at most; past that, two lanes and "+N more" per day, with a chevron in the gutter to show them all; a selected bar in the hidden lanes unfolds them (ME-040). Bars cut at the week's edge show a chevron, and a timed event across days says "until 5 PM" on its last day (ME-090). The Day list says "From 9 AM" / "Until 5 PM" / "All day" for an event across days (ME-089) and has a scroll bar (MB-068). Month day numbers are 28 pt to click (MA-041). Events read their calendar and the user's answer to VoiceOver (MA-048), and the selected one is the active descendant (MA-002). Pointer: the arrow everywhere but the Join chip (MA-079). Frame-wide values (now, the first day) are computed once per frame, events aren't looked up by id while drawing, and the go-to-date text is parsed as it's typed (MC-051, MC-056, MC-059).
- **Needs:** Nothing beyond CAL-3: the whole future is cached, so next/previous event (62 days ahead) always finds what's there.
- **Today (polish W4):** The time grid is a cached view (`CalendarPart`), redrawn only when the calendar notifies, so the panel and header animate without relaying the grid (MC-036/MD-004); it reflows once when a docked panel starts to open (the panel then slides into the room) and once when it starts to close (the panel slides out over the grid), so the grid stays cached through the slide, and the Day view's side column hides with some slack (MD-003). Under VoiceOver the grid is drawn every frame so it stays in the accessibility tree. Two-finger horizontal swipes page Day, Week and Month; scrolling locks to one axis (MA-080). Tentative and unanswered events are outlined with stripes; free ones are lighter (ME-051). Month: day numbers sit at the top left, days outside the month are tinted deeper than weekends (`cal_outside_bg`) and their events faded, and "+N more" opens a popover listing the day (ME-086, ME-087). Block times read "9–10:30" compactly with tabular figures (ME-088). The Today button is quiet while today is in view (ME-093); with week numbers on, Day and Week titles end "· W44", and the gutter's top names the zone offset (ME-094). The mini month dots days with events (ME-099). Today and jumps to an event already on screen glide (MD-040). New, restored and duplicated events fade in, keyed on the event's id and start so a reload doesn't replay it; deleted ones fade out, also under Reduce Motion (MD-038). The now-line ticks on the minute (MD-034).
- **Notes:** Week start is fixed to Monday (`time::week_start()`); Google's `weekStart` setting could drive it (CAL-41).

#### CAL-4a Go to a date

- **Today:** Clicking the header title, `g d`, `cmd-shift-t`, palette or Go › Go to Date… opens a field over a small month. Reads typed dates ("nov 12", "next fri", "tomorrow", "in 2 weeks", "dec 2027"), English only. `toggle_date_picker()`, `go_to_date()`, `render_title()`, `time::parse_date()`.
- **Needs:** Nothing beyond CAL-3.

#### CAL-18 Today and the now-line

- **Today:** Today highlighted; a red now-line in Day and Week; past events faded. `today()`, `now()` from the system clock; Google's times are converted to the same local wall clock (CAL-12).
- **Today (polish W2):** With today on screen the grid opens two hours above now (launch, and back from Month), else at 7:30; a faint now-line crosses the week's other days (ME-043).
- **Needs:** None. The calendar repaints every 30 s, so the line moves.

#### CAL-19 Event search in the palette

- **Today:** In Calendar mode the palette finds events by title and jumps to the next occurrence. `PaletteCommand::Event` items in `palette_items()`; `reveal_event()`.
- **Needs:** `events.list` with `q` (matches title, description, location, attendees) and a cap, as PAL-2.
- **Notes:** Locally only titles are searched. With Google, the master of a series Pigeon can't expand also matches, and jumps to its first start (CAL-5).

#### CAL-23 Persisted calendar UI state

- **Today:** View and week numbers are saved with PER-3. Visibility changes queue `CalOp::SetSelected` (`calendarList.selected`), kept in demo mode's store until quit. W5: the day shown is saved too (`Saved::calendar_date`, written on every calendar change) and the calendar reopens on it (`CalendarView::apply_saved()`); `t` still goes to today.
- **Needs:** Done; passed on the real account (sprint 2 T1c). Nothing for visibility (`calendarList.patch`, W7; CAL-2).

### Events

#### CAL-7 Edit an event in the panel

- **Today:** Click, or `e`/`enter` on the selection, opens the right panel. Title, calendar, location, video link, guests, notes, reminder and busy/free edit in place; changes apply on every keystroke (`on_field_event()`); Esc closes. `open_event()`, `close_panel()`; `EventPanel` (`event_panel.rs`). A footer under the fields says so ("Changes save as you make them") with a **Done** button (⌘S, "Save Event" in ⌘K) that closes the panel, sending what's still settling; read-only events have no footer. While "Apply this change to" is asked, ⌘S/Done waits for the answer.
- **Today (Discard):** Once anything has changed since the panel opened on the event, the footer also shows **Discard** (⇧⌘D, ⌘K "Discard Changes to Event"; on a new event the same shortcut is its Discard). It puts back every field the panel edits (calendar, title, times, all day, repeat, place, link, notes, guests, reminders, busy/free; `Event::restore_edits()`), including edits already sent: one `events.patch` restoring them goes through the same path as any edit (debounce flush, ETag, rollback toast, "Send updates to guests?"), then the panel closes. Nothing is sent when the edits hadn't gone yet and the result equals the original. A change still waiting for "Apply this change to" was never sent and is just dropped. After "This event" the base is the occurrence as the series had it (the occurrence stays a record of its own, now equal to the series); after "This and following" it is the new series as the old one had it (the split stays; ⌘Z/Undo in the toast undoes the split itself). The Google Meet link isn't taken back (Google keeps a conference once made; demo mode's made-up link is). `discard_base`, `discard_changes()` in `event_panel.rs`.
- **Today (writes, W7):** Edits settle, then go as one `events.patch` per event: when the panel leaves the event or after 1.5 s idle (`EDIT_DEBOUNCE`; `patched()`, `settle_edit()`, `flush_edit()` in `mod.rs`). The body carries only the changed fields (`backend::calendar::patch_body()`), with `If-Match` on the stored ETag; Google's answer is stored, so the next patch has the new ETag. A failure reloads Google's version (rollback) with a toast; a 412 says "Changed elsewhere; reloaded" (STA-5). A calendar sync runs 2 s after each committed write (`Backend::sync_calendar_soon()`). Read-only events (CAL-22) show their fields read-only.
- **Needs:** A tester pass on the real account (T2c). Edits still settling are lost if the app quits within 1.5 s. CalDAV: `PUT` with `If-Match`. Discard after a split or a detach leaves a second series or an instance record that matches the original; merging them back (`UNTIL` removal, instance `status: cancelled`→restore) waits for a check on the real account. Removing a Meet needs `conferenceData` deletion, which isn't written.
- **Today (polish W2):** The title wraps to four lines while it isn't being edited (a click or Tab edits it in the field; ME-037). The footer's buttons are the shared push buttons with press states, ⌘S and Discard's keys as keycaps; for an invite the footer holds the RSVP row (CAL-14). The body has a scroll bar (MB-068). Header buttons "Copy link to event" and "Open in Google Calendar" use the event's `htmlLink` (MB-061; shown only when it has one, so not on the demo data); ⌘K's rows for them are still gated until the calendar root handles the actions. Guest rows show the address in a tooltip; typed guests split on commas, and what isn't an address stays with "“x” isn't an email address" under the field (MB-024); the remove ✕ shows while the panel has keyboard focus (MA-047). Calendar, repeat and reminder pop-ups use the shared menu rows with a leading check (MB-019). Panel time edits go on the undo stack as "Move Event" / "Resize Event" (MA-071).
- **Today (polish W4):** Closing slides the panel out with its last content, which takes no clicks while it goes (MD-002); a floating panel flips to the event's other side with a fade (MD-006) and sits flush against the event on the left (MD-005). The series scope card, the delete question and the custom repeat editor ease in (MD-037); added guests and reminders fade in, and Add Meet spins while it waits (MD-041). Guests count maybe and no answers too; links in read-only notes are clickable (ME-097). The header's "…" button opens Duplicate, Email Guests, Copy Link to Event and Open in Google Calendar (ME-097; it replaces the copy-link icon). Date and time pickers fade in; the calendar, reminder, repeat and header calendars menus drop from their button (MD-011). The scope answers read "This Event / This and Following / All Events" everywhere (MC-070). The date picker draws the mini month's grid (`mini_month::month_grid`), so it has the same day labels and states (MC-069). A timed, busy event that overlaps another busy one on a shown calendar says "Conflicts with Standup (and 2 more)" in red text (`text_danger`) under its times (ME-046, `policy::conflicts()`).
- **Notes:** Done only closes (Discard, ⇧⌘D, takes the changes back); there's no staged save for existing events, so failed writes must keep the user's text (STA-2). A sync that reloads the open event can replace fields being edited; the title, location and notes are read back from the panel's fields before each patch (`read_panel_text()`), the rest isn't.

#### CAL-8 Date, time and all-day editing

- **Today:** Date and time fields with pickers; typed values ("tomorrow", "Oct 27", "10/27", "3pm", "15:30", "noon"); up/down by a day or 15 minutes; all-day switch. Tab reaches every control in reading order; enter/space work switch and pickers. `commit_field()`, `step_field()`, `set_date()`, `set_time()`, `toggle_all_day()`; `time::parse_date()`, `parse_time()`. Changing the start keeps the duration; an end at or before the start moves to the next day; ends are ≥ 15 minutes after the start.
- **Today (polish W2):** A timed event that ends on a later day shows its end date beside the end time, in the tab order, with "+2 days · 2 days 8 hr" under it; typing an end time keeps the end's day (a day later only if it would come before the start), and the end date keeps the end time (ME-036, was a silent change to the start's day). The end time picker lists the end day's slots once it's on a later day. While a date or time is typed, the line under it reads it back ("Wed, Oct 28") or says "Not a date yet" / "Not a time yet" with a red field (MA-085); what doesn't read still reverts on leaving. The end date picker greys out days before the start and tints the days between (ME-091). All day is a checkbox (MB-Q9); turning it off brings back the time and length the event had, else 9 AM for the default event length (ME-047).
- **Today (writes, W7):** All-day writes `start.date`/`end.date` (exclusive end) and clears `dateTime`; timed writes the instant (UTC) in `dateTime` with `timeZone` (CAL-12) and clears `date`. Checked on the real account: all-day and back.
- **Needs:** A tester pass on the real account (T2c).

#### CAL-9 Create an event

- **Today:** `c` or the header "+" creates one at the next half hour (today or the selected day), opens it, focuses the title. Dragging or double-clicking the grid also creates (CAL-21). Goes to the default calendar (see Default calendar below), else the primary one when shown, else the first shown calendar the user can write to; its default reminder (CAL-15), busy, user as organizer. `new_event()`, `create_event()`. A new event with nothing filled in (no title, place, notes, link, guests or repeat; `Event::has_content()`) is dropped when its panel closes, without a question or a toast.
- **Today (Save and Discard):** While a new event is open, the panel's footer says "Not saved yet" with **Discard** (Esc) and **Save** (⌘S, "Save Event" in ⌘K); the header X reads "Discard new event", and the "Delete event" row is hidden. Save without a title focuses the title and toasts "Add a title to save the event". Discard, X, Escape and ⇧⌘D drop the event; nothing reaches the backend (`discard_new_event()`).
- **Today (leaving an unsaved event):** Anything that would move the panel off a new event with something filled in first asks **"Save this event?"** with **Save** (↵ or ⌘S), **Discard** (⇧⌘D) and **Cancel** (Esc; stays on the new event), as a card over the grid like "Send invitations to guests?": clicking or dragging another event, arrow keys or `j`/`k` with the panel following, jumping to an event from ⌘K, making a new event (`c`, the grid, quick add), and stepping a day to one with no events. After Save or Discard it carries on with what the user was doing (`Leave`, `ask_to_leave()`, `answer_leave()`). Dragging the new event itself doesn't ask. Save on an untitled one with other fields filled in focuses the title and toasts "Add a title to save the event". If Save raises "Send invitations to guests?", that is answered first and the click isn't repeated. Switching to Mail doesn't move the panel (the unsaved event stays open in the calendar), so it doesn't ask.
- **Today (Undo a discard):** Discarding a new event that had something filled in (button, X, Escape, ⇧⌘D or Discard in the question) shows a toast "Discarded “Title”" (or "Discarded new event") with Undo (⌘Z). Undo brings the event back, opens the panel on it with the title focused, still unsaved: nothing is inserted and no guest question is asked until Save (`CalendarEvent::Discarded`, `UndoAction::DiscardEvent`, `restore_discarded()`). It's its own undo, apart from "Deleted …" (`UndoAction::DeleteEvent`, `Removal`), so each toast restores only its own thing; from Mail it switches to the calendar first. If another unsaved new event is open then, Undo asks about that one first. Cancel on "Send invitations to guests?" after Save reopens the event, still unsaved. A new event never asks "Apply this change to" (no series exists yet).
- **Today (writes, W7):** `events.insert` once the new event has a title and is saved (Save, or Save in the "Save this event?" question; `CalendarView::unsent`, held back in `send_edit()` while `new_event` is open); an untitled one never reaches Google. With guests it asks "Send invitations to guests?" first (CAL-13); a Meet request goes with the insert (CAL-34). Quick add (CAL-39) waits for Save the same way.
- **Needs:** A tester pass on the real account (T2c). A staged new event lives only in memory until saved; quitting with it open loses it.
- **Today (default calendar):** Settings → Calendar → Default calendar lists Automatic (the default) and every calendar the user can write to (Owner or Writer). Every way of making an event goes through `create_event()`: `c`, the header "+", drag or double-click on the grid, quick add (CAL-39). The choice is the calendar's id, kept in `Saved::default_calendar` (`CalendarPrefs::default_calendar`) in the local store like the other settings, for the one account Pigeon has. If that calendar is gone, read-only now, or not loaded yet (before the first sync), the event goes where Automatic would (`calendar::new_event_calendar()`). The reminders and the Meet auto-add follow the calendar chosen. A default calendar that's hidden still gets the event: the calendar is shown first (`toggle_calendar()`, as when jumping to one of its events), so the new event doesn't vanish from the grid; that is `calendarList.patch` `selected: true` (CAL-2), so it stays shown afterwards.
- **Needs:** Nothing from the backend: it's a Pigeon preference and needs no Google API; the id comes from the calendar list (CAL-2). It isn't synced between Macs.
- **Notes:** Showing a hidden default calendar also changes its checkbox in Google Calendar.

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
- **Today (W5):** Google's `timezone` setting is read with the other settings (CAL-41). Times still show in the macOS zone; when Google's differs, a clock icon in the calendar header (tooltip and VoiceOver label) and Settings → Calendar → Time zone say so.
- **Needs:** A tester pass (T2c).
- **Today (polish W2):** The panel says "Set in Europe/London" under the times of a timed event whose own `timeZone` isn't this Mac's (ME-095); the times still show at this Mac's (no tz database, so no converted times).
- **Notes:** No zone picker or showing events in Google's zone.

#### CAL-15 Reminders

- **Today:** One reminder (none, at start, 5, 10, 15, 30 min, 1 h, 1 day). `reminder_minutes`. Nothing fires.
- **Today (writes, W7):** Saved as `reminders` `{useDefault: false, overrides: [{popup, m}]}`; "No reminder" is `useDefault: false` with no overrides. Written only when changed, so a calendar default stays in use until the user picks one (`Event::reminder_use_default`; picking any row, even the default's own value, makes it explicit). Inserts carry the same setting (`reminders_json()`): a new event (`c`, drag) starts on its calendar's default (`useDefault: true`, shown as that calendar's default, following the calendar picker) as in Google Calendar; the new series of a "This and following" split and an Undo re-insert keep the original's default or overrides.
- **Today (W5):** The panel lists every reminder (CAL-33). While Pigeon runs, popup reminders that fall due are found by `calendar::reminders::Scheduler`.
- **Today (sprint 2 W7b):** The scheduler runs app-wide, in `app::today` (the model the menu bar reads), so reminders fire with the window closed: it ticks just past each minute boundary on the calendar as last read from the store (reloaded on `EventsChanged`, `EventsRemoved`, `CalendarsChanged`, committed ops and sign-in, not per tick), and Google's `remindOnRespondedEventsOnly` with it. Each due reminder goes through W1's `notify::notify` (`reminder_notice()`): title the event, body "In 10 min · 11 – 11:45 AM" (or "Now", or "All day · …") and the location, tagged `event-<calendar>-<id>-<start>` so a later reminder of the same occurrence replaces it, a click opens the event in the calendar (`NoticeTarget::Event`), and a Join action opens its video link (`policy::video_link`). Logged as `reminder: <title> at <start> (<n> min before)` and `notify: tag=event-…`. The window's `CalendarView` no longer ticks reminders. Tested with a fake clock and a recording sink (`app::today` tests).
- **Today (polish W3, MB-051, MB-052, MC-064):** Settings › Notifications › "Event reminders" (`Saved::event_reminders`, on by default): off, a due reminder is still marked fired but not posted (logged `reminder: … not shown (reminders off)`), so turning it on later doesn't bring back missed ones. Reminder notices offer "Snooze 5 Minutes" (and Join with a video link): `today::snooze()` posts the same reminder again 5 minutes later under the same tag, saying how long is left then ("In 5 min", or "Now"); snoozes live in memory, so a quit drops them. They play the default sound with "Play sound" on, group under their own `threadIdentifier` ("events") and show "Event reminder" when previews are hidden. Not Time Sensitive: that needs a restricted entitlement the self-signed build can't carry (decision 3). The scheduler's look-ahead reads each calendar's and event's reminder minutes without cloning them. ⌘K "Turn Event Reminders On / Off" flips the setting (W3 fix step).
- **Needs:** A tester pass (T2c), and user checks of the notification itself: arriving with the window closed, clicking it, Join.

#### CAL-16 Busy and free

- **Today:** Busy/Free choice; drawn the same. `busy` on `Event`.
- **Today (writes, W7):** Saved as `transparency` (`opaque`/`transparent`). Busy/free and reminder changes alone don't ask about guests.
- **Needs:** A tester pass (T2c). It affects free/busy for others (CAL-27).

#### CAL-17 Video links

- **Today:** Shows and edits a link with a Join button (`cx.open_url()`). `video_link` in JSON. From Google (W4): `conferenceData.entryPoints` (the `video` one) or else `hangoutLink` fills `Event::conference_uri` and `video_link`, so Join works.
- **Today (writes, W7):** A link from `conferenceData`/`hangoutLink` is read-only in the panel and never written. A link typed in Pigeon is saved into the location when that's empty, else at the top of the description (6.4 item 9; `place()`); after the next sync it shows there, not in the video field.
- **Today (W5):** `calendar::policy::video_link()` also finds a Meet, Zoom, Teams or Webex URL in the location or description (for the menu bar and quick join, W1; `next_joinable()`). Creating Meet links: CAL-34.
- **Today (join in the panel):** The panel's Join button uses `policy::video_link()` too, so an event whose only link is in its location or notes (a Zoom URL pasted into the description, say) gets Join beside the empty video field, and "Add Google Meet" is hidden since the event already has a call.
- **Today (polish W2):** The panel's Join is the shared primary button, its tooltip naming the service and ⌘J (ME-096).
- **Today (polish W2, grid):** A meeting under way, or starting within 10 minutes, shows a Join chip on its block (ME-096). ⌘J (⌘K "Join Meeting") joins the selected event's call, or says it has none.
- **Today (polish W5 fix, MB-036):** A typed video link that isn't a web link (`policy::is_link`) shows "Not a link" under the field once it loses focus, with the field's error border; typing clears it. Such a link gets no Join, in the panel, the grid or the menu bar (`policy::video_link` leaves it out).
- **Needs:** A tester pass (T2c). Phone and PIN entry points aren't shown; the panel's video field itself still shows only `video_link`.

#### CAL-21 Drag to create, move and resize

- **Today:** Day and Week grids: drag empty space to create (live preview snapped to 15 minutes, times floating above; opens with title focused on release); double-click for 30 minutes; drag to move across time and days; drag the bottom edge to resize (≥ 15 min). Auto-scroll near edges; < 4 px is a click; Escape cancels. `src/ui/calendar/drag.rs` (`GridDrag`, `press_grid()`, `press_occurrence()`, `drag_to()`, `finish_drag()`) calls `create_event()` and `retime_occurrence()`. The all-day lane and Month view can't be dragged (a read-only event there still says why when dragged, CAL-22).
- **Today (writes, W7):** A move or resize writes once on release (`flush_edit()` in `finish_drag()`), asking about guests when the event has any; a drag-create inserts once titled (CAL-9). Series ask "This event / This and following / All events" first (CAL-6). Failures reload Google's version, which snaps the event back (STA-2). Read-only events (CAL-22) don't move and have no resize edge.
- **Today (polish W2):** A double click on the grid makes an event of the default length (Settings or Google's, as `c`; ME-047). A double click in the all-day lane or on a Month day makes an all-day event there (ME-044, first part). A click on empty grid, lane or Month space clears the selection and closes the panel; an unsaved new event with something filled in asks "Save this event?" first (ME-049, ME-Q6). The pointer stays the arrow while moving, shows the resize arrows on a bottom edge, and "not allowed" when a read-only event is dragged (MA-084). Right-click or Control-click on an event selects it and opens a menu instead of a drag (CAL-43). The auto-scroll timer starts only once a press becomes a drag, never in Month (MC-060). A move or resize emits `Retimed` ("Move Event" / "Resize Event") for the undo stack (MA-071); the app's subscription to it is still to be wired (W2-SHELL).
- **Today (polish W4):** The drag ghost follows on a spring and lifts while held; a cancelled drag (Esc, or Cancel on the move question) glides home (MD-035, MD-036). Snapping to another event's start or end taps the trackpad once (MA-074). Holding at the grid's left or right edge for half a second pages to the previous or next period (MA-092). Holding ⌥ at the drop makes a copy instead (pressing or letting go of ⌥ without moving counts too; copy cursor; a series copies the one occurrence as a new event, through `copy_event()` as CAL-45). A create ghost takes the default calendar's color and its label adds the length ("9 – 10:30 · 1h 30m"; ME-085).
- **Needs:** A tester pass on the real account (T2c). Not done: dragging all-day and Month events to other days (`DragKind::MoveDays`, the second part of ME-044). The ⌥ copy is a plain `events.insert` like CAL-45.

#### CAL-22 Organizer and read-only permissions

- **Today (W7):** `backend::calendar::access()` gives Edit, Rsvp or ReadOnly: a reader or free/busy calendar is read-only; on a writer/owner calendar the organizer (`organizer.self`, or the user's address, as in demo mode) or a guest with `guestsCanModify` edits; birthdays and events from Gmail don't; an invited user only answers. Read-only events show their fields read-only with a line saying why, no guest field, no resize edge, and can't be dragged; Delete is hidden. For an invite, Delete reads "Decline and remove" and declines it, then deletes the user's copy (CAL-10). The calendar picker (CAL-40) is only for the organizer of a whole event, and lists calendars the user can't write to faded. Applies in demo mode too.
- **Today (sprint 2 W6):** A refused change now says why in a toast (`backend::calendar::refusal()`, `CalendarView::refuse()`): dragging or resizing a read-only event (once per drag, in the time grid, the all-day lane and the Month view: `press_read_only()`; it stays put and the release opens it), changing its times, all day or any field through the panel, and Delete / ⌫ on it. On a reader or free/busy calendar the toast and the panel's line read "“<calendar>” is read-only"; someone else's invite "Only the organizer can change this event"; birthdays and events from Gmail "You can't change this event".
- **Today (panel):** For an event the user can't change (read-only or an invite they only answer), the panel has no editors at all: empty location, video link, guests and description rows are left out; filled ones are plain text (a video link opens on click, Join works, guests still open their contact card), the date and time are plain text with "All day" in words instead of the toggle, Busy/Free is text, there's no footer (an invite's holds only its RSVP row, CAL-14), and Tab only visits the calendar name and title. Reminders show as text too, with no "Add reminder": on a reader calendar Google refuses every write to its events, per-user reminder overrides included. For an invite on the user's own calendar Google would take their own reminders, but Pigeon writes no per-user overrides yet, so they stay locked (a "Needs" below). `render_when()`, `render_place()`, `render_notes()`, `render_guests()`, `plain_text()` in `event_panel.rs`.
- **Needs:** Done; passed on a subscribed read-only calendar (sprint 2 T2c, after fix-7, and the user's check). Per-user reminders and busy/free for an invited event (`reminders.overrides`, `transparency` on the guest's copy) aren't editable yet. `locked` and `privateCopy` aren't read.
- **Notes:** Guests can usually change only their RSVP, reminders and, on their copy, busy/free.

#### CAL-28 Event colours and labels

- **Needs:** Per-event colour override: legacy `colorId` from the `colors.get` event palette, or the newer labels: `eventLabelId` pointing at the calendar's `labelProperties.eventLabels` (custom name and colour, up to 200 per calendar; requires `eventLabelVersion=1`, which makes `colorId` ignored). Managing labels needs owner access (`calendars.update`).

#### CAL-30 Event attachments

- **Needs:** Show `attachments` (Drive `fileUrl`, `title`, `mimeType`, `iconLink`; up to 25) and open them in the browser; adding them needs `supportsAttachments=true` and a Drive file (§9).

#### CAL-31 Event visibility and guest permissions

- **Needs:** `visibility` (`default`, `public`, `private`) and the guest switches `guestsCanModify`, `guestsCanInviteOthers`, `guestsCanSeeOtherGuests` in the panel.
- **Notes:** On a recurring event, making one instance more private applies to the whole series.

#### CAL-33 Several reminders and default reminders

- **Today (W5):** Events keep up to 5 reminders, each a notification or an email (`Event::reminders`, `set_reminders()`; `reminder_minutes` stays the earliest). Calendars keep all their `defaultReminders` (`Calendar::default_reminders`, schema block 130). The panel lists the reminders that apply (the calendar's marked "Default"; "Calendar default · none" when that calendar has none), each with a Notification/Email toggle and a remove button; "Add reminder" (`⇧A`) offers "Calendar default" and 0 min to 1 week. Written as `reminders.overrides` with their methods; "Calendar default" writes `useDefault: true` with an empty `overrides`, since a PATCH merges and Google refuses both at once. Refused calendar writes log Google's message. Popup reminders fire while Pigeon runs (`calendar::reminders`): never ones missed while asleep or before launch, recomputed after a wake or a clock change (the wall clock against `Instant`), each (calendar, event, occurrence, offset) once, kept in the store for a week (`fired_reminders`); Google's `remindOnRespondedEventsOnly` skips unanswered invites. Shared rules for W1 in `calendar::policy` (`reminders_for()`, `next_joinable()`, `video_link()`).
- **Today (polish W2):** Each reminder's method is an "Alert | Email" segmented control instead of a chip that flipped on click (MB-030); a "Calendar default" line heads the list while the calendar's apply.
- **Needs:** Done; passed on the real account (sprint 2 T1c, after fix-2). Still open: editing a calendar's own `defaultReminders` (`calendarList.patch`). `notificationSettings` on `calendarList` controls Google's own emails.

#### CAL-34 Create Google Meet links

- **Today (W5):** "Add Meet" in the panel's video row (`⇧G`, ⌘K "Add Google Meet"), only on calendars whose `conferenceProperties.allowedConferenceSolutionTypes` has `hangoutsMeet` (in demo mode any writable calendar, which gets a made-up link at once). It sets `Event::meet_request`; the patch or insert carries `conferenceData.createRequest` (`requestId`, `hangoutsMeet`) with `conferenceDataVersion=1`, then reads the event again up to 8 times, a second apart, until `status.statusCode` is `success`. "failure" or no answer in time fails the op ("Couldn't add Google Meet: …"); the button then reads "Retry Meet" and sends the same request id. Other writes never send `conferenceData`, so Google keeps the conference. New events get a Meet when Google's `autoAddHangouts` is on (CAL-41).
- **Needs:** Done; passed on the real account (sprint 2 T1c).
- **Notes:** Google may not retry a request id it already refused; a retry after "failure" may need a new id.

#### CAL-39 Quick add from text

- **Today (W5):** In ⌘K, "New event: Lunch with Sam Fri 1pm" (or "new event …") shows a row "New event: Lunch with Sam · Fri, Oct 30 · 1 – 2 PM"; enter opens the event in the panel with the title and times filled in, on that day. `⇧C` in Calendar opens ⌘K with "New event: " typed. ⌘K "New Event" and "New Event from Text…" are offered in Mail too; they switch to Calendar (`Show::NewEvent`, the quick add's own switch). Parsed locally (`calendar::quick_add`: dates and times as the panel reads them, ranges, "for 30 min", "all day"; a date without a time is all day, a time already past is tomorrow, "at 3" is 3 PM); no `events.quickAdd`.
- **Today (polish W4, ME-099):** A capitalized place after "at" or "@" becomes the location ("Lunch with Sam at Nopa Fri 1pm" → Nopa); times are read first, so "Meet at noon" stays a time. Capitalized names after "with" (joined by "and") are looked up in the contacts by first name and added as guests; they stay in the title. Polish W5: the palette's row ends with the place it read ("Fri, Oct 30 · 1 – 2 PM · Nopa"); guests show only in the title.
- **Needs:** Done; passed on the real account (sprint 2 T1c). The place and guest parsing (polish W4) waits on a tester pass; guests added this way get invitations on Save (CAL-13).

#### CAL-40 Move an event to another calendar

- **Today:** The panel's calendar picker (`render_calendar_picker()` in `event_panel.rs`) changes the event's calendar. W7: it goes as `events.move` with `destination` (`CalOp::Move`, before any patch of the same edit); Google's answer is stored under the new calendar. Reminders and the event color are per calendar, and Google drops them right after a move (its move answer still shows them, with an etag already stale), so the same commit patches an explicit `reminders` override and any `colorId` back onto the moved event (without `If-Match`), storing the event with them in between so the panel never flips to the new calendar's default; an event on its calendar's default reminders moves alone (checked live, Pigeon Test → Pigeon Test 2). Only the organizer of a whole event (not one occurrence) gets the picker; calendars the user can't write to are faded, listed after the writable ones (each group in list order; `picker_order()`), and can't be picked.
- **Needs:** A tester pass on the real account (T2c; not checked yet, as it touches a second calendar). Birthdays, focus time, out of office and working location events can't move.

### Recurrence

#### CAL-5 Recurring events

- **Today:** `recurrence` is a `Rule` (`src/calendar/recurrence.rs`): `freq`, `interval`, `by_day` (with optional ordinals), `by_month_day`, `count`, `until` (the series' end, exclusive; RRULE's inclusive UNTIL is converted both ways), plus `unsupported` and `custom` flags. The placeholder data's keywords become constructors; `Rule::label()` says "Every weekday", "Every 2 weeks", "Every week on Mon, Wed", "Every month on the last Fri", or "Custom repeat". `Rule::parse()` / `to_rrule()` read and write RRULE; `parse_recurrence()` reads Google's `recurrence` lines (RRULE, EXDATE in the local zone, a date, or UTC). Expansion (`recurrence::occurrences()`, visible range only) covers `FREQ` daily to yearly, `INTERVAL`, `BYDAY` (ordinals for monthly), `BYMONTHDAY` (negative too), `COUNT`, `UNTIL` and exceptions. With Google (W4), series come once with their rule (`singleEvents=false`); every instance Google lists on its own, moved or cancelled, is stored (cancelled ones as hidden rows) and the series' `exceptions` are recomputed as its EXDATEs plus those instances' `original_start`. Rules outside the subset (BYSETPOS, BYMONTH, BYWEEKNO, hourly…), RDATE/EXRULE, EXDATEs in another zone, and timed series kept in another zone are flagged `unsupported`: not expanded, and `events.instances` (180 days back to 400 ahead) is stored as concrete occurrences instead. Checked against Google's own expansion (`singleEvents=true`) on the test account: the same 913 occurrences over 70 days, across the DST change.
- **Needs:** Nothing more for reading. Writes of series scopes are CAL-6 (W7).
- **Notes:** Google lists an instance for occurrences nobody changed too (on the test account 1,293 of 1,412 listed instances have the series' times and fields), so the panel's repeat line says "· this one changed separately" only when the occurrence differs from its series: moved (start ≠ `original_start`), resized, or with its own title, place, notes, link, guests, reminders, busy/free or all-day (`Event::differs_from_series()`); otherwise it shows just the series' rule. The line wraps instead of running off the panel. Instances of an unsupported series are stored as concrete occurrences and follow the same rule. Their window doesn't slide until the series changes or the calendar fully resyncs.
- **Notes:** The UI can't set a rule (CAL-29); new events never repeat.

#### CAL-6 This event or all events

- **Today:** Changing, dragging or deleting one occurrence asks which ones. In the panel, the first change shows "Apply this change to: This event / This and following / All events" (shown on the whole series until answered; closing the panel, clicking an empty grid slot, opening another event, dragging or deleting before answering takes the change back, like Google Calendar's Cancel, and nothing is written: `EditScope`, `drop_unscoped_edit()`). Escape from any panel field takes it back too and leaves the panel open on the event as it was; an open date or time picker (or guest suggestions) closes on the first escape, the change goes on the next. Dropping a drag asks "This event / This and following / All events" over the grid; enter = "This event", escape cancels. Delete asks "This event / This and following / All events" (escape cancels), then "Send cancellation to guests?" when there are guests (Cancel keeps the event); a panel change answered "All events" then asks about guests, and Cancel there puts the series back. Undo restores any delete. "This event" copies the occurrence into its own event (`detach()`) and adds an exception; "This and following" on a delete sets `until` and drops later changed occurrences; "All" changes or deletes the series and its exceptions. "This and following" on a change or a drag splits the series like Google Calendar (`Event::split()`, `split_series()`): the series, as it was, gets `until` at the occurrence, and a new series (new client id, CAL-11) starts at the occurrence with the edited fields, guests and rule; a COUNT becomes what's left (skipped occurrences count), an UNTIL or no end stays. The old series' occurrences changed on their own from there on are dropped, not carried over. From the series' first occurrence, or on someone else's series, it's "All events". Guests are asked as for "All" (Cancel puts the series back and removes the new one). A toast "Changed “…” from … on" offers Undo: the series back, the new one deleted, the dropped occurrences back. `SeriesPrompt`, `answer_prompt()`, `delete_occurrence()`, `restore()` (`mod.rs`); `choose_edit_scope()` (`event_panel.rs`).
- **Today (writes, W7):** "This event": the occurrence takes Google's instance id (CAL-11); the commit finds the instance with `events.instances` around its original start and patches only what differs from the series (`If-Match` on the instance's ETag); a delete deletes the instance. "This and following": the series' RRULE gets `UNTIL` just before the occurrence (`rewrite_rrule()`: COUNT dropped, other parts and EXDATE lines kept as Google wrote them), with `If-Match`. "All": patches the series; a retime that moves the series' end rewrites `UNTIL` the same way. Changing an instance changes its series' ETag, so the series is read again after each instance write. The panel holds its edits back until "This / All" is answered, so a "This event" change never touches the series in Google: the series' own patch on detaching (only its new exception, so an empty body, not sent) doesn't read the panel's text, which is the occurrence's (`send_edit()`; before, it carried the new title to the series and every occurrence). All three checked on the real account's `[Pigeon test] series`; the panel's "This event" re-checked live after that fix (only the one instance changed). The split for changes commits as one op through the outbox (W5: `split_ops()` builds a `CalOp::Split`): the series' RRULE end is rewritten with `If-Match` first, and the new series is inserted only once that succeeded; if Google refuses the patch (412, changed elsewhere), nothing goes in and both reload as Google has them; queued offline, the op goes whole later. Undo queues the series' patch back, then the new series' delete. Checked live on a throwaway `[Pigeon test] split count` series (weekly, COUNT=6, split at the 3rd with a new title an hour later): the old series kept 2, the new one had 4 at the new time and title; undo brought back all 6 (COUNT=6); both deleted afterwards.
- **Today (polish W2):** Questions ask ("Delete this repeating event?", "Move this repeating event?") with Cancel first and the default last (MB-027). Delete's answers say what they delete ("Delete This Event", "Delete This and Following", "Delete All Events") in red; ↵ still takes "Delete This Event", now a red button rather than the accent (MB-028, MB-Q11). Away from the panel, the delete question, "Save this event?" (Discard in red) and the guest questions show near the top over a dimmed calendar, and nothing else answers until they're answered (MB-041, MB-Q7); a drop's move question stays at the bottom of the grid. Undo of a "This and following" change is titled "Change Event" (`Removal::undo_title()`; the app still says "Delete Event" until W2-SHELL reads it).
- **Needs:** A tester pass (T2c), including the split for changes and drags. CalDAV: `RECURRENCE-ID` overrides or `EXDATE`; a split there is an `UNTIL` on the old series plus a new `VEVENT`.
- **Notes:** Series edits don't reach occurrences changed on their own, as in Google Calendar. A split doesn't rewrite `BYDAY` when the occurrence moves to another weekday (neither does "All"). A Meet link from `conferenceData` isn't copied to the new series.

#### CAL-29 Set and edit repeat rules

- **Today (W5):** A Repeat row in the panel (`⇧R`, ⌘K "Repeat…", tab order after All day): "Does not repeat", every day, every weekday, weekly on the day, monthly on the nth (or last) weekday, monthly on the date, yearly, and "Custom…" (every N days/weeks/months/years, weekdays, monthly by date or weekday, ends never, on a date or after N times). `ui/calendar/repeat.rs` only makes rules the expander supports. Rules show in words with their end ("Every week on Tue, 10 times"), and custom RRULEs from Google are described. On a series, a repeat change offers only "This and following" or "All events". The write replaces the whole RRULE (only an end change keeps Google's RRULE with its end rewritten); EXDATE and RDATE lines stay; "Does not repeat" removes it and drops the occurrences changed on their own. "This and following" with a new rule (or none) starts the new series with it, and the panel, now on the new series, asks again at its next change; it's one op (`CalOp::Split`) that inserts the new series only after the old one was shortened. An event changed in the app and not yet committed keeps the app's copy: a sync or a commit doesn't replace it while the change settles, waits for its scope or guests answer, or sits in the outbox (`Outbox::queued_events`), so a change to the new series right after a split goes after the split, to its id, and isn't undone by the split's sync. Not offered on an occurrence changed on its own.
- **Today (polish W2):** The custom editor uses the shared controls (MB-030): unit and end are segmented controls, weekdays seven equal checkboxes named in full, monthly "On day 27" / "On the 4th Tue" built from the rules themselves (MC-057). Every N and the count are fields you can type in (Enter or leaving applies, clamped to 1–99 and 1–730), with −/+ (Shift-click steps by 10) and ↑/↓ (MB-031). Buttons read [Cancel] [Apply] (MB-027, ME-098).
- **Needs:** Done; passed on the real account (sprint 2 T1c, after fix-4).

### Invites and RSVP

#### CAL-13 Guests and invitations

- **Today:** The panel lists organizer and guests with RSVP icons and a summary ("3 people · 2 yes · 1 awaiting"). "Add guests" suggests contacts (CON-1); `enter` adds, `backspace` on empty removes the last. `render_guests()`, `update_guest_suggestions()`, `add_guest()`, `remove_guest()`. Nothing is sent and the user isn't asked.
- **Today (writes, W7):** Guest changes save `attendees`, keeping Google's entries for the guests who stay (rooms included, though not shown). When a save or delete touches an event with guests other than the user, a card over the grid asks "Send updates to guests?" (or "Send invitations…", "Send cancellation…"): Send (enter) → `sendUpdates=all`, Don't send → `none`, Cancel (esc) puts the event back (`GuestPrompt`, `answer_guests()`). Changes only to the user's reminder or busy/free don't ask (`asks_guests()`). A new question while one is open waits for it (polish W2, MB-041; it used to answer the old one "Don't send" silently). The buttons read Cancel, Don't Send, Send ↵.
- **Needs:** A tester pass on the real account (T2c); whether invites arrive at the second address is checked by the user later (6.4 item 6). `externalOnly` isn't offered. CalDAV: iMIP. Free/busy for guests: CAL-27.
- **Notes:** With `attendeesOmitted` the guest list is never written.

#### CAL-14 My RSVP

- **Today:** For others' events the panel shows "Going? Yes / Maybe / No". Unanswered invites are outlined; declined ones struck through. `set_rsvp()`, `Event::my_status()`, `Look` (`week_view.rs`).
- **Today (writes, W7):** The last answer goes after 1.5 s (or when the panel leaves the event) as an `events.patch` of `attendees` where only the user's own entry (`self`) changes `responseStatus`, with `sendUpdates=all` so the organizer is told.
- **Today (polish W2, grid):** ⇧Y / ⇧M / ⇧N answer the selected invite (or the open one) through `set_rsvp()`, and ⌘K offers Accept Invitation, Reply Maybe and Decline Invitation when it's an invite (`When::CalendarInvite`; ME-034). `Event::my_status()` now follows Google's `organizer.self` too, and `policy::my_answer()` reads it, so the two can't disagree (MC-Q18).
- **Needs:** A tester pass with a real invitation (T3b item 3).
- **Today (polish W2):** The RSVP row moved to the panel footer, always in view, highlighted until answered (ME-035, ME-Q7); it's the shared segmented control (a radio group, nothing chosen while unanswered) with the keys in its tooltip (⇧Y / ⇧M / ⇧N, ME-034).
- **Notes:** Answering for one occurrence of a series isn't supported (CAL-6).

#### CAL-20 Invite emails linked to events

- **Today:** Nothing. The invite card above invitation emails (title, time, conflicts timeline, Yes / Maybe / No, Open in Calendar, Remove from calendar for cancellations, and their `shift-y` / `shift-m` / `shift-n` / `shift-c` / `shift-x` shortcuts) was removed on 27 Oct 2026, with `Message::invite`, the iMIP UID/METHOD parsing, the `iCalUID` lookup and the `bodies.invite_*` columns (dropped in migration 180). Invitations are answered from the calendar (CAL-14); a named `.ics` shows as an ordinary attachment.
- **Needs:** A rethought card. It would need the `text/calendar` part's UID and METHOD (and its details, for an event not in the calendar) parsed and stored with the body, resolution to the stored event by `iCalUID` (an index on `events (account, ical_uid)`), a lookup for unstored ones with `events.list iCalUID=` on the primary calendar, RSVP through `set_rsvp()` / `CalOp::Rsvp`, removing a cancelled event's own copy with `sendUpdates=none`, and for non-Google invites `events.import` (iMIP `REQUEST`, `CANCEL`, `REPLY`).

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

- **Needs:** `freebusy.query` (`timeMin`, `timeMax`, `items` of calendar IDs or guest emails) to show guests' busy blocks in the panel or grid and suggest a free slot; include shared calendars in conflict checks.

### Settings

#### CAL-42 Open the location in Maps

- **Today (polish W2, MB-046):** Every non-empty location opens (MB-Q18): an http(s) address as itself, anything else as an Apple Maps search (`https://maps.apple.com/?q=…`); a location that holds a video call link doesn't (Join opens it). `location_target()` in `event_panel.rs`. In a read-only panel the location is a link; in an editable one a map button sits beside the field. ⌘K "Open Location in Maps" (`OpenLocation`) works from the panel; its row stays gated until the calendar root handles it.
- **Needs:** Nothing: it uses the stored `location`.

#### CAL-43 Event context menu

- **Today (polish W2):** Right-click or Control-click on a block, bar, Month item or Day list row selects the event (the panel follows when open) and opens a native menu (MB-010): Open; Accept / Maybe / Decline with a check on the current answer, for an invite; Join Meeting and Open Location in Maps when there's a link or a place; Add Google Meet (without a call), Repeat…, Add Reminder… when the user can edit; Copy Link to Event and Open in Google Calendar when the event has an `htmlLink`; Delete ("Delete…" for a series, "Decline and Remove" for an invite). Each item dispatches the action its shortcut does (`CalendarView::event_menu()`); unavailable ones are left out. Right-click never starts a drag.
- **Today (polish W4):** Duplicate (CAL-45) when the user can edit the event.
- **Needs:** Nothing beyond the actions' own items (CAL-14, CAL-17, CAL-34, CAL-29, CAL-33, CAL-42, CAL-10, MB-061).

#### CAL-44 Nudge an event with the keyboard

- **Today (polish W4, MA-092):** ⌥↑ / ⌥↓ move the selected event 15 minutes earlier or later, ⌥← / ⌥→ a day back or forward (⌘K "Move Event 15 Minutes Earlier/Later", "Move Event to the Day Before/After"). It goes through the same path as a drag (`nudge()` → `retime_occurrence()`), so a series asks "Move this repeating event?" and guests are asked about updates. Read-only events say why.
- **Needs:** A tester pass; then the same `events.patch` as CAL-21.

#### CAL-45 Duplicate an event

- **Today (polish W4, ME-097):** ⌘K "Duplicate Event", the panel's "…" menu or the context menu copy the selected event at the same time on its calendar (one occurrence of a series becomes a plain event, without the call or Google link, the user as organizer), select it and open it in the panel. `CalendarView::copy_event()`. No shortcut of its own.
- **Needs:** A tester pass; it's an `events.insert` like CAL-9, sent with `sendUpdates=none`, so copied guests aren't invited.

#### CAL-46 Email guests

- **Today (polish W4, ME-097):** ⌘K "Email Guests" or the panel's "…" menu opens a new message in Pigeon's composer to the organizer and every guest but the user, with the event's title as the subject (a `mailto:` through `app::shell::open_urls()`, as links from other apps); with no one else on the event it says so in a toast.
- **Needs:** A tester pass; sending is CMP-8.

#### CAL-41 Calendar settings from Google

- **Today (W5):** `settings.list` is read at most every 6 hours during sync (`GoogleSettings`, kept in sync state; errors only logged): `weekStart` sets the week's first day, `defaultEventLength` a new event's length, `autoAddHangouts` Meet on new events (CAL-34), `remindOnRespondedEventsOnly` the reminders (CAL-33), `timezone` the zone notice (CAL-12). Settings → Calendar has "Default event length", which follows Google until changed in Pigeon (`Saved::event_minutes`). The clock no longer follows Google's `format24HourTime` (no longer read): it's the app-wide Settings → General → Time format (System / 12-hour / 24-hour, see [§8.4](#84-features-with-no-backend-need)); an old saved `time_24h` migrates to it.
- **Needs:** Done; passed on the real account (sprint 2 T1c). Not used: `settings.watch`; `format24HourTime`, `hideWeekends`, `showDeclinedEvents`, `hideInvitations`, `locale`, `dateFieldOrder`, `useKeyboardShortcuts` aren't used.

---

## 7. Sync, notifications and states

### 7.1 Sync and push (SYN)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| SYN-1 | Gmail full sync | 🟢 Fully linked | `threads.list` / `messages.list` + batched `get`; store `historyId` |
| SYN-2 | Gmail partial sync | 🟢 Fully linked | `history.list` (`startHistoryId`, `historyTypes`) |
| SYN-3 | Gmail push | ⚪ Not done | `watch`, `stop`, Cloud Pub/Sub |
| SYN-4 | Calendar incremental sync | 🟢 Fully linked | `events.list` / `calendarList.list` with `syncToken` |
| SYN-5 | Calendar push | ⚪ Not done | `events.watch`, `calendarList.watch`, `acl.watch`, `settings.watch`, `channels.stop` |
| SYN-6 | Contacts sync | 🟢 Fully linked | `people.connections.list`, `otherContacts.list` with `syncToken` |
| SYN-7 | Batching, quotas and backoff | 🟢 Fully linked | HTTP batch; `429` / `403 rateLimitExceeded` |

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

- **Today:** `backend::calendar::sync_pass(transport, store, account)` (W4) does one pass, blocking, off the main thread: `colors.get` once (cached), `calendarList.list` (full with `showHidden`, then by sync token; calendars gone from a full list or marked `deleted` lose their events), then per calendar `events.list singleEvents=false maxResults=2500 timeZone=<local>` with `timeMin=now−180d` on a full sync, paged to the end, storing `nextSyncToken` in `calendars.sync_token`; later passes send only `syncToken`. Cancelled items delete the event (and its instances) or, for an instance, become an exception of its series. A 410 wipes that calendar's events and syncs it in full again; a full listing replaces what was stored. With two or more calendars holding a sync token, their first token pages go in one HTTP batch to `batch/calendar/v3` (`first_pages()`, polish W1 MC-013); paging, 410 and per-calendar errors work as before, and a calendar whose part didn't come lists on its own. Touched ids are split into changed and removed with one indexed query (`Store::existing_event_ids()`, MC-002). A calendar Google refuses (403/404) is skipped and named in `CalendarChanges::errors`; a network failure stops the pass with an error. It returns `CalendarChanges { calendars, changed, removed }`, and `backend_events()` turns them into `CalendarsChanged` / `EventsChanged` / `EventsRemoved`; the app reloads just those ids in place (an event shared into several calendars is replaced copy by copy), so the view's date and selection stay.
- **Needs:** The scheduler (W3's `sync.rs`) calling it every 2 min while the window is active, on focus, and ~2 s after a committed calendar write, then emitting `backend_events()` (merge). Status line wiring for errors (ACC-5).

#### SYN-5 Calendar push

- **Needs:** `events.watch` (and `calendarList`, `acl`, `settings` watch) to an HTTPS webhook; channels expire and must be renewed; `channels.stop` to cancel. Each notification triggers SYN-4.
- **Notes:** Needs a public HTTPS server; a desktop-only backend polls SYN-4 instead.

#### SYN-6 Contacts sync

- **Today:** `src/backend/people.rs`: `people.connections.list personFields=names,emailAddresses` and `otherContacts.list readMask=names,emailAddresses`, both `requestSyncToken=true`, paged by 1,000, then by `syncToken` (deleted people go, changed ones are replaced). An expired token (400 `EXPIRED_SYNC_TOKEN` or 410) lists everything again. Stored per address in `contacts` with the tokens in `sync_state` (`people_sync_token`, `other_contacts_sync_token`), in one transaction (`Store::put_people()`). Run on launch, then every 30 minutes after a mail pass (`sync_contacts()` in `src/backend/sync.rs`); each list only when its scope (`contacts.readonly`, `contacts.other.readonly`) was granted. Emits `ContactsChanged` when anything changed. Read-only.
- **Needs:** Nothing more for this sprint.

#### SYN-7 Batching, quotas and backoff

- **Today:** W2, in `src/backend/http.rs`. `GoogleTransport` (what `Backend::transport()` hands sync) adds the access token, waits on a token-bucket limiter in Gmail quota units (6,000/min, 1,000 kept for user actions: `send_user()`; every `Request` declares `cost`), keeps at most 4 calls in flight, and retries network errors, 429, 5xx and 403 `rateLimitExceeded`/`userRateLimitExceeded` with truncated exponential backoff (1, 2, 4 … 64 s plus up to 1 s jitter, 8 tries). `UreqTransport` logs each call as `METHOD host/path status ms`, query redacted. HTTP batch encode/decode is from W1; the decoder splits on the boundary's bytes (CRLF or LF) and copies each part once (polish W1 MC-015). The User-Agent says `Pigeon (gzip)`, so Google gzips its answers (MC-011); ureq keeps 4 idle connections per host, as many as are in flight (MC-022); the access token goes on as a header without copying the request (`Transport::send_authorized()`, MC-014). `http::batch()` sends any API's batch with the part retries (Gmail and Calendar, MC-013). `Gmail::batch` (`gmail.rs`) sends at most 20 calls per batch (Gmail runs a batch's calls in parallel and answers 429 "Too many concurrent requests for user" at 50), one batch at a time across sync and body fetches, and retries just the calls a batch answered with 429, 5xx or a rate-limit 403, on the same backoff schedule; other per-call errors (404 etc.) are answered as before.
- **Needs:** W3/W5 to batch their gets and modifies; `fields=` partial responses; a "Waiting for Gmail…" status during long backoffs (2.9). Group calls into HTTP batch requests (up to 100 calls; Gmail advises at most 50) for list-then-get and bulk actions (ACT-6). Respect per-user rate limits with exponential backoff on `429` and `403 rateLimitExceeded`/`userRateLimitExceeded`. Ask for partial responses (`fields`) to cut payloads.

### 7.2 Notifications and background (NTF)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| NTF-1 | New mail notifications | 🟢 Fully linked | From SYN-2 / SYN-3 |
| NTF-2 | Unread badge | 🟢 Fully linked | `labels.get` (`INBOX`) |
| NTF-3 | Background sync when the window is closed | 🟢 Fully linked | SYN loop |
| NTF-6 | Open at login | 🟡 Partially done | None |
| NTF-4 | Menu bar panel | 🟡 Partially done | None (reads the cache) |
| NTF-5 | Quick join | 🟡 Partially done | None |

#### NTF-1 New mail notifications

- **Today (sprint 2 W1):** gpui's own API (`cx.show_system_notification`, over `UNUserNotificationCenter`; it posts at once and reports no delivery result). `src/notify.rs` owns `notify(Notice, cx)`, `dismiss(tag, cx)` and the click routing (`on_system_notification_response` → `shell::show`, also for notices from before a relaunch, by tag); W5 and W7 use it. Every new Inbox mail, any category: the partial sync carries history's `messagesAdded` ids through the metadata fetch to its last step (`Step::SetHistory { history, added }`, `src/backend/sync.rs`), where each stored message is judged on its current labels and `internalDate` (`backend/new_mail.rs` `eligible()`: INBOX + UNREAD, not SPAM/TRASH/DRAFT, received by `gmail::received()` so self-mail and aliases count, within 15 min of the poll). Notified ids go in the store's `notified` table (migration 100, last 500 per account) in the same transaction as the history id, so a retry, crash, relaunch or offline gap neither repeats nor drops one. A full sync or a history reset never notifies. One notice per conversation (`mail-<thread id>`; title the sender, body subject and snippet); more than 3 at once become one summary under `mail-summary`, replaced by the next. Nothing for a conversation the frontmost main window shows: in Mail, Settings closed, open or in the current view's list (`in_view()` in `src/app/shell.rs`, polish W2 MB-004); other views, the window in the background or closed, and the menu bar panel still notify. A notice goes (by tag) once its conversations are read or out of the Inbox anywhere, seen in the next sync. Clicking opens the thread (the window opens if closed). Settings › Notifications: "New mail notifications" (on by default, saved as `notifications`) and "Send test notification", which also brings up macOS's permission prompt; ⌘K "Send Test Notification", "Turn New Mail Notifications On / Off". Each notice submitted is logged as `notify: tag=… title=…` in `<data dir>/pigeon.log`; clicks as `notify: clicked …`, dismissals as `notify: dismissed …`. In test-instance mode (`PIGEON_DEV=1`) tags start with `dev-`. Notifications only work when Pigeon runs from its bundle (PER-4). Polish W3: Settings › Notifications also has "Play sound" (`Saved::notification_sound`, on by default) and "Event reminders" (`Saved::event_reminders`, on by default), and the test button reads "Send Test".
- **Today (polish W3, MA-073, MB-050, MB-053, MB-054, MB-Q13):** Through a patch to the vendored gpui-pre-macos (`@` options on `SystemNotification::actions`, `vendor/gpui-pre-macos/PIGEON_PATCHES.md` item 3): a single new Primary message and a reminder play the default sound when "Play sound" is on (summaries never do); mail outside Primary comes at the Passive level, quietly (`notify::Kind::Mail { primary }`, read from the store's labels when the mail arrives; unknown counts as Primary); mail groups under `threadIdentifier` "mail", reminders "events", Pigeon's own notices "pigeon"; with previews hidden they read "New mail" / "Event reminder". Category ids are named after their actions, so a notice from before a relaunch keeps its buttons. A single-conversation notice has Mark as Read and Archive, done in place with the menu bar panel's ops (`ui::menu_bar::archive_thread` / `mark_thread_read`) without opening the window, also with it closed; the notice goes after. Summary title "5 New Messages"; the test notice reads "Test Notification" / "New mail and event reminders show up like this." Permission: nothing is posted while macOS hasn't been asked (`notify::Permission`, read with `getNotificationSettings` at launch and after asking; logged `notify: tag=… not shown (Not asked yet)`); it's asked from a card after the first sync (or with demo data) with Turn On / Not Now (`ui::status::notifications_card`; "Not Now" is kept in `NSUserDefaults`), when "New mail notifications" is turned on in Settings, or by Send Test, which then posts once allowed. A sign-in that expires while no Pigeon window is in front posts one "Sign In Again" notice that opens Settings › Accounts, taken back once signed in (`notify::SIGN_IN_TAG`, from `app::today`). Polish W3 fix step: the card sits under the sidebar's status line; Settings › Notifications › "macOS permission" says what macOS has (read again when the section opens), with "Turn On" while not asked and "Open System Settings" when off; notice actions reach the backend through the shell, so they work before the menu bar model exists at relaunch.
- **Needs:** Done; submission passed on the real account (sprint 2 T1a). The user allows notifications once. Polish W3 needs a user check in the bundled app: the sound, Passive level, grouping, Mark as Read / Archive / Snooze with the window closed (also right after a relaunch), the permission card under the sidebar's status line, and Settings › Notifications › "macOS permission". Respect Focus (macOS does); per-label mute if added. W4a's categories don't change which mail notifies (decision 4).

#### NTF-2 Unread badge

- **Today (sprint 2 W1):** The Dock badge is the Inbox's unread conversation count from Gmail's `labels.get INBOX` (`Backend::category_counts()` since W4a; counted from the store on the placeholder data), set through `NSApp.dockTile.setBadgeLabel` (`native::set_dock_badge()`), none at zero. Updated app-wide on `CountsChanged`, thread changes and committed ops, with the window closed too (`app/shell.rs`); each change is logged as `badge: N`. Testers read it with `lsappinfo info -long <ASN> | grep StatusLabel`. Polish W3: Settings › Notifications › "Dock badge" (Off / Primary / All Inbox, `Saved::dock_badge`, default Primary; MB-055). All Inbox sums every category's unread count (`update_badge()`, `src/app/shell.rs`), Off clears the badge, and `shell::set_dock_badge()` re-badges at once when the setting changes. While Google's sign-in has expired the badge is cleared (MB-054), and it comes back once synced.
- **Needs:** Done; passed (sprint 2 T1a, T2a). It counts Primary unread in the Inbox since W4a.

#### NTF-3 Background sync when the window is closed

- **Today (sprint 2 W1):** Closing the window leaves Pigeon running in the Dock (gpui's macOS default; the quit on last window closed is gone); ⌘Q quits and flushes the outbox as before, from any state. `src/app/shell.rs`: `ensure_main_window(cx)` is the one way to the main window and `show(Show, cx)` the one way to put something in it (`Main`, `Thread`, `Event`, `Compose(Option<Mailto>)`, `NewEvent`, `Settings`), used by the Dock (`on_reopen`), notification clicks, a second launch, `mailto:`, the menu bar panel and the app-level handlers. `show` calls the views directly rather than re-dispatching actions (an unhandled action would fall back to the app-level handler and loop), and when the main window is mid-update (an action from it reached an app-level handler) it runs once that update is done, instead of taking the window for closed and opening another (fix for `n` in the menu bar panel hanging Pigeon). App-level handlers run Compose, New Event, Open Thread, Settings, Sync Now and the W1 commands with no window open; ⌘O / Window › Pigeon (`ShowMainWindow`) reopens it. Settings are app-owned (`settings::AppSettings`, a gpui global every save goes through), so a reopened window has the latest appearance, layout and preferences. Closing the window saves an open composer to Drafts, writes the settings and drops the window's state and timers (`PigeonApp::before_close()`). Sync goes on with the window closed: Gmail every 60 s and Calendar every 2 min while notifications are on (5 min otherwise), and at once on wake (gpui's `cx.on_system_wake` since polish W1, logged `wake: syncing now`).
- **Needs:** Done; passed (sprint 2 T1a, T1a-2). ⌘O with no window open can't be driven without the Dock; a user check. Calendar reminders now fire app-wide through `notify` (CAL-15, W7b).

#### NTF-6 Open at login

- **Today (sprint 2 W1):** Settings › General › Open at login registers `SMAppService.mainApp` through small runtime bindings to ServiceManagement (`native::login_item()`, `set_login_item()`; the framework is linked, no crate). Off by default; only offered when Pigeon runs from `/Applications` and not in test-instance mode, otherwise the row says why. The "Login item" row shows macOS's real status: on, off, needs approval in System Settings › General › Login Items (which turning it on opens), or the error macOS gave. ⌘K "Turn Open at Login On / Off".
- **Needs:** A user check: turning Open at login on in the installed copy and logging in again.

#### NTF-4 Menu bar panel

- **Today (sprint 2 W7b):** A status item with Pigeon's bird (since polish W3 Pigeon's own mark from the bundle, `MenuBarIcon.png`, MA-056; SF Symbol `bird` under `cargo run`; a template image, so it follows the menu bar's light or dark), with a dot while there's unread Primary mail newer than what the panel showed when last opened. Beside it, by default, the next meeting from `policy::next_joinable`: "Design review in 12m" from three hours before, "Join · Design review" while it runs, the title cut at 20 characters; redrawn at minute boundaries only. Clicking it (on mouse down, on the screen clicked) opens a panel under it, design A "Agenda first" (`docs/design/menubar.html`, `src/ui/menu_bar.rs`): the date and sync status; a card for the next meeting (countdown chip, time, title, video service and guests, Join `j`, Details opens it in the calendar); "Later today" (time, calendar colour, title, location; all-day events first; up to 6, then "N more today"); tomorrow's first event when nothing timed is left today; "Recent mail", the 4 newest Primary Inbox conversations (unread dot, sender, subject, time); a footer with Compose `c`, Event `n`, Open Pigeon ⌘O and a gear whose menu has "Settings…" (⌘,, opens Settings in the main window) and "Quit Pigeon" ⌘Q (polish W3, ME-072). ↑/↓ move through the card, events and mail; Enter opens the selected one in the main window (opening it if closed); `e` archives and ⇧I marks read the selected conversation with the same outbox ops as the main app (archive waits out the undo window; "Undo `z`" in the panel cancels it, or moves it back once it went); Esc, a click elsewhere, or the item again close it. The panel grows with its content up to 640 pt and scrolls past that. All of it from an app-level model (`src/app/today.rs`), reloaded from the store when the backend reports changes, so it works with the window closed: `Backend::load_inbox_heads()` reads only the Inbox threads (plus those a queued change touches) without bodies, off the main thread (polish W1 MC-009). Events and calendars reload by what changed (`EventsChanged` / `EventsRemoved` ids, `Backend::load_calendars()`); the whole calendar only after sign-in or a demo commit, not after every mail op (polish W2, MC-010). Native side in `src/native/menu_bar.rs`: one gpui pop-up window (a non-activating `NSPanel` on every Space and over full-screen apps), opened hidden and made key once at launch while see-through (so the first open renders as fast as the rest), then only shown and hidden natively: it fades in and slides down 6 pt (since polish W4: over 180 ms on `EASE_PANEL`, MD-031) and fades out over 100 ms with no slide (polish W4, MD-030; it counts as closed from the start, a reopen mid-fade takes it back at full opacity; instant with Reduce Motion and when the main window opens what was picked; resizes stay instant), and it can become key but never the main window, so a Pigeon window that was main keeps its active traffic lights while the panel is up; click-away through a global mouse monitor and losing key, with a 300 ms guard so the click that closed it doesn't reopen it. ⌃⌥⌘Space (Settings › Menu Bar: ⌃⌥⌘Space, ⌃⌥⌘P or off) opens it from any app, under the item when it's on the pointer's screen, else at the top of that screen, right of centre; Carbon `RegisterEventHotKey`, so no Accessibility permission. Settings › Menu Bar: show menu bar icon, show next meeting, the two shortcuts and whether macOS gave them (a taken one says so). ⌘K "Open Menu Bar Panel". Logged: `menu-bar: <source> → ordered front in X ms` and `menu-bar: shown in X ms`. In test-instance mode (`PIGEON_DEV=1`) no global shortcuts unless `PIGEON_HOTKEY=1`; `PIGEON_MENU_BAR_SELFTEST=1` there opens and closes it three times at launch. Measured on the placeholder data (release): 2–4 ms from open to rendered, the first open included. Polish W3: the keyboard's row and the selected card use `selected_bg`, as in every overlay (ME-080); Join Meeting and Details have hover and pressed states (ME-081); the countdown chip has tabular figures (MD-033); the lists show a scroll bar (MB-068); section headers are no longer upper case; today's agenda is computed once per render and the keys read it from there (MC-042); `archive_thread` / `mark_thread_read` are shared with mail notices (MB-050). Polish W4 (MD-032): the "No meeting to join…" notice and the undo bar rise in and fade out on presence springs (`SPRING_POP` / `SPRING_EXIT`, a fade under Reduce Motion); the window keeps its height until a leaving bar is gone, then shrinks at once.
- **Needs:** The panel's layout, keys and actions passed through ⌘K (sprint 2 T1d, after fix-3). User checks left: the status item itself and its dot, clicking it on each screen and over a full-screen app, click-away, the global shortcut, the open animation, and that a Pigeon window stays active-looking under the panel.

#### NTF-5 Quick join

- **Today (sprint 2 W7b):** ⌃⌥⌘J (Settings › Menu Bar: ⌃⌥⌘J, ⌃⌥⌘M or off) and ⌘K "Join Next Meeting" open the video link (Meet, Zoom, Teams, Webex; CAL-17) of the meeting under way or starting within 10 minutes (`today::quick_join()`); with none, the panel opens and says "No meeting to join in the next 10 minutes." The panel's `j` joins its card's meeting. In test-instance mode every join is a dry run that only logs `quick-join: <url>`.
- **Needs:** The join dry run passed (sprint 2 T1d). User checks left: the global shortcut and actually opening the link.

### 7.3 Loading, error and offline states (STA)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| STA-1 | Initial load and loading states | 🟢 Fully linked | None |
| STA-2 | Action failure and rollback | 🟢 Fully linked | API error responses |
| STA-3 | Offline mode and queued changes | 🟢 Fully linked | None |
| STA-4 | Expired sign-in | 🟢 Fully linked | OAuth `invalid_grant`, 401 |
| STA-5 | Conflicts with server changes | 🟢 Fully linked | ETags, `historyId`, `syncToken` |

#### STA-1 Initial load and loading states

- **Today:** Launch reads the cache (`Backend::load_mail()`) and never waits on the network. List: skeleton rows while a view's first page loads, "Couldn't load <view>" if the first sync failed, the empty text only once loaded (`view_loaded()`, W3). Reader: "Loading…" per message, "Couldn't load this message" with Retry. The sidebar status line is W2's.
- **Today (W11):** The status line shows Offline and "Waiting for Gmail…" over sync's state (ACC-5, STA-3).
- **Needs:** A tester pass (T3b items 1–2): an unloaded body while offline reads "isn't downloaded yet", Retry after reconnecting.

#### STA-2 Action failure and rollback

- **Today:** Every action (W5) changes `data` at once and queues an op. When Google refuses one (anything but 401 or no network), the op fails for good: thread actions apply what went to the store and read the refused threads again with `threads.get` (`outbox::commit_threads()`); sends go back to Drafts (CMP-12); calendar writes reload Google's version of the event, into the open event panel too (its title, place, link, notes and times, unless the user changed the event again since; `CalendarView::events_reloaded()`). The backend emits what changed (`ThreadsChanged`, `EventsChanged`: the app reloads them, the rollback) and then `OpFailed`, whose toast reads e.g. "Couldn't archive 2 conversations: Invalid label". Retryable errors (429, 5xx, rate-limit 403) back off first (transport and `Gmail::batch`, 8 tries).
- **Today (W11):** `OpFailed` shows an error toast (`PigeonApp::show_error_toast()`): the danger tone from `theme.rs` with an alert icon, no Undo, `Role::Alert`.
- **Today (polish W1, ME-056, MA-050):** Toasts have a kind (`ToastKind`: Done with a check, Info with no icon, Working, Error; `src/app/toast.rs`). Every toast holds while the pointer is on it. Errors stay until dismissed (✕, Escape, or the next toast) and can carry a fix: a failed send offers "Open Drafts", a failed attachment download "Try Again". File and network errors read as plain sentences with a next step (`toast::plain_error()`), the raw error goes to `pigeon.log`. A new toast never loses an undo: it stays in the history (UND-4). Polish W4 (MD-008): toasts rise and fade in and fade out (`presence("toast")`); the leaving one is a ghost that takes no clicks.
- **Today (failure drills, sprint 2 W6, dev only):** With `PIGEON_DEV=1`, a file in the data folder makes the **next** Google write fail once and is then deleted: `simulate-refusal` answers 400 (`"Simulated refusal (drill)"`, Gmail's error shape), `simulate-conflict` answers 412 (`Precondition Failed`, as for a stale ETag). Only writes (POST, PUT, PATCH, DELETE, or a batch holding one) to `*.googleapis.com`, never reads, read-only batches or the sign-in's token calls. Each is logged as `drill: <file>: <call>` in `pigeon.log`. Inert without `PIGEON_DEV=1` (`drill()` in `src/backend/http.rs`, in `UreqTransport::send()` after the offline switch). Results, covered by tests: a thread action rolls back and toasts "Couldn't archive the conversation: Simulated refusal (drill)" (a batch: "Couldn't delete 2 conversations: Precondition Failed"); a send goes back to Drafts with "Not sent: …"; a calendar write reloads Google's version with "Google Calendar didn't take the change: Simulated refusal (drill)", or "Changed elsewhere; reloaded" for 412. No gaps found in the outbox's handling.
- **Needs:** The rollback keeps the list position only as far as `update_threads()` does.

#### STA-3 Offline mode and queued changes

- **Today (indicator, W11):** Offline shows within about 10 s of the network going away: a probe (TCP connect to `gmail.googleapis.com:443`, no request) runs every 8 s while the window is active (60 s in the background); two failures in a row are Offline, as are two network failures in a row in the transport. Offline, probes back off 15 s → 2 min; the first that connects, or a sync that succeeds, ends it and runs a sync and the queue at once. Once the transport is offline, a network error fails at once instead of retrying 8 times (`GoogleTransport::send_with()`); polls do the retrying. The line reads "Offline · N changes waiting" from the outbox's count, recounted when a change is queued.
- **Today (dev switch, 6.4 item 6):** With `PIGEON_DEV=1`, every request fails as a network error while `<data dir>/simulate-offline` exists (`touch` it / `rm` it; `UreqTransport::send()` in `src/backend/http.rs`, and the probe). Never active without `PIGEON_DEV=1`.
- **Today (queue, W5):** Any op that can't reach Google (a network error, or 401 / an expired sign-in), whether a thread action, a draft save or delete, a send or a calendar write, stays queued (`Outbox::retry_later()`, again after 15 s, `Outcome::Later`) with nothing rolled back and no error toast; the status goes Offline or "Sign in again". Thread changes still show as made through the overlay; a send stays out of Drafts and its "Message sent" toast comes when it goes; a deleted Gmail draft stays in the store until `drafts.delete` goes. Nothing goes while the sign-in needs renewing (STA-4).
- **Today (polish W2, MA-068):** The rail can show the status as one glyph above Settings, so it's visible in Calendar and with the sidebar hidden: syncing (or waiting for Gmail), offline (warning tint) and "Sign in again" (a button), with the status line's words in its tooltip; nothing while synced (`rail::render_with_status`).
- **Needs:** A tester pass of offline actions, sends and calendar writes going once back online (T4). Reading from the cache (PER-1); the pending items listed, not only counted.

#### STA-4 Expired sign-in

- **Today:** W2. `invalid_grant` on refresh marks the sign-in expired: requests fail with `TransportError::AuthExpired`, the status line shows a "Sign In Again" button (also on the rail while the sidebar is hidden or in Calendar, MA-068), Calendar without its scope shows "Calendar access wasn't granted" with a "Sign In Again" button (ME-058, `status::missing_scope()`), and signing in again (same browser flow) keeps the cache and the outbox. A missing `google_client.json` with a stored account shows the same state. Polish W3: with the window open, the change to expired shows the error toast "Google signed Pigeon out" with "Sign In Again" (`on_status`, `src/app/backend_events.rs`); with it in the background, one notice (NTF-1).
- **Needs:** Detect auth errors on every call, refresh (ACC-7), and if refresh fails (`invalid_grant`) show "sign in again" while keeping local changes and the outbox.

#### STA-5 Conflicts with server changes

- **Today:** A sync that changes the open thread reloads it in place, keeping the selection (W3); drafts open in the composer aren't replaced. W11: the reader's expanded and focused messages are kept by message id across the reload (`open_messages()` / `restore_open_messages()` in `src/app/backend_events.rs`), so a new message doesn't shift them, and the message that was last stays open when a newer one arrives.
- **Today (events, W7):** Event patches carry `If-Match`; a 412 reloads Google's version into the store and the view, drops the op, and shows "Changed elsewhere; reloaded" (checked on the real account with a stale ETag).
- **Today (drill, sprint 2 W6):** With `PIGEON_DEV=1`, `touch <data dir>/simulate-conflict` makes the next Google write answer 412 once, so the conflict path can be checked on real data without a second client (see STA-2). Gmail thread actions have no ETags; a 412 there is handled as any refusal (rollback and toast).
- **Needs:** Rules for drafts changed elsewhere. Links to DRF-6, LST-8.

---

## 8. App-wide

### 8.1 Storage and persistence (PER)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| PER-1 | Local mailbox store | 🟢 Fully linked | Holds SYN data |
| PER-2 | Outbox and drafts that survive quit | 🟢 Fully linked | None |
| PER-3 | Persisted UI state and settings | 🟢 Fully linked | None |
| PER-4 | App bundle and install | 🟢 Fully linked | None |
| PER-5 | Single instance | 🟢 Fully linked | None |

#### PER-4 App bundle and install

- **Today (sprint 2 W1):** `scripts/bundle.sh` builds `Pigeon.app` in the target dir (`release/Pigeon.app`): `packaging/Info.plist` (bundle id `com.pigeon.dev`, `mailto:` URL type, not `LSUIElement`), a placeholder icon drawn by `packaging/make_icon.py` (standard library, then `sips` and `iconutil`), signed with the "Pigeon Dev" identity like before (ad hoc, with a warning, without it). `scripts/run.sh` builds the bundle and runs its binary directly, so the environment and terminal output pass through and notifications work in dev. `scripts/install.sh` quits the installed copy, copies the bundle to `/Applications` and registers it with Launch Services. Logs go to `<data dir>/pigeon.log` (`src/logging.rs`): stderr when launched from the Dock, plus gpui's `log` warnings and notification lines; capped at 2 MB, then moved to `pigeon.log.1`.
- **Today (polish W3, MA-056, MA-057, MB-008, MB-063):** A custom bird mark replaces the placeholder envelope: a white bird on a coral squircle, away from Mail's blue. `packaging/make_icon.py` draws it from signed distances (standard library), renders 1024 px and scales down with `sips`, drawing 16–64 px again at their own size (the wing and eye left out under 24 px), and writes into `<target>/icon/`: `Pigeon.icns`, `AppIcon.png` (256 px, on the sign-in screen), and the menu bar's template `MenuBarIcon.png` / `@2x` (the same bird, black on clear), all copied into the bundle's Resources by `scripts/bundle.sh`. No layered `.icon` / Assets.car: `actool` needs Xcode, which isn't installed (Command Line Tools only), so macOS 26 shows the flat 1024-sourced icon. `Info.plist` also declares the `pigeon:` URL scheme (MB-063; routing is `src/app/shell.rs`).
- **Needs:** Done; passed (sprint 2 T1a). The new icon and menu bar mark need a look in the Dock, Finder and the menu bar (light, dark, tinted). A layered macOS 26 icon once Xcode's `actool` is available. The user installs after each merge; making Pigeon the default mail app is a user check (LNK-3).

#### PER-5 Single instance

- **Today (sprint 2 W1):** `src/instance.rs`: the first Pigeon holds a `flock` on `<data dir>/pigeon.lock` with its pid in it; a second launch on the same data folder posts a distributed notification (object: the data folder) and activates the first, which shows its window, then exits (`native::hand_over()`; logged `instance: …`). Another data folder (`PIGEON_DATA_DIR`) is another Pigeon. Test-instance mode (`PIGEON_DEV=1`) registers no login item, no global shortcuts (`instance::global_shortcuts_allowed()`, for W7b) and prefixes notification tags with `dev-`.
- **Needs:** Nothing from the server.

#### PER-1 Local mailbox store

- **Today:** SQLite (`rusqlite`, bundled) in `src/backend/store/`: `Store` (one connection behind a mutex, WAL, file 0600 in a 0700 folder), the full schema as migration block 001 in `schema.sql` (settings, labels, threads, messages, bodies, drafts, calendars, events, contacts, sync_state, outbox, images; account-scoped tables keyed by `account`), and reads and writes for mail (`store/mail.rs`), calendar (`store/calendar.rs`, events kept whole as JSON in `model`), contacts (`store/contacts.rs`) and the outbox (`store/outbox.rs`). Demo mode uses an in-memory store seeded from the JSON (`backend/demo.rs`); the UI's `MailData` and `CalendarData` are built from the store at launch (`Backend::load_mail()`, `load_calendar()`). Polish W1: thread and event reads by id filter in SQL (`json_each`), not in Rust (MC-001, MC-002); write loops use cached statements (cache of 64, MC-004); migration 190 indexes events by id and by series (MC-005); draft saves, discards, label and calendar deletes each run in one transaction, so an older draft save can't overwrite a newer one between check and write (MC-006); raw messages are parsed once per forward or draft (`mime::parse()`, MC-008); contact ranking uses hash maps (MC-007).
- **Needs:** Google mode on the file store at `~/Library/Application Support/Pigeon/pigeon.sqlite` (or `$PIGEON_DATA_DIR`) once W2 signs in; syncs writing to it (SYN-1/2/4/6); body eviction (300 MB LRU). The image cache (ATT-1) is in (W10, 64 MB). Later migrations go in their reserved blocks (`docs/BACKEND_SPRINT_1.md`, 4.2).

#### PER-2 Outbox and drafts that survive quit

- **Today:** Every `MailOp` and `CalOp` is a row in the store's `outbox` table (`src/backend/outbox.rs`) before it commits. Sends, discards, moves, deletes and event deletes wait out the 5 s undo window (`UNDO_WINDOW`); Undo inside it deletes the row (`Backend::cancel()`), past it the UI queues the inverse op (`Backend::mail_now()`). Consecutive label changes to the same threads merge; a newer save of a draft replaces an older one; ops on one thread, event or draft go one at a time, also across bulk ops. The worker (`Backend::schedule()`) commits due rows: thread ops through `outbox::commit_threads()` (W5), drafts and sends through `compose::commit_compose()` (W6), calendar writes through `calendar::commit()` (W7), all at the user's priority (`outbox::UserPriority`); a mail sync runs 2 s after a committed mail write, a calendar sync after a calendar write. Demo mode commits by applying the op to its in-memory store (`Store::apply_mail_op()`, `apply_cal_op()`), so nothing survives a quit there. Rows left pending or in flight at quit go on the next launch (`Outbox::new()` puts in-flight rows back), and `load_mail()` shows them as made; a draft with a queued discard or send stays hidden (`Outbox::overlay()`), so it never comes back. `Backend::queued_sends()` names the threads with a send still queued, for "Waiting to send" (MA-069, shown by polish W2). Quit (⌘Q, the menu; closing the window no longer quits, NTF-3) runs `Backend::flush_on_quit()` first (W11, `src/main.rs` `quit()`): sends and discarded drafts still in their undo window become due at once (`Outbox::send_now()`), and Quit waits until nothing due is left to commit, at most 5 s (`health::QUIT_WAIT`); other queued rows stay for the next launch.
- **Needs:** A tester pass of Quit inside a send's undo window (T4). Calendar writes waiting offline have no overlay: a calendar sync before they go can show the old event until they commit. Deviation from the plan: the store keeps the server's state and takes an op's change only when it commits, so Undo never reverts the store; reads (`load_mail()`, `load_threads()`) lay the queued thread ops over it (`Outbox::overlay()`): that is the pending-op overlay.

#### PER-3 Persisted UI state and settings

- **Today:** Saved to the `settings` table (one row per key, JSON values) of the on-disk store, in demo mode too: appearance, auto-advance, default search scope, list density, text size, sidebar shown, sidebar and list widths, Archived expanded, calendar view, week numbers, and remote images (Ask/Always/Never, for ATT-1). `Saved` (`src/settings.rs`) is the snapshot; `PigeonApp::save_settings()` hands it to `Backend::save_settings()` whenever the app or the calendar notifies and it changed (written 300 ms after it settles, and on quit); `shell::init()` loads it at launch into the app-wide `AppSettings` (sprint 2 W1, NTF-3), and each new window's `PigeonApp::apply_settings()` applies the latest before its first frame; Settings › Notifications saves `notifications`. Unknown keys are ignored and bad values take their default. The file is `~/Library/Application Support/Pigeon/pigeon.sqlite`, or `$PIGEON_DATA_DIR/pigeon.sqlite`. Polish W2: also the window's frame, full screen and display (`Saved::window`, by display uuid; reopened there when at least 100 pt of it still shows, moved and shrunk to fit, else centered, MA-034), Mail or Calendar and the mail view (a label by id; a missing one falls back to the Inbox), folded labels (MC-048), the last 20 palette commands (ME-062) and the first-run tip (MA-094). Nothing is selected at launch, so nothing is marked read or opened. In memory: each view's selection and scroll for the session (MB-071; a draft isn't selected again), `expanded_messages`, recent searches.
- **Today (W11):** Settings › Mail › Load remote images (Ask / Always / Never) changes and saves `saved.remote_images`, which the reader reads (ATT-1). The plan's list (appearance, motion, auto-advance, search scope, density, text size, pane widths, images) is all saved.
- **Needs:** Nothing from the server.
- **Notes:** `expanded_messages` is per thread; keep it or accept the reset.

### 8.2 Command palette (PAL)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| PAL-1 | Command list | 🟢 Fully linked | Each command's own |
| PAL-2 | Thread search in the palette | 🟢 Fully linked | `threads.list` with `q` |
| PAL-3 | Label jumps | 🟢 Fully linked | `labels.list` |

#### PAL-1 Command list

- **Today:** `cmd-k` or `cmd-shift-p` lists every command in `COMMANDS` (`src/actions.rs`) with its shortcut and runs the same handlers. `open_palette()`, `palette_items()`, `on_palette_event()` (`src/app.rs`). Unavailable commands are left out (`When`, `command_available()` in `src/app/mail.rs`). Recent commands first on an empty query, the last five under "Recent" (polish W2: the last 20 are saved, PER-3, ME-062). `Command::action_name` names a command's action without building it, and the shortcut contexts are built once per opening (MC-047). In Calendar, Find (⌘F, ⌥⌘F, Edit › Find…, ⌘K "Search Events") opens the palette on the events (MA-081). Each `Group` has a `scope()` (Mail, Calendar, Global); the palette and `?` sheet list the current mode's commands plus global ones. `v` and `l` open it scoped to folders or labels.
- **Today (polish W2-NAV):** The results are grouped under header rows the arrows skip (ME-062): "Recent", "Commands", "Labels" (and "Calendars" in Calendar) with no query; with one, "Commands", "Labels", "Conversations", "Events", each section placed by its best match. Matching ranks by score and computes highlight positions only for the rows kept (MC-039); highlights change color only, so titles don't shift (ME-079). Toggle commands are found by their title and their sheet name ("Unstar" by "star", "Hide Sidebar" by "show"). The footer counts results only for a query, and ↵ reads "Open", "Create" or "Run" for the selected row (ME-076); a name prompt is one row tall (ME-077). The palette is a Dialog with a ListBox of results, the highlighted row its active descendant, and a scroll bar (MB-076, MA-002, MB-068).
- **Needs:** Each command inherits its action's needs: ACT-1–6, CMP-1–4, CMP-8, DRF-1, DRF-4, UND-1–3, ACT-1a, FOL-5, CAL-9, CAL-10. Navigation, view and app commands need none.
- **Notes:** Sign In with Google, Sign Out and Use Demo Data work (W2: ACC-3, ACC-7). The backend's other commands are in already, doing nothing (or saying so in a toast) until their workstream lands: Sync Now (`cmd-r`, or `cmd-shift-n` in the Apple Mail preset; also Mailbox › Sync Now; SYN-2), Retry Loading Message (RDR-6), Load Older Conversations (LST-7; works since W9).
- **Today (polish W1):** Rows are gated by more `When`s (ME-054): event commands by `Event` / `EventOpen` / `CalendarInvite`, Sync Now and Sign Out by `SignedIn`, Sign In and Use Demo Data by `SignedOut`, Redo by `Redo`; "Undo Delete" is gone, merged into the global Undo (App group, both apps). Toggle commands carry a `Toggle` (ME-055) and `Command::title()` names what will happen ("Unstar", "Hide Sidebar", "Enter Full Screen"); the static "A / B" name stays for the `?` sheet and settings search. New rows (ME-060): About Pigeon, Make Text Bigger / Smaller / Normal Size, Enter / Exit Full Screen, Minimize, Zoom, Bring All to Front, one "… Settings" row per Settings section (`OpenSettingsSection`), Save Draft, Select All Conversations, First / Last Conversation, Clear Recent Searches, Move to Category…, Open in Gmail, Open Draft in New Window, Join Meeting, Accept Invitation / Reply Maybe / Decline Invitation, Open Location in Maps, Copy Link to Event, Open in Google Calendar, Turn Event Reminders On / Off.
- **Needs (polish):** Every row has its handler now (polish W2–W3); `When::Pending` is gone. Polish W3: Open Draft in New Window works while a draft is open (CMP-16), and Turn Event Reminders On / Off flips Settings › Notifications › Event reminders. Open in Gmail / Google Calendar and Copy Link need the server ids (thread id with `authuser`, event `htmlLink`); Move to Category needs `threads.modify` with `CATEGORY_*` labels through the outbox; Accept / Maybe / Decline need `events.patch` on the attendee's `responseStatus`.

#### PAL-2 Thread search in the palette

- **Today:** Matches subject or last non-self sender name, as substring, up to 30 threads after commands. Rows read "sender · date" under the subject, so same-subject conversations tell apart (polish W2, ME-078). `update_matches()` (`src/ui/command_palette.rs`). No body text, unlike the search bar. Searches the local cache only, which is instant (W9 decision); the search bar asks Gmail.
- **Needs:** A tester pass (T3a).

#### PAL-3 Label jumps

- **Today:** "Go to Label: <name>" per label; `g l` opens the palette with `label: `. `go_to_label()`. With Google the labels are Gmail's (FOL-3) and refresh with each label sync.
- **Needs:** A tester pass; new labels (FOL-6) add entries.

### 8.3 Data model (DAT)

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| DAT-1 | JSON as the data source | 🟡 Partially done | Replaced by SYN data |
| DAT-2 | Thread model: one folder per thread | 🟢 Fully linked | Gmail per-message `labelIds` |
| DAT-3 | Message model: missing fields | 🟡 Partially done | `Message` resource |
| DAT-4 | Threading headers | 🟢 Fully linked | `payload.headers` |
| DAT-5 | Local IDs for drafts and sent messages | 🟢 Fully linked | Draft `id`, message `id`, `threadId` |
| DAT-6 | Search index rebuild cost | 🟡 Partially done | None |
| DAT-7 | Import and insert messages | ⚪ Not done | `messages.import`, `messages.insert` |

#### DAT-1 JSON as the data source

- **Today:** The UI reads only from the backend's store. The JSON (`include_str!` in `backend::demo`) seeds the in-memory store of demo mode, which every launch is until sign-in lands (W2); `PIGEON_DEMO=1` will keep forcing it. `demo::mail_data()` makes it look like a Gmail sync: message ids `<thread>-m<N>` (the JSON's own message ids are still dropped), `internal_date` from the date, Gmail label ids per message from the thread's folder and flags, snippets cleaned. `MailData::load()` and `CalendarData::load()` are gone.
- **Needs:** Google data in the store (W3, W4); the JSON stays as demo data and test fixture.

#### DAT-2 Thread model: one folder per thread

- **Today:** Decided and implemented: `gmail::derive_folder()` (`src/backend/gmail.rs`) takes each message's labels (drafts left out): any `TRASH` → Trash, any `SPAM` → Spam, any `INBOX` → Inbox; Sent when every message is one the user sent to others; else Archive. A message counts as received when it lacks `SENT` or the user sent it to one of their own addresses (To/Cc, aliases from `sendAs`, `Store::me()`). Drafts are one-message threads with `folder = Drafts`. `Thread::label_ids` is the union over messages; `labels` (user labels), `unread` (any `UNREAD`) and `starred` (any `STARRED`) are derived from it when the store loads a thread. The store applies `MailOp::Modify`/`Trash`/`Untrash` to every message like `threads.modify` and derives the folder again when a folder label changed. UI moves map to Gmail ops in `MailOp::moves()`.
- **Needs:** Decision 6.4/5 of the sprint plan: the Sent view lists every thread with a message the user sent (W3, `nav_contains`).
- **Notes:** The most important data-model decision; it touches FOL, LST and ACT.

#### DAT-3 Message model: missing fields

- **Today:** `Message` (`src/data.rs`) has `id`, `label_ids`, from/to/cc/bcc, `date` (shown) and `internal_date` (the sort key), snippet, `body: Body` (`NotLoaded | Loading | Loaded { html } | Failed`; `Message::html()` is empty until loaded), attachments (now with `attachment_id`, `part_id`), `message_id`, `in_reply_to`, `references`, `reply_to`, `size_estimate`. `Thread` adds `label_ids`, `history_id`, `loaded` (`Metadata | Full`). `Account` adds `aliases`; `is_me()` checks them (ACC-2). `Event` and `Calendar` gained Google's fields (see CAL-12, CAL-22); `Recurrence` became `Rule` (CAL-5).
- **Needs:** Filling them from Gmail (W3); the plain-text body and the part tree (ATT-2, ATT-4).

#### DAT-4 Threading headers

- **Today:** Stored for every synced message (W3, without angle brackets). With Google (W6) every message sent or saved gets a new `Message-ID` on the sender's domain (`mime::new_message_id()`); replies carry `In-Reply-To` = the parent's id and `References` = its chain plus its id (`mime::reply_threading()`), checked on the real account; the sent copy is stored with its `Message-ID` (CMP-13).
- **Needs:** A tester pass (T2b).

#### DAT-5 Local IDs for drafts and sent messages

- **Today:** `draft-local-<uuid>` / `sent-local-<uuid>` (`new_id()` in `src/app/compose.rs`), messages `local-<uuid>`, new events a bare UUID (32 hex digits, valid as a client-chosen Google event id, CAL-11). `BackendEvent::IdReplaced` swaps a thread's id in the view model, the draft origin, the selection and an open composer (`on_id_replaced()`, `src/app/backend_events.rs`); in the store, the commit writes the thread under its new id (`backend::compose`; the unused `Store::replace_thread_id()` went in polish W5).
- **Needs:** The server answering with ids: `drafts.create`, `messages.send` (W6).

#### DAT-6 Search index rebuild cost

- **Today:** The index is built once when a thread is stored (`Thread::search_text()`) and kept in `threads.search_text`; loading takes it from there (`Thread::set_search()`), so launch parses no HTML for search. Saves and sends still rebuild that thread's index; `update_threads()` re-sorts everything.
- **Needs:** Bodies fetched later (RDR-6) re-index their thread when stored (W3).

#### DAT-7 Import and insert messages

- **Needs:** Later, for migration: `messages.import` (delivery-like scanning, up to 150 MB) or `messages.insert` (like IMAP `APPEND`, no scanning) to bring mail from another account into Gmail. Neither sends mail.

### 8.4 Features with no backend need

Real features that don't depend on the backend, listed so the tables aren't read as the whole UI:

- Pane resizing for sidebar and list (`PaneResize`, `resize_pane()` in `src/app.rs`); sidebar show/hide (`cmd-\`, `toggle_sidebar()`); dragging the sidebar's divider 60 pt past its minimum hides it and keeps its width (polish W2, MB-075).
- The window title names the view: "Inbox — Pigeon", a label's name, "Calendar — Pigeon", "Settings — Pigeon" (polish W2, MB-011), for Mission Control, the Window menu and VoiceOver.
- Paging in the reader (polish W2, MB-045): Space / Page Down and ⇧Space / Page Up by a screen less 40 pt (Space presses a focused control instead), Home / ⌘↑ and End / ⌘↓ to the top and bottom; the list pages its selection (W2-LIST).
- A first-run tip, bottom right: "Every command ⌘K · Shortcuts ?", gone for good once either is used or it's dismissed (polish W2, MA-094, `Saved::seen_tip`).
- Keyboard shortcuts sheet (`?`, `src/ui/shortcuts_help.rs`), built from the live keymap, scoped like the palette. The bindings are looked up once per keymap and scope, not every frame (polish W2, MC-040, `HelpBindings`). Its filter matches names, group titles and keys, with spelled-out modifiers ("cmd", "shift", "opt", "ctrl", or ⌘ ⇧ ⌥ ⌃; ME-082), and says what to try when nothing matches. A Dialog with headings and a scroll bar (MB-076, MB-068).
- The menu bar (polish W1, `menus()` in `src/main.rs`): Pigeon, File, Edit, Format, View, Mailbox, Message, Event, Window, Help, with every existing command that has a place there; items show their shortcut (single keys too) and dim where the action can't run. Built from a `MenuState` (checkmarks on Mail/Calendar, Day/Week/Month, Compact List, Week Numbers; Show/Hide Sidebar, Star/Unstar, Enter/Exit Full Screen and "Undo Archive" titles) and rebuilt only when it changes (`set_menu_state()`). The Dock menu has New Message, New Event, Sync Now and Join Next Meeting. About Pigeon is the standard panel (`native::show_about()`, `NSHumanReadableCopyright`). Window › Main Window (⌘O) and Bring All to Front.
- Keyboard shortcut presets (MA-038, MA-089): Gmail (default) or Apple Mail, from `Saved::keymap`; `set_keymap()` (`src/main.rs`) clears and rebinds the keymap and rebuilds the menus and the Dock menu; Settings › Shortcuts goes through it. The event panel's Discard keycap reads the live binding. Apple Mail changes only ⌘ shortcuts: ⌘R Reply, ⇧⌘R Reply All, ⇧⌘F Forward, ⇧⌘D Send, ⇧⌘N Sync Now, ⇧⌘⌫ Discard Draft and Discard Changes to Event. In both: ⌘S saves the draft, ⌘. closes like Escape in the overlays, Settings, the event panel and pop-ups, ⌘W closes Settings, the palette, the shortcuts sheet and the contact card before the window (MA-077), ⌃⌘S and ⌘\ the sidebar, ⌃⌘F full screen, ⌥⌘N New Event, ⌥⌘O / ⌥⌘B / ⌥⌘Q numbered list, bulleted list and quote besides Gmail's ⌘& / ⌘* / ⌘( (MA-088), `g x` for Spam besides `g !`. Some new bindings (RSVP, ⌘J, ⌘Y) wait for their handlers in later polish waves; Home/End select the first and last conversation, paging works in the reader and list (polish W2).
- Calendar views, navigation and keyboard selection (CAL-4).
- Light and dark following the system (`Theme::sync()` in `src/theme.rs`) or the Appearance setting, including the Increase Contrast appearances (vendored `gpui-pre-macos`, `vendor/gpui-pre-macos/PIGEON_PATCHES.md`), with the system display options and accent color (A11Y-3 to A11Y-6).
- Time format (Settings → General): System (default; macOS's Date & Time setting, read with `NSDateFormatter` template `j` in `native::system_uses_24_hour()`, again when the locale changes), 12-hour or 24-hour (`Saved::time_format`, `shell::set_time_format()`). Every time in mail and calendar (list, reader, forwarded header, menu bar panel, contact card, last sync, calendar views, panel, reminders) goes through `src/calendar/time.rs`.
- The settings page (⌘,, `src/ui/settings.rs`) with search and keyboard control, apart from saving values (PER-3). Its rows use the shared controls (A11Y); Default calendar and Default event length are pop-up menus; Appearance › Transparency is a slider (←/→ in steps of 10%, Enter resets; locked to opaque while Reduce Transparency is on); Shortcuts › Shortcut preset switches Gmail / Apple Mail ⌘ shortcuts and rebinds the keymap at once. Polish W3: General is the first section (ME-067); the Reduce motion switch is gone, Pigeon follows the system's Reduce Motion alone and ignores the old saved `reduce_motion` key (MB-072); the two Menu Bar shortcuts are pop-up menus (ME-068); a pop-up takes ↑/↓, Enter and Esc, and Enter on a closed one opens it (MC-069); the search field has ✕ (clears and keeps the keyboard there) and an accent border while focused (MA-082); search highlights are by color only (ME-079); the keyboard's row uses `selected_bg` and the ring shows only in the key window (ME-080, MB-042); Return on the Sign Out confirmation is Cancel unless → moved to Sign Out (MB-028); the rows and sections show a scroll bar (MB-068); the account row reads "Google · address · Mail, Calendar and Contacts allowed" (MB-058); every command's bindings are read once per open or preset change (MC-040); `settings::get(cx)` borrows the settings instead of cloning them (MC-046); `Appearance::window_appearance()` and `Theme::sync_for()` are the one appearance mapping (MC-085). Polish W4: a setting jumped to from the search or ⌘K lights at once, holds 900 ms, then fades to the cursor's color on `SPRING_PANE`; moving the cursor away ends it (MD-026). Its pop-up menus drop from their button (MD-011).
- Keyboard focus between list, reader, composer and search; selectable message text.
- Window title bar and drag area (`window_drag_area()` in `src/ui/mod.rs`); Quit (`cmd-q`).
- Cached panes (polish W4, MC-036, MD-004, `src/app/panes.rs`): the sidebar, list and reader are child views drawn through `.cached()`, so a hover, spring or input inside one pane redraws only that pane. Every app notify still redraws all three, as do contact changes; calendar changes redraw all three too (the inbox-zero next event, and the calendar's minute tick moves relative times), the shortcuts filter only the overlay layer, the reader's n/p glide only the reader. Uncached while the window fades in after sign-in and while VoiceOver is on (the accessibility tree is rebuilt every frame). Relative times ("5 min ago", "Synced …") tick with the calendar's minute timer.
- Mail and shell motion (polish W4, `src/ui/motion.rs`: `presence`, `pop`, `turn`, `Glide`, `spinner`, `after_loading_delay`): the sidebar slides in and out on `SPRING_PANE` with the list header's show-sidebar button fading (MD-012 to MD-014); double-clicking a divider glides the pane back to its default width (MD-015), the divider's cursor shows which way it can still go (MA-093) and the trackpad ticks at the default width (MA-074); the palette, shortcuts sheet and contact card fade and drop out on close and take no clicks while leaving (MD-007); the contact card hung at a name drops in like the other popovers (MD-011); toasts fade in and out, the leaving one drawn as a ghost (MD-008); archived, moved and synced-away rows collapse and new rows fade in, Undo's rows grow back (MD-017, MD-024); archiving the last Inbox conversation centers the all-caught-up line at once; the search field opens from 60% width (MD-016, MA-095) and its summary row reveals (MD-043); stars pop (MD-027), disclosure chevrons turn (MD-021), n/p glides the reader to the message (MD-018), a clicked message expands with a fade (MD-020), the reply card rises and fades in under the conversation and fades out after a send, Esc or discard, taking no clicks while it goes (MD-019), composer chips and errors fade in (MD-028), menus drop from their button and suggestions fade (MD-011), the rail's unread dot grows and shrinks (MD-029), and the signed-in view fades in (MD-042). Skeletons and placeholders wait 300 ms, then fade in; Syncing, searching, Load Older and the sign-in wait show a spinner (MA-066). Dropped: elastic overscroll in the list, reader and week grid (MB-043); it fights keeping the scroll position, the reader's n/p glide, drag autoscroll and the cached grid, and gains little on macOS, where the system already rubber-bands trackpad scrolls.
- Profiling mode (polish W3, `src/app/profile.rs`): `PIGEON_PROFILE=1` with `PIGEON_DEMO=1` runs a fixed sequence with no input (opening conversations and the largest HTML one, re-renders, the sidebar, the palette with a filter, Calendar week, period steps, the event panel's spring), logs each scenario's frame count and p50 / p95 / max frame time (from `PigeonApp::render` to a marker painted last) to the log and `<data dir>/profile.txt`, and quits on its own in about 30 s. A scenario that drew no frame reads "0 frames: not measured"; a run that hangs appends "timed out after 120 s" to `profile.txt` and exits with status 3. The largest HTML conversation is picked from the stored bodies and logged with its id and size in bytes (the demo data's largest is under 1 KB). Each step runs from the sequence's own task; keystrokes, actions and calendar steps run outside the app's update, so their handlers don't re-enter it. Off unless the variable is set.

### 8.5 Accessibility (A11Y)

VoiceOver, keyboard access, contrast and the system display options (Increase Contrast, Reduce Transparency, Reduce Motion). No backend need.

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| A11Y-1 | Shared controls with role, label, state, press and focus | 🟡 Partially done | None |
| A11Y-2 | Reduce Motion: fades instead of movement | 🟡 Partially done | None |
| A11Y-3 | Increase Contrast | 🟡 Partially done | None |
| A11Y-4 | Reduce Transparency and the glass amount | 🟡 Partially done | None |
| A11Y-5 | System accent and highlight colors | 🟡 Partially done | None |
| A11Y-6 | Text contrast: labels, status colors, events | 🟡 Partially done | None |
| A11Y-7 | Text size (⌘= / ⌘-) grows controls, icons and avatars with the text | 🟡 Partially done | None |
| A11Y-8 | VoiceOver reads plain text and announces status changes | 🟡 Partially done | None |
| A11Y-9 | Control-click opens context menus | 🟢 Fully linked | None |
| A11Y-10 | Differentiate Without Color | 🟡 Partially done | None |

#### A11Y-1 Shared controls with role, label, state, press and focus

- **Today (polish W1):** `src/ui/controls.rs` has one `push_button(kind)` (Plain, Primary, Danger, Destructive, Quiet), `switch`, `segmented` (equal-width segments, a thumb that slides on `SPRING_POP`), `stepper`, `slider`, `toggle_chip`, `menu_row` with a leading `menu_check` column, `dropdown`, `popup`, `popover_surface`, `sheet_surface`, `person_row`, `error_line` and `bar_colors`. Each sets its role (Button, Switch, RadioGroup/RadioButton, SpinButton, Slider, MenuItemRadio) and label, shows hover and a press state, and takes keyboard focus with the shared ring. Settings uses them; its switch knob springs across (`SPRING_CONTROL`).
- **Needs:** The calendar header, event panel, composer and the other private copies move to them (polish W2). Tab in the event panel (`NextField`, its own field list) skips the shared segmented controls (RSVP, repeat units, reminder method), the footer buttons and the chips; they take focus by click and VO only. Adding them needs focus handles for each and a check by hand (W2-R26).

#### A11Y-2 Reduce Motion: fades instead of movement

- **Today (polish W1):** `src/ui/motion.rs` has the spring tokens (`SPRING_PANE`, `SPRING_POP`, `SPRING_EXIT`, `SPRING_CONTROL`, `SPRING_SCROLL`, `SPRING_SNAP`), `MOTION_RISE`, `Rise`, `card_motion`, `enter(id, rise, el)` and `fade()`, an opacity spring that keeps running under Reduce Motion while movement snaps. The settings switch's track color fades under Reduce Motion while its knob snaps. Polish W3: Settings no longer has its own Reduce motion switch; the system's setting alone decides (MB-072). Polish W4: the menu bar panel's notice and undo bar fade under Reduce Motion (MD-032), as does the fade-out of a setting jumped to from the search (MD-026); the panel itself shows and hides at once. Polish W4 (mail and shell): the palette, shortcuts sheet, contact card, toasts, the reply card, the search summary, the rail's unread dot and the signed-in view keep their fades under Reduce Motion (`motion::presence`; a surface already open at launch doesn't fade in); the sidebar's slide, drops, pops, turns, glides and row collapses snap. Polish W4 fix: `enter()` fades under Reduce Motion instead of snapping (`motion::Appear`, a small element of its own), so everything that fades in on appear keeps its fade: popovers and pickers, fresh rows, composer chips, a clicked message, added events, guest and reminder rows, the panel's side flip; deleted events fade out too. Loading placeholders keep their 300 ms wait under Reduce Motion (MA-066).
- **Needs:** a check by eye with Reduce Motion on.

#### A11Y-3 Increase Contrast

- **Today (polish W1):** `Theme::build` picks a high-contrast variant when `native::increase_contrast()` is on: opaque panes and window, borders at 0x40, secondary and tertiary text at 7:1 or more, stronger hover, selection, switch track and calendar lines, past events at full strength (`Theme::high_contrast`). Read at launch, on `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification` and when Pigeon comes forward (`native::Signal::DisplayChanged`, `shell::refresh_theme`). The vendored `gpui-pre-macos` maps the four `NSAppearanceNameAccessibilityHighContrast*` appearances, which upstream turned into Light (MA-011).
- **Today (polish W5):** The calendar's recurring derived colors are Theme methods that follow Increase Contrast: `event_secondary` (an event's time and "until" line, full strength with it on), `event_answer_fill` (pending and declined fills, 0.25 → 0.5) and `event_free` (free events' fill and dot).
- **Needs:** A user check on a Mac with Increase Contrast on, in light and dark. A few one-off inline alphas remain (the drag ghost's border and shadow, the date picker's range border, keycaps on an accent fill).

#### A11Y-4 Reduce Transparency and the glass amount

- **Today (polish W1):** `Saved::glass` (0–100, default 0: the theme's own tints) blends every pane toward opaque over the material gray (`Theme::thicken`); at 100, or with Reduce Transparency or Increase Contrast on, the panes are solid and both windows switch to `WindowBackgroundAppearance::Opaque` (`Theme::apply`, `shell::window_background`). `shell::set_glass` re-themes every window at once and saves 400 ms after the slider rests. The slider locks, with a note, while Reduce Transparency or Increase Contrast is on. Sidebar, rail and menu bar tints rose to 0xcc alpha (MA-016).
- **Needs:** A user check of the opaque look and the slider in both appearances.

#### A11Y-5 System accent and highlight colors

- **Today (polish W1):** With an accent set in System Settings › Appearance (`native::accent_color()`, none for Multicolor, which keeps Pigeon's blue), the accent, focus bar, focused selection, checked rows, context ring, today's column, switch track, palette matches and links take its hue at 3:1 (4.5:1 for text); `accent_text` is white or near-black by contrast; text selection follows the highlight color. Re-read on `NSSystemColorsDidChangeNotification`. gpui-component's inputs, tooltips and links use Pigeon's selection, caret, ring, popover and link colors (MA-017).
- **Needs:** A user check with a few accents (Purple, Yellow, Graphite).

#### A11Y-6 Text contrast: labels, status colors, events

- **Today (polish W1):** `theme::readable`, `contrast` and `on_fill` live in `src/theme.rs`. Label chip text reaches 4.5:1 on its chip (tested over every label color, both appearances). Status colors split into fills (`success`, `danger`, `warning`) and text (`text_success`, `text_danger`, `text_warning`, `danger_text` on a danger fill); `text_disabled` (≈3:1) replaces `opacity(0.4)` in the event panel and the stepper; `text_dim` is gone (read rows use `text_muted`). Email text colors under 4.5:1 on the opaque reading pane are mirrored, then moved toward the readable end (`theme::readable` goes lighter on a dark background, darker on a light one). Selected events, the current-time label (a darker `cal_now`), calendar checkboxes and RSVP badges use `Theme::on_fill`. Past events fade their fill, not their text. The sync status line shows a warning dot and muted text when offline, an accent dot while syncing (ME-057).
- **Needs:** The mail views move to the text tokens (polish W1-KITB, W2).

#### A11Y-7 Text size (⌘= / ⌘-) grows controls, icons and avatars with the text

- **Today (polish W1):** `src/ui/style.rs` has rem control heights (`CONTROL_XS/SM/MD/LG` 20/24/28/32, `ROW_SM` 30, `ROW_PALETTE` 38), inline icon sizes (`ICON_SM/MD/LG/XL/HERO` 12/14/16/18/28; toolbar `ICON_TOOLBAR` 16 and rail `ICON_RAIL` 18 stay in pixels), avatar sizes (`AVATAR_XS/SM/MD/LG` 20/24/32/48), `LINE_UI` (1.25) and `LINE_READING` (1.55), macOS title sizes `TEXT_TITLE1` (22) and `TEXT_TITLE2` (17), and `MONO`. `HIT_TARGET` (28) is the minimum click box: `icon_button` is 28 with a 16 glyph, `icon_button_small` a 12 glyph in a 28 box. Glyphs at 13px and under get a 2.25 stroke and hero glyphs a 1.25 one (`Assets::load` rewrites the SVG's stroke width; `Stroke::for_size`). Avatars show one initial at 20 and never set initials under 10pt; their text color follows the fill (`Theme::on_fill`). The thread list, sidebar, rail, reader, composer, onboarding, shortcuts sheet, command palette (its list height is in rems too), contact card and menu bar panel use them; keycaps read ↩ and ⎋ and shifted symbols as ⇧ plus the key (⌘& reads ⇧⌘7), labels sit before their keycaps, and onboarding's primary button uses `keycap_on_accent`.
- **Today (polish W5, MA-021):** ⌘= / ⌘- go from 90% to 200% (`TEXT_SCALE_RANGE`). `style::type_size` is public: widths and heights of boxes that hold text are rems (menus and dropdowns, suggestion lists, the event panel's time, number and date fields, the time picker, the settings section column, the shortcuts filter, onboarding's buttons, the menu bar panel's time column, the palette's title cap, attachment chips). Pane top bars are `.h(HEADER_MIN).min_h(HEADER_HEIGHT)`: 52 pt until their controls need more, and `ui::header_height(window)` places what sits under them. The calendar's all-day lanes, Month lanes, day numbers and weekday row, the Week day header and now label, and the drag ghost's time label scale with the text.
- **Today (polish W5 fix):** `push_button` is `CONTROL_MD` tall with a `HIT_TARGET` floor, so its label and keycaps fit at 200% and it stays 28 pt at 90%; the Month day number keeps its 28 pt floor too. The Week and Day hour gutter scales with the text (`week_view::gutter_width(window)`, shared by the grid, the Day view's column and the panel's side check), as do the hour labels' centering and the now-label gap. The event panel's time rows wrap when the fields outgrow the panel, and the palette's title shrinks with an ellipsis instead of pushing its subtitle out.
- **Needs:** A check by eye at 200%. Left in points on purpose: the hour height (the grid's scale, not text), pane widths (the user drags them), the event panel's and palette's widths, the menu bar panel's width (a native window), and the mini month's 28 pt cells (they must fit the sidebar).

#### A11Y-8 VoiceOver reads plain text and announces status changes

- **Today (polish W1):** `ui::label(id, text)` puts text in the accessibility tree (empty states, the bulk-selection title, the contact card's empty list, the onboarding heading); `ui::live_text(id, text, live)` announces changes (the composer's save state, a message body loading, onboarding's wait for the browser, toasts: polite, assertive for errors). Tooltips go through `ui::tooltip` at 11pt.
- **Today (polish W2, reader):** Every `Selectable` (message paragraphs, the subject, the sender's address) is a Label with its text as the value; email headings and the subject are Headings with their level; linked text holds a Link node per link with its URL; buttons and linked images in emails are Links named by their text or alt (MA-001, MA-003). The quoted-text button, "N more messages" and "+N others" carry `aria_expanded`; collapsed messages are disclosure triangles; a loading body is a Status.
- **Today (polish W2, list):** Thread rows say what they show and are the list's active descendant when selected, as is the highlighted search suggestion. The list title is a heading with the full label path; the search count is a label and Gmail's search state a polite live region; the select-all checkbox reads "Select all, 3 of 12 selected" and is mixed while only some are.
- **Today (polish W5 fix):** Field errors (`controls::error_line`: the composer's, the event panel's guests and video link), the sign-in error and Settings' status rows (`Control::Info`, the default mail app error) are selectable text, so they can be copied (MB-035). Palette rows and toolbar `icon_button`s carry their shortcut (`aria_keyshortcuts`, from `ui::binding_text`; the toolbar's from the keymap index `ui::index_shortcuts`, rebuilt by `bind_keys`); row checkboxes read "Select {subject}" (MA-048).
- **Needs:** List and search status and pane titles elsewhere (polish W2, the cross-cutting checklist). A user check with VoiceOver on a long newsletter.

#### A11Y-9 Control-click opens context menus

- **Today (polish W1):** `ui::is_secondary` (right click, or Control with the left button) opens the thread row and label menus; a Control-click no longer selects the row or opens the label. Polish W2: also the reader's menus (RDR-10).

#### A11Y-10 Differentiate Without Color

- **Today (polish W2, MA-024):** With System Settings › Accessibility › Display › Differentiate without color on (`native::differentiate_without_color()`), the list's label dots become small chips with the label's first letter.
- **Needs:** The rest of the app (status dots, calendar colors) and a user check.

### 8.6 System integration (SYS)

macOS features beyond links (LNK-4) and sharing (ACT-8, MB-061 step 2), and the MCP server for AI agents (SYS-4).

| ID | Feature | Status | Google API |
| --- | --- | --- | --- |
| SYS-1 | Spotlight | ⚪ Not done | None (reads the cache) |
| SYS-2 | Handoff | ⚪ Not done | None (Gmail and Calendar web URLs) |
| SYS-3 | Widgets | ⚪ Not done | None (reads the cache) |
| SYS-4 | MCP server for AI agents | 🟡 Partially done | None (Gmail and Calendar through the existing outbox) |

#### SYS-1 Spotlight

- **Idea (MB-064):** Index conversations and upcoming events with `CSSearchableIndex` after sync, remove them on sign-out, and open a result through `continueUserActivity` (a patch to gpui's app delegate, or added at runtime).
- **Needs:** Check what Google's Limited Use policy (ACC-8) allows before mail goes into the system index.

#### SYS-2 Handoff

- **Idea (MB-065):** An `NSUserActivity` with a `webpageURL` (the Gmail thread URL with `authuser`, or the event's `htmlLink`) while a conversation or event is open, invalidated when it closes.
- **Needs:** The server ids (as Open in Gmail, MB-061).

#### SYS-3 Widgets

- **Idea (MB-066):** A WidgetKit extension showing the agenda and unread mail from a JSON snapshot in an App Group container, reloaded after sync. The menu bar panel (NTF-4) covers the glanceable view meanwhile.
- **Needs:** A Swift build step, entitlements and a provisioning profile.

#### SYS-4 MCP server for AI agents

- **Today:** While Pigeon runs and Settings › MCP › MCP server is on, Pigeon serves the Model Context Protocol (Streamable HTTP, protocol versions 2026-07-28, 2025-11-25, 2025-06-18 and 2025-03-26) at `http://127.0.0.1:6427/mcp`, or the next free port (the real one shows in Settings). One `POST /mcp` answered with `application/json`; GET is 405, DELETE ends a session. `src/mcp/server.rs` is the transport: `std::net` on 127.0.0.1 only, `httparse` for the request head, one request per connection, 16 KB head and 1 MB body limits, 10 s I/O timeouts and a 30 s whole-request deadline, at most 8 connections. Every request needs `Authorization: Bearer <token>` (401 otherwise); a present `Origin` must be localhost or 127.0.0.1 and `Host` must name the server (403 otherwise, against DNS rebinding). Both protocol eras share the endpoint. **2026-07-28 (modern, stateless):** a request whose `_meta` carries `io.modelcontextprotocol/protocolVersion` (or whose `MCP-Protocol-Version` header isn't a legacy version) is served on its own, with no session: `server/discover` (supported versions, `tools` capability, instructions), `tools/list` and `tools/call`; results carry `resultType: "complete"` and `_meta` `io.modelcontextprotocol/serverInfo`, and `server/discover` and `tools/list` add `ttlMs` (1 h) and `cacheScope: "public"`. The `MCP-Protocol-Version`, `Mcp-Method` and (for `tools/call`) `Mcp-Name` headers are required and must match the body (Base64 `=?base64?…?=` names decoded), else 400 with `HeaderMismatch` -32020; a version Pigeon doesn't speak is 400 with `UnsupportedProtocolVersionError` -32022 listing every supported version; a missing `protocolVersion` or `clientCapabilities` is 400 with -32602; an unknown method (`ping` included, removed in 2026-07-28) is 404 with -32601. The activity log names the client from `io.modelcontextprotocol/clientInfo`; an `Mcp-Session-Id` on a modern request is ignored. **2025-11-25 and earlier (legacy):** `initialize` negotiates the version in its body and hands out an `Mcp-Session-Id` that names the client in the activity log; an unknown session is 404; `ping`, notifications and results are as before (no `resultType`); a request without `MCP-Protocol-Version` is taken as 2025-03-26. The token (`pgn_` + 32 random bytes from SecRandom, base64url) is in the login Keychain (service "Pigeon MCP", account the data folder), shown masked in Settings with Copy and Regenerate (which asks first: agents using the old token are disconnected). A Keychain that can't be read keeps the server off with the reason in Settings, rather than replacing the token agents use; a token that can't be saved works for this run, with a warning under it.
  - **Tools** (`src/mcp/tools.rs`), each behind a switch: Read mail (`search_mail` with Pigeon's search syntax over the cache plus Gmail's search when signed in, `read_conversation` with plain-text bodies and attachment metadata, fetching missing bodies, `list_labels`); Organize mail (`archive`, `trash`, `mark_read`, `star`, `add_label`, `remove_label`); Write drafts (`create_draft`, new or reply); Send mail (`send_mail`, new, reply or a draft by id); Read calendar (`list_events`, `get_event`); Edit calendar (`create_event`, `update_event`, `delete_event`, `respond_to_event`). `tools/list` lists all of them, each description naming its switch; a tool whose switch is off returns a tool error naming it.
  - **Writes** are the UI's backend ops (`MailOp`, `CalOp`) through the outbox, so the undo window (archive, trash, send, delete wait 5 s), offline queueing, rollback on failure and sync behave as for the user's own changes. Mail ops are queued as the menu bar panel queues them, with `ThreadsChanged` so the windows read them back; calendar changes go through the open calendar's in-memory events and `queue()` as its own edits do, or straight to the backend with no window open. Calls run on the main thread (an `async-channel` to a foreground task). Each carries a 30 s deadline: the HTTP thread waits a little past it, and a call the main thread reaches after it is never run, so an agent told "Pigeon was busy and didn't run this" can safely retry; writes finish in the turn they start. An agent can't change an event the user is changing (panel open with changes or on a new event, an edit settling, an answer pending, a guest or scope question open); it gets a tool error saying so. An agent's change to the event the panel shows reloads the panel, and its delete closes it, as a sync does. Dates outside 1900–2200 are refused. Local ids the backend replaces (`IdReplaced`) still resolve. A draft open in a composer can't be sent over MCP.
  - **Settings › MCP** (`src/ui/settings.rs`): MCP server switch, Server address (Copy), Access token (Copy, Regenerate), the six capability switches, "Add to your agent" with a JSON config snippet (Copy includes the token), and Activity (the last 50 calls this session: time, client, tool, OK or Error; in memory). Saved as `Saved::mcp` (`McpPrefs`): off by default; reads on, writes off until switched on. ⌘K: "MCP Settings".
- **Needs:** A check with real agents (Claude Code, Codex, Cursor) against a real account. Not in the undo stack: an agent's archive or send shows no Undo toast in the window (the outbox's 5 s window still applies, and the change can be reversed by hand). A failed agent send gets no notice of its own in Pigeon beyond the window's usual handling of a failed op (the draft goes back to Drafts); the agent was already told "queued". With no window open, calendar reads (`list_events`, `get_event`) come from the store, which has an agent's change only once it's committed (about 1 s online; offline, once back online; a delete after its 5 s undo window); the write's result says so in a `note`. Keeping the agent's uncommitted events and overlaying them would close that gap. Not done: recurring-event time changes and "this and following" over MCP, Meet links, attachments on outgoing mail, label creation, SSE streaming, `listChanged` and `subscriptions/listen`, and 2026-07-28's optional parts (pagination, `notifications/progress`, input-required results; none needed by today's tools). A check with a real 2026-07-28 client once one ships.
- **Notes:** Google's Limited Use policy (ACC-8): this hands Gmail data to whatever agent the user connects, which may send it to a model provider. In the user's own local build that's their choice; a published, verified app must disclose it in the privacy policy and consent screen, keep it user-initiated and off by default (it is), and Google's Limited Use rules forbid transferring Gmail data to third parties except to provide user-facing features the user asked for, and forbid its use for training models; check with Google during verification (ACC-8) whether an MCP bridge to arbitrary third-party AI clients is allowed, or limit what Read mail exposes. The token is the only guard against other local processes; any app running as the user can read it from Settings' Copy or the Keychain item it trusts.

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
- **Not in Google's APIs at all:** snooze, scheduled send and server-side undo send. Snooze (ACT-11) and send later (CMP-17) are app-side: local timers plus a hidden label and the outbox; they only fire while Pigeon runs. Gmail's own snoozes and scheduled sends aren't readable.

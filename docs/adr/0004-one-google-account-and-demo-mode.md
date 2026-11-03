# 0004. Support one Google account, Google APIs only, with a demo mode

- **Status:** Accepted (2026-10-26)
- **Context:** Pigeon's first real backend had to link one mailbox end to end, and every extra provider or account multiplies the sync, auth and store work. Sprint 1 fixed the scope: only one account, and Google's APIs (Gmail, Calendar, People) rather than IMAP/SMTP. The app also had to keep running with no network and no sign-in, so the generated `assets/*.json` stayed as a demo source.
- **Decision:** One Google account per install, talked to over Google's REST APIs only. Demo mode seeds an in-memory store from the generated assets and shares the same code paths; there is no IMAP/SMTP client yet.
- **Consequences:**
  - `PigeonApp::data` is a single `MailData`, so multiple accounts (ACC-4) need a model change, not a setting.
  - No IMAP means non-Google mail can't be added; the roadmap keeps IMAP/SMTP equivalents as notes for later.
  - Demo mode must stay honest: it exercises the same store, outbox and UI, and its changes go nowhere.
  - Revisit when adding a second account or a non-Google provider, or when demo mode stops covering a code path.

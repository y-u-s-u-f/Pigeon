# 0002. Keep mail on the Mac with no server

- **Status:** Accepted (2026-10-26)
- **Context:** Gmail's useful scopes are restricted, and storing or sending that data through our own servers would trigger a security assessment (`docs/ROADMAP.md` ACC-8). Gmail push needs Cloud Pub/Sub and Calendar push needs an HTTPS webhook, so both need a server. Sprint 1 fixed the constraint: no server, no Pub/Sub, no webhooks.
- **Decision:** Be local-first: a SQLite cache on the Mac, a durable outbox for writes, and polling Gmail, Calendar and People to stay current. No backend of ours.
- **Consequences:**
  - Changes arrive on the poll interval (45 s active, 5 min background), not as push (SYN-3, SYN-5 stay not done).
  - Offline work needs the outbox, the overlay and rollback, not just a request path (STA-2, STA-3).
  - Data lives in one place, so multiple Macs don't share a cache and sign-out must delete it (ACC-7).
  - Revisit if push or multi-device sync becomes a requirement, or if Google's verification rules change.

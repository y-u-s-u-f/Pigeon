# 0003. Use blocking HTTP with ureq

- **Status:** Accepted (2026-10-26)
- **Context:** The backend needs HTTP for Google, but an async runtime would be a heavy dependency and a second execution model next to GPUI. GPUI already builds `rustls` and `ring`, so a blocking client over them adds no compile time.
- **Decision:** Use `ureq` 3 (rustls, gzip) for all requests, on background threads, with at most 4 calls in flight. No tokio, no async HTTP, no async OAuth crate.
- **Consequences:**
  - Sync and transport code is plain blocking Rust, easy to test with `FakeTransport` and a fake clock.
  - Each request holds a thread; concurrency is capped by design, so a slow call delays the queue behind it.
  - Cancellation is coarse: there is no async select, and quitting waits on the outbox flush.
  - Revisit if the app needs many parallel calls or streaming beyond what SSE over `ureq` gives.

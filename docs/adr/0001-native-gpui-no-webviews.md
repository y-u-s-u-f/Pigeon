# 0001. Render the UI in native GPUI, without webviews

- **Status:** Accepted (2026-10-26)
- **Context:** Pigeon is a macOS mail and calendar client that must feel fast and stay light. A webview would bring a browser engine and its memory, and would make the whole UI a web app. GPUI 0.8 (`gpui-kit`, with gpui-component) was already the shell the app was built on.
- **Decision:** Draw everything with GPUI. No webview anywhere, including for HTML mail, which Pigeon parses and lays out itself.
- **Consequences:**
  - Mail HTML goes through Pigeon's own parser and renderer (`src/html.rs`, `src/ui/html_view.rs`), so the supported markup is a subset.
  - No web runtime means no AMP for Gmail and no sender-side markup (`docs/ROADMAP.md` §11).
  - UI work needs GPUI knowledge, not CSS or DOM tooling; shared controls come from `src/ui/controls.rs`.
  - Revisit if real mail renders too badly to fix in the subset, or if a feature genuinely needs a web engine.

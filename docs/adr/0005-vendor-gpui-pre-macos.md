# 0005. Vendor and patch gpui-pre-macos

- **Status:** Accepted (2026-10-27)
- **Context:** The app needs macOS behavior the crates.io `gpui-pre-macos` 0.3.9 doesn't give: Increase Contrast appearances mapped correctly, content options on system notifications, and cancelled scroll/magnify gestures. Upstream is a copy of Zed's `gpui_macos` and isn't ours to change quickly.
- **Decision:** Copy the crate into `vendor/gpui-pre-macos`, patch it there, and wire it in with `[patch.crates-io]` in `Cargo.toml`. Every change is listed in `vendor/gpui-pre-macos/PIGEON_PATCHES.md` with its roadmap ID.
- **Consequences:**
  - Patches apply to the whole app, including gpui-component, without forking GPUI itself.
  - Upgrades are manual: a new upstream version means re-applying each listed patch and re-running its test.
  - The vendored crate is a path dependency, so its warnings are silenced with a lint override rather than fixed upstream.
  - Revisit when upstream ships a fix for a listed patch, or when the patch list grows past what we can maintain.

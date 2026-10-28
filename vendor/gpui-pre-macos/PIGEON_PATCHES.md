# Pigeon's patches to gpui-pre-macos

Upstream: `gpui-pre-macos` 0.3.9 from crates.io (Zed's `gpui_macos`, zed@93d7456),
copied unchanged and wired in with `[patch.crates-io]` in Pigeon's `Cargo.toml`.
`.cargo-ok` and `Cargo.lock` were dropped. Keep changes minimal and list each here.

1. `src/window_appearance.rs` (MA-011): appearances are matched by name, and the
   four `NSAppearanceNameAccessibilityHighContrast*` appearances (Increase
   Contrast) map to Light, Dark, VibrantLight and VibrantDark. Upstream logged
   "unknown appearance" and returned Light, so dark mode with Increase Contrast
   turned light. Unit test: `cargo test -p gpui-pre-macos window_appearance`.
2. `Cargo.toml`: `[lints.rust.deprecated] level = "allow"`. As a path dependency
   the crate's many `cocoa` deprecation warnings would show in every build;
   from crates.io they were capped.
3. `src/system_notifications.rs` (MA-073, MB-051, MB-053; MB-Q13, MB-Q17):
   `SystemNotification` lives in `gpui-pre`, which isn't vendored, so content
   options travel as actions whose id starts with `@`; they're taken out
   before the buttons are made. `@sound` plays the default sound, `@thread=<id>`
   sets `threadIdentifier` (grouping), `@level=passive|active|time-sensitive`
   sets `interruptionLevel`, `@hidden=<text>` is the category's
   `hiddenPreviewsBodyPlaceholder`. Category identifiers are named after
   their action ids (upstream numbered them in the order registered), so a
   notification delivered before a relaunch keeps its buttons. Pigeon asks for
   no Time Sensitive entitlement, so it never sends `time-sensitive`.
   `Cargo.toml`: the `UNNotificationSound` feature of objc2-user-notifications.

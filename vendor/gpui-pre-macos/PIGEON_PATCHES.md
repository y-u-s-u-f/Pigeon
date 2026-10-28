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

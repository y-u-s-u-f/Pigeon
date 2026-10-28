//! The shared scales: type, spacing and radii, plus the focus ring. Motion
//! lives in `motion.rs`. Views use these instead of literal sizes.
//!
//! Type sizes are in rems so ⌘= / ⌘- scale every text in the app at once
//! (`PigeonApp` sets the window's rem size). Control heights, inline icons
//! and avatars are in rems too, so what holds text grows with it. Spacing
//! and radii stay in pixels, so the layout keeps its rhythm.

use gpui::{
    BoxShadow, DefiniteLength, Div, InteractiveElement, Pixels, Rems, Stateful,
    StatefulInteractiveElement as _, Styled, point, px, relative, rems,
};

use crate::theme::Theme;

/// The rem size at a text scale of 1.
pub const BASE_REM: f32 = 16.;

const fn type_size(px: f32) -> Rems {
    rems(px / BASE_REM)
}

/// 11: captions, keycaps, section headers, the time gutter.
pub const TEXT_XS: Rems = type_size(11.);
/// 12: secondary text, snippets, dates, buttons.
pub const TEXT_SM: Rems = type_size(12.);
/// 13: the UI's body: rows, menus, fields.
pub const TEXT_BASE: Rems = type_size(13.);
/// 14: message bodies.
pub const TEXT_MD: Rems = type_size(14.);
/// 15: pane and sheet titles.
pub const TEXT_LG: Rems = type_size(15.);
/// 17 (macOS Title 2): the reader's subject, event and contact titles.
pub const TEXT_TITLE2: Rems = type_size(17.);
/// 20: week day numbers, which have to fit their circle.
pub const TEXT_XL: Rems = type_size(20.);
/// 22 (macOS Title 1): the calendar's period, the onboarding title.
pub const TEXT_TITLE1: Rems = type_size(22.);

/// Line height for UI text: rows, labels, descriptions, toasts. GPUI's
/// default (1.618) is far looser than macOS.
pub const LINE_UI: DefiniteLength = relative(1.25);
/// Line height for message bodies and other running text.
pub const LINE_READING: DefiniteLength = relative(1.55);

/// The monospace family: search operators, message source, `<pre>`.
/// Menlo ships with every macOS; GPUI panics on a missing family.
pub const MONO: &str = "Menlo";

/// Heights of text-bearing controls, in rems so they grow with ⌘=.
/// 20: keycaps, label chips, compact buttons.
pub const CONTROL_XS: Rems = type_size(20.);
/// 24: small buttons, segments, pop-ups.
pub const CONTROL_SM: Rems = type_size(24.);
/// 28: buttons and fields.
pub const CONTROL_MD: Rems = type_size(28.);
/// 32: search fields, primary buttons.
pub const CONTROL_LG: Rems = type_size(32.);
/// 30: sidebar, shortcut and menu rows.
pub const ROW_SM: Rems = type_size(30.);
/// 38: command palette rows.
pub const ROW_PALETTE: Rems = type_size(38.);

/// Icon sizes. Inline icons use the rem sizes so they track the text next
/// to them; toolbar and rail glyphs stay in pixels.
/// 12: inside chips, compact buttons, metadata.
pub const ICON_SM: Rems = type_size(12.);
/// 14: next to body text, menu rows, fields.
pub const ICON_MD: Rems = type_size(14.);
/// 16: list and pane rows, file icons.
pub const ICON_LG: Rems = type_size(16.);
/// 18: search fields and other large inline glyphs.
pub const ICON_XL: Rems = type_size(18.);
/// 28: empty states and onboarding.
pub const ICON_HERO: Rems = type_size(28.);
/// Toolbar buttons' glyph.
pub const ICON_TOOLBAR: Pixels = px(16.);
/// The rail's glyph.
pub const ICON_RAIL: Pixels = px(18.);

/// Avatar sizes, in rems so initials grow with the text.
/// 20: chips and compact rows (one initial).
pub const AVATAR_XS: Rems = type_size(20.);
/// 24: account switcher, collapsed messages, guests.
pub const AVATAR_SM: Rems = type_size(24.);
/// 32: list rows and message headers.
pub const AVATAR_MD: Rems = type_size(32.);
/// 48: the contact card.
pub const AVATAR_LG: Rems = type_size(48.);

pub const SPACE_2XS: Pixels = px(2.);
pub const SPACE_XS: Pixels = px(4.);
pub const SPACE_SM: Pixels = px(8.);
pub const SPACE_MD: Pixels = px(12.);
pub const SPACE_LG: Pixels = px(16.);
pub const SPACE_XL: Pixels = px(24.);
pub const SPACE_2XL: Pixels = px(32.);

/// Chips, keycaps, small bars.
pub const RADIUS_SM: Pixels = px(6.);
/// Buttons, rows, fields.
pub const RADIUS_MD: Pixels = px(8.);
/// Cards and overlays.
pub const RADIUS_LG: Pixels = px(12.);

/// The smallest mouse target for an icon button (Apple's 28pt).
pub const HIT_TARGET: Pixels = px(28.);

/// Text scale steps for ⌘= and ⌘-.
pub const TEXT_SCALE_STEP: f32 = 0.1;
pub const TEXT_SCALE_RANGE: (f32, f32) = (0.8, 1.6);

/// The keyboard focus ring: 2px of accent inside the element's edge, so it
/// never shifts the layout or gets clipped by a scroll view.
pub fn focus_ring(theme: &Theme) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: theme.accent,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: true,
    }]
}

/// Makes a control reachable with Tab, with the focus ring while it has
/// keyboard focus (not after a click).
pub fn focusable_control(el: Stateful<Div>, theme: &Theme) -> Stateful<Div> {
    let ring = focus_ring(theme);
    el.key_context(crate::actions::CONTROL)
        .focusable()
        .tab_stop(true)
        .focus_visible(move |s| s.shadow(ring))
}

/// Digits of equal width, so counts and dates line up and don't jiggle.
pub fn tabular_figures() -> gpui::FontFeatures {
    gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

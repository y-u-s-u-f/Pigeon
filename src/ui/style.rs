//! The shared scales: type, spacing and radii, plus the focus ring and the
//! entrance motion. Views use these instead of literal sizes.
//!
//! Type sizes are in rems so ⌘= / ⌘- scale every text in the app at once
//! (`PigeonApp` sets the window's rem size). Spacing and radii stay in
//! pixels, so the layout keeps its rhythm while the text grows.

use std::time::Duration;

use gpui::{
    Animation, AnimationElement, AnimationExt as _, BoxShadow, Div, ElementId, InteractiveElement,
    IntoElement, Pixels, Rems, Stateful, StatefulInteractiveElement as _, Styled, ease_out_quint,
    point, px, rems,
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
/// 20: the subject in the reader, the calendar's period.
pub const TEXT_XL: Rems = type_size(20.);

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

/// Overlays and toasts appearing.
pub const MOTION_FAST: Duration = Duration::from_millis(120);
/// Panes opening, closing and resizing.
pub const MOTION_PANE: Duration = Duration::from_millis(160);

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

/// Fades an overlay in while it rises 4px into place. Skipped with Reduce
/// Motion, which GPUI's animations honor.
pub fn enter<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    el: E,
) -> AnimationElement<E> {
    el.with_animation(
        id,
        Animation::new(MOTION_FAST).with_easing(ease_out_quint()),
        |el, delta| el.relative().top(px(4. * (1. - delta))).opacity(delta),
    )
}

/// The ease panes resize with.
pub fn pane_transition() -> gpui::base::motion::Transition {
    gpui::base::motion::Transition::new(MOTION_PANE).ease(ease_out_quint())
}

/// Digits of equal width, so counts and dates line up and don't jiggle.
pub fn tabular_figures() -> gpui::FontFeatures {
    gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

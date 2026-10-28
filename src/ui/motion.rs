//! Motion tokens and helpers. Springs use SwiftUI's response / damping
//! model (`ω = 2π/response`, `k = ω²`, `c = 2ζω`, `m = 1`).
//!
//! Two drivers: `gpui::base::spring(id, target, TOKEN, window, cx)` for
//! anything that opens and closes (sample it on every render, switch the
//! token rather than the id), and `enter(id, rise, el)` for things that only
//! play when they appear. Under Reduce Motion GPUI snaps both; `fade` keeps
//! opacity moving.

use std::time::{Duration, Instant};

use gpui::{
    AnimationExt as _, App, ElementId, IntoElement, Pixels, SpringAnimation,
    SpringAnimationElement, SpringConfig, SpringState, Styled, Window,
    base::{Spring, TransitionId},
    px,
};

/// SwiftUI-style spring parameters as GPUI's physical ones.
pub const fn spring_config(response_s: f32, damping_ratio: f32) -> SpringConfig {
    let frequency = std::f32::consts::TAU / response_s;
    SpringConfig::new(frequency * frequency, 2. * damping_ratio * frequency, 1.)
}

const fn policy(response_ms: u64, damping_ratio: f32, epsilon: f32) -> Spring {
    Spring::new(Duration::from_millis(response_ms))
        .with_damping(damping_ratio)
        .with_epsilon(epsilon)
}

/// Panes and widths, height reveals, chevron rotation. No overshoot.
#[allow(dead_code)] // used by W4-MAIL, W4-CAL
pub const SPRING_PANE: Spring = policy(240, 1., 0.002);
/// Anything arriving: overlays, toasts, cards, the segmented thumb.
pub const SPRING_POP: Spring = policy(200, 0.825, 0.002);
/// Anything leaving. Shorter than the arrival, never overshoots.
#[allow(dead_code)] // used by W4-MAIL, W4-CAL, W4-MISC
pub const SPRING_EXIT: Spring = policy(160, 1., 0.002);
/// Switch knobs, in pixels.
pub const SPRING_CONTROL: Spring = policy(200, 0.7, 0.1);
/// Keyboard and reveal scrolling, in pixels.
#[allow(dead_code)] // used by W4-MAIL, W4-CAL
pub const SPRING_SCROLL: Spring = policy(200, 1., 0.5);
/// The drag ghost gliding between 15-minute slots, in pixels.
#[allow(dead_code)] // used by W4-CAL
pub const SPRING_SNAP: Spring = policy(90, 1., 0.5);

/// `SPRING_POP` for `with_spring`, which takes GPUI's config.
const POP: SpringConfig = spring_config(0.2, 0.825);
const POP_EPSILON: f32 = 0.002;

/// Opacity under Reduce Motion: about 180 ms, no overshoot.
const FADE: SpringConfig = spring_config(0.18, 1.);
const FADE_EPSILON: f32 = 0.005;

/// How far an arriving card travels.
pub const MOTION_RISE: Pixels = px(6.);
/// The calendar's period step nudge (MD-Q6).
#[allow(dead_code)] // used by W4-CAL
pub const MOTION_NUDGE: Pixels = px(12.);
/// Wait before any loading placeholder shows.
#[allow(dead_code)] // used by W4-MAIL
pub const LOADING_DELAY: Duration = Duration::from_millis(300);

/// Which way an arriving card travels: up for things anchored to the
/// bottom (toasts, prompts), down for things hanging under a control or the
/// top edge (popovers, menus, the palette), none for inline content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Up and None used by W2 and W4
pub enum Rise {
    Up,
    Down,
    None,
}

/// A card at progress `p` (0 hidden, 1 in place): faded and offset by up
/// to `MOTION_RISE` on the side it comes from.
pub fn card_motion<E: Styled>(el: E, p: f32, rise: Rise) -> E {
    let travel = MOTION_RISE * (1. - p);
    let el = el.opacity(p.clamp(0., 1.));
    match rise {
        Rise::Up => el.relative().top(travel),
        Rise::Down => el.relative().top(-travel),
        Rise::None => el,
    }
}

/// Plays `card_motion` once when `el` mounts, on `SPRING_POP`; no exit.
/// The id must stay the same across renders.
pub fn enter<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    rise: Rise,
    el: E,
) -> SpringAnimationElement<E> {
    el.with_spring(
        id,
        SpringAnimation::new(POP)
            .to(1f32)
            .from(0f32)
            .with_epsilon(POP_EPSILON),
        move |el, p| card_motion(el, p, rise),
    )
}

#[derive(Clone, Copy)]
struct Fading {
    state: SpringState,
    target: f32,
    at: Instant,
}

/// An opacity that fades from `from` (on first use) towards `target`,
/// whatever Reduce Motion says. Only for when `cx.reduce_motion()` is on:
/// `let p = if cx.reduce_motion() { fade(..) } else { spring(..) }`.
pub fn fade(
    id: impl Into<TransitionId>,
    from: f32,
    target: f32,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let id = ElementId::NamedChild(ElementId::from(id.into()).into(), "fade".into());
    let now = cx.background_executor().now();
    let state = window.use_keyed_state(id, cx, |_, _| Fading {
        state: SpringState {
            position: from,
            velocity: 0.,
        },
        target,
        at: now,
    });
    let last = *state.read(cx);
    if last.state.position == target && last.state.velocity == 0. {
        return target;
    }
    let elapsed = now.saturating_duration_since(last.at).as_secs_f32();
    let mut stepped = FADE.step(last.state, last.target, elapsed);
    let settled = FADE.is_settled(stepped, target, FADE_EPSILON);
    if settled {
        stepped = SpringState {
            position: target,
            velocity: 0.,
        };
    } else {
        window.request_animation_frame();
    }
    state.update(cx, |state, _| {
        *state = Fading {
            state: stepped,
            target,
            at: now,
        }
    });
    stepped.position
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_config_follows_the_response_model() {
        let config = spring_config(0.2, 0.825);
        let (frequency, damping) = config.canonical();
        assert!((frequency - std::f32::consts::TAU / 0.2).abs() < 1e-3);
        assert!((damping - 0.825).abs() < 1e-4);
    }

    #[test]
    fn pop_is_most_of_the_way_in_about_a_tenth_of_a_second() {
        let start = SpringState::default();
        let p = POP.step(start, 1., 0.105).position;
        assert!((0.8..=1.05).contains(&p), "{p}");
        assert!(FADE.is_settled(FADE.step(start, 1., 0.4), 1., FADE_EPSILON));
    }
}

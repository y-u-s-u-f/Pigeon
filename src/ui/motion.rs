//! Motion tokens and helpers. Springs use SwiftUI's response / damping
//! model (`ω = 2π/response`, `k = ω²`, `c = 2ζω`, `m = 1`).
//!
//! Two drivers: `gpui::base::spring(id, target, TOKEN, window, cx)` for
//! anything that opens and closes (sample it on every render, switch the
//! token rather than the id), and `enter(id, rise, el)` for things that only
//! play when they appear. Under Reduce Motion GPUI snaps springs; `fade`,
//! `presence` and `enter` keep opacity moving. `Glide` drives scroll offsets by hand.

use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationElement, AnimationExt as _, AnyElement, App, Bounds, Div, Element,
    ElementId, GlobalElementId, InspectorElementId, InteractiveElement as _, IntoElement, LayoutId,
    Pixels, SpringAnimation, SpringAnimationElement, SpringConfig, SpringState, Styled, Svg,
    Transformation, Window,
    base::{Spring, TransitionId},
    div, px, radians, size,
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
pub const SPRING_PANE: Spring = policy(240, 1., 0.002);
/// Anything arriving: overlays, toasts, cards, the segmented thumb.
pub const SPRING_POP: Spring = policy(200, 0.825, 0.002);
/// Anything leaving. Shorter than the arrival, never overshoots.
pub const SPRING_EXIT: Spring = policy(160, 1., 0.002);
/// Switch knobs, in pixels.
pub const SPRING_CONTROL: Spring = policy(200, 0.7, 0.1);
/// Keyboard and reveal scrolling, in pixels.
pub const SPRING_SCROLL: Spring = policy(200, 1., 0.5);
/// The drag ghost gliding between 15-minute slots, in pixels.
pub const SPRING_SNAP: Spring = policy(90, 1., 0.5);

/// `SPRING_POP`, `SPRING_CONTROL` and `SPRING_PANE` for `with_spring`,
/// which takes GPUI's config.
const POP: SpringConfig = spring_config(0.2, 0.825);
const CONTROL: SpringConfig = spring_config(0.2, 0.7);
const PANE: SpringConfig = spring_config(0.24, 1.);
const POP_EPSILON: f32 = 0.002;

/// Opacity under Reduce Motion: about 180 ms, no overshoot.
const FADE: SpringConfig = spring_config(0.18, 1.);
const FADE_EPSILON: f32 = 0.005;

/// How far an arriving card travels.
pub const MOTION_RISE: Pixels = px(6.);
/// Wait before any loading placeholder shows.
pub const LOADING_DELAY: Duration = Duration::from_millis(300);
/// How long a loading placeholder takes to fade in after the wait.
const LOADING_FADE: Duration = Duration::from_millis(180);

/// Which way an arriving card travels: up for things anchored to the
/// bottom (toasts, prompts), down for things hanging under a control or the
/// top edge (popovers, menus, the palette), none for inline content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
/// Under Reduce Motion it only fades. The id must stay the same across
/// renders.
pub fn enter<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    rise: Rise,
    el: E,
) -> Appear<E> {
    Appear {
        id: id.into(),
        play: Play::Enter(rise),
        element: Some(el),
    }
}

/// A glyph that pops from 1.2× to its size as it mounts, on
/// `SPRING_CONTROL` (MD-027). Mount it only when the change just happened.
pub fn pop(id: impl Into<ElementId>, glyph: Svg) -> SpringAnimationElement<Svg> {
    glyph.with_spring(
        id,
        SpringAnimation::new(CONTROL)
            .to(1f32)
            .from(1.2f32)
            .with_epsilon(POP_EPSILON),
        |glyph, s| glyph.with_transformation(Transformation::scale(size(s, s))),
    )
}

/// A disclosure chevron turned a quarter turn (`turns` of them) as it
/// opens, on `SPRING_PANE` (MD-021). One glyph; mounted, it starts where
/// it's going.
pub fn turn(
    id: impl Into<ElementId>,
    glyph: Svg,
    open: bool,
    turns: f32,
) -> SpringAnimationElement<Svg> {
    glyph.with_spring(
        id,
        SpringAnimation::new(PANE)
            .to(if open { 1f32 } else { 0. })
            .with_epsilon(POP_EPSILON),
        move |glyph, p| {
            glyph.with_transformation(Transformation::rotate(radians(
                p * turns * std::f32::consts::FRAC_PI_2,
            )))
        },
    )
}

/// A loading placeholder: nothing for `LOADING_DELAY`, then a fade in, so
/// a fast load never flashes it (MA-066). Plays as it mounts, Reduce Motion
/// or not: the wait isn't motion.
pub fn after_loading_delay<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    el: E,
) -> Appear<E> {
    Appear {
        id: id.into(),
        play: Play::AfterLoadingDelay,
        element: Some(el),
    }
}

/// A glyph turning while something is under way (MA-066), at 30 frames a
/// second; still under Reduce Motion.
pub fn spinner(id: impl Into<ElementId>, glyph: Svg) -> AnimationElement<Svg> {
    glyph.with_animation(
        id,
        Animation::new(Duration::from_millis(1000))
            .repeat()
            .with_max_fps(30.),
        |glyph, t| {
            glyph.with_transformation(Transformation::rotate(radians(t * std::f32::consts::TAU)))
        },
    )
}

/// `rise`, or none under Reduce Motion, which keeps only the fade.
pub fn moving(rise: Rise, cx: &App) -> Rise {
    if cx.reduce_motion() { Rise::None } else { rise }
}

/// A surface that opens and closes, as progress (0 gone, 1 shown):
/// `SPRING_POP` in, `SPRING_EXIT` out, a fade under Reduce Motion. Sample it
/// on every render, open or not, under one id; a closed surface stays drawn
/// while this is above 0.
pub fn presence(id: impl Into<ElementId>, open: bool, window: &mut Window, cx: &mut App) -> f32 {
    let id = id.into();
    let target = if open { 1. } else { 0. };
    if cx.reduce_motion() {
        return fade(id, target, target, window, cx);
    }
    let policy = if open { SPRING_POP } else { SPRING_EXIT };
    gpui::base::spring(id, target, policy, window, cx)
}

/// Laid over content that is leaving, so it takes no clicks while it goes
/// (its focus has already moved on).
pub fn inert_cover() -> Div {
    div().absolute().inset_0().occlude()
}

/// A value gliding to a target on `SPRING_SCROLL`, held by its owner
/// rather than by an element, so a key press retargets it (keeping its
/// speed, so held keys don't queue) and a wheel can drop it (MD-018).
#[derive(Clone, Copy, Debug)]
pub struct Glide {
    state: SpringState,
    target: f32,
    at: Instant,
    config: SpringConfig,
    epsilon: f32,
}

const SCROLL: SpringConfig = spring_config(0.2, 1.);
const SCROLL_EPSILON: f32 = 0.5;
const EXIT: SpringConfig = spring_config(0.16, 1.);

impl Glide {
    /// Pixels, on `SPRING_SCROLL`.
    pub fn new(from: f32, target: f32) -> Self {
        Self::with(SCROLL, SCROLL_EPSILON, from, target)
    }

    /// Progress from 0 to 1 on `SPRING_EXIT`: something leaving.
    pub fn exit() -> Self {
        Self::with(EXIT, POP_EPSILON, 0., 1.)
    }

    /// Progress from 0 to 1 on `SPRING_PANE`: something growing in.
    pub fn pane() -> Self {
        Self::with(PANE, POP_EPSILON, 0., 1.)
    }

    fn with(config: SpringConfig, epsilon: f32, from: f32, target: f32) -> Self {
        Self {
            state: SpringState {
                position: from,
                velocity: 0.,
            },
            target,
            at: Instant::now(),
            config,
            epsilon,
        }
    }

    /// Where it was at the last step.
    pub fn value(&self) -> f32 {
        self.state.position
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn retarget(&mut self, target: f32) {
        self.target = target;
    }

    /// Where it is now, and whether it has arrived.
    pub fn step(&mut self) -> (f32, bool) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.at).as_secs_f32();
        self.at = now;
        self.state = self.config.step(self.state, self.target, elapsed);
        if self
            .config
            .is_settled(self.state, self.target, self.epsilon)
        {
            self.state = SpringState {
                position: self.target,
                velocity: 0.,
            };
            return (self.target, true);
        }
        (self.state.position, false)
    }
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

#[derive(Clone, Copy)]
enum Play {
    Enter(Rise),
    AfterLoadingDelay,
}

/// What `enter` and `after_loading_delay` return: `el`, drawn at its
/// progress since it mounted.
pub struct Appear<E> {
    id: ElementId,
    play: Play,
    element: Option<E>,
}

struct AppearState {
    spring: SpringState,
    at: Instant,
    mounted: Instant,
}

impl<E: IntoElement + Styled + 'static> IntoElement for Appear<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: IntoElement + Styled + 'static> Element for Appear<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let play = self.play;
        let el = self.element.take().expect("laid out once");
        window.with_element_state(id.expect("Appear has an id"), |state, window| {
            let now = Instant::now();
            let mut state = state.unwrap_or(AppearState {
                spring: SpringState::default(),
                at: now,
                mounted: now,
            });
            let (el, done) = match play {
                Play::Enter(rise) => {
                    let (config, epsilon, rise) = if cx.reduce_motion() {
                        (FADE, FADE_EPSILON, Rise::None)
                    } else {
                        (POP, POP_EPSILON, rise)
                    };
                    let elapsed = now.saturating_duration_since(state.at).as_secs_f32();
                    state.spring = config.step(state.spring, 1., elapsed);
                    let done = config.is_settled(state.spring, 1., epsilon);
                    if done {
                        state.spring = SpringState {
                            position: 1.,
                            velocity: 0.,
                        };
                    }
                    (card_motion(el, state.spring.position, rise), done)
                }
                Play::AfterLoadingDelay => {
                    let shown = now.saturating_duration_since(state.mounted);
                    let p = (shown.saturating_sub(LOADING_DELAY).as_secs_f32()
                        / LOADING_FADE.as_secs_f32())
                    .min(1.);
                    (el.opacity(p), p >= 1.)
                }
            };
            state.at = now;
            if !done {
                window.request_animation_frame();
            }
            let mut el = el.into_any_element();
            ((el.request_layout(window, cx), el), state)
        })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        el: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        el.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        el: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        el.paint(window, cx);
    }
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

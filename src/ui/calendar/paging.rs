//! Period changes that move (MD-044). Two fingers drag the period along
//! with them, the next or previous one beside it, and on release it pages
//! or springs back; the arrows, keys and every other jump slide the new
//! period in a little as it fades in. Under Reduce Motion a swipe pages
//! once it's far enough and every change only fades.

use std::time::Instant;

use chrono::{Datelike as _, NaiveDate};
use gpui::{
    AnyElement, Context, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, ScrollDelta, ScrollWheelEvent, Stateful, Styled, TouchPhase, Window, div, point, px,
    relative,
};

use super::{CalendarView, ViewKind};
use crate::ui::motion::{
    Glide, PAGE_JUMP_TRAVEL as JUMP_TRAVEL, SWIPE_AXIS_LOCK as AXIS_LOCK,
    SWIPE_FLICK_MIN as FLICK_MIN, SWIPE_FLICK_STALE as FLICK_STALE,
    SWIPE_FLICK_VELOCITY as FLICK_VELOCITY, SWIPE_MAX_VELOCITY as MAX_VELOCITY,
    SWIPE_PAGE_SHARE as PAGE_SHARE, SWIPE_REDUCED_PAGE as REDUCED_PAGE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    Horizontal,
    Vertical,
}

/// The axis a gesture keeps, once it has gone far enough to tell.
pub(crate) fn lock_axis(dx: f32, dy: f32) -> Option<Axis> {
    if dx.abs().max(dy.abs()) < AXIS_LOCK {
        None
    } else if dx.abs() > dy.abs() {
        Some(Axis::Horizontal)
    } else {
        Some(Axis::Vertical)
    }
}

/// The period's offset for the fingers' travel: with them, up to a whole
/// width either way, where the period beside has taken its place.
pub(crate) fn follow(dx: f32, width: f32) -> f32 {
    dx.clamp(-width, width)
}

/// On release at `offset` moving at `velocity`: `Some(forward)` pages,
/// `None` springs back. A flick against the travel springs back.
pub(crate) fn release(offset: f32, velocity: f32, width: f32) -> Option<bool> {
    if offset == 0. {
        return None;
    }
    let along = velocity * offset.signum();
    let far = offset.abs() >= width * PAGE_SHARE && along > -FLICK_VELOCITY;
    let flick = offset.abs() >= FLICK_MIN && along >= FLICK_VELOCITY;
    // Fingers to the left (a negative offset) bring the next period in.
    (far || flick).then_some(offset < 0.)
}

/// Where the current period starts settling after a release at `offset`
/// that paged (`Some(forward)`) or not; `None` when it's already in place.
pub(crate) fn settle_from(offset: f32, width: f32, paged: Option<bool>) -> Option<f32> {
    // The new period was a width beyond the old one.
    let from = offset
        + match paged {
            Some(true) => width,
            Some(false) => -width,
            None => 0.,
        };
    (from.abs() >= 0.5).then_some(from)
}

/// A settle's offset after its spring stepped to `stepped`: never past 0
/// from `from`'s side, so the period beside stays the one leaving, nor
/// beyond a width. Done once it reaches 0.
pub(crate) fn settle_clamp(from: f32, stepped: f32, done: bool, width: f32) -> (f32, bool) {
    if done || stepped == 0. || stepped.signum() != from.signum() {
        return (0., true);
    }
    (stepped.clamp(-width, width), false)
}

/// The fingers' velocity after a step of `delta` px over `dt` s, smoothed
/// with the last.
pub(crate) fn track_velocity(last: f32, delta: f32, dt: f32) -> f32 {
    if dt <= 0. {
        return last;
    }
    let now = (delta / dt).clamp(-MAX_VELOCITY, MAX_VELOCITY);
    0.6 * now + 0.4 * last
}

/// A two-finger gesture over the grid, from its start.
pub(crate) struct Swipe {
    total: (f32, f32),
    axis: Option<Axis>,
    velocity: f32,
    at: Instant,
    /// The grid's scroll when it locked sideways, kept while it goes.
    keep_y: Pixels,
    /// Under Reduce Motion: it paged already.
    paged: bool,
}

/// The period moving, if it is.
pub(crate) enum Turn {
    /// Following the fingers.
    Drag { offset: f32 },
    /// Settling at 0 from `from` after a release, in px, with `other`
    /// beside: the period leaving, or the one it sprang back from. The
    /// grid's scroll stays at `keep_y`, so the momentum after a sideways
    /// swipe doesn't scroll it.
    Settle {
        glide: Glide,
        from: f32,
        other: (NaiveDate, f32),
        keep_y: Pixels,
    },
    /// The new period sliding in from `dir` (1 from the right) and fading
    /// in, as progress.
    Jump { dir: f32, glide: Glide },
}

/// What the grid draws this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageFrame {
    /// The current period's offset, in px, positive to the right.
    pub offset: f32,
    pub opacity: f32,
    /// The period drawn beside it: its anchor, and its side (-1 left,
    /// 1 right).
    pub other: Option<(NaiveDate, f32)>,
}

impl PageFrame {
    pub const STILL: Self = PageFrame {
        offset: 0.,
        opacity: 1.,
        other: None,
    };

    pub fn moving(&self) -> bool {
        *self != Self::STILL
    }
}

/// `current`, with `other` beside it on the frame's side, moved and faded
/// as the frame says, in a clip `id` as big as its parent. Each page has
/// its own id, so the two never share element state.
pub(crate) fn page_strip(
    id: impl Into<ElementId>,
    frame: &PageFrame,
    current: AnyElement,
    other: Option<AnyElement>,
) -> Stateful<Div> {
    let side = frame.other.map_or(0., |(_, side)| side);
    let mut pages = div()
        .relative()
        .size_full()
        .left(px(frame.offset))
        .opacity(frame.opacity.clamp(0., 1.))
        .child(div().id("page-current").size_full().child(current));
    if let Some(other) = other.filter(|_| side != 0.) {
        pages = pages.child(
            div()
                .id("page-other")
                .absolute()
                .top_0()
                .size_full()
                .left(relative(side))
                .child(other),
        );
    }
    div().id(id).size_full().overflow_hidden().child(pages)
}

impl CalendarView {
    /// The anchor one period on or back.
    pub(crate) fn stepped_anchor(&self, forward: bool) -> NaiveDate {
        let sign = if forward { 1 } else { -1 };
        match self.view {
            ViewKind::Day => self.anchor + chrono::Duration::days(sign),
            ViewKind::Week => self.anchor + chrono::Duration::days(7 * sign),
            ViewKind::Month => crate::calendar::time::add_months(self.anchor, sign as i32),
        }
    }

    /// The period's width as it slides: the day columns, or the month.
    pub(crate) fn page_width(&self) -> f32 {
        let width = match self.view {
            ViewKind::Month => self.grid_area_width(),
            _ => f32::from(self.grid_bounds.get().size.width),
        };
        width.max(1.)
    }

    /// The period shown, to tell when it changed.
    fn shown_key(&self) -> (ViewKind, NaiveDate) {
        (self.view, self.visible_range().0)
    }

    /// The period shown as one number, for ids that change with it.
    pub(crate) fn period_key(&self) -> usize {
        let view = match self.view {
            ViewKind::Day => 0,
            ViewKind::Week => 1,
            ViewKind::Month => 2,
        };
        (self.visible_range().0.num_days_from_ce() as usize) * 3 + view
    }

    /// Two fingers across the trackpad (MA-080). Sideways, the period
    /// follows them; up and down, the grid scrolls; the axis holds for the
    /// gesture. Mouse wheels, whose steps are lines, and the momentum after
    /// a gesture (moves with no start) don't page.
    pub(crate) fn on_swipe(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let ScrollDelta::Pixels(delta) = event.delta else {
            return;
        };
        let reduced = cx.reduce_motion();
        match event.touch_phase {
            TouchPhase::Started => {
                // Fingers down catch a page still settling: it lands.
                if self.turn.is_some() {
                    self.turn = None;
                    cx.notify();
                }
                self.swipe = Some(Swipe {
                    total: (0., 0.),
                    axis: None,
                    velocity: 0.,
                    at: Instant::now(),
                    keep_y: self.grid_scroll.offset().y,
                    paged: false,
                });
            }
            TouchPhase::Moved => {
                let modal = self.modal();
                let Some(swipe) = &mut self.swipe else {
                    // Momentum after the swipe: the grid keeps still.
                    if let Some(Turn::Settle { keep_y, .. }) = self.turn {
                        let x = self.grid_scroll.offset().x;
                        self.grid_scroll.set_offset(point(x, keep_y));
                    }
                    return;
                };
                let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                swipe.total.0 += dx;
                swipe.total.1 += dy;
                if swipe.axis.is_none() {
                    swipe.axis = lock_axis(swipe.total.0, swipe.total.1);
                    if swipe.axis == Some(Axis::Horizontal) {
                        swipe.keep_y = self.grid_scroll.offset().y;
                    }
                }
                if swipe.axis != Some(Axis::Horizontal) || modal {
                    return;
                }
                let now = Instant::now();
                let dt = now.saturating_duration_since(swipe.at).as_secs_f32();
                swipe.velocity = track_velocity(swipe.velocity, dx, dt);
                swipe.at = now;
                // Sideways only: undo what the grid scrolled meanwhile.
                let x = self.grid_scroll.offset().x;
                self.grid_scroll.set_offset(point(x, swipe.keep_y));
                let total = swipe.total.0;
                if reduced {
                    if !swipe.paged && total.abs() > REDUCED_PAGE {
                        swipe.paged = true;
                        self.page(total < 0., cx);
                    }
                    return;
                }
                let offset = follow(total, self.page_width());
                self.turn = Some(Turn::Drag { offset });
                cx.notify();
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some(swipe) = self.swipe.take() else {
                    return;
                };
                let Some(Turn::Drag { offset }) = self.turn else {
                    return;
                };
                let stale = swipe.at.elapsed().as_secs_f32() > FLICK_STALE;
                let velocity = if stale { 0. } else { swipe.velocity };
                let width = self.page_width();
                let cancelled = event.touch_phase == TouchPhase::Cancelled;
                let paged = release(offset, velocity, width).filter(|_| !cancelled);
                let other = match paged {
                    Some(forward) => {
                        let leaving = self.anchor;
                        self.step(forward, cx);
                        self.shown = Some(self.shown_key());
                        (leaving, if forward { -1. } else { 1. })
                    }
                    None => self.beside(offset),
                };
                self.turn = settle_from(offset, width, paged).map(|from| Turn::Settle {
                    glide: Glide::new(from, 0.).with_velocity(velocity),
                    from,
                    other,
                    keep_y: swipe.keep_y,
                });
                cx.notify();
            }
        }
    }

    /// A swipe paging at once, under Reduce Motion: it fades.
    fn page(&mut self, forward: bool, cx: &mut Context<Self>) {
        if !self.modal() {
            self.step(forward, cx);
        }
    }

    /// Where the period is this frame, moving its springs on; a change of
    /// period since the last frame starts a jump (MD-044). Before the grid
    /// draws.
    pub(crate) fn page_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> PageFrame {
        let key = self.shown_key();
        let reduced = cx.reduce_motion();
        if let Some(shown) = self.shown.replace(key)
            && shown != key
        {
            // A new view comes as it is; a new period of it moves in.
            self.turn = (shown.0 == key.0).then(|| {
                let later = key.1 > shown.1;
                Turn::Jump {
                    dir: if reduced {
                        0.
                    } else if later {
                        1.
                    } else {
                        -1.
                    },
                    glide: if reduced {
                        Glide::exit()
                    } else {
                        Glide::pane()
                    },
                }
            });
        }
        let width = self.page_width();
        // A drag whose gesture went without an end springs back.
        if let Some(Turn::Drag { offset }) = self.turn
            && self.swipe.is_none()
        {
            let offset = follow(offset, width);
            let other = self.beside(offset);
            self.turn = settle_from(offset, width, None).map(|from| Turn::Settle {
                glide: Glide::new(from, 0.),
                from,
                other,
                keep_y: self.grid_scroll.offset().y,
            });
        }
        let frame = match &mut self.turn {
            None => return PageFrame::STILL,
            Some(Turn::Drag { offset }) => {
                let offset = follow(*offset, width);
                PageFrame {
                    offset,
                    opacity: 1.,
                    other: (offset != 0.).then(|| self.beside(offset)),
                }
            }
            Some(Turn::Settle {
                glide, from, other, ..
            }) => {
                let (stepped, done) = glide.step();
                let (offset, done) = settle_clamp(*from, stepped, done, width);
                if done {
                    self.turn = None;
                    return PageFrame::STILL;
                }
                PageFrame {
                    offset,
                    opacity: 1.,
                    other: Some(*other),
                }
            }
            Some(Turn::Jump { dir, glide }) => {
                let dir = *dir;
                let (p, done) = glide.step();
                if done {
                    self.turn = None;
                    return PageFrame::STILL;
                }
                PageFrame {
                    offset: dir * JUMP_TRAVEL * (1. - p),
                    opacity: p,
                    other: None,
                }
            }
        };
        if !matches!(self.turn, Some(Turn::Drag { .. })) {
            window.request_animation_frame();
        }
        frame
    }

    /// The period beside one moved by `offset`, and its side: content moved
    /// left shows the next period on the right.
    fn beside(&self, offset: f32) -> (NaiveDate, f32) {
        let forward = offset < 0.;
        (self.stepped_anchor(forward), if forward { 1. } else { -1. })
    }

    /// Renders `f` as if the period anchored at `anchor` were shown: the
    /// period beside. Its menus stay with the period shown.
    pub(crate) fn as_period<T>(&mut self, anchor: NaiveDate, f: impl FnOnce(&Self) -> T) -> T {
        let anchor = std::mem::replace(&mut self.anchor, anchor);
        let menu = self.menu.take();
        let out = f(self);
        self.anchor = anchor;
        self.menu = menu;
        out
    }
}

/// Over a moving period, so it takes no clicks while it goes; scrolling
/// still reaches the grid and the swipe.
pub(crate) fn moving_cover() -> impl IntoElement {
    div()
        .id("page-moving")
        .absolute()
        .inset_0()
        .block_mouse_except_scroll()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swipe_axis_waits_then_locks_to_the_larger_travel() {
        assert_eq!(lock_axis(3., 2.), None);
        assert_eq!(lock_axis(-8., 3.), Some(Axis::Horizontal));
        assert_eq!(lock_axis(2., -9.), Some(Axis::Vertical));
        // A tie scrolls.
        assert_eq!(lock_axis(7., 7.), Some(Axis::Vertical));
    }

    #[test]
    fn swipe_follows_the_fingers_up_to_a_width() {
        assert_eq!(follow(-120., 700.), -120.);
        assert_eq!(follow(900., 700.), 700.);
        assert_eq!(follow(-900., 700.), -700.);
    }

    #[test]
    fn swipe_pages_when_far_or_flicked_and_springs_back_otherwise() {
        let width = 1000.;
        // Slow and short: back.
        assert_eq!(release(-100., 0., width), None);
        // Slow but past the share: pages, left brings the next.
        assert_eq!(release(-450., 0., width), Some(true));
        assert_eq!(release(450., 0., width), Some(false));
        // A flick its way pages from a short distance.
        assert_eq!(release(-40., -900., width), Some(true));
        assert_eq!(release(40., 900., width), Some(false));
        // A flick back springs back, even from far.
        assert_eq!(release(-600., 900., width), None);
        // Too short to be a flick.
        assert_eq!(release(-5., -2000., width), None);
        assert_eq!(release(0., -2000., width), None);
    }

    /// Every frame of a settle from `from` released at `velocity`, at 60 fps.
    fn settle_frames(from: f32, velocity: f32, width: f32) -> Vec<f32> {
        let mut glide = Glide::new(from, 0.).with_velocity(velocity);
        let mut frames = Vec::new();
        for _ in 0..600 {
            let (stepped, done) = glide.advance(1. / 60.);
            let (offset, done) = settle_clamp(from, stepped, done, width);
            frames.push(offset);
            if done {
                return frames;
            }
        }
        panic!("never settled");
    }

    #[test]
    fn a_full_width_swipe_lands_at_once() {
        let width = 1000.;
        let paged = release(-width, -MAX_VELOCITY, width);
        assert_eq!(paged, Some(true));
        assert_eq!(settle_from(-width, width, paged), None);
        assert_eq!(settle_from(width, width, Some(false)), None);
    }

    #[test]
    fn a_fast_near_full_width_swipe_never_settles_past_zero() {
        let width = 1000.;
        for (offset, velocity) in [
            (-980., -MAX_VELOCITY),
            (980., MAX_VELOCITY),
            (-900., -3000.),
        ] {
            let paged = release(offset, velocity, width);
            let from = settle_from(offset, width, paged).unwrap();
            let frames = settle_frames(from, velocity, width);
            assert_eq!(frames.last(), Some(&0.));
            assert!(
                frames
                    .iter()
                    .all(|&o| o == 0. || o.signum() == from.signum()),
                "{offset}: {frames:?}"
            );
        }
        // Springing back, it stays within a width.
        let frames = settle_frames(-300., -MAX_VELOCITY, width);
        assert!(frames.iter().all(|o| o.abs() <= width));
    }

    #[test]
    fn swipe_velocity_is_smoothed_and_bounded() {
        assert_eq!(track_velocity(100., 10., 0.), 100.);
        let v = track_velocity(0., 10., 0.01);
        assert!((v - 600.).abs() < 1e-3, "{v}");
        let fast = track_velocity(0., 1000., 0.001);
        assert!(fast <= MAX_VELOCITY);
    }

    #[test]
    fn a_still_frame_is_not_moving() {
        assert!(!PageFrame::STILL.moving());
        let sliding = PageFrame {
            offset: -3.,
            ..PageFrame::STILL
        };
        assert!(sliding.moving());
    }
}

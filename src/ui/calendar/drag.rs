//! Dragging in the Day and Week grids: on empty space to make an event, on
//! an event to move it (up and down in time, sideways across days), and on
//! its bottom edge to resize it. Times snap to 15 minutes.
//!
//! A press starts a `GridDrag`; while one is on, a layer over the window
//! takes the mouse, so moves and the release reach the calendar wherever
//! they happen. The grid repaints only when the snapped times change. A
//! press that never moves past a few pixels is a click: on an event it
//! selects and opens it, on empty space it does nothing. A double click on
//! empty space makes a 30-minute event. Escape cancels a drag. Events the
//! user can't change (CAL-22) don't move; pressing them is a click. A move
//! or resize goes to Google at once (CAL-21).

use chrono::{Duration, NaiveDate, NaiveDateTime};
use gpui::{
    AnyElement, Bounds, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Point, Styled, Window, anchored,
    deferred, div, point, prelude::FluentBuilder as _, px,
};

use super::{CalendarView, HOUR_HEIGHT, PromptKind, Selection, SeriesPrompt, ViewKind};
use crate::ui::style::*;
use crate::{
    calendar::{
        Occurrence,
        time::{self, time_range_label},
    },
    theme::Theme,
    ui::popover_shadow,
};

const LABEL_HEIGHT: f32 = 20.;

pub const SNAP_MINUTES: i64 = 15;
/// How far the mouse moves before a press becomes a drag.
const THRESHOLD: f32 = 4.;
const DAY_MINUTES: i64 = 24 * 60;
/// How close to the grid's top or bottom a drag starts scrolling it.
const AUTOSCROLL_EDGE: f32 = 48.;
/// Pixels scrolled per tick per pixel into the edge.
const AUTOSCROLL_SPEED: f32 = 0.35;
const AUTOSCROLL_TICK: std::time::Duration = std::time::Duration::from_millis(16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragKind {
    /// Pressed on empty space, in the slot starting at `anchor`.
    Create { anchor: NaiveDateTime },
    /// Pressed on an event, `grab` after its start.
    Move {
        occurrence: Occurrence,
        grab: Duration,
    },
    /// Pressed on an event's bottom edge, in the column of `day`.
    Resize {
        occurrence: Occurrence,
        day: NaiveDate,
    },
}

pub struct GridDrag {
    pub kind: DragKind,
    /// Where the press was, in the window.
    origin: Point<Pixels>,
    /// Where the mouse is now, for scrolling while it rests near an edge.
    pointer: Point<Pixels>,
    /// Moved past the threshold; until then it's a click.
    pub active: bool,
    /// The times it would set, snapped.
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

impl GridDrag {
    pub fn occurrence(&self) -> Option<Occurrence> {
        match self.kind {
            DragKind::Move { occurrence, .. } | DragKind::Resize { occurrence, .. } => {
                Some(occurrence)
            }
            DragKind::Create { .. } => None,
        }
    }
}

fn floor_snap(minutes: f32) -> i64 {
    (minutes / SNAP_MINUTES as f32).floor() as i64 * SNAP_MINUTES
}

fn round_snap(minutes: f32) -> i64 {
    (minutes / SNAP_MINUTES as f32).round() as i64 * SNAP_MINUTES
}

impl CalendarView {
    /// The days the time grid shows.
    pub(crate) fn grid_days(&self) -> Vec<NaiveDate> {
        let (from, to) = self.visible_range();
        from.iter_days().take_while(|d| *d < to).collect()
    }

    /// The day column under `position` (clamped to the grid's sides) and
    /// how many minutes into that day it is (not clamped).
    fn grid_time(&self, position: Point<Pixels>) -> Option<(NaiveDate, f32)> {
        let bounds: Bounds<Pixels> = self.grid_bounds.get();
        let width = f32::from(bounds.size.width);
        if width <= 0. || self.view == ViewKind::Month {
            return None;
        }
        let days = self.grid_days();
        let x = f32::from(position.x - bounds.origin.x) / width;
        let col = ((x * days.len() as f32).floor() as i64).clamp(0, days.len() as i64 - 1);
        let minutes = f32::from(position.y - bounds.origin.y) / HOUR_HEIGHT * 60.;
        Some((days[col as usize], minutes))
    }

    fn begin_drag(&mut self, kind: DragKind, origin: Point<Pixels>, cx: &mut Context<Self>) {
        let (start, end) = match kind {
            DragKind::Create { anchor } => (anchor, anchor + Duration::minutes(SNAP_MINUTES)),
            DragKind::Move { occurrence, .. } | DragKind::Resize { occurrence, .. } => {
                (occurrence.start, occurrence.end)
            }
        };
        self.drag = Some(GridDrag {
            kind,
            origin,
            pointer: origin,
            active: false,
            start,
            end,
        });
        self.prompt = None;
        self._autoscroll = Some(cx.spawn(async |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSCROLL_TICK).await;
                match this.update(cx, |this, cx| this.autoscroll(cx)) {
                    Ok(true) => {}
                    _ => break,
                }
            }
        }));
        cx.notify();
    }

    /// While a drag rests near the top or bottom of the grid, scroll it,
    /// faster the closer to the edge. False once the drag is over.
    fn autoscroll(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = &self.drag else {
            return false;
        };
        if !drag.active {
            return true;
        }
        let pointer = drag.pointer;
        let viewport = self.grid_scroll.bounds();
        let y = f32::from(pointer.y);
        let top = f32::from(viewport.origin.y);
        let bottom = top + f32::from(viewport.size.height);
        let speed = if y < top + AUTOSCROLL_EDGE {
            -(top + AUTOSCROLL_EDGE - y).min(AUTOSCROLL_EDGE)
        } else if y > bottom - AUTOSCROLL_EDGE {
            (y - (bottom - AUTOSCROLL_EDGE)).min(AUTOSCROLL_EDGE)
        } else {
            return true;
        } * AUTOSCROLL_SPEED;
        let offset = self.grid_scroll.offset();
        let max = f32::from(self.grid_scroll.max_offset().y);
        let new_y = (f32::from(offset.y) - speed).clamp(-max, 0.);
        let delta = new_y - f32::from(offset.y);
        if delta != 0. {
            self.grid_scroll.set_offset(point(offset.x, px(new_y)));
            // The grid moves with the scroll before it next paints.
            let mut bounds = self.grid_bounds.get();
            bounds.origin.y += px(delta);
            self.grid_bounds.set(bounds);
            self.update_drag(pointer, cx);
            cx.notify();
        }
        true
    }

    /// A press on empty grid space.
    pub(crate) fn press_grid(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_grid(window, cx);
        let Some((day, minutes)) = self.grid_time(event.position) else {
            return;
        };
        if event.click_count == 2 {
            let slot = ((minutes / 30.).floor() as i64 * 30).clamp(0, DAY_MINUTES - 30);
            let start = time::midnight(day) + Duration::minutes(slot);
            self.drag = None;
            self.create_event(start, start + Duration::minutes(30), false, window, cx);
            return;
        }
        let slot = floor_snap(minutes).clamp(0, DAY_MINUTES - SNAP_MINUTES);
        let anchor = time::midnight(day) + Duration::minutes(slot);
        self.begin_drag(DragKind::Create { anchor }, event.position, cx);
    }

    /// A press on an event, or on its bottom edge with `resize`.
    pub(crate) fn press_occurrence(
        &mut self,
        occurrence: Occurrence,
        resize: bool,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_grid(window, cx);
        let Some((day, minutes)) = self.grid_time(event.position) else {
            return;
        };
        let kind = if resize {
            DragKind::Resize { occurrence, day }
        } else {
            let pointer = time::midnight(day) + Duration::minutes(minutes.round() as i64);
            DragKind::Move {
                occurrence,
                grab: pointer - occurrence.start,
            }
        };
        self.begin_drag(kind, event.position, cx);
    }

    pub(crate) fn drag_to(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The release happened somewhere we didn't hear of.
        if event.pressed_button != Some(MouseButton::Left) {
            return self.finish_drag(window, cx);
        }
        let Some(drag) = &mut self.drag else { return };
        drag.pointer = event.position;
        let moved = event.position - drag.origin;
        let distance = f32::from(moved.x).hypot(f32::from(moved.y));
        if !drag.active && distance < THRESHOLD {
            return;
        }
        self.update_drag(event.position, cx);
    }

    /// The times the drag would set with the mouse at `position`.
    fn update_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else { return };
        // Events the user can't change stay put; the press is a click.
        if let Some(occurrence) = drag.occurrence()
            && !self
                .data
                .events
                .get(occurrence.event)
                .is_some_and(|e| self.can_edit(&e.id))
        {
            return;
        }
        let Some((day, minutes)) = self.grid_time(position) else {
            return;
        };
        let snap = Duration::minutes(SNAP_MINUTES);
        let (start, end) = match drag.kind {
            DragKind::Create { anchor } => {
                let day = anchor.date();
                let slot = floor_snap(minutes).clamp(0, DAY_MINUTES - SNAP_MINUTES);
                let current = time::midnight(day) + Duration::minutes(slot);
                if current >= anchor {
                    (anchor, current + snap)
                } else {
                    (current, anchor + snap)
                }
            }
            DragKind::Move { occurrence, grab } => {
                let pointer = time::midnight(day) + Duration::minutes(minutes.round() as i64);
                let wanted = pointer - grab;
                let into = time::minutes_into(wanted.date(), wanted) as f32;
                let start = time::midnight(wanted.date()) + Duration::minutes(round_snap(into));
                (start, start + (occurrence.end - occurrence.start))
            }
            DragKind::Resize { occurrence, day } => {
                let end = time::midnight(day) + Duration::minutes(round_snap(minutes));
                (occurrence.start, end.max(occurrence.start + snap))
            }
        };
        let drag = self.drag.as_mut().expect("checked above");
        if !drag.active || (start, end) != (drag.start, drag.end) {
            drag.active = true;
            drag.start = start;
            drag.end = end;
            cx.notify();
        }
    }

    /// The release: make, move or resize the event, or treat it as a click.
    pub(crate) fn finish_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        cx.notify();
        match (drag.kind, drag.active) {
            (DragKind::Create { .. }, true) => {
                self.create_event(drag.start, drag.end, false, window, cx);
            }
            (DragKind::Create { .. }, false) => {}
            (DragKind::Move { occurrence, .. } | DragKind::Resize { occurrence, .. }, false) => {
                self.click_occurrence(occurrence, window, cx)
            }
            (DragKind::Move { occurrence, .. } | DragKind::Resize { occurrence, .. }, true) => {
                let Some(event) = self.data.events.get(occurrence.event) else {
                    return;
                };
                let id = event.id.clone();
                let moved = (drag.start, drag.end) != (occurrence.start, occurrence.end);
                if moved && event.is_series() {
                    // Asked first; drawn at the new times meanwhile.
                    self.prompt = Some(SeriesPrompt {
                        event_id: id,
                        occurrence: occurrence.start,
                        kind: PromptKind::Move {
                            start: drag.start,
                            end: drag.end,
                        },
                    });
                    return;
                }
                if moved {
                    self.retime_occurrence(&id, occurrence.start, drag.start, drag.end, window, cx);
                    // Drags go at once.
                    self.flush_edit(cx);
                }
                // The panel, when open, follows the selection.
                if self.panel.is_some() {
                    self.open_event(&id, drag.start, false, window, cx);
                }
                self.selected = Some(Selection {
                    event_id: id,
                    start: drag.start,
                });
            }
        }
    }

    /// Escape during a drag.
    pub(crate) fn cancel_drag(&mut self, cx: &mut Context<Self>) -> bool {
        let cancelled = self.drag.take().is_some();
        if cancelled {
            cx.notify();
        }
        cancelled
    }

    /// Whether `occurrence` is being moved or resized, or waits to be, so
    /// its block fades.
    pub(crate) fn is_dragged(&self, occurrence: &Occurrence) -> bool {
        self.drag
            .as_ref()
            .is_some_and(|d| d.active && d.occurrence().as_ref() == Some(occurrence))
            || self.prompt.as_ref().is_some_and(|p| {
                matches!(p.kind, PromptKind::Move { .. })
                    && p.occurrence == occurrence.start
                    && self.data.events[occurrence.event].id == p.event_id
            })
    }

    /// What the grid previews: the drag under way, or a move waiting for
    /// "This event" or "All events". Its times, and the event it moves.
    fn ghost(&self) -> Option<(NaiveDateTime, NaiveDateTime, Option<usize>)> {
        if let Some(drag) = self.drag.as_ref().filter(|d| d.active) {
            return Some((drag.start, drag.end, drag.occurrence().map(|o| o.event)));
        }
        let prompt = self.prompt.as_ref()?;
        let PromptKind::Move { start, end } = prompt.kind else {
            return None;
        };
        Some((start, end, self.data.event_ix(&prompt.event_id)))
    }

    /// The layer over the window that takes the mouse during a drag.
    pub(crate) fn render_drag_layer(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let size = window.viewport_size();
        let kind = self.drag.as_ref().map(|d| (d.kind, d.active));
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id("drag-layer")
                    .occlude()
                    .w(size.width)
                    .h(size.height)
                    .map(|d| match kind {
                        Some((DragKind::Resize { .. }, _)) => d.cursor_ns_resize(),
                        Some((DragKind::Move { .. }, true)) => d.cursor_grabbing(),
                        Some((DragKind::Move { .. }, false)) => d.cursor_pointer(),
                        _ => d.cursor_default(),
                    })
                    .on_mouse_move(cx.listener(Self::drag_to))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.finish_drag(window, cx)),
                    ),
            ),
        )
        .with_priority(3)
        .into_any_element()
    }

    /// The preview of a drag on `date`'s column: the event at its new
    /// times, or the one being made.
    pub(crate) fn render_drag_ghost(&self, date: NaiveDate, theme: &Theme) -> Option<AnyElement> {
        let (start, end, event) = self.ghost()?;
        let top = time::minutes_into(date, start).max(0);
        let bottom = time::minutes_into(date, end).min(DAY_MINUTES);
        if bottom <= top {
            return None;
        }
        let (title, color) = match event {
            Some(ix) => {
                let event = self.data.events.get(ix)?;
                (event.display_title().to_string(), self.data.color_of(event))
            }
            None => ("New event".to_string(), theme.accent),
        };
        let top_px = top as f32 / 60. * HOUR_HEIGHT + 1.;
        let height = ((bottom - top) as f32 / 60. * HOUR_HEIGHT - 2.).max(18.);
        let compact = height < 34.;
        let text = theme.cal_selected_text;
        let time = time_range_label(start.time(), end.time());
        // The times, floating above the block (below it at the top of the
        // day), so the snap reads at a glance.
        let label = (date == start.date() || top == 0).then(|| {
            let above = top_px >= LABEL_HEIGHT + 4.;
            div()
                .absolute()
                .left(px(2.))
                .map(|d| {
                    if above {
                        d.top(px(top_px - LABEL_HEIGHT - 4.))
                    } else {
                        d.top(px(top_px + height + 4.))
                    }
                })
                .h(px(LABEL_HEIGHT))
                .px(SPACE_SM)
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .bg(theme.text)
                .text_color(theme.overlay_bg.alpha(1.))
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .shadow(popover_shadow(theme))
                .child(time.clone())
        });
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .child(
                    div()
                        .absolute()
                        .top(px(top_px))
                        .left(px(2.))
                        .right(px(6.))
                        .h(px(height))
                        .px(SPACE_SM)
                        .py(px(if compact { 0. } else { 4. }))
                        .flex()
                        .when(compact, |d| d.items_center().gap(SPACE_SM))
                        .when(!compact, |d| d.flex_col())
                        .overflow_hidden()
                        .rounded(RADIUS_SM)
                        .bg(theme.event_selected_fill(color))
                        .border_1()
                        .border_color(text.alpha(0.5))
                        .shadow(vec![gpui::BoxShadow {
                            color: theme.overlay_shadow.alpha(0.45),
                            offset: point(px(0.), px(6.)),
                            blur_radius: px(16.),
                            spread_radius: px(-2.),
                            inset: false,
                        }])
                        .text_size(TEXT_SM)
                        .line_height(gpui::relative(1.25))
                        .text_color(text)
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        ),
                )
                .children(label)
                .into_any_element(),
        )
    }
}

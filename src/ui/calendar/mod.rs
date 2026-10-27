//! The calendar: a header, the Day, Week or Month grid, and the event panel
//! docked on the right. `CalendarView` owns the calendar's state and handles
//! its actions; the files beside this one render it.
//!
//! What the mouse drags in the grid (`drag.rs`) and the invite card in the
//! reader go through:
//! - `create_event` makes an event over a time range and opens it.
//! - `retime_occurrence` moves or resizes one occurrence (its whole series;
//!   pass the series' `start` as the occurrence to set the series itself).
//! - `set_rsvp` answers an invite for the user.
//!
//! Every change also goes to the backend as a `CalOp` (`queue`, `patched`).
//! Edits settle first: one patch per event once the panel leaves it or
//! after `EDIT_DEBOUNCE` idle (`flush_edit`); drags go at once. A change
//! guests would see asks first whether to email them (`GuestPrompt`).
//! Events the user can't change (`Access`, CAL-22) only take an answer.
//!
//! A repeating event changes as a whole series unless the user picks "This
//! event": the occurrence then becomes an event of its own and the series
//! lists it as an exception (`detach`). Drags and deletes of an occurrence
//! ask first (`SeriesPrompt`); the panel asks after the first edit.

pub mod agenda;
pub mod drag;
pub mod event_panel;
pub mod header;
pub mod mini_month;
pub mod month_view;
pub mod week_view;

use std::{cell::Cell, rc::Rc, time::Duration as StdDuration};

use chrono::{Duration, Local, NaiveDate, NaiveDateTime};
use gpui::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Bounds, Context, Entity,
    EventEmitter, FocusHandle, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled, Subscription, Task, Window, actions, anchored,
    component::input::{InputEvent, InputState},
    deferred, div, ease_out_quint, point,
    prelude::FluentBuilder as _,
    px,
};

use self::{drag::GridDrag, event_panel::EventPanel};
use crate::ui::style::*;
use crate::{
    actions::*,
    backend::{
        Backend, CalOp, OpId, SendUpdates,
        calendar::{Access, access, asks_guests, has_guests, instance_id, is_organizer},
    },
    calendar::{
        CalendarData, Event, Occurrence, RsvpStatus, SeriesScope, occurrences,
        time::{self, LocalZone, add_months, month_grid, next_half_hour, week_start},
    },
    contacts::SharedContactBook,
    theme::Theme,
    ui::{keycap, popover_shadow},
};

actions!(
    calendar,
    [
        /// The day after the selected one.
        NextDay,
        /// The day before the selected one.
        PrevDay,
        /// Jump to a typed or picked date.
        GoToDate,
        ToggleWeekNumbers,
    ]
);

/// Height of an hour in the Day and Week grids.
pub const HOUR_HEIGHT: f32 = 50.;
/// The narrowest the grid and header get beside a docked event panel; in a
/// narrower window the panel floats over the grid instead.
const GRID_MIN_BESIDE_PANEL: f32 = 680.;
/// The hour the grids open scrolled to.
const FIRST_VISIBLE_HOUR: f32 = 7.5;
/// How far next and previous event look past the period shown.
const SEARCH_DAYS: i64 = 62;
/// How long edits and answers to an invite settle before they go.
const EDIT_DEBOUNCE: StdDuration = StdDuration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Day,
    Week,
    Month,
}

/// The selected occurrence: its event, and which time it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub event_id: String,
    pub start: NaiveDateTime,
}

/// The open dropdown, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// Show and hide calendars, from the header.
    Calendars,
    /// The open event's calendar.
    EventCalendar,
    Reminder,
    /// The date field and month under the header's title.
    GoToDate,
}

/// A question about one occurrence of a series, waiting for an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesPrompt {
    /// The series.
    pub event_id: String,
    /// Where the rule puts the occurrence.
    pub occurrence: NaiveDateTime,
    pub kind: PromptKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Delete,
    /// Dropped at new times, still drawn there until answered.
    Move {
        start: NaiveDateTime,
        end: NaiveDateTime,
    },
}

impl PromptKind {
    /// The question and the answers offered, first one taken by enter.
    fn choices(self) -> (&'static str, &'static [(SeriesScope, &'static str)]) {
        match self {
            PromptKind::Delete => (
                "Delete repeating event",
                &[
                    (SeriesScope::This, "This event"),
                    (SeriesScope::Following, "This and following"),
                    (SeriesScope::All, "All events"),
                ],
            ),
            PromptKind::Move { .. } => (
                "Move repeating event",
                &[
                    (SeriesScope::This, "This event"),
                    (SeriesScope::All, "All events"),
                ],
            ),
        }
    }
}

/// An event changed and not yet handed to the backend.
#[derive(Clone, Debug)]
struct PendingEdit {
    id: String,
    /// As it was before.
    before: Event,
}

/// "Send updates to guests?", waiting for an answer (CAL-13).
#[derive(Clone, Debug)]
pub struct GuestPrompt {
    pub event_id: String,
    pub kind: GuestAsk,
}

#[allow(clippy::large_enum_variant)] // One at a time.
#[derive(Clone, Debug)]
pub enum GuestAsk {
    /// Edits or a new event: the ops to queue, and the event as it was, to
    /// put back on Cancel (`None`: a new event, which goes).
    Save {
        ops: Vec<CalOp>,
        before: Option<Event>,
    },
    Delete {
        occurrence: NaiveDateTime,
        scope: SeriesScope,
    },
}

impl GuestPrompt {
    fn question(&self) -> &'static str {
        match self.kind {
            GuestAsk::Save { before: None, .. } => "Send invitations to guests?",
            GuestAsk::Save { .. } => "Send updates to guests?",
            GuestAsk::Delete { .. } => "Send cancellation to guests?",
        }
    }
}

const GUEST_CHOICES: &[(SendUpdates, &str)] = &[
    (SendUpdates::All, "Send"),
    (SendUpdates::None, "Don't send"),
];

/// The op with `send` as its `sendUpdates`.
fn with_send(op: CalOp, send: SendUpdates) -> CalOp {
    match op {
        CalOp::Insert { event, .. } => CalOp::Insert {
            event,
            send_updates: send,
        },
        CalOp::Patch {
            id,
            before,
            after,
            scope,
            ..
        } => CalOp::Patch {
            id,
            before,
            after,
            scope,
            send_updates: send,
        },
        op => op,
    }
}

/// What a delete took out, to put back on undo.
#[derive(Clone, Debug, Default)]
pub struct Removal {
    /// Events removed, each with the index it had, by index.
    removed: Vec<(usize, Event)>,
    /// The series as it was, when the delete changed it rather than
    /// removing it (an exception, or an earlier end).
    series_before: Option<Event>,
    select: Option<Selection>,
    /// The delete, while the backend holds it back.
    op: Option<OpId>,
}

pub enum CalendarEvent {
    /// The app shows a toast that can undo it.
    Deleted { message: String, removal: Removal },
}

pub struct CalendarView {
    pub data: CalendarData,
    pub view: ViewKind,
    /// The selected day, which the views are built around: the day shown,
    /// or one in the week or month shown. The selected event is on it.
    pub anchor: NaiveDate,
    pub selected: Option<Selection>,
    pub panel: Option<EventPanel>,
    /// The panel sliding shut, drawn until its animation ends.
    closing_panel: Option<EventPanel>,
    pub menu: Option<Menu>,
    pub prompt: Option<SeriesPrompt>,
    pub show_week_numbers: bool,
    pub focus_handle: FocusHandle,
    pub grid_scroll: ScrollHandle,
    /// The mouse drag in the time grid, if one is on.
    pub drag: Option<GridDrag>,
    /// Where the day columns of the time grid were last painted, in the
    /// window, scrolled with them: maps the mouse to a day and a time.
    grid_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The month the mini months show, after their arrows moved it away
    /// from the selected day's.
    mini_month: Option<NaiveDate>,
    date_input: Entity<InputState>,
    book: SharedContactBook,
    backend: Entity<Backend>,
    /// The event made last, while it's open and may still be dropped for
    /// having no title.
    new_event: Option<String>,
    /// Edits settling (`flush_edit`).
    pending: Option<PendingEdit>,
    /// A new event that goes in once it has a title.
    unsent: Option<String>,
    _edit_timer: Option<Task<()>>,
    /// An answer to an invite, settling.
    pending_rsvp: Option<(String, RsvpStatus)>,
    _rsvp_timer: Option<Task<()>>,
    pub guest_prompt: Option<GuestPrompt>,
    /// Repaints every half minute so the current-time line moves.
    _tick: Task<()>,
    _closing: Option<Task<()>>,
    _autoscroll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CalendarEvent> for CalendarView {}

/// A key for an occurrence's elements, unique per event, start and `part`
/// (a day it's drawn on, say).
pub fn occurrence_key(occurrence: &Occurrence, part: i64) -> u64 {
    let minutes = occurrence.start.and_utc().timestamp() / 60;
    ((occurrence.event as u64) << 40) ^ ((minutes as u64) << 8) ^ (part as u64 & 0xff)
}

/// The start of an event's next occurrence from today, or of its last one.
pub fn next_start(event: &Event) -> NaiveDateTime {
    let today = time::midnight(today());
    occurrences(
        std::slice::from_ref(event),
        |_| true,
        today,
        today + Duration::days(400),
    )
    .first()
    .map_or(event.start, |o| o.start)
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

impl CalendarView {
    pub fn new(
        data: CalendarData,
        book: SharedContactBook,
        backend: Entity<Backend>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tick = cx.spawn(async |this, cx| {
            loop {
                cx.background_executor()
                    .timer(StdDuration::from_secs(30))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let date_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Go to date: nov 12, next fri, tomorrow")
        });
        let date_events = cx.subscribe_in(&date_input, window, Self::on_date_input_event);
        let this = Self {
            data,
            view: ViewKind::Week,
            anchor: today(),
            selected: None,
            panel: None,
            closing_panel: None,
            menu: None,
            prompt: None,
            show_week_numbers: false,
            focus_handle: cx.focus_handle(),
            grid_scroll: ScrollHandle::new(),
            drag: None,
            grid_bounds: Rc::default(),
            mini_month: None,
            date_input,
            book,
            backend,
            new_event: None,
            pending: None,
            unsent: None,
            _edit_timer: None,
            pending_rsvp: None,
            _rsvp_timer: None,
            guest_prompt: None,
            _tick: tick,
            _closing: None,
            _autoscroll: None,
            _subscriptions: vec![date_events],
        };
        this.scroll_to_morning();
        this
    }

    pub fn me(&self) -> &str {
        &self.data.account.email
    }

    /// The days on screen, end exclusive.
    pub fn visible_range(&self) -> (NaiveDate, NaiveDate) {
        match self.view {
            ViewKind::Day => (self.anchor, self.anchor + Duration::days(1)),
            ViewKind::Week => {
                let start = week_start(self.anchor);
                (start, start + Duration::days(7))
            }
            ViewKind::Month => {
                let (start, weeks) = month_grid(self.anchor);
                (start, start + Duration::days(7 * weeks as i64))
            }
        }
    }

    /// Occurrences of the visible calendars' events from `from` to `to`.
    pub fn occurrences_between(&self, from: NaiveDateTime, to: NaiveDateTime) -> Vec<Occurrence> {
        occurrences(&self.data.events, |e| self.data.is_visible(e), from, to)
    }

    /// Occurrences of the visible calendars' events on screen.
    pub fn visible_occurrences(&self) -> Vec<Occurrence> {
        let (from, to) = self.visible_range();
        self.occurrences_between(time::midnight(from), time::midnight(to))
    }

    pub fn event(&self, id: &str) -> Option<&Event> {
        self.data.event_ix(id).map(|ix| &self.data.events[ix])
    }

    fn event_mut(&mut self, id: &str) -> Option<&mut Event> {
        let ix = self.data.event_ix(id)?;
        Some(&mut self.data.events[ix])
    }

    pub fn is_selected(&self, occurrence: &Occurrence) -> bool {
        self.selected.as_ref().is_some_and(|s| {
            s.start == occurrence.start && s.event_id == self.data.events[occurrence.event].id
        })
    }

    fn selection_of(&self, occurrence: &Occurrence) -> Selection {
        Selection {
            event_id: self.data.events[occurrence.event].id.clone(),
            start: occurrence.start,
        }
    }

    fn scroll_to_morning(&self) {
        self.grid_scroll
            .set_offset(point(px(0.), px(-FIRST_VISIBLE_HOUR * HOUR_HEIGHT)));
    }

    /// Scroll the time grid so `[start, end)` minutes into the day show.
    fn reveal_minutes(&self, start: i64, end: i64) {
        let viewport = self.grid_scroll.bounds().size.height;
        if viewport <= px(0.) {
            return;
        }
        let top = px(start as f32 / 60. * HOUR_HEIGHT);
        let bottom = px(end as f32 / 60. * HOUR_HEIGHT);
        let offset = self.grid_scroll.offset();
        let visible_top = -offset.y;
        let margin = px(HOUR_HEIGHT / 2.);
        let y = if top < visible_top + margin {
            top - margin
        } else if bottom > visible_top + viewport - margin {
            (bottom + margin - viewport).min(top - margin)
        } else {
            return;
        };
        let max = self.grid_scroll.max_offset().y;
        self.grid_scroll
            .set_offset(point(offset.x, -y.clamp(px(0.), max.max(px(0.)))));
    }

    pub fn set_view(&mut self, view: ViewKind, cx: &mut Context<Self>) {
        self.drag = None;
        if self.view != view {
            let from_month = self.view == ViewKind::Month;
            self.view = view;
            if from_month {
                self.scroll_to_morning();
            }
        }
        cx.notify();
    }

    /// Show `date` in the Day view.
    pub fn open_day(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        self.anchor = date;
        self.set_view(ViewKind::Day, cx);
    }

    /// Select a day without leaving the period shown: a click on a day in
    /// the Month grid's edge weeks keeps the month.
    fn select_day_in_view(&mut self, date: NaiveDate) {
        let same_period = match self.view {
            ViewKind::Day => date == self.anchor,
            ViewKind::Week => week_start(date) == week_start(self.anchor),
            ViewKind::Month => time::month_start(date) == time::month_start(self.anchor),
        };
        if same_period {
            self.anchor = date;
        }
    }

    /// Jump to `date`, keeping the view.
    pub fn go_to_date(&mut self, date: NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        self.drag = None;
        self.anchor = date;
        self.mini_month = None;
        if self.menu == Some(Menu::GoToDate) {
            self.menu = None;
            window.focus(&self.focus_handle, cx);
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|s| s.start.date() != date)
        {
            self.selected = None;
        }
        cx.notify();
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.drag = None;
        let sign = if forward { 1 } else { -1 };
        self.anchor = match self.view {
            ViewKind::Day => self.anchor + Duration::days(sign),
            ViewKind::Week => self.anchor + Duration::days(7 * sign),
            ViewKind::Month => add_months(self.anchor, sign as i32),
        };
        self.mini_month = None;
        cx.notify();
    }

    /// Left and right: the day before or after. A selected event moves to
    /// the event nearest its time on the new day, if it has one.
    fn step_day(&mut self, days: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.drag = None;
        self.mini_month = None;
        let day = self.anchor + Duration::days(days);
        self.anchor = day;
        if let Some(selected) = self.selected.clone() {
            let minute = time::minutes_into(selected.start.date(), selected.start);
            let from = time::midnight(day);
            let nearest = self
                .occurrences_between(from, from + Duration::days(1))
                .into_iter()
                .min_by_key(|o| {
                    let spanning = self.data.events[o.event].is_spanning();
                    let distance = (time::minutes_into(day, o.start).max(0) - minute).abs();
                    (spanning, distance)
                });
            match nearest {
                Some(occurrence) => {
                    let selection = self.selection_of(&occurrence);
                    self.select(selection, window, cx);
                }
                None => {
                    self.selected = None;
                    if self.panel.is_some() {
                        self.close_panel(window, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    // Selection.

    fn select(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        let Some((duration, spanning)) = self
            .event(&selection.event_id)
            .map(|e| (e.duration(), e.is_spanning()))
        else {
            return;
        };
        // The selected day follows; a bar that started before the period
        // shown keeps the period.
        let (from, to) = self.visible_range();
        let end = selection.start + duration;
        let day = selection.start.date();
        self.anchor = if time::midnight(from) < end && selection.start < time::midnight(to) {
            day.clamp(from, to - Duration::days(1))
        } else {
            day
        };
        if !spanning && self.view != ViewKind::Month {
            let start = time::minutes_into(day, selection.start).max(0);
            let end = time::minutes_into(day, end).min(24 * 60);
            self.reveal_minutes(start, end);
        }
        // With the panel open, it follows the selection.
        if self.panel.is_some() {
            self.open_event(&selection.event_id, selection.start, false, window, cx);
        }
        self.selected = Some(selection);
        cx.notify();
    }

    /// Down and up: the next or previous event in time, past the period
    /// shown if need be.
    fn select_adjacent(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (from, to) = self.visible_range();
        let reach = Duration::days(SEARCH_DAYS);
        let all =
            self.occurrences_between(time::midnight(from) - reach, time::midnight(to) + reach);
        if all.is_empty() {
            return;
        }
        let next = match all.iter().position(|o| self.is_selected(o)) {
            Some(ix) if forward => ix + 1,
            Some(ix) => match ix.checked_sub(1) {
                Some(ix) => ix,
                None => return,
            },
            // Nothing selected yet: from now on today, or from the start of
            // the selected day.
            None => {
                let day = time::midnight(self.anchor);
                let pivot = if self.anchor == today() { now() } else { day };
                if forward {
                    all.iter()
                        .position(|o| o.start >= pivot)
                        .unwrap_or(all.len() - 1)
                } else {
                    let before = if self.anchor == today() {
                        pivot
                    } else {
                        day + Duration::days(1)
                    };
                    all.iter().rposition(|o| o.start < before).unwrap_or(0)
                }
            }
        };
        let Some(occurrence) = all.get(next) else {
            return;
        };
        let selection = self.selection_of(occurrence);
        self.select(selection, window, cx);
    }

    /// Select an occurrence and open it in the panel; clicking an event does
    /// this.
    pub fn click_occurrence(
        &mut self,
        occurrence: Occurrence,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.selection_of(&occurrence);
        self.select_day_in_view(selection.start.date());
        self.selected = Some(selection.clone());
        self.open_event(&selection.event_id, selection.start, false, window, cx);
    }

    /// Open an event's occurrence in the panel. With `focus_title`, the
    /// title field takes focus (for a new event); otherwise the calendar
    /// keeps it, so shortcuts go on working.
    pub fn open_event(
        &mut self,
        id: &str,
        occurrence: NaiveDateTime,
        focus_title: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let already_open = self
            .panel
            .as_ref()
            .is_some_and(|p| p.event_id == id && p.occurrence == occurrence);
        if !already_open {
            self.leave_panel(window, cx);
            let Some(event) = self.event(id).cloned() else {
                return;
            };
            self.menu = None;
            self.closing_panel = None;
            self.panel = Some(EventPanel::new(&event, occurrence, window, cx));
        }
        if focus_title {
            if let Some(panel) = &self.panel {
                let title = panel.title.clone();
                title.update(cx, |input, cx| input.focus(window, cx));
            }
        } else {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    /// The panel stops showing its event: typed dates and times apply, and
    /// a new event left without a title goes.
    fn leave_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_panel(window, cx);
        let new_event = self.new_event.take();
        let mut never_went = false;
        if let Some(id) = &new_event
            && self.event(id).is_some_and(|e| e.title.trim().is_empty())
            && self.unsent.as_ref() == Some(id)
        {
            // Never went in; nothing to take back.
            self.unsent = None;
            never_went = true;
        }
        self.flush_edit(cx);
        self.flush_rsvp(cx);
        let Some(id) = new_event else {
            return;
        };
        let untitled = self.event(&id).is_some_and(|e| e.title.trim().is_empty());
        if untitled && let Some(ix) = self.data.event_ix(&id) {
            self.data.events.remove(ix);
            if !never_went {
                // It went in before its title was cleared.
                self.queue(
                    CalOp::Delete {
                        id: id.clone(),
                        occurrence: None,
                        scope: SeriesScope::All,
                        send_updates: SendUpdates::None,
                    },
                    cx,
                );
            }
            if self.selected.as_ref().is_some_and(|s| s.event_id == id) {
                self.selected = None;
            }
            if self.panel.as_ref().is_some_and(|p| p.event_id == id) {
                self.panel = None;
            }
        }
    }

    pub fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_panel(window, cx);
        // Slides shut, unless its event just went.
        self.closing_panel = self.panel.take();
        if self.closing_panel.is_some() {
            self._closing = Some(cx.spawn(async |this, cx| {
                cx.background_executor().timer(MOTION_PANE).await;
                this.update(cx, |this, cx| {
                    this.closing_panel = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        self.menu = None;
        if self
            .prompt
            .as_ref()
            .is_some_and(|p| p.kind == PromptKind::Delete)
        {
            self.prompt = None;
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    // Changing events.

    /// 32 hex digits, which Google takes as an event id (CAL-11).
    fn new_id(&mut self) -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    /// Hand the backend a change.
    pub(crate) fn queue(&self, op: CalOp, cx: &mut Context<Self>) -> OpId {
        self.backend.update(cx, |b, cx| b.calendar(op, cx))
    }

    /// What the user may do with `event` (CAL-22).
    pub fn access_of(&self, event: &Event) -> Access {
        let role = self
            .data
            .calendar(&event.calendar)
            .map(|c| c.access_role)
            .unwrap_or_default();
        access(event, role, self.me())
    }

    pub fn can_edit(&self, id: &str) -> bool {
        self.event(id)
            .is_some_and(|e| self.access_of(e) == Access::Edit)
    }

    /// The event changed from `before`. The change settles, then goes as
    /// one patch (`flush_edit`); a change to another event first sends the
    /// one settling.
    pub(crate) fn patched(&mut self, before: Event, cx: &mut Context<Self>) {
        // A new event goes in whole.
        if self.unsent.as_deref() != Some(before.id.as_str()) {
            if self.pending.as_ref().is_some_and(|p| p.id != before.id) {
                self.flush_edit(cx);
            }
            if self.pending.is_none() {
                self.pending = Some(PendingEdit {
                    id: before.id.clone(),
                    before,
                });
            }
        }
        self.settle_edit(cx);
    }

    /// Send the edit in `EDIT_DEBOUNCE`, unless more come.
    fn settle_edit(&mut self, cx: &mut Context<Self>) {
        self._edit_timer = Some(cx.spawn(async |this, cx| {
            cx.background_executor().timer(EDIT_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                // A repeating event waits for "This event" or "All events".
                if !this.asking_scope() {
                    this.flush_edit(cx);
                }
            })
            .ok();
        }));
    }

    /// Hand the backend the settling edit now: the new event (once it has
    /// a title), or a move and a patch from how it was. A change to a
    /// repeating event still waiting for "This event" or "All events" never
    /// goes: it's taken back (closing the panel cancels it).
    pub(crate) fn flush_edit(&mut self, cx: &mut Context<Self>) {
        self.drop_unscoped_edit(cx);
        self.read_panel_text(cx);
        self.send_edit(cx);
    }

    /// `flush_edit` without reading the panel's text: for when the panel's
    /// text belongs to another event than the one settling.
    fn send_edit(&mut self, cx: &mut Context<Self>) {
        self._edit_timer = None;
        let unsent = self
            .unsent
            .as_ref()
            .and_then(|id| self.event(id))
            .filter(|e| !e.title.trim().is_empty())
            .cloned();
        if let Some(event) = unsent {
            self.unsent = None;
            let ops = vec![CalOp::Insert {
                event: event.clone(),
                send_updates: SendUpdates::None,
            }];
            self.save(ops, None, &event, cx);
        }
        let Some(PendingEdit { id, before }) = self.pending.take() else {
            return;
        };
        let Some(after) = self.event(&id).cloned() else {
            return;
        };
        if before == after {
            return;
        }
        let mut ops = Vec::new();
        if after.calendar != before.calendar {
            ops.push(CalOp::Move {
                id: id.clone(),
                to_calendar: after.calendar.clone(),
            });
        }
        ops.push(CalOp::Patch {
            id,
            before: before.clone(),
            after: after.clone(),
            scope: SeriesScope::All,
            send_updates: SendUpdates::None,
        });
        self.save(ops, Some(before), &after, cx);
    }

    /// Queue `ops`, which save `after`, asking first about the guests.
    fn save(
        &mut self,
        ops: Vec<CalOp>,
        before: Option<Event>,
        after: &Event,
        cx: &mut Context<Self>,
    ) {
        if asks_guests(before.as_ref(), after, self.me()) {
            let event_id = after.id.clone();
            self.ask_guests(
                GuestPrompt {
                    event_id,
                    kind: GuestAsk::Save { ops, before },
                },
                cx,
            );
        } else {
            for op in ops {
                self.queue(op, cx);
            }
        }
    }

    /// The panel's text fields hold what was typed last, even if a reload
    /// from the store replaced the event meanwhile.
    fn read_panel_text(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = &self.panel else { return };
        let settling = self
            .pending
            .as_ref()
            .is_some_and(|p| p.id == panel.event_id)
            || self.unsent.as_ref() == Some(&panel.event_id);
        if !settling {
            return;
        }
        let (id, title, location, notes) = (
            panel.event_id.clone(),
            panel.title.read(cx).value().to_string(),
            panel.location_text(cx),
            panel.notes_text(cx),
        );
        if let Some(event) = self.event_mut(&id) {
            event.title = title;
            event.location = location;
            event.description = notes;
        }
    }

    /// Ask about emailing the guests. A question still open (about an
    /// earlier change) is answered "Don't send".
    fn ask_guests(&mut self, prompt: GuestPrompt, cx: &mut Context<Self>) {
        if let Some(GuestPrompt {
            kind: GuestAsk::Save { ops, .. },
            ..
        }) = self.guest_prompt.take()
        {
            for op in ops {
                self.queue(op, cx);
            }
        }
        self.guest_prompt = Some(prompt);
        cx.notify();
    }

    /// Send or Don't send; `None` is Cancel, which puts the event back.
    pub fn answer_guests(
        &mut self,
        send: Option<SendUpdates>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.guest_prompt.take() else {
            return;
        };
        let id = prompt.event_id.clone();
        match (prompt.kind, send) {
            (GuestAsk::Save { ops, .. }, Some(send)) => {
                for op in ops {
                    self.queue(with_send(op, send), cx);
                }
            }
            (GuestAsk::Save { before, .. }, None) => match before {
                Some(before) => {
                    if let Some(event) = self.event_mut(&id) {
                        *event = before.clone();
                    }
                    // The panel's fields show it as it was again.
                    if let Some(panel) = &self.panel
                        && panel.event_id == id
                    {
                        let occurrence = if before.is_series() {
                            panel.occurrence
                        } else {
                            before.start
                        };
                        self.panel = Some(EventPanel::new(&before, occurrence, window, cx));
                        if let Some(selected) = &mut self.selected
                            && selected.event_id == id
                        {
                            selected.start = occurrence;
                        }
                    }
                }
                None => {
                    self.data.events.retain(|e| e.id != id);
                    if self.selected.as_ref().is_some_and(|s| s.event_id == id) {
                        self.selected = None;
                    }
                    if self.panel.as_ref().is_some_and(|p| p.event_id == id) {
                        self.panel = None;
                    }
                }
            },
            (GuestAsk::Delete { occurrence, scope }, Some(send)) => {
                self.delete_with(&id, occurrence, scope, Some(send), window, cx);
            }
            (GuestAsk::Delete { .. }, None) => {}
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Make an event over `[start, end)` and open it with its title focused.
    /// All-day events take whole days. Returns the new event's id.
    pub fn create_event(
        &mut self,
        start: NaiveDateTime,
        end: NaiveDateTime,
        all_day: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        // The primary calendar when shown, else the first shown one the
        // user can write to. Before the first sync there may be none.
        let calendars = &self.data.calendars;
        let writable = |c: &&crate::calendar::Calendar| {
            matches!(
                c.access_role,
                crate::calendar::AccessRole::Owner | crate::calendar::AccessRole::Writer
            )
        };
        let calendar = calendars
            .iter()
            .find(|c| c.primary && c.visible)
            .or_else(|| calendars.iter().filter(writable).find(|c| c.visible))
            .or_else(|| calendars.iter().find(writable))
            .or(calendars.first())
            .map(|c| c.id.clone())
            .unwrap_or_default();
        let (start, end) = if all_day {
            let first = start.date();
            let last = (end - Duration::minutes(1)).date().max(first);
            (
                time::midnight(first),
                time::midnight(last + Duration::days(1)),
            )
        } else {
            (start, end.max(start + Duration::minutes(15)))
        };
        let id = self.new_id();
        let event = Event {
            id: id.clone(),
            calendar,
            start,
            end,
            all_day,
            organizer: self.data.me(),
            organizer_self: true,
            reminder_minutes: Some(10),
            busy: true,
            ..Event::default()
        };
        self.data.events.push(event);
        // It goes in once it has a title (`flush_edit`).
        self.flush_edit(cx);
        self.unsent = Some(id.clone());
        if !all_day && self.view != ViewKind::Month {
            let day = start.date();
            self.reveal_minutes(
                time::minutes_into(day, start),
                time::minutes_into(day, end).min(24 * 60),
            );
        }
        self.select_day_in_view(start.date());
        self.selected = Some(Selection {
            event_id: id.clone(),
            start,
        });
        self.open_event(&id, start, true, window, cx);
        self.new_event = Some(id.clone());
        id
    }

    /// Move or resize the occurrence of `id` starting at `occurrence` to
    /// `[start, end)`. A recurring event moves as a whole series by the same
    /// amount. The selection and the panel follow.
    pub fn retime_occurrence(
        &mut self,
        id: &str,
        occurrence: NaiveDateTime,
        start: NaiveDateTime,
        end: NaiveDateTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let end =
            end.max(start + Duration::minutes(if self.is_all_day(id) { 24 * 60 } else { 15 }));
        let Some(before) = self.event(id).cloned() else {
            return;
        };
        if self.access_of(&before) != Access::Edit {
            self.sync_panel_times(window, cx);
            return;
        }
        let event = self.event_mut(id).expect("checked above");
        let shift = start - occurrence;
        event.start += shift;
        event.end = event.start + (end - start);
        // The series' exceptions and end are where the rule put them.
        for exception in &mut event.exceptions {
            *exception += shift;
        }
        if let Some(until) = event.recurrence.as_mut().and_then(|r| r.until.as_mut()) {
            *until += shift;
        }
        self.patched(before, cx);
        if let Some(selected) = &mut self.selected
            && selected.event_id == id
            && selected.start == occurrence
        {
            selected.start = start;
        }
        if let Some(panel) = &mut self.panel
            && panel.event_id == id
        {
            panel.occurrence = start;
            self.sync_panel_times(window, cx);
        }
        cx.notify();
    }

    /// Take one occurrence of a series out into an event of its own, at
    /// `[start, end)`, with the fields of `edited` (the series as the user
    /// changed it); the series skips it from then on. Returns the new
    /// event's id: Google's id for that occurrence (CAL-6).
    fn detach(
        &mut self,
        series_id: &str,
        occurrence: NaiveDateTime,
        start: NaiveDateTime,
        end: NaiveDateTime,
        edited: Event,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let before = self.event(series_id)?.clone();
        let mut id = instance_id(series_id, occurrence, before.all_day, &LocalZone::system());
        if self.event(&id).is_some() {
            id = self.new_id();
        }
        let occurrence_of = |series: &Event, start, end| Event {
            id: id.clone(),
            start,
            end,
            recurrence: None,
            exceptions: Vec::new(),
            recurring_event_id: Some(series_id.to_string()),
            original_start: Some(occurrence),
            ..series.clone()
        };
        // The occurrence as the rule had it, and as changed.
        let as_was = occurrence_of(&before, occurrence, occurrence + before.duration());
        self.data.events.push(occurrence_of(&edited, start, end));
        self.event_mut(series_id)?.exceptions.push(occurrence);
        self.patched(before, cx);
        // The panel may still show the series as edited: that's the
        // occurrence's now, not the series'.
        self.send_edit(cx);
        self.pending = Some(PendingEdit {
            id: id.clone(),
            before: as_was,
        });
        self.settle_edit(cx);
        Some(id)
    }

    fn is_all_day(&self, id: &str) -> bool {
        self.event(id).is_some_and(|e| e.all_day)
    }

    /// Answer an invite for the user: their own attendee entry takes
    /// `status`. Does nothing when they aren't invited.
    pub fn set_rsvp(&mut self, id: &str, status: RsvpStatus, cx: &mut Context<Self>) {
        let me = self.data.account.email.clone();
        if let Some(event) = self.event_mut(id)
            && let Some(attendee) = event
                .attendees
                .iter_mut()
                .find(|a| a.email.eq_ignore_ascii_case(&me))
        {
            attendee.status = status;
            // The last answer goes, once it settles (CAL-14).
            if self.pending_rsvp.as_ref().is_some_and(|(p, _)| p != id) {
                self.flush_rsvp(cx);
            }
            self.pending_rsvp = Some((id.to_string(), status));
            self._rsvp_timer = Some(cx.spawn(async |this, cx| {
                cx.background_executor().timer(EDIT_DEBOUNCE).await;
                this.update(cx, |this, cx| this.flush_rsvp(cx)).ok();
            }));
            cx.notify();
        }
    }

    fn flush_rsvp(&mut self, cx: &mut Context<Self>) {
        self._rsvp_timer = None;
        if let Some((id, status)) = self.pending_rsvp.take() {
            self.queue(CalOp::Rsvp { id, status }, cx);
        }
    }

    /// Delete an event outright: a series goes with every occurrence.
    pub fn delete_event(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let start = self.event(id).map(|e| e.start);
        if let Some(start) = start {
            self.delete_occurrence(id, start, SeriesScope::All, window, cx);
        }
    }

    /// Delete the occurrence of `id` at `occurrence` (where its rule puts
    /// it), the ones after it too, or the whole series. An event that
    /// doesn't repeat just goes.
    pub fn delete_occurrence(
        &mut self,
        id: &str,
        occurrence: NaiveDateTime,
        scope: SeriesScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_with(id, occurrence, scope, None, window, cx);
    }

    /// `delete_occurrence`, telling the guests or not; `None` asks first
    /// when there are guests. Someone else's event is declined and removed
    /// from the user's calendar instead (CAL-22).
    fn delete_with(
        &mut self,
        id: &str,
        occurrence: NaiveDateTime,
        scope: SeriesScope,
        send: Option<SendUpdates>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_unscoped_edit(window, cx);
        let Some(event) = self.event(id) else {
            return;
        };
        if self.access_of(event) == Access::ReadOnly {
            return;
        }
        let me = self.me().to_string();
        let declines = !is_organizer(event, &me);
        let title = event.display_title().to_string();
        let scope = match scope {
            _ if !event.is_series() => SeriesScope::All,
            SeriesScope::Following if occurrence <= event.start => SeriesScope::All,
            scope => scope,
        };
        let never_went = self.unsent.as_deref() == Some(id);
        if send.is_none() && !declines && !never_went && has_guests(event, &me) {
            self.ask_guests(
                GuestPrompt {
                    event_id: id.to_string(),
                    kind: GuestAsk::Delete { occurrence, scope },
                },
                cx,
            );
            self.prompt = None;
            return;
        }
        let send = send.unwrap_or_default();
        // What's settling goes first, unless the event goes with it.
        if never_went {
            self.unsent = None;
        }
        if scope == SeriesScope::All && self.pending.as_ref().is_some_and(|p| p.id == id) {
            self.pending = None;
        }
        self.flush_edit(cx);
        if self.pending_rsvp.as_ref().is_some_and(|(p, _)| p == id) {
            self.pending_rsvp = None;
        }
        if self.event(id).is_none() {
            return;
        }
        let mut removal = Removal {
            select: Some(Selection {
                event_id: id.to_string(),
                start: occurrence,
            }),
            ..Removal::default()
        };
        // Its occurrences changed on their own go with the ones deleted.
        let goes = |e: &Event| {
            e.id == id
                || e.recurring_event_id.as_deref() == Some(id)
                    && match scope {
                        SeriesScope::All => true,
                        SeriesScope::Following => e.original_start >= Some(occurrence),
                        SeriesScope::This => false,
                    }
        };
        let day = time::date_label(occurrence.date(), today());
        let message = match scope {
            _ if declines => format!("Declined “{title}” and removed it"),
            SeriesScope::All => format!("Deleted “{title}”"),
            SeriesScope::This => format!("Deleted “{title}” on {day}"),
            SeriesScope::Following => format!("Deleted “{title}” from {day} on"),
        };
        if scope != SeriesScope::All {
            let series = self.event_mut(id).expect("checked above");
            removal.series_before = Some(series.clone());
            if scope == SeriesScope::This {
                series.exceptions.push(occurrence);
            } else if let Some(rule) = &mut series.recurrence {
                rule.until = Some(occurrence);
            }
        }
        let mut ix = 0;
        while ix < self.data.events.len() {
            let event = &self.data.events[ix];
            if goes(event) && !(scope != SeriesScope::All && event.id == id) {
                removal
                    .removed
                    .push((ix + removal.removed.len(), self.data.events.remove(ix)));
            } else {
                ix += 1;
            }
        }
        let gone = |sel_id: &str, start: NaiveDateTime| {
            removal.removed.iter().any(|(_, e)| e.id == sel_id)
                || sel_id == id
                    && match scope {
                        SeriesScope::All => true,
                        SeriesScope::This => start == occurrence,
                        SeriesScope::Following => start >= occurrence,
                    }
        };
        if let Some(panel) = &self.panel
            && gone(&panel.event_id, panel.occurrence)
        {
            self.panel = None;
            self.menu = None;
            window.focus(&self.focus_handle, cx);
        }
        if let Some(selected) = &self.selected
            && gone(&selected.event_id, selected.start)
        {
            self.selected = None;
        }
        if self.new_event.as_deref() == Some(id) {
            self.new_event = None;
        }
        self.prompt = None;
        if !never_went {
            removal.op = Some(self.queue(
                CalOp::Delete {
                    id: id.to_string(),
                    occurrence: Some(occurrence),
                    scope,
                    send_updates: send,
                },
                cx,
            ));
        }
        cx.emit(CalendarEvent::Deleted { message, removal });
        cx.notify();
    }

    /// Undo a delete: the events and the series come back as they were,
    /// the occurrence selected.
    pub fn restore(&mut self, removal: Removal, window: &mut Window, cx: &mut Context<Self>) {
        // Inside the undo window the delete never went; past it, put the
        // events back.
        let cancelled = removal
            .op
            .is_some_and(|op| self.backend.update(cx, |b, _| b.cancel(op)));
        if !cancelled {
            // Someone else's event was only declined; it can't be made
            // again here.
            let me = self.me().to_string();
            for (_, event) in removal.removed.iter().filter(|(_, e)| is_organizer(e, &me)) {
                self.queue(
                    CalOp::Insert {
                        event: event.clone(),
                        send_updates: SendUpdates::None,
                    },
                    cx,
                );
            }
            if let Some(before) = &removal.series_before
                && let Some(now) = self.event(&before.id).cloned()
            {
                self.queue(
                    CalOp::Patch {
                        id: before.id.clone(),
                        before: now,
                        after: before.clone(),
                        scope: SeriesScope::All,
                        send_updates: SendUpdates::None,
                    },
                    cx,
                );
            }
        }
        if let Some(before) = removal.series_before
            && let Some(series) = self.event_mut(&before.id)
        {
            *series = before;
        }
        for (index, event) in removal.removed {
            let index = index.min(self.data.events.len());
            self.data.events.insert(index, event);
        }
        if let Some(select) = removal.select {
            self.select_day_in_view(select.start.date());
            self.selected = Some(select);
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub fn toggle_calendar(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(calendar) = self.data.calendars.get_mut(ix) {
            calendar.visible = !calendar.visible;
            let op = CalOp::SetSelected {
                calendar: calendar.id.clone(),
                selected: calendar.visible,
            };
            self.queue(op, cx);
            cx.notify();
        }
    }

    /// Jump to an event from anywhere (the palette): its next occurrence
    /// from today, or its last one, selected and open.
    pub fn reveal_event(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(event) = self.data.events.get(ix) else {
            return;
        };
        let id = event.id.clone();
        let start = next_start(event);
        if let Some(calendar) = self
            .data
            .calendar_ix(&event.calendar)
            .filter(|&c| !self.data.calendars[c].visible)
        {
            self.data.calendars[calendar].visible = true;
            let op = CalOp::SetSelected {
                calendar: self.data.calendars[calendar].id.clone(),
                selected: true,
            };
            self.queue(op, cx);
        }
        self.anchor = start.date();
        if self.view == ViewKind::Month {
            self.view = ViewKind::Week;
        }
        self.selected = Some(Selection {
            event_id: id.clone(),
            start,
        });
        if !event.is_spanning() {
            let day = start.date();
            let minutes = time::minutes_into(day, start);
            self.reveal_minutes(minutes, minutes + event.duration().num_minutes());
        }
        self.open_event(&id, start, false, window, cx);
    }

    // Repeating events: "This event" or "All events".

    /// Ask what a delete of the occurrence should take.
    fn ask_delete(&mut self, id: &str, occurrence: NaiveDateTime, cx: &mut Context<Self>) {
        self.prompt = Some(SeriesPrompt {
            event_id: id.to_string(),
            occurrence,
            kind: PromptKind::Delete,
        });
        cx.notify();
    }

    /// Answer the open prompt.
    pub fn answer_prompt(
        &mut self,
        scope: SeriesScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut prompt) = self.prompt.take() else {
            return;
        };
        // A change in the panel still waiting for its own answer doesn't
        // go along (CAL-6).
        if let Some((series, shift)) = self.cancel_unscoped_edit(window, cx)
            && series == prompt.event_id
        {
            prompt.occurrence -= shift;
        }
        let id = prompt.event_id.as_str();
        match prompt.kind {
            PromptKind::Delete => self.delete_occurrence(id, prompt.occurrence, scope, window, cx),
            PromptKind::Move { start, end } if scope == SeriesScope::This => {
                let Some(series) = self.event(id).cloned() else {
                    return;
                };
                let Some(new_id) = self.detach(id, prompt.occurrence, start, end, series, cx)
                else {
                    return;
                };
                self.flush_edit(cx);
                let selection = Selection {
                    event_id: new_id.clone(),
                    start,
                };
                if self.panel.is_some() {
                    self.open_event(&new_id, start, false, window, cx);
                }
                self.selected = Some(selection);
            }
            PromptKind::Move { start, end } => {
                self.retime_occurrence(id, prompt.occurrence, start, end, window, cx);
                self.flush_edit(cx);
                if self.panel.is_some() {
                    self.open_event(id, start, false, window, cx);
                }
                self.selected = Some(Selection {
                    event_id: id.to_string(),
                    start,
                });
            }
        }
        cx.notify();
    }

    // Action handlers.

    fn show_day(&mut self, _: &ShowDayView, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected) = &self.selected
            && self.view != ViewKind::Day
        {
            let (from, to) = self.visible_range();
            let day = selected.start.date();
            if day >= from && day < to {
                self.anchor = day;
            }
        }
        self.set_view(ViewKind::Day, cx);
    }

    fn show_week(&mut self, _: &ShowWeekView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(ViewKind::Week, cx);
    }

    fn show_month(&mut self, _: &ShowMonthView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(ViewKind::Month, cx);
    }

    fn go_to_today(&mut self, _: &GoToToday, _: &mut Window, cx: &mut Context<Self>) {
        self.anchor = today();
        self.mini_month = None;
        if self.view != ViewKind::Month {
            let minutes = time::minutes_into(today(), now());
            self.reveal_minutes(minutes, minutes + 60);
        }
        cx.notify();
    }

    fn next_period(&mut self, _: &NextPeriod, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn prev_period(&mut self, _: &PrevPeriod, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn next_day(&mut self, _: &NextDay, window: &mut Window, cx: &mut Context<Self>) {
        self.step_day(1, window, cx);
    }

    fn prev_day(&mut self, _: &PrevDay, window: &mut Window, cx: &mut Context<Self>) {
        self.step_day(-1, window, cx);
    }

    fn select_next(&mut self, _: &SelectNextEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(true, window, cx);
    }

    fn select_prev(&mut self, _: &SelectPrevEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(false, window, cx);
    }

    fn toggle_go_to_date(&mut self, _: &GoToDate, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_date_picker(window, cx);
    }

    pub fn toggle_date_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu == Some(Menu::GoToDate) {
            self.menu = None;
            window.focus(&self.focus_handle, cx);
        } else {
            self.menu = Some(Menu::GoToDate);
            self.mini_month = None;
            let input = self.date_input.clone();
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn on_date_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The month under the field follows what's typed.
            InputEvent::Change => {
                self.mini_month = self.typed_date(cx).map(time::month_start);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                if let Some(date) = self.typed_date(cx) {
                    self.go_to_date(date, window, cx);
                }
            }
            _ => {}
        }
    }

    /// The date typed in the go-to-date field, if it reads as one.
    pub fn typed_date(&self, cx: &gpui::App) -> Option<NaiveDate> {
        time::parse_date(self.date_input.read(cx).value().as_ref(), today())
    }

    fn toggle_week_numbers(
        &mut self,
        _: &ToggleWeekNumbers,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_week_numbers = !self.show_week_numbers;
        cx.notify();
    }

    /// `c`: an hour from the next half hour today, or from 9 on another
    /// selected day.
    fn new_event(&mut self, _: &NewEvent, window: &mut Window, cx: &mut Context<Self>) {
        let today = today();
        let day = self.anchor;
        let start = if day == today {
            next_half_hour(now())
        } else {
            time::at(day, 9, 0)
        };
        self.create_event(start, start + Duration::hours(1), false, window, cx);
    }

    /// Enter: answers an open prompt with its first choice, or opens the
    /// selected event.
    fn open_selected(&mut self, _: &OpenEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.guest_prompt.is_some() {
            return self.answer_guests(Some(SendUpdates::All), window, cx);
        }
        if self.prompt.is_some() {
            return self.answer_prompt(SeriesScope::This, window, cx);
        }
        if self.selected.is_none() {
            self.select_adjacent(true, window, cx);
        }
        if let Some(selected) = self.selected.clone() {
            self.open_event(&selected.event_id, selected.start, false, window, cx);
        }
    }

    fn delete_selected(&mut self, _: &DeleteEvent, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .selected
            .as_ref()
            .map(|s| (s.event_id.clone(), s.start))
            .or_else(|| {
                self.panel
                    .as_ref()
                    .map(|p| (p.event_id.clone(), p.occurrence))
            });
        let Some((id, start)) = target else {
            return;
        };
        if self
            .event(&id)
            .is_none_or(|e| self.access_of(e) == Access::ReadOnly)
        {
            return;
        }
        if self.event(&id).is_some_and(|e| e.is_series()) {
            self.ask_delete(&id, start, cx);
        } else {
            self.delete_event(&id, window, cx);
        }
    }

    /// Escape: the innermost open thing closes: a drag, a prompt, a dropdown
    /// or picker, then the panel, then the selection.
    fn close_event(&mut self, _: &CloseEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.cancel_drag(cx) {
            return;
        }
        if self.guest_prompt.is_some() {
            self.answer_guests(None, window, cx);
        } else if self.prompt.take().is_some() {
            cx.notify();
        } else if let Some(menu) = self.menu.take() {
            if menu == Menu::GoToDate {
                window.focus(&self.focus_handle, cx);
            }
            cx.notify();
        } else if self.dismiss_panel_popup(cx) {
            cx.notify();
        } else if self.panel.is_some() {
            self.close_panel(window, cx);
        } else if self.selected.take().is_some() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn toggle_calendars_menu(
        &mut self,
        _: &ToggleCalendarsMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_menu(Menu::Calendars, cx);
        window.focus(&self.focus_handle, cx);
    }

    pub fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }

    /// A click on the grid takes focus back from the panel's fields.
    fn focus_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            window.focus(&self.focus_handle, cx);
        }
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// An invisible layer over the window under an open dropdown: a click
    /// anywhere else closes the dropdown, and does nothing more.
    pub(crate) fn menu_backdrop(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let size = window.viewport_size();
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id("menu-backdrop")
                    .occlude()
                    .w(size.width)
                    .h(size.height)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            if this.menu.take() == Some(Menu::GoToDate) {
                                window.focus(&this.focus_handle, cx);
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            ),
        )
        .with_priority(1)
        .into_any_element()
    }

    /// The width beside the rail.
    fn content_width(window: &Window) -> f32 {
        f32::from(window.viewport_size().width - crate::app::RAIL_WIDTH)
    }

    /// Whether the open event panel sits beside the grid (true) or over it.
    pub fn panel_docked(&self, window: &Window) -> bool {
        Self::content_width(window) - event_panel::PANEL_WIDTH >= GRID_MIN_BESIDE_PANEL
    }

    /// Whether a floating panel goes on the left, so it doesn't cover the
    /// selected event's day.
    fn panel_on_left(&self, window: &Window) -> bool {
        let Some(selected) = &self.selected else {
            return false;
        };
        let (from, to) = self.visible_range();
        let day = selected.start.date().clamp(from, to - Duration::days(1));
        let (left, ix) = match self.view {
            ViewKind::Day => return false,
            ViewKind::Week => (week_view::GUTTER, (day - from).num_days()),
            ViewKind::Month => (0., (day - from).num_days() % 7),
        };
        let width = Self::content_width(window);
        let column = (width - left) / 7.;
        let center = left + (ix as f32 + 0.5) * column;
        center > width - event_panel::PANEL_WIDTH
    }

    /// What VoiceOver reads for an occurrence: title, day and time.
    pub fn occurrence_label(&self, occurrence: &Occurrence) -> String {
        let event = &self.data.events[occurrence.event];
        let day = time::date_label(occurrence.start.date(), today());
        if event.all_day {
            format!("{}, {day}, all day", event.display_title())
        } else {
            let times = time::time_range_label(occurrence.start.time(), occurrence.end.time());
            format!("{}, {day}, {times}", event.display_title())
        }
    }

    pub fn calendar_name(&self, event: &Event) -> SharedString {
        self.data
            .calendar(&event.calendar)
            .map(|c| c.name.clone().into())
            .unwrap_or_default()
    }

    fn render_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let occurrences = self.visible_occurrences();
        match self.view {
            ViewKind::Day => self.render_day(&occurrences, window, cx),
            ViewKind::Week => self.render_time_grid(self.grid_days(), &occurrences, window, cx),
            ViewKind::Month => self.render_month(&occurrences, window, cx),
        }
    }

    /// The buttons answering a prompt about a repeating event.
    pub(crate) fn render_prompt_choices(
        &self,
        kind: PromptKind,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.render_scope_buttons(kind.choices().1, true, theme, cx, Self::answer_prompt)
    }

    /// A row of buttons, one per choice; the first is the default (which
    /// enter takes, with `enter_hint`).
    pub(crate) fn render_scope_buttons<T: Copy + 'static>(
        &self,
        choices: &'static [(T, &'static str)],
        enter_hint: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
        pick: fn(&mut Self, T, &mut Window, &mut Context<Self>),
    ) -> gpui::Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(SPACE_XS)
            .children(choices.iter().enumerate().map(|(ix, &(scope, label))| {
                let first = ix == 0;
                focusable_control(div().id(("prompt-choice", ix)), theme)
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .h(HIT_TARGET)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::MEDIUM)
                    .map(|d| {
                        if first {
                            d.bg(theme.accent)
                                .text_color(theme.accent_text)
                                .hover(|s| s.opacity(0.9))
                        } else {
                            d.bg(theme.chip_bg)
                                .text_color(theme.text)
                                .hover(|s| s.bg(theme.hover_bg))
                        }
                    })
                    .child(label)
                    .when(first && enter_hint, |d| {
                        d.child(
                            div()
                                .text_size(TEXT_XS)
                                .text_color(theme.accent_text.alpha(0.75))
                                .child("↵"),
                        )
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| pick(this, scope, window, cx)))
            }))
    }

    /// The prompt over the bottom of the grid, when the panel isn't the
    /// place for it.
    fn render_floating_prompt(
        &self,
        prompt: &SeriesPrompt,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (question, _) = prompt.kind.choices();
        let title = self
            .event(&prompt.event_id)
            .map(|e| e.display_title().to_string())
            .unwrap_or_default();
        let buttons = self.render_prompt_choices(prompt.kind, theme, cx);
        self.render_prompt_card(question, title, buttons, theme, cx, |this, _, cx| {
            this.prompt = None;
            cx.notify();
        })
    }

    /// "Send updates to guests?" over the bottom of the grid (CAL-13).
    fn render_guest_prompt(
        &self,
        prompt: &GuestPrompt,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = self
            .event(&prompt.event_id)
            .map(|e| e.display_title().to_string())
            .unwrap_or_default();
        let buttons =
            self.render_scope_buttons(GUEST_CHOICES, true, theme, cx, |this, send, window, cx| {
                this.answer_guests(Some(send), window, cx)
            });
        self.render_prompt_card(
            prompt.question(),
            title,
            buttons,
            theme,
            cx,
            |this, window, cx| this.answer_guests(None, window, cx),
        )
    }

    /// A question card over the bottom of the grid: the question, the
    /// event's title, the answers and Cancel.
    fn render_prompt_card(
        &self,
        question: &'static str,
        title: String,
        buttons: gpui::Div,
        theme: &Theme,
        cx: &mut Context<Self>,
        cancel: fn(&mut Self, &mut Window, &mut Context<Self>),
    ) -> AnyElement {
        div()
            .absolute()
            .bottom(SPACE_XL)
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(enter(
                "series-prompt",
                div()
                    .id("series-prompt-card")
                    .occlude()
                    .role(gpui::Role::Dialog)
                    .aria_label(question)
                    .max_w(px(560.))
                    .mx(SPACE_LG)
                    .p(SPACE_SM)
                    .pl(SPACE_LG)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(SPACE_MD)
                    .rounded(RADIUS_LG)
                    .bg(theme.overlay_bg)
                    .border_1()
                    .border_color(theme.overlay_border)
                    .shadow(popover_shadow(theme))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(TEXT_BASE)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(question),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(TEXT_SM)
                                    .text_color(theme.text_muted)
                                    .child(title),
                            ),
                    )
                    .child(buttons)
                    .child(
                        div()
                            .id("prompt-cancel")
                            .role(gpui::Role::Button)
                            .aria_label("Cancel")
                            .h(HIT_TARGET)
                            .px(SPACE_SM)
                            .flex()
                            .items_center()
                            .gap(SPACE_XS)
                            .rounded(RADIUS_MD)
                            .text_size(TEXT_SM)
                            .text_color(theme.text_muted)
                            .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                            .child("Cancel")
                            .child(keycap("esc", theme))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(
                                cx.listener(move |this, _, window, cx| cancel(this, window, cx)),
                            ),
                    ),
            ))
            .into_any_element()
    }
}

impl Render for CalendarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let open = self.panel.is_some();
        let panel = match (&self.panel, &self.closing_panel) {
            (Some(panel), _) | (None, Some(panel)) => self.render_panel(panel, window, cx),
            (None, None) => None,
        };
        let docked = self.panel_docked(window);
        let on_left = !docked && self.panel_on_left(window);
        // In the panel when it shows the event; over the grid otherwise.
        let floating_prompt = self.prompt.as_ref().filter(|p| {
            !(p.kind == PromptKind::Delete
                && self
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.event_id == p.event_id))
        });
        let floating_prompt = match &self.guest_prompt {
            Some(p) => Some(self.render_guest_prompt(p, &theme, cx)),
            None => floating_prompt.map(|p| self.render_floating_prompt(p, &theme, cx)),
        };
        div()
            .id("calendar")
            .key_context(CALENDAR)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::show_day))
            .on_action(cx.listener(Self::show_week))
            .on_action(cx.listener(Self::show_month))
            .on_action(cx.listener(Self::go_to_today))
            .on_action(cx.listener(Self::next_period))
            .on_action(cx.listener(Self::prev_period))
            .on_action(cx.listener(Self::next_day))
            .on_action(cx.listener(Self::prev_day))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::toggle_go_to_date))
            .on_action(cx.listener(Self::toggle_week_numbers))
            .on_action(cx.listener(Self::new_event))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::delete_selected))
            .on_action(cx.listener(Self::close_event))
            .on_action(cx.listener(Self::toggle_calendars_menu))
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_row()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(theme.cal_bg)
                    .child(self.render_header(window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| this.focus_grid(window, cx)),
                            )
                            .child(self.render_body(window, cx)),
                    )
                    .children(floating_prompt),
            )
            .when_some(panel, |this, panel| {
                // Slides open from the edge, and shut into it.
                let width = event_panel::PANEL_WIDTH;
                this.child(
                    div()
                        .h_full()
                        .flex_none()
                        .overflow_hidden()
                        .when(!docked, |d| {
                            // Opaque, so the grid doesn't show through.
                            d.absolute()
                                .top_0()
                                .map(|d| {
                                    if on_left {
                                        d.left_0().border_r_1().border_color(theme.border)
                                    } else {
                                        d.right_0()
                                    }
                                })
                                .occlude()
                                .bg(theme.overlay_bg)
                                .shadow(crate::ui::popover_shadow(&theme))
                        })
                        .child(panel)
                        .with_animation(
                            if open {
                                "event-panel-open"
                            } else {
                                "event-panel-close"
                            },
                            Animation::new(MOTION_PANE).with_easing(ease_out_quint()),
                            move |el, delta| {
                                let shown = if open { delta } else { 1. - delta };
                                el.w(px(width * shown))
                            },
                        ),
                )
            })
            .when(self.drag.is_some(), |this| {
                this.child(self.render_drag_layer(window, cx))
            })
    }
}

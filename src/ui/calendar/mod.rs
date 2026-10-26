//! The calendar: a header, the Day, Week or Month grid, and the event panel
//! docked on the right. `CalendarView` owns the calendar's state and handles
//! its actions; the files beside this one render it.
//!
//! Seams for what comes next (mouse drag, the invite card in the reader):
//! - `create_event` makes an event over a time range and opens it.
//! - `retime_occurrence` moves or resizes one occurrence (its whole series;
//!   pass the series' `start` as the occurrence to set the series itself).
//! - `set_rsvp` answers an invite for the user.

pub mod event_panel;
pub mod header;
pub mod month_view;
pub mod week_view;

use std::{rc::Rc, time::Duration as StdDuration};

use chrono::{Duration, Local, NaiveDate, NaiveDateTime};
use gpui::{
    AnyElement, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, ScrollHandle, SharedString, Styled, Task, Window, anchored, deferred,
    div, point, prelude::FluentBuilder as _, px,
};

use self::event_panel::EventPanel;
use crate::{
    actions::*,
    calendar::{
        CalendarData, Event, Occurrence, RsvpStatus, occurrences,
        time::{self, add_months, month_grid, next_half_hour, week_start},
    },
    theme::Theme,
    ui::composer::ContactBook,
};

/// Height of an hour in the Day and Week grids.
pub const HOUR_HEIGHT: f32 = 50.;
/// The hour the grids open scrolled to.
const FIRST_VISIBLE_HOUR: f32 = 7.5;

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
}

pub enum CalendarEvent {
    /// The app shows a toast that can undo it.
    Deleted { event: Event, index: usize },
}

pub struct CalendarView {
    pub data: CalendarData,
    pub view: ViewKind,
    /// The day the views are built around: the day shown, or one in the week
    /// or month shown. Moved by today, next and previous.
    pub anchor: NaiveDate,
    pub selected: Option<Selection>,
    pub panel: Option<EventPanel>,
    pub menu: Option<Menu>,
    pub focus_handle: FocusHandle,
    pub grid_scroll: ScrollHandle,
    book: Rc<ContactBook>,
    next_id: usize,
    /// Repaints every half minute so the current-time line moves.
    _tick: Task<()>,
}

impl EventEmitter<CalendarEvent> for CalendarView {}

/// A key for an occurrence's elements, unique per event, start and `part`
/// (a day it's drawn on, say).
pub fn occurrence_key(occurrence: &Occurrence, part: i64) -> u64 {
    let minutes = occurrence.start.and_utc().timestamp() / 60;
    ((occurrence.event as u64) << 40) ^ ((minutes as u64) << 8) ^ (part as u64 & 0xff)
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

impl CalendarView {
    pub fn new(data: CalendarData, book: Rc<ContactBook>, cx: &mut Context<Self>) -> Self {
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
        let this = Self {
            data,
            view: ViewKind::Week,
            anchor: today(),
            selected: None,
            panel: None,
            menu: None,
            focus_handle: cx.focus_handle(),
            grid_scroll: ScrollHandle::new(),
            book,
            next_id: 0,
            _tick: tick,
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

    /// Occurrences of the visible calendars' events on screen.
    pub fn visible_occurrences(&self) -> Vec<Occurrence> {
        let (from, to) = self.visible_range();
        occurrences(
            &self.data.events,
            |e| self.data.is_visible(e),
            time::midnight(from),
            time::midnight(to),
        )
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

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let sign = if forward { 1 } else { -1 };
        self.anchor = match self.view {
            ViewKind::Day => self.anchor + Duration::days(sign),
            ViewKind::Week => self.anchor + Duration::days(7 * sign),
            ViewKind::Month => add_months(self.anchor, sign as i32),
        };
        cx.notify();
    }

    // Selection.

    fn select(&mut self, selection: Selection, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(event) = self.event(&selection.event_id)
            && !event.is_spanning()
            && self.view != ViewKind::Month
        {
            let day = selection.start.date();
            let start = time::minutes_into(day, selection.start).max(0);
            let end = time::minutes_into(day, selection.start + event.duration()).min(24 * 60);
            self.reveal_minutes(start, end);
        }
        // With the panel open, it follows the selection.
        if self.panel.is_some() {
            self.open_event(&selection.event_id, selection.start, false, window, cx);
        }
        self.selected = Some(selection);
        cx.notify();
    }

    fn select_adjacent(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let occurrences = self.visible_occurrences();
        if occurrences.is_empty() {
            return;
        }
        let current = occurrences.iter().position(|o| self.is_selected(o));
        let next = match current {
            Some(ix) if forward => (ix + 1).min(occurrences.len() - 1),
            Some(ix) => ix.saturating_sub(1),
            // Nothing selected yet: start from what's next, if it's on screen.
            None => {
                let now = now();
                let timed = |o: &Occurrence| !self.data.events[o.event].is_spanning();
                occurrences
                    .iter()
                    .position(|o| timed(o) && o.start >= now)
                    .or_else(|| occurrences.iter().position(|o| o.end > now))
                    .unwrap_or(if forward { 0 } else { occurrences.len() - 1 })
            }
        };
        let selection = self.selection_of(&occurrences[next]);
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
        let Some(event) = self.event(id).cloned() else {
            return;
        };
        let already_open = self
            .panel
            .as_ref()
            .is_some_and(|p| p.event_id == id && p.occurrence == occurrence);
        if !already_open {
            self.commit_panel(window, cx);
            self.menu = None;
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

    pub fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_panel(window, cx);
        self.panel = None;
        self.menu = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    // Changing events. Recurring events change as a whole series: editing
    // one occurrence edits them all (see CAL-6 in the backend doc).

    fn new_id(&mut self) -> String {
        self.next_id += 1;
        format!("e-local-{}", self.next_id)
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
        let calendar = self
            .data
            .calendars
            .iter()
            .find(|c| c.visible)
            .unwrap_or(&self.data.calendars[0])
            .id
            .clone();
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
        self.data.events.push(Event {
            id: id.clone(),
            calendar,
            title: String::new(),
            start,
            end,
            all_day,
            location: String::new(),
            description: String::new(),
            attendees: Vec::new(),
            organizer: self.data.me(),
            recurrence: None,
            reminder_minutes: Some(10),
            video_link: String::new(),
            busy: true,
        });
        if !all_day && self.view != ViewKind::Month {
            let day = start.date();
            self.reveal_minutes(
                time::minutes_into(day, start),
                time::minutes_into(day, end).min(24 * 60),
            );
        }
        self.selected = Some(Selection {
            event_id: id.clone(),
            start,
        });
        self.open_event(&id, start, true, window, cx);
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
        let Some(event) = self.event_mut(id) else {
            return;
        };
        let shift = start - occurrence;
        event.start += shift;
        event.end = event.start + (end - start);
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
            cx.notify();
        }
    }

    pub fn delete_event(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.data.event_ix(id) else {
            return;
        };
        if self.panel.as_ref().is_some_and(|p| p.event_id == id) {
            self.panel = None;
            self.menu = None;
            window.focus(&self.focus_handle, cx);
        }
        if self.selected.as_ref().is_some_and(|s| s.event_id == id) {
            self.selected = None;
        }
        let event = self.data.events.remove(index);
        cx.emit(CalendarEvent::Deleted { event, index });
        cx.notify();
    }

    /// Undo a delete: the event comes back where it was, selected.
    pub fn restore_event(
        &mut self,
        event: Event,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = Selection {
            event_id: event.id.clone(),
            start: event.start,
        };
        let index = index.min(self.data.events.len());
        self.data.events.insert(index, event);
        self.selected = Some(selection);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub fn toggle_calendar(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(calendar) = self.data.calendars.get_mut(ix) {
            calendar.visible = !calendar.visible;
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
        let today = time::midnight(today());
        let ahead = occurrences(
            std::slice::from_ref(event),
            |_| true,
            today,
            today + Duration::days(400),
        );
        let start = ahead.first().map_or(event.start, |o| o.start);
        if let Some(calendar) = self
            .data
            .calendar_ix(&event.calendar)
            .filter(|&c| !self.data.calendars[c].visible)
        {
            self.data.calendars[calendar].visible = true;
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

    fn select_next(&mut self, _: &SelectNextEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(true, window, cx);
    }

    fn select_prev(&mut self, _: &SelectPrevEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(false, window, cx);
    }

    /// `c`: an hour from the next half hour today, or from 9 on another day
    /// shown in the Day view.
    fn new_event(&mut self, _: &NewEvent, window: &mut Window, cx: &mut Context<Self>) {
        let today = today();
        let day = if self.view == ViewKind::Day {
            self.anchor
        } else {
            let (from, to) = self.visible_range();
            if today >= from && today < to {
                today
            } else {
                from.max(self.anchor)
            }
        };
        let start = if day == today {
            next_half_hour(now())
        } else {
            time::at(day, 9, 0)
        };
        self.create_event(start, start + Duration::hours(1), false, window, cx);
    }

    fn open_selected(&mut self, _: &OpenEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_none() {
            self.select_adjacent(true, window, cx);
        }
        if let Some(selected) = self.selected.clone() {
            self.open_event(&selected.event_id, selected.start, false, window, cx);
        }
    }

    fn delete_selected(&mut self, _: &DeleteEvent, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .selected
            .as_ref()
            .map(|s| s.event_id.clone())
            .or_else(|| self.panel.as_ref().map(|p| p.event_id.clone()));
        if let Some(id) = id {
            self.delete_event(&id, window, cx);
        }
    }

    /// Escape: the innermost open thing closes: a dropdown or picker, then
    /// the panel, then the selection.
    fn close_event(&mut self, _: &CloseEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() || self.dismiss_panel_popup(cx) {
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
                        cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            ),
        )
        .with_priority(1)
        .into_any_element()
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
            ViewKind::Day => self.render_time_grid(vec![self.anchor], &occurrences, window, cx),
            ViewKind::Week => {
                let start = week_start(self.anchor);
                let days = (0..7).map(|i| start + Duration::days(i)).collect();
                self.render_time_grid(days, &occurrences, window, cx)
            }
            ViewKind::Month => self.render_month(&occurrences, window, cx),
        }
    }
}

impl Render for CalendarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let panel = self.panel.is_some().then(|| self.render_panel(window, cx));
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
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::new_event))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::delete_selected))
            .on_action(cx.listener(Self::close_event))
            .on_action(cx.listener(Self::toggle_calendars_menu))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_row()
            .child(
                div()
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
                    ),
            )
            .when_some(panel, |this, panel| this.child(panel))
    }
}

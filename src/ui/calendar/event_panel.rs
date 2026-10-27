//! The event panel, docked on the right. Every field edits the event in
//! place: text as you type, dates and times when committed (enter, leaving
//! the field, a pick from its picker, or up and down).

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable as _, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, actions, anchored,
    component::input::{Input, InputEvent, InputState, Textarea, TextareaState},
    deferred, div,
    prelude::FluentBuilder as _,
    px, relative,
};

use super::{CalendarView, Menu, PromptKind, today};
use crate::ui::style::*;
use crate::{
    actions::*,
    assets::Icon,
    backend::calendar::{Access, is_organizer},
    calendar::{
        Attendee, Event, RsvpStatus, SeriesScope,
        time::{
            self, add_months, date_label, duration_label, month_start, parse_date, parse_time,
            time_label, week_start,
        },
    },
    data::Contact,
    theme::Theme,
    ui::{
        HEADER_HEIGHT, avatar,
        composer::{RemoveLastRecipient, SuggestNext, SuggestPrev, parse_address},
        icon, icon_button, keycap, popover_shadow, window_drag_area,
    },
};

actions!(
    event_panel,
    [
        NextField, PrevField, StepUp, StepDown, Activate, MenuNext, MenuPrev
    ]
);

pub const PANEL_WIDTH: f32 = 372.;
/// Slots in the time pickers.
const SLOT_MINUTES: i64 = 15;

const REMINDERS: [Option<u32>; 8] = [
    None,
    Some(0),
    Some(5),
    Some(10),
    Some(15),
    Some(30),
    Some(60),
    Some(24 * 60),
];

fn reminder_label(minutes: Option<u32>) -> String {
    match minutes {
        None => "No reminder".into(),
        Some(0) => "At start time".into(),
        Some(m) if m % (24 * 60) == 0 => match m / (24 * 60) {
            1 => "1 day before".into(),
            d => format!("{d} days before"),
        },
        Some(m) if m % 60 == 0 => match m / 60 {
            1 => "1 hour before".into(),
            h => format!("{h} hours before"),
        },
        Some(m) => format!("{m} minutes before"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The calendar picker in the header.
    Calendar,
    Title,
    StartDate,
    StartTime,
    EndTime,
    EndDate,
    AllDay,
    Location,
    Video,
    Guests,
    Notes,
    Reminder,
}

impl Field {
    fn is_date(self) -> bool {
        matches!(self, Field::StartDate | Field::EndDate)
    }

    fn is_time(self) -> bool {
        matches!(self, Field::StartTime | Field::EndTime)
    }
}

pub struct EventPanel {
    pub event_id: String,
    /// The start of the occurrence shown. Edits apply to its whole series,
    /// moved by as much as this occurrence moves.
    pub occurrence: NaiveDateTime,
    pub title: Entity<InputState>,
    start_date: Entity<InputState>,
    end_date: Entity<InputState>,
    start_time: Entity<InputState>,
    end_time: Entity<InputState>,
    location: Entity<InputState>,
    video: Entity<InputState>,
    guests: Entity<InputState>,
    notes: Entity<TextareaState>,
    calendar_focus: FocusHandle,
    all_day_focus: FocusHandle,
    reminder_focus: FocusHandle,
    /// The row of the open calendar or reminder menu that up, down and
    /// enter work on.
    menu_highlight: usize,
    /// The date or time field showing its picker.
    picker: Option<Field>,
    /// The month the date picker shows.
    picker_month: NaiveDate,
    picker_scroll: ScrollHandle,
    /// A repeating event's series as it was before the panel's first change
    /// to it, until the user says whether the change is for this event or
    /// all of them.
    pub snapshot: Option<Event>,
    /// Where the occurrence was when that first change came.
    origin: NaiveDateTime,
    /// The answer, once given.
    pub scope: Option<SeriesScope>,
    /// Indices into the contact book, best first.
    suggestions: Vec<usize>,
    highlighted: usize,
    /// The user moved through the suggestions, so enter takes one even over
    /// a typed address.
    navigated: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventPanel {
    pub fn new(
        event: &Event,
        occurrence: NaiveDateTime,
        window: &mut Window,
        cx: &mut Context<CalendarView>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let mut input = |field: Field, placeholder: &'static str, value: String| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            });
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this: &mut CalendarView, _, event, window, cx| {
                    this.on_field_event(field, event, window, cx)
                },
            ));
            input
        };
        let title = input(Field::Title, "Add title", event.title.clone());
        let start_date = input(Field::StartDate, "Date", String::new());
        let end_date = input(Field::EndDate, "Date", String::new());
        let start_time = input(Field::StartTime, "Start", String::new());
        let end_time = input(Field::EndTime, "End", String::new());
        let location = input(Field::Location, "Add location", event.location.clone());
        let video = input(
            Field::Video,
            "Add video call link",
            event.video_link.clone(),
        );
        let guests = input(Field::Guests, "Add guests", String::new());
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 24)
                .placeholder("Add description")
                .default_value(event.description.clone())
        });
        subscriptions.push(cx.subscribe_in(
            &notes,
            window,
            |this: &mut CalendarView, _, event, window, cx| {
                this.on_field_event(Field::Notes, event, window, cx)
            },
        ));

        let mut panel = Self {
            event_id: event.id.clone(),
            occurrence,
            title,
            start_date,
            end_date,
            start_time,
            end_time,
            location,
            video,
            guests,
            notes,
            calendar_focus: cx.focus_handle(),
            all_day_focus: cx.focus_handle(),
            reminder_focus: cx.focus_handle(),
            menu_highlight: 0,
            picker: None,
            picker_month: month_start(occurrence.date()),
            picker_scroll: ScrollHandle::new(),
            snapshot: None,
            origin: occurrence,
            scope: None,
            suggestions: Vec::new(),
            highlighted: 0,
            navigated: false,
            _subscriptions: subscriptions,
        };
        panel.sync_times(event, window, cx);
        panel
    }

    fn input(&self, field: Field) -> Option<&Entity<InputState>> {
        Some(match field {
            Field::Title => &self.title,
            Field::StartDate => &self.start_date,
            Field::EndDate => &self.end_date,
            Field::StartTime => &self.start_time,
            Field::EndTime => &self.end_time,
            Field::Location => &self.location,
            Field::Video => &self.video,
            Field::Guests => &self.guests,
            Field::Notes | Field::Calendar | Field::AllDay | Field::Reminder => return None,
        })
    }

    fn focus_handle_of(&self, field: Field, cx: &gpui::App) -> FocusHandle {
        match field {
            Field::Calendar => self.calendar_focus.clone(),
            Field::AllDay => self.all_day_focus.clone(),
            Field::Reminder => self.reminder_focus.clone(),
            Field::Notes => self.notes.focus_handle(cx),
            _ => self.input(field).expect("text fields").focus_handle(cx),
        }
    }

    /// The fields in tab order: as they read, top to bottom.
    fn fields(all_day: bool) -> &'static [Field] {
        if all_day {
            &[
                Field::Calendar,
                Field::Title,
                Field::StartDate,
                Field::EndDate,
                Field::AllDay,
                Field::Location,
                Field::Video,
                Field::Guests,
                Field::Notes,
                Field::Reminder,
            ]
        } else {
            &[
                Field::Calendar,
                Field::Title,
                Field::StartDate,
                Field::StartTime,
                Field::EndTime,
                Field::AllDay,
                Field::Location,
                Field::Video,
                Field::Guests,
                Field::Notes,
                Field::Reminder,
            ]
        }
    }

    fn focused_field(&self, window: &Window, cx: &gpui::App) -> Option<Field> {
        Self::fields(false)
            .iter()
            .copied()
            .chain([Field::EndDate])
            .find(|&f| self.focus_handle_of(f, cx).is_focused(window))
    }

    fn is_focused(&self, field: Field, window: &Window, cx: &gpui::App) -> bool {
        self.focus_handle_of(field, cx).is_focused(window)
    }

    /// The occurrence's start and end.
    fn times(&self, event: &Event) -> (NaiveDateTime, NaiveDateTime) {
        (self.occurrence, self.occurrence + event.duration())
    }

    pub fn location_text(&self, cx: &gpui::App) -> String {
        self.location.read(cx).value().to_string()
    }

    pub fn notes_text(&self, cx: &gpui::App) -> String {
        self.notes.read(cx).value().to_string()
    }

    /// Write the occurrence's dates and times into their fields.
    fn sync_times(&mut self, event: &Event, window: &mut Window, cx: &mut Context<CalendarView>) {
        let (start, end) = self.times(event);
        let today = today();
        let last_day = if event.all_day {
            (end - Duration::days(1)).date().max(start.date())
        } else {
            end.date()
        };
        let values = [
            (&self.start_date, date_label(start.date(), today)),
            (&self.end_date, date_label(last_day, today)),
            (&self.start_time, time_label(start.time())),
            (&self.end_time, time_label(end.time())),
        ];
        for (input, value) in values {
            if input.read(cx).value() != value {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
    }
}

impl CalendarView {
    /// Whether the event may go to another calendar (CAL-40): Google moves
    /// only whole events, and only for their organizer.
    fn can_move(&self, event: &Event) -> bool {
        self.access_of(event) == Access::Edit
            && is_organizer(event, self.me())
            && event.recurring_event_id.is_none()
    }

    fn panel_event(&self) -> Option<(&EventPanel, &Event)> {
        let panel = self.panel.as_ref()?;
        Some((panel, self.event(&panel.event_id)?))
    }

    pub(crate) fn sync_panel_times(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(event) = self
            .panel
            .as_ref()
            .and_then(|p| self.event(&p.event_id))
            .cloned()
        else {
            return;
        };
        if let Some(panel) = &mut self.panel {
            panel.sync_times(&event, window, cx);
        }
    }

    /// Apply whatever is typed in the focused date or time field, before the
    /// panel closes or shows another event.
    pub(crate) fn commit_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .panel
            .as_ref()
            .and_then(|p| p.focused_field(window, cx));
        if let Some(field) = focused.filter(|f| f.is_date() || f.is_time()) {
            self.commit_field(field, window, cx);
        }
    }

    /// Escape closes an open picker or the guest suggestions first.
    pub(crate) fn dismiss_panel_popup(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(panel) = &mut self.panel else {
            return false;
        };
        if panel.picker.take().is_some() {
            cx.notify();
            return true;
        }
        if !panel.suggestions.is_empty() {
            panel.suggestions.clear();
            cx.notify();
            return true;
        }
        false
    }

    /// Before the panel's first change to a repeating event: keep the series
    /// as it was, so "This event" can put it back.
    fn before_panel_edit(&mut self) {
        let Some(panel) = &self.panel else { return };
        if panel.scope.is_some() || panel.snapshot.is_some() {
            return;
        }
        let Some(event) = self.event(&panel.event_id).filter(|e| e.is_series()) else {
            return;
        };
        let snapshot = event.clone();
        let panel = self.panel.as_mut().expect("checked above");
        panel.origin = panel.occurrence;
        panel.snapshot = Some(snapshot);
    }

    /// "This event": the occurrence, as edited, becomes an event of its own
    /// and the series goes back to how it was. "All events": the change
    /// stays on the series.
    fn choose_edit_scope(&mut self, scope: SeriesScope, cx: &mut Context<Self>) {
        let Some(panel) = &mut self.panel else { return };
        let Some(snapshot) = panel.snapshot.take() else {
            return;
        };
        panel.scope = Some(scope);
        if scope == SeriesScope::This {
            let (series_id, origin, start) =
                (panel.event_id.clone(), panel.origin, panel.occurrence);
            let Some(edited) = self.event(&series_id).cloned() else {
                return;
            };
            // The series goes back; the occurrence takes the change.
            if let Some(series) = self.event_mut(&series_id) {
                *series = snapshot;
            }
            let end = start + edited.duration();
            let Some(id) = self.detach(&series_id, origin, start, end, edited, cx) else {
                return;
            };
            if let Some(panel) = &mut self.panel {
                panel.event_id = id.clone();
            }
            self.selected = Some(super::Selection {
                event_id: id,
                start,
            });
        } else {
            // The change waited for the answer.
            self.settle_edit(cx);
        }
        cx.notify();
    }

    fn edit_event(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Event)) {
        let Some(id) = self.panel.as_ref().map(|p| p.event_id.clone()) else {
            return;
        };
        if !self.can_edit(&id) {
            return;
        }
        self.before_panel_edit();
        let Some(before) = self.event(&id).cloned() else {
            return;
        };
        if let Some(event) = self.event_mut(&id) {
            f(event);
            self.patched(before, cx);
            cx.notify();
        }
    }

    fn on_field_event(
        &mut self,
        field: Field,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = &self.panel else { return };
        match event {
            InputEvent::Change => {
                let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
                match field {
                    Field::Title => {
                        let title = value(&panel.title);
                        self.edit_event(cx, |e| e.title = title);
                    }
                    Field::Location => {
                        let location = value(&panel.location);
                        self.edit_event(cx, |e| e.location = location);
                    }
                    Field::Video => {
                        let video = value(&panel.video).trim().to_string();
                        self.edit_event(cx, |e| e.video_link = video);
                    }
                    Field::Notes => {
                        let notes = panel.notes.read(cx).value().to_string();
                        self.edit_event(cx, |e| e.description = notes);
                    }
                    Field::Guests => self.update_guest_suggestions(cx),
                    _ => {}
                }
            }
            InputEvent::Focus if field.is_date() || field.is_time() => {
                let Some((panel, event)) = self.panel_event() else {
                    return;
                };
                if self.access_of(event) != Access::Edit {
                    return;
                }
                let (start, end) = panel.times(event);
                let shown = if field == Field::EndDate {
                    end - Duration::days(1)
                } else {
                    start
                };
                if let Some(panel) = &mut self.panel {
                    panel.picker = Some(field);
                    panel.picker_month = month_start(shown.date());
                }
                self.scroll_time_picker();
                cx.notify();
            }
            InputEvent::Blur => {
                if field.is_date() || field.is_time() {
                    self.commit_field(field, window, cx);
                }
                if let Some(panel) = &mut self.panel {
                    if panel.picker == Some(field) {
                        panel.picker = None;
                    }
                    if field == Field::Guests {
                        panel.suggestions.clear();
                    }
                }
                cx.notify();
            }
            InputEvent::PressEnter {
                secondary: false, ..
            } => match field {
                Field::Guests => self.commit_guest(window, cx),
                f if f.is_date() || f.is_time() => {
                    self.commit_field(f, window, cx);
                    self.move_field(true, window, cx);
                }
                Field::Notes => {}
                // Done with the title, location or link: back to the grid.
                _ => window.focus(&self.focus_handle, cx),
            },
            _ => {}
        }
    }

    /// Apply a date or time typed into `field`; what doesn't parse is put
    /// back as it was.
    fn commit_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let Some((panel, event)) = self.panel_event() else {
            return;
        };
        let Some(input) = panel.input(field) else {
            return;
        };
        let text = input.read(cx).value().to_string();
        let (start, end) = panel.times(event);
        let all_day = event.all_day;
        let duration = event.duration();
        let id = event.id.clone();
        let today = today();

        let change = match field {
            Field::StartDate => parse_date(&text, today).map(|date| {
                let start = date.and_time(start.time());
                (start, start + duration)
            }),
            Field::EndDate => parse_date(&text, today).map(|date| {
                let end = time::midnight(date + Duration::days(1));
                (start, end.max(start + Duration::days(1)))
            }),
            Field::StartTime if !all_day => parse_time(&text).map(|t| {
                let start = start.date().and_time(t);
                (start, start + duration)
            }),
            Field::EndTime if !all_day => parse_time(&text).map(|t| {
                let mut end = start.date().and_time(t);
                if end <= start {
                    end += Duration::days(1);
                }
                (start, end)
            }),
            _ => None,
        };
        match change {
            Some((new_start, new_end)) if (new_start, new_end) != (start, end) => {
                self.before_panel_edit();
                self.retime_occurrence(&id, start, new_start, new_end, window, cx)
            }
            _ => self.sync_panel_times(window, cx),
        }
    }

    /// Up and down in a date field move a day; in a time field, a slot.
    fn step_field(&mut self, delta: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((panel, event)) = self.panel_event() else {
            return;
        };
        let Some(field) = panel.focused_field(window, cx) else {
            return cx.propagate();
        };
        let (start, end) = panel.times(event);
        let id = event.id.clone();
        let day = Duration::days(delta);
        let slot = Duration::minutes(SLOT_MINUTES * delta);
        let (new_start, new_end) = match field {
            Field::StartDate => (start + day, end + day),
            Field::EndDate => (start, (end + day).max(start + Duration::days(1))),
            Field::StartTime => (start + slot, end + slot),
            Field::EndTime => (
                start,
                (end + slot).max(start + Duration::minutes(SLOT_MINUTES)),
            ),
            _ => return cx.propagate(),
        };
        self.before_panel_edit();
        self.retime_occurrence(&id, start, new_start, new_end, window, cx);
        if let Some(panel) = &mut self.panel {
            panel.picker_month = month_start(
                match field {
                    Field::EndDate => new_end - Duration::days(1),
                    _ => new_start,
                }
                .date(),
            );
        }
        self.scroll_time_picker();
    }

    fn step_up(&mut self, _: &StepUp, window: &mut Window, cx: &mut Context<Self>) {
        self.step_field(-1, window, cx);
    }

    fn step_down(&mut self, _: &StepDown, window: &mut Window, cx: &mut Context<Self>) {
        self.step_field(1, window, cx);
    }

    fn move_field(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((panel, event)) = self.panel_event() else {
            return;
        };
        let fields = EventPanel::fields(event.all_day);
        let current = panel
            .focused_field(window, cx)
            .and_then(|f| fields.iter().position(|&x| x == f));
        let next = match (current, forward) {
            (None, _) => fields.iter().position(|&f| f == Field::Title).unwrap_or(0),
            (Some(i), true) => (i + 1).min(fields.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        let handle = panel.focus_handle_of(fields[next], cx);
        self.menu = None;
        window.focus(&handle, cx);
        cx.notify();
    }

    /// The control (calendar, all-day, reminder) with focus, if any.
    fn focused_control(&self, window: &Window, cx: &gpui::App) -> Option<Field> {
        let panel = self.panel.as_ref()?;
        [Field::Calendar, Field::AllDay, Field::Reminder]
            .into_iter()
            .find(|&f| panel.is_focused(f, window, cx))
    }

    fn control_menu(field: Field) -> Option<Menu> {
        match field {
            Field::Calendar => Some(Menu::EventCalendar),
            Field::Reminder => Some(Menu::Reminder),
            _ => None,
        }
    }

    /// Open a control's menu with its current choice highlighted.
    fn open_control_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        let Some((_, event)) = self.panel_event() else {
            return;
        };
        let allowed = match menu {
            Menu::EventCalendar => self.can_move(event),
            _ => self.access_of(event) == Access::Edit,
        };
        if !allowed {
            return;
        }
        let current = match menu {
            Menu::EventCalendar => self.data.calendar_ix(&event.calendar).unwrap_or(0),
            _ => REMINDERS
                .iter()
                .position(|&m| m == event.reminder_minutes)
                .unwrap_or(0),
        };
        if let Some(panel) = &mut self.panel {
            panel.menu_highlight = current;
        }
        self.menu = Some(menu);
        cx.notify();
    }

    /// Choose row `ix` of the open calendar or reminder menu.
    fn choose_menu_row(&mut self, menu: Menu, ix: usize, cx: &mut Context<Self>) {
        match menu {
            Menu::EventCalendar => {
                let writable = |c: &&crate::calendar::Calendar| {
                    matches!(
                        c.access_role,
                        crate::calendar::AccessRole::Owner | crate::calendar::AccessRole::Writer
                    )
                };
                if let Some(id) = self
                    .data
                    .calendars
                    .get(ix)
                    .filter(writable)
                    .map(|c| c.id.clone())
                {
                    self.edit_event(cx, |e| e.calendar = id);
                }
            }
            Menu::Reminder => {
                if let Some(&minutes) = REMINDERS.get(ix) {
                    self.edit_event(cx, |e| e.reminder_minutes = minutes);
                }
            }
            Menu::Calendars | Menu::GoToDate => {}
        }
        self.menu = None;
        cx.notify();
    }

    fn click_control_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        if self.menu == Some(menu) {
            self.menu = None;
            cx.notify();
        } else {
            self.open_control_menu(menu, cx);
        }
    }

    /// Enter or space on a control: flip all-day, or open its menu, or take
    /// the highlighted row of the open one.
    fn activate(&mut self, _: &Activate, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.focused_control(window, cx) else {
            return cx.propagate();
        };
        match Self::control_menu(field) {
            None => self.toggle_all_day(window, cx),
            Some(menu) if self.menu == Some(menu) => {
                let ix = self.panel.as_ref().map_or(0, |p| p.menu_highlight);
                self.choose_menu_row(menu, ix, cx);
            }
            Some(menu) => self.open_control_menu(menu, cx),
        }
    }

    /// Up and down on a control with a menu: move through it, opening it
    /// first if it's closed.
    fn menu_step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self
            .focused_control(window, cx)
            .and_then(Self::control_menu)
        else {
            return cx.propagate();
        };
        if self.menu != Some(menu) {
            return self.open_control_menu(menu, cx);
        }
        let count = match menu {
            Menu::EventCalendar => self.data.calendars.len(),
            _ => REMINDERS.len(),
        } as isize;
        if let Some(panel) = &mut self.panel {
            panel.menu_highlight =
                (panel.menu_highlight as isize + delta).rem_euclid(count) as usize;
        }
        cx.notify();
    }

    fn menu_next(&mut self, _: &MenuNext, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_step(1, window, cx);
    }

    fn menu_prev(&mut self, _: &MenuPrev, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_step(-1, window, cx);
    }

    /// Tab out of the guest field takes the guest typed there first.
    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        let typing_guest = self.panel.as_ref().is_some_and(|p| {
            p.is_focused(Field::Guests, window, cx) && !p.guests.read(cx).value().trim().is_empty()
        });
        if typing_guest {
            self.commit_guest(window, cx);
        } else {
            self.move_field(true, window, cx);
        }
    }

    fn prev_field(&mut self, _: &PrevField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_field(false, window, cx);
    }

    /// The time picker keeps its chosen slot in view.
    fn scroll_time_picker(&self) {
        let Some((panel, event)) = self.panel_event() else {
            return;
        };
        let (start, end) = panel.times(event);
        let ix = match panel.picker {
            Some(Field::StartTime) => time::minutes_into(start.date(), start) / SLOT_MINUTES,
            Some(Field::EndTime) => (end - start).num_minutes() / SLOT_MINUTES - 1,
            _ => return,
        };
        // Centered in the picker, which is 240 tall with 30 tall rows.
        let y = (ix.max(0) as f32 * 30. + 4. + 15. - 120.).max(0.);
        panel.picker_scroll.set_offset(gpui::point(px(0.), px(-y)));
    }

    fn set_date(
        &mut self,
        field: Field,
        date: NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((panel, event)) = self.panel_event() else {
            return;
        };
        let (start, end) = panel.times(event);
        let id = event.id.clone();
        let (new_start, new_end) = if field == Field::EndDate {
            let end = time::midnight(date + Duration::days(1));
            (start, end.max(start + Duration::days(1)))
        } else {
            let shift = date - start.date();
            (start + shift, end + shift)
        };
        self.before_panel_edit();
        self.retime_occurrence(&id, start, new_start, new_end, window, cx);
        window.focus(&self.focus_handle, cx);
    }

    fn set_time(
        &mut self,
        field: Field,
        value: NaiveDateTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((panel, event)) = self.panel_event() else {
            return;
        };
        let (start, end) = panel.times(event);
        let id = event.id.clone();
        let (new_start, new_end) = if field == Field::EndTime {
            (start, value)
        } else {
            (value, value + (end - start))
        };
        self.before_panel_edit();
        self.retime_occurrence(&id, start, new_start, new_end, window, cx);
        window.focus(&self.focus_handle, cx);
    }

    fn toggle_all_day(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((panel, event)) = self.panel_event() else {
            return;
        };
        if self.access_of(event) != Access::Edit {
            return;
        }
        let (start, end) = panel.times(event);
        let id = event.id.clone();
        let all_day = !event.all_day;
        let (new_start, new_end) = if all_day {
            let last = (end - Duration::minutes(1)).date().max(start.date());
            (
                time::midnight(start.date()),
                time::midnight(last + Duration::days(1)),
            )
        } else {
            let start = time::at(start.date(), 9, 0);
            (start, start + Duration::hours(1))
        };
        self.before_panel_edit();
        let before = self.event(&id).cloned();
        if let Some(event) = self.event_mut(&id) {
            event.all_day = all_day;
        }
        if let Some(before) = before {
            self.patched(before, cx);
        }
        self.retime_occurrence(&id, start, new_start, new_end, window, cx);
    }

    // Guests.

    fn update_guest_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some((panel, event)) = self.panel_event() else {
            return;
        };
        let query = panel.guests.read(cx).value().to_string();
        let mut taken: Vec<String> = event
            .attendees
            .iter()
            .map(|a| a.email.to_lowercase())
            .collect();
        taken.push(event.organizer.email.to_lowercase());
        let book = self.book.clone();
        let panel = self.panel.as_mut().unwrap();
        panel.suggestions = book
            .borrow()
            .suggest(&query, |c| taken.contains(&c.email.to_lowercase()));
        panel.highlighted = 0;
        panel.navigated = false;
        cx.notify();
    }

    fn add_guest(&mut self, contact: Contact, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_event(cx, |e| {
            if !e
                .attendees
                .iter()
                .any(|a| a.email.eq_ignore_ascii_case(&contact.email))
            {
                e.attendees.push(Attendee {
                    name: contact.name,
                    email: contact.email,
                    status: RsvpStatus::NeedsAction,
                });
            }
        });
        if let Some(panel) = &self.panel {
            let guests = panel.guests.clone();
            guests.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.update_guest_suggestions(cx);
    }

    /// Enter in the guest field: the highlighted suggestion, or the address
    /// as typed. A typed address wins unless the user picked a suggestion.
    fn commit_guest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = &self.panel else { return };
        let text = panel.guests.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let contact = match panel.suggestions.get(panel.highlighted) {
            Some(&ix) if panel.navigated || !text.contains('@') => {
                Some(self.book.borrow().contact(ix).clone())
            }
            _ => self
                .book
                .borrow()
                .find(&text)
                .cloned()
                .or_else(|| parse_address(&text)),
        };
        if let Some(contact) = contact {
            self.add_guest(contact, window, cx);
        }
    }

    fn remove_guest(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.edit_event(cx, |e| {
            if ix < e.attendees.len() {
                e.attendees.remove(ix);
            }
        });
    }

    fn suggest(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(panel) = &mut self.panel else {
            return cx.propagate();
        };
        let count = panel.suggestions.len();
        if count == 0 {
            return cx.propagate();
        }
        panel.highlighted =
            (panel.highlighted as isize + delta).rem_euclid(count as isize) as usize;
        panel.navigated = true;
        cx.notify();
    }

    fn suggest_next(&mut self, _: &SuggestNext, _: &mut Window, cx: &mut Context<Self>) {
        self.suggest(1, cx);
    }

    fn suggest_prev(&mut self, _: &SuggestPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.suggest(-1, cx);
    }

    fn remove_last_guest(
        &mut self,
        _: &RemoveLastRecipient,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let empty = self
            .panel
            .as_ref()
            .is_some_and(|p| p.guests.read(cx).value().is_empty());
        let count = self.panel_event().map_or(0, |(_, e)| e.attendees.len());
        if empty && count > 0 {
            self.remove_guest(count - 1, cx);
        } else {
            cx.propagate();
        }
    }

    // Rendering.

    /// The panel for `panel`'s event (the open one, or the one sliding
    /// shut); nothing once the event is gone.
    pub(crate) fn render_panel(
        &self,
        panel: &EventPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).clone();
        let event = self.event(&panel.event_id)?;
        let color = self.data.color_of(event);
        let accent = theme.event_accent(color, false);
        let access = self.access_of(event);
        let editable = access == Access::Edit;
        // Why the fields don't take typing (CAL-22).
        let read_only_note = (!editable).then(|| {
            div()
                .pl(SPACE_LG)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .text_size(TEXT_SM)
                .text_color(theme.text_muted)
                .child(if access == Access::Rsvp {
                    "Only the organizer can change this event"
                } else {
                    "You can't change events in this calendar"
                })
        });

        let header = window_drag_area(div().id("event-panel-header"))
            .h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .pl(SPACE_LG)
            .pr(SPACE_MD)
            .child(self.render_calendar_picker(event, &theme, window, cx))
            .child(div().flex_1())
            .child(icon_button(
                "event-close",
                Icon::X,
                "Close",
                Box::new(CloseEvent),
                &theme,
            ));

        let title = div()
            .px(SPACE_XL)
            .pt(SPACE_XS)
            .pb(SPACE_LG)
            .flex()
            .flex_col()
            .gap(SPACE_SM)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(SPACE_MD)
                    .child(
                        div()
                            .mt(SPACE_XS)
                            .w(px(4.))
                            .h(px(24.))
                            .flex_none()
                            .rounded_full()
                            .bg(accent),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&panel.title)
                                .readonly(!editable)
                                .appearance(false)
                                .px(px(0.))
                                .h(px(32.))
                                .text_size(TEXT_XL)
                                .font_weight(FontWeight::SEMIBOLD),
                        ),
                    ),
            )
            .children(self.repeat_line(event).map(|line| {
                div()
                    .pl(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_muted)
                    .child(icon(Icon::Repeat, px(13.), theme.text_faint))
                    .child(line)
            }))
            .children(read_only_note)
            // After the first change to a repeating event: to which?
            .when(panel.snapshot.is_some() && panel.scope.is_none(), |d| {
                d.child(enter(
                    "scope-choice",
                    div()
                        .ml(SPACE_LG)
                        .mt(SPACE_XS)
                        .p(SPACE_SM)
                        .flex()
                        .flex_col()
                        .gap(SPACE_SM)
                        .rounded(RADIUS_MD)
                        .border_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_muted)
                                .child("Apply this change to"),
                        )
                        .child(self.render_scope_buttons(
                            &[
                                (SeriesScope::This, "This event"),
                                (SeriesScope::All, "All events"),
                            ],
                            false,
                            &theme,
                            cx,
                            |this, scope, _, cx| this.choose_edit_scope(scope, cx),
                        )),
                ))
            });

        let body = div()
            .id("event-panel-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb(SPACE_XL)
            .child(title)
            .child(divider(&theme))
            .child(self.render_when(panel, event, &theme, window, cx))
            .child(divider(&theme))
            .child(self.render_place(panel, event, &theme, window, cx))
            .child(divider(&theme))
            .child(self.render_guests(panel, event, &theme, window, cx))
            .child(divider(&theme))
            .child(section(
                Icon::Text,
                &theme,
                field_box(panel.is_focused(Field::Notes, window, cx), &theme)
                    .h_auto()
                    .min_h(px(30.))
                    .py(SPACE_XS)
                    .px(px(0.))
                    .child(
                        div()
                            .w_full()
                            .text_size(TEXT_BASE)
                            .line_height(relative(1.5))
                            .mx(px(-2.))
                            .child(
                                Textarea::new(&panel.notes)
                                    .readonly(!editable)
                                    .appearance(false)
                                    .text_size(TEXT_BASE),
                            ),
                    ),
            ))
            .child(divider(&theme))
            .child(self.render_options(event, &theme, window, cx))
            .child(self.render_delete(event, &theme, cx));

        div()
            .key_context(EVENT_PANEL)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::prev_field))
            .on_action(cx.listener(Self::step_up))
            .on_action(cx.listener(Self::step_down))
            .on_action(cx.listener(Self::suggest_next))
            .on_action(cx.listener(Self::suggest_prev))
            .on_action(cx.listener(Self::remove_last_guest))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::menu_next))
            .on_action(cx.listener(Self::menu_prev))
            .relative()
            .w(px(PANEL_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.cal_panel_bg)
            .border_l_1()
            .border_color(theme.border)
            .child(header)
            .child(body)
            .into_any_element()
            .into()
    }

    /// "Repeats every week", or for an occurrence changed on its own, the
    /// series it belongs to.
    fn repeat_line(&self, event: &Event) -> Option<String> {
        if let Some(recurrence) = &event.recurrence {
            return Some(recurrence.label());
        }
        let series = self.event(event.recurring_event_id.as_deref()?)?;
        let label = series
            .recurrence
            .as_ref()
            .map_or_else(|| "Repeating".to_string(), |r| r.label());
        Some(format!("{label} · this one changed separately"))
    }

    /// The delete row, with its shortcut; for a repeating event, once
    /// asked, which occurrences to delete.
    fn render_delete(&self, event: &Event, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let asking = self.prompt.as_ref().filter(|p| {
            p.kind == PromptKind::Delete && p.event_id == event.id && self.panel.is_some()
        });
        if let Some(prompt) = asking {
            return div()
                .mx(SPACE_XL)
                .mt(SPACE_LG)
                .p(SPACE_MD)
                .flex()
                .flex_col()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.danger.alpha(0.35))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(TEXT_BASE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Delete repeating event"),
                        )
                        .child(keycap("esc", theme)),
                )
                .child(self.render_prompt_choices(prompt.kind, theme, cx))
                .into_any_element();
        }
        // Someone else's event: deleting declines it and hides it (CAL-22).
        let label = match self.access_of(event) {
            Access::ReadOnly => return div().into_any_element(),
            Access::Edit if is_organizer(event, self.me()) => {
                if event.is_series() {
                    "Delete event…"
                } else {
                    "Delete event"
                }
            }
            _ => "Decline and remove",
        };
        div()
            .px(SPACE_XL)
            .pt(SPACE_LG)
            .child(
                focusable_control(div().id("event-delete-button"), theme)
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .h(HIT_TARGET)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_BASE)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.danger)
                    .hover(|s| s.bg(theme.danger.alpha(0.1)))
                    .child(icon(Icon::Trash, px(15.), theme.danger))
                    .child(label)
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(TEXT_XS)
                            .text_color(theme.danger.alpha(0.7))
                            .child("⌫"),
                    )
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(DeleteEvent), cx)),
            )
            .into_any_element()
    }

    fn render_calendar_picker(
        &self,
        event: &Event,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.menu == Some(Menu::EventCalendar);
        let color = self.data.color_of(event);
        let highlight = self.panel.as_ref().map_or(0, |p| p.menu_highlight);
        let menu = open.then(|| {
            let rows = self
                .data
                .calendars
                .iter()
                .enumerate()
                .map(|(ix, calendar)| {
                    let current = calendar.id == event.calendar;
                    let writable = matches!(
                        calendar.access_role,
                        crate::calendar::AccessRole::Owner | crate::calendar::AccessRole::Writer
                    );
                    menu_row(("calendar-choice", ix), ix == highlight, theme)
                        .role(gpui::Role::MenuItem)
                        .aria_label(calendar.name.clone())
                        .when(!writable, |d| d.opacity(0.4))
                        .child(div().size(px(10.)).rounded_full().bg(calendar.color))
                        .child(div().flex_1().child(calendar.name.clone()))
                        .when(current, |d| {
                            d.child(icon(Icon::Check, px(14.), theme.accent))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_menu_row(Menu::EventCalendar, ix, cx)
                        }))
                });
            dropdown(px(34.), px(210.), theme).children(rows)
        });
        let focus = self.panel.as_ref().map(|p| p.calendar_focus.clone());
        div()
            .child(
                control(div().id("event-calendar"), focus, theme)
                    .h(px(28.))
                    .pl(SPACE_SM)
                    .pr(SPACE_SM)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .when(open, |d| d.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                    .child(div().size(px(10.)).rounded_full().bg(color))
                    .child(self.calendar_name(event))
                    .when(self.can_move(event), |d| {
                        d.child(icon(Icon::ChevronDown, px(13.), theme.text_faint))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.click_control_menu(Menu::EventCalendar, cx)
                    })),
            )
            .when(open, |d| d.child(self.menu_backdrop(window, cx)))
            .children(menu.map(|m| {
                deferred(anchored().snap_to_window_with_margin(px(8.)).child(m)).with_priority(2)
            }))
            .into_any_element()
    }

    fn render_when(
        &self,
        panel: &EventPanel,
        event: &Event,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (start, end) = panel.times(event);
        let field = |f: Field, width: Option<f32>, cx: &mut Context<Self>| {
            self.render_date_time_field(panel, event, f, width, theme, window, cx)
        };
        let dash = || div().flex_none().text_color(theme.text_faint).child("–");
        let rows = if event.all_day {
            div()
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .child(field(Field::StartDate, None, cx))
                .child(dash())
                .child(field(Field::EndDate, None, cx))
        } else {
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2XS)
                .child(field(Field::StartDate, None, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_XS)
                        .child(field(Field::StartTime, Some(92.), cx))
                        .child(dash())
                        .child(field(Field::EndTime, Some(92.), cx))
                        .child(
                            div()
                                .ml(SPACE_SM)
                                .text_size(TEXT_SM)
                                .text_color(theme.text_faint)
                                .child(
                                    if end.date() != start.date()
                                        && end != time::midnight(start.date() + Duration::days(1))
                                    {
                                        format!("{} · next day", duration_label(end - start))
                                    } else {
                                        duration_label(end - start)
                                    },
                                ),
                        ),
                )
        };
        let all_day = event.all_day;
        let toggle = control(
            div().id("all-day"),
            Some(panel.all_day_focus.clone()),
            theme,
        )
        .mt(SPACE_SM)
        .px(SPACE_SM)
        .h(px(26.))
        .w_auto()
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .text_size(TEXT_SM)
        .text_color(theme.text_muted)
        .hover(|s| s.bg(theme.hover_bg))
        .child(switch(all_day, theme))
        .child("All day")
        .on_click(cx.listener(|this, _, window, cx| this.toggle_all_day(window, cx)));
        section(
            Icon::Clock,
            theme,
            div()
                .flex()
                .flex_col()
                .child(rows)
                .child(div().flex().child(toggle)),
        )
        .into_any_element()
    }

    /// A date or time field, with its picker under it while focused.
    #[allow(clippy::too_many_arguments)]
    fn render_date_time_field(
        &self,
        panel: &EventPanel,
        event: &Event,
        field: Field,
        width: Option<f32>,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input = panel.input(field).expect("date and time fields are inputs");
        let focused = panel.is_focused(field, window, cx);
        let picker = (focused && panel.picker == Some(field)).then(|| {
            let content = if field.is_date() {
                self.render_date_picker(panel, event, field, theme, cx)
            } else {
                self.render_time_picker(panel, event, field, theme, cx)
            };
            below(content)
        });
        div()
            .key_context(if field.is_date() {
                DATE_FIELD
            } else {
                TIME_FIELD
            })
            .flex()
            .flex_col()
            .map(|d| match width {
                Some(w) => d.w(px(w)).flex_none(),
                None => d.flex_1().min_w_0(),
            })
            .child(
                field_box(focused, theme).child(
                    Input::new(input)
                        .readonly(self.access_of(event) != Access::Edit)
                        .appearance(false)
                        .px(px(0.))
                        .h(px(28.))
                        .text_size(TEXT_BASE)
                        .font_weight(FontWeight::MEDIUM),
                ),
            )
            .children(picker)
            .into_any_element()
    }

    fn render_date_picker(
        &self,
        panel: &EventPanel,
        event: &Event,
        field: Field,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (start, end) = panel.times(event);
        let chosen = if field == Field::EndDate {
            (end - Duration::days(1)).date()
        } else {
            start.date()
        };
        let month = panel.picker_month;
        let grid_start = week_start(month);
        let today = today();
        let nav = |id: &'static str, glyph: Icon, months: i32| {
            div()
                .id(id)
                .size(HIT_TARGET)
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM)
                .hover(|s| s.bg(theme.hover_bg))
                .child(icon(glyph, px(14.), theme.icon))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(panel) = &mut this.panel {
                        panel.picker_month = add_months(panel.picker_month, months);
                        cx.notify();
                    }
                }))
        };
        let weekdays = ["M", "T", "W", "T", "F", "S", "S"].map(|d| {
            div()
                .w(px(32.))
                .flex()
                .justify_center()
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_faint)
                .child(d)
        });
        let weeks = (0..6).map(|w| {
            div().flex().children((0..7).map(|d| {
                let date = grid_start + Duration::days(w * 7 + d);
                let is_chosen = date == chosen;
                let in_month = date.month() == month.month();
                div()
                    .id(("picker-day", date.num_days_from_ce() as usize))
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .text_size(TEXT_SM)
                    .map(|d| {
                        if is_chosen {
                            d.bg(theme.accent)
                                .text_color(theme.accent_text)
                                .font_weight(FontWeight::SEMIBOLD)
                        } else {
                            d.text_color(if date == today {
                                theme.accent
                            } else if in_month {
                                theme.text
                            } else {
                                theme.text_faint
                            })
                            .when(date == today, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .hover(|s| s.bg(theme.hover_bg))
                        }
                    })
                    .child(date.day().to_string())
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_date(field, date, window, cx)
                    }))
            }))
        });
        div()
            .occlude()
            .mt(SPACE_XS)
            .p(SPACE_MD)
            .rounded(RADIUS_LG)
            .bg(theme.overlay_bg)
            .border_1()
            .border_color(theme.overlay_border)
            .shadow(popover_shadow(theme))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .child(
                div()
                    .pb(SPACE_SM)
                    .pl(SPACE_SM)
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(TEXT_BASE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(month.format("%B %Y").to_string()),
                    )
                    .child(nav("picker-prev", Icon::ChevronLeft, -1))
                    .child(nav("picker-next", Icon::ChevronRight, 1)),
            )
            .child(div().flex().pb(SPACE_2XS).children(weekdays))
            .children(weeks)
            .into_any_element()
    }

    fn render_time_picker(
        &self,
        panel: &EventPanel,
        event: &Event,
        field: Field,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (start, end) = panel.times(event);
        // Start: every slot of the day. End: every slot after the start, up
        // to a day later, each with how long that makes the event.
        let slots: Vec<(NaiveDateTime, Option<String>)> = if field == Field::StartTime {
            (0..24 * 60 / SLOT_MINUTES)
                .map(|i| {
                    (
                        time::midnight(start.date()) + Duration::minutes(i * SLOT_MINUTES),
                        None,
                    )
                })
                .collect()
        } else {
            (1..=24 * 60 / SLOT_MINUTES)
                .map(|i| {
                    let length = Duration::minutes(i * SLOT_MINUTES);
                    (start + length, Some(duration_label(length)))
                })
                .collect()
        };
        let current = if field == Field::StartTime {
            start
        } else {
            end
        };
        let rows = slots.into_iter().enumerate().map(|(ix, (value, length))| {
            let chosen = value == current;
            div()
                .id(("time-slot", ix))
                .h(px(30.))
                .flex_none()
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_SM)
                .text_size(TEXT_BASE)
                .map(|d| {
                    if chosen {
                        d.bg(theme.selected_bg).font_weight(FontWeight::SEMIBOLD)
                    } else {
                        d.hover(|s| s.bg(theme.hover_bg))
                    }
                })
                .child(time_label(value.time()))
                .children(length.map(|l| {
                    div()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_faint)
                        .child(l)
                }))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.set_time(field, value, window, cx)),
                )
        });
        div()
            .occlude()
            .mt(SPACE_XS)
            .w(px(if field == Field::EndTime { 190. } else { 140. }))
            .rounded(RADIUS_MD)
            .bg(theme.overlay_bg)
            .border_1()
            .border_color(theme.overlay_border)
            .shadow(popover_shadow(theme))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .child(
                div()
                    .id("time-picker")
                    .h(px(240.))
                    .p(SPACE_XS)
                    .overflow_y_scroll()
                    .track_scroll(&panel.picker_scroll)
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_place(
        &self,
        panel: &EventPanel,
        event: &Event,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let link = event.video_link.clone();
        let editable = self.access_of(event) == Access::Edit;
        // A Meet link from Google stays as Google made it (CAL-17).
        let video_readonly = !editable || !event.conference_uri.is_empty();
        div()
            .flex()
            .flex_col()
            .child(section(
                Icon::MapPin,
                theme,
                field_box(panel.is_focused(Field::Location, window, cx), theme)
                    .text_size(TEXT_BASE)
                    .child(field_input(&panel.location, !editable)),
            ))
            .child(
                section(
                    Icon::Video,
                    theme,
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child({
                            let focused = panel.is_focused(Field::Video, window, cx);
                            let input = panel.video.clone();
                            field_box(focused, theme)
                                .flex_1()
                                .min_w_0()
                                .text_size(TEXT_BASE)
                                .map(|d| {
                                    if focused || link.is_empty() {
                                        d.child(field_input(&panel.video, video_readonly))
                                    } else {
                                        // A long link ends in an ellipsis
                                        // until it's edited.
                                        d.child(
                                            div()
                                                .id("video-link")
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .cursor_text()
                                                .child(link.clone())
                                                .on_click(move |_, window, cx| {
                                                    input.update(cx, |input, cx| {
                                                        input.focus(window, cx)
                                                    })
                                                }),
                                        )
                                    }
                                })
                        })
                        .when(!link.is_empty(), |d| {
                            d.child(
                                div()
                                    .id("join-call")
                                    .flex_none()
                                    .h(px(28.))
                                    .px(SPACE_MD)
                                    .flex()
                                    .items_center()
                                    .gap(SPACE_SM)
                                    .rounded(RADIUS_MD)
                                    .bg(theme.accent)
                                    .text_color(theme.accent_text)
                                    .text_size(TEXT_SM)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .hover(|s| s.opacity(0.9))
                                    .child(icon(Icon::Video, px(14.), theme.accent_text))
                                    .child("Join")
                                    .on_click(move |_, _, cx| cx.open_url(&link)),
                            )
                        }),
                )
                .pt(px(0.)),
            )
            .into_any_element()
    }

    fn render_guests(
        &self,
        panel: &EventPanel,
        event: &Event,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let me = self.me();
        let is_me = |email: &str| email.eq_ignore_ascii_case(me);
        let editable = self.access_of(event) == Access::Edit;
        let organizer_attends = event
            .attendees
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(&event.organizer.email));

        let person = |id: (&'static str, usize),
                      contact: &Contact,
                      status: Option<RsvpStatus>,
                      role: Option<&'static str>,
                      removable: Option<usize>,
                      cx: &mut Context<Self>| {
            let name: SharedString = if is_me(&contact.email) {
                "You".into()
            } else {
                contact.display_name().to_string().into()
            };
            let detail = role
                .map(|r| r.to_string())
                .or_else(|| status.map(|s| status_text(s).to_string()))
                .unwrap_or_default();
            div()
                .id(id)
                .group("guest")
                .h(px(40.))
                .px(SPACE_SM)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .hover(|s| s.bg(theme.hover_bg))
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .child(avatar(
                            contact.initials(),
                            theme.avatar_color(&contact.email),
                            px(28.),
                            theme,
                        ))
                        .children(status.map(|s| status_badge(s, theme))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(TEXT_BASE)
                                .font_weight(FontWeight::MEDIUM)
                                .child(name),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_faint)
                                .child(detail),
                        ),
                )
                .children(removable.map(|ix| {
                    div()
                        .id(("guest-remove", ix))
                        .size(HIT_TARGET)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .opacity(0.)
                        .group_hover("guest", |s| s.opacity(1.))
                        .hover(|s| s.bg(theme.selected_bg))
                        .child(icon(Icon::X, px(12.), theme.text_faint))
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_guest(ix, cx)))
                }))
        };

        let mut rows: Vec<AnyElement> = Vec::new();
        if !event.attendees.is_empty() && !organizer_attends {
            rows.push(
                person(
                    ("organizer", 0),
                    &event.organizer,
                    None,
                    Some("Organizer"),
                    None,
                    cx,
                )
                .into_any_element(),
            );
        }
        for (ix, attendee) in event.attendees.iter().enumerate() {
            let organizer = attendee.email.eq_ignore_ascii_case(&event.organizer.email);
            rows.push(
                person(
                    ("guest", ix),
                    &attendee.contact(),
                    Some(attendee.status),
                    organizer.then_some("Organizer"),
                    (editable && !is_me(&attendee.email)).then_some(ix),
                    cx,
                )
                .into_any_element(),
            );
        }

        // Everyone listed: the organizer, who goes, and the guests.
        let count = rows.len();
        let yes = event
            .attendees
            .iter()
            .filter(|a| a.status == RsvpStatus::Accepted)
            .count()
            + usize::from(!event.attendees.is_empty() && !organizer_attends);
        let awaiting = event
            .attendees
            .iter()
            .filter(|a| a.status == RsvpStatus::NeedsAction)
            .count();
        let summary = (count > 0).then(|| {
            let mut parts = vec![format!(
                "{count} {}",
                if count == 1 { "person" } else { "people" }
            )];
            if yes > 0 {
                parts.push(format!("{yes} yes"));
            }
            if awaiting > 0 {
                parts.push(format!("{awaiting} awaiting"));
            }
            div()
                .px(SPACE_SM)
                .pb(SPACE_2XS)
                .text_size(TEXT_SM)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted)
                .child(parts.join(" · "))
        });

        let focused = panel.is_focused(Field::Guests, window, cx);
        let suggestions = (focused && !panel.suggestions.is_empty()).then(|| {
            let rows = panel.suggestions.iter().enumerate().map(|(row, &ix)| {
                let contact = self.book.borrow().contact(ix).clone();
                div()
                    .id(("guest-suggestion", row))
                    .h(px(40.))
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .when(row == panel.highlighted, |d| d.bg(theme.selected_bg))
                    .hover(|d| d.bg(theme.hover_bg))
                    .child(avatar(
                        contact.initials(),
                        theme.avatar_color(&contact.email),
                        px(26.),
                        theme,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(TEXT_BASE)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(contact.display_name().to_string()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(TEXT_SM)
                                    .text_color(theme.text_muted)
                                    .child(contact.email.clone()),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let contact = this.book.borrow().contact(ix).clone();
                        this.add_guest(contact, window, cx);
                    }))
            });
            below(
                div()
                    .occlude()
                    .mt(SPACE_XS)
                    .w(px(300.))
                    .p(SPACE_XS)
                    .flex()
                    .flex_col()
                    .rounded(RADIUS_MD)
                    .bg(theme.overlay_bg)
                    .border_1()
                    .border_color(theme.overlay_border)
                    .shadow(popover_shadow(theme))
                    .children(rows),
            )
        });

        section(
            Icon::Users,
            theme,
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .mx(px(-8.))
                        .flex()
                        .flex_col()
                        .children(summary)
                        .children(rows),
                )
                .when(editable, |d| {
                    d.child(
                        div()
                            .key_context(GUESTS)
                            .flex()
                            .flex_col()
                            .child(
                                field_box(focused, theme)
                                    .text_size(TEXT_BASE)
                                    .child(field_input(&panel.guests, false)),
                            )
                            .children(suggestions),
                    )
                }),
        )
        .into_any_element()
    }

    /// Reminder, busy or free, and the user's answer to an invite.
    fn render_options(
        &self,
        event: &Event,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.menu == Some(Menu::Reminder);
        let current = event.reminder_minutes;
        let highlight = self.panel.as_ref().map_or(0, |p| p.menu_highlight);
        let menu = open.then(|| {
            let rows = REMINDERS.iter().enumerate().map(|(ix, &minutes)| {
                menu_row(("reminder", ix), ix == highlight, theme)
                    .child(div().flex_1().child(reminder_label(minutes)))
                    .when(minutes == current, |d| {
                        d.child(icon(Icon::Check, px(14.), theme.accent))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_menu_row(Menu::Reminder, ix, cx)
                    }))
            });
            deferred(
                anchored()
                    .snap_to_window_with_margin(px(8.))
                    .child(dropdown(px(32.), px(200.), theme).children(rows)),
            )
            .with_priority(2)
        });

        let reminder = section(
            Icon::Bell,
            theme,
            div()
                .flex()
                .child(
                    control(
                        div().id("reminder"),
                        self.panel.as_ref().map(|p| p.reminder_focus.clone()),
                        theme,
                    )
                    .h(px(28.))
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_BASE)
                    .when(open, |d| d.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(reminder_label(current))
                    .child(icon(Icon::ChevronDown, px(13.), theme.text_faint))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.click_control_menu(Menu::Reminder, cx)),
                    ),
                )
                .when(open, |d| d.child(self.menu_backdrop(window, cx)))
                .children(menu),
        );

        let busy = event.busy;
        let availability = section(
            Icon::Briefcase,
            theme,
            div().flex().child(segmented(
                [("Busy", busy), ("Free", !busy)],
                "busy",
                theme,
                cx,
                |this, ix, cx| this.edit_event(cx, |e| e.busy = ix == 0),
            )),
        );

        let rsvp = event.my_status(self.me()).map(|status| {
            let id = event.id.clone();
            section(
                Icon::Check,
                theme,
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .child(
                        div()
                            .text_size(TEXT_BASE)
                            .text_color(theme.text_muted)
                            .child("Going?"),
                    )
                    .child(segmented(
                        [
                            ("Yes", status == RsvpStatus::Accepted),
                            ("Maybe", status == RsvpStatus::Tentative),
                            ("No", status == RsvpStatus::Declined),
                        ],
                        "rsvp",
                        theme,
                        cx,
                        move |this, ix, cx| {
                            let status = [
                                RsvpStatus::Accepted,
                                RsvpStatus::Tentative,
                                RsvpStatus::Declined,
                            ][ix];
                            this.set_rsvp(&id, status, cx)
                        },
                    )),
            )
        });

        div()
            .flex()
            .flex_col()
            .children(rsvp)
            .child(reminder)
            .child(availability.pt(px(0.)))
            .into_any_element()
    }
}

/// A row of the panel: an icon in the gutter, then its content.
fn section(glyph: Icon, theme: &Theme, content: impl IntoElement) -> gpui::Div {
    div()
        .px(SPACE_XL)
        .py(SPACE_SM)
        .flex()
        .gap(SPACE_SM)
        .child(
            div()
                .w(px(18.))
                .h(px(30.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(icon(glyph, px(16.), theme.text_faint)),
        )
        .child(div().flex_1().min_w_0().flex().flex_col().child(content))
}

fn field_input(input: &Entity<InputState>, readonly: bool) -> Input {
    Input::new(input)
        .readonly(readonly)
        .appearance(false)
        .px(px(0.))
        .text_size(TEXT_BASE)
}

fn divider(theme: &Theme) -> impl IntoElement {
    div().mx(SPACE_XL).my(SPACE_SM).h(px(1.)).bg(theme.border)
}

/// The box around an editable field: plain text until hovered or focused.
fn field_box(focused: bool, theme: &Theme) -> gpui::Div {
    div()
        .h(px(30.))
        .px(SPACE_SM)
        .flex()
        .items_center()
        .rounded(RADIUS_MD)
        .border_1()
        .map(|d| {
            if focused {
                d.bg(theme.search_bg).border_color(theme.accent.alpha(0.55))
            } else {
                d.border_color(gpui::transparent_black())
                    .hover(|s| s.bg(theme.hover_bg))
            }
        })
}

/// A popup under the element before it: a zero-height anchor in the flow,
/// so the popup starts where the next sibling would.
fn below(content: impl IntoElement) -> impl IntoElement {
    div().w_full().h(px(0.)).child(
        deferred(anchored().snap_to_window_with_margin(px(8.)).child(content)).with_priority(2),
    )
}

fn dropdown(top: gpui::Pixels, width: gpui::Pixels, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("dropdown")
        .occlude()
        .mt(top)
        .w(width)
        .p(SPACE_XS)
        .flex()
        .flex_col()
        .rounded(RADIUS_MD)
        .bg(theme.overlay_bg)
        .border_1()
        .border_color(theme.overlay_border)
        .shadow(popover_shadow(theme))
}

fn menu_row(
    id: (&'static str, usize),
    highlighted: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(30.))
        .px(SPACE_MD)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .text_size(TEXT_BASE)
        .when(highlighted, |d| d.bg(theme.selected_bg))
        .hover(|s| s.bg(theme.hover_bg))
}

/// A button of the panel that takes focus: tab reaches it, enter and space
/// work it (see `activate`), and a ring shows it has focus from the keys.
fn control(
    el: gpui::Stateful<gpui::Div>,
    focus: Option<FocusHandle>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let ring = theme.accent.alpha(0.7);
    el.key_context(PANEL_CONTROL)
        .border_1()
        .border_color(gpui::transparent_black())
        .when_some(focus, |d, focus| d.track_focus(&focus))
        .focus_visible(move |s| s.border_color(ring))
}

fn switch(on: bool, theme: &Theme) -> impl IntoElement {
    div()
        .w(px(28.))
        .h(px(16.))
        .flex_none()
        .p(SPACE_2XS)
        .flex()
        .rounded_full()
        .bg(if on { theme.accent } else { theme.selected_bg })
        .when(on, |d| d.justify_end())
        .child(
            div()
                .size(px(12.))
                .rounded_full()
                .bg(gpui::white())
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::black().alpha(0.25),
                    offset: gpui::point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                    inset: false,
                }]),
        )
}

/// A small segmented control; `on_pick` gets the index clicked.
fn segmented<const N: usize>(
    options: [(&'static str, bool); N],
    id: &'static str,
    theme: &Theme,
    cx: &mut Context<CalendarView>,
    on_pick: impl Fn(&mut CalendarView, usize, &mut Context<CalendarView>) + Clone + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .p(SPACE_2XS)
        .gap(SPACE_2XS)
        .rounded(RADIUS_MD)
        .bg(theme.chip_bg)
        .children(
            options
                .into_iter()
                .enumerate()
                .map(|(ix, (label, active))| {
                    let on_pick = on_pick.clone();
                    div()
                        .id((id, ix))
                        .h(px(24.))
                        .px(SPACE_MD)
                        .flex()
                        .items_center()
                        .rounded(RADIUS_SM)
                        .text_size(TEXT_SM)
                        .font_weight(FontWeight::MEDIUM)
                        .map(|d| {
                            if active {
                                d.bg(theme.overlay_bg).text_color(theme.text).shadow(vec![
                                    gpui::BoxShadow {
                                        color: theme.overlay_shadow.alpha(0.18),
                                        offset: gpui::point(px(0.), px(1.)),
                                        blur_radius: px(2.),
                                        spread_radius: px(0.),
                                        inset: false,
                                    },
                                ])
                            } else {
                                d.text_color(theme.text_muted)
                                    .hover(|s| s.text_color(theme.text))
                            }
                        })
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| on_pick(this, ix, cx)))
                }),
        )
}

fn status_text(status: RsvpStatus) -> &'static str {
    match status {
        RsvpStatus::Accepted => "Going",
        RsvpStatus::Tentative => "Maybe",
        RsvpStatus::Declined => "Not going",
        RsvpStatus::NeedsAction => "Awaiting reply",
    }
}

fn status_color(status: RsvpStatus, theme: &Theme) -> Hsla {
    match status {
        RsvpStatus::Accepted => theme.success,
        RsvpStatus::Tentative => theme.star,
        RsvpStatus::Declined => theme.danger,
        RsvpStatus::NeedsAction => theme.text_faint,
    }
}

/// The answer, as a small badge on the guest's avatar.
fn status_badge(status: RsvpStatus, theme: &Theme) -> impl IntoElement {
    let glyph = match status {
        RsvpStatus::Accepted => Icon::Check,
        RsvpStatus::Tentative => Icon::Help,
        RsvpStatus::Declined => Icon::X,
        RsvpStatus::NeedsAction => Icon::Minus,
    };
    div()
        .absolute()
        .right(px(-5.))
        .bottom(px(-5.))
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(status_color(status, theme))
        .border_2()
        .border_color(theme.overlay_bg)
        .child(icon(glyph, px(8.), theme.cal_selected_text))
}

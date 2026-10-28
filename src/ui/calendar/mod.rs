//! The calendar: a header, the Day, Week or Month grid, and the event panel
//! docked on the right. `CalendarView` owns the calendar's state and handles
//! its actions; the files beside this one render it.
//!
//! What the mouse drags in the grid (`drag.rs`) and the event panel go
//! through:
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
//! lists it as an exception (`detach`). "This and following" ends the series
//! before the occurrence and starts a new one there (`split_series`). Drags
//! and deletes of an occurrence ask first (`SeriesPrompt`); the panel asks
//! after the first edit.

pub mod agenda;
pub mod drag;
pub mod event_panel;
pub mod header;
pub mod mini_month;
pub mod month_view;
pub mod repeat;
pub mod week_view;

use std::{cell::Cell, rc::Rc, time::Duration as StdDuration};

use chrono::{Duration, Local, NaiveDate, NaiveDateTime};
use gpui::{
    AnyElement, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, Render,
    ScrollDelta, ScrollHandle, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Subscription, Task, TouchPhase, WeakEntity, Window, actions, anchored,
    base::spring,
    component::{
        input::{InputEvent, InputState},
        native_menu::NativeMenu,
    },
    deferred, div, point,
    prelude::FluentBuilder as _,
    px,
};

use self::{drag::GridDrag, event_panel::EventPanel};
use crate::ui::style::*;
use crate::{
    actions::*,
    app::Retime,
    backend::{
        Backend, BackendEvent, CalOp, OpId, SendUpdates,
        calendar::{Access, access, asks_guests, has_guests, instance_id, is_organizer, refusal},
    },
    calendar::{
        CalendarData, Event, GoogleSettings, Occurrence, RsvpStatus, SeriesScope, occurrences,
        policy, quick_add,
        time::{self, LocalZone, add_months, month_grid, next_half_hour, week_start},
    },
    contacts::SharedContactBook,
    theme::Theme,
    ui::{
        controls::{ButtonKind, popover_surface, push_button},
        keys_caps,
        motion::{Rise, SPRING_EXIT, SPRING_PANE, SPRING_SCROLL},
    },
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
/// The hour the grids open scrolled to, away from today.
const FIRST_VISIBLE_HOUR: f32 = 7.5;
/// With today on screen, how far above now the grids open.
const NOW_LEAD_HOURS: f32 = 2.;
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
    /// "Add reminder" (CAL-33).
    Reminder,
    /// The repeat presets (CAL-29).
    Repeat,
    /// The date field and month under the header's title.
    GoToDate,
    /// A Month day's events, from its "+N more" (ME-087).
    DayList(NaiveDate),
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
    /// The question asked.
    pub(crate) fn question(self) -> &'static str {
        match self {
            PromptKind::Delete => "Delete this repeating event?",
            PromptKind::Move { .. } => "Move this repeating event?",
        }
    }

    /// The answers offered, the first one taken by enter. Deletes say what
    /// they delete (MB-028).
    fn choices(self) -> &'static [(SeriesScope, &'static str)] {
        match self {
            PromptKind::Delete => &[
                (SeriesScope::This, "Delete This Event"),
                (SeriesScope::Following, "Delete This and Following"),
                (SeriesScope::All, "Delete All Events"),
            ],
            PromptKind::Move { .. } => SCOPE_CHOICES,
        }
    }

    /// How the default answer and the others look.
    fn tones(self) -> (ButtonKind, ButtonKind) {
        match self {
            PromptKind::Delete => (ButtonKind::Destructive, ButtonKind::Danger),
            PromptKind::Move { .. } => (ButtonKind::Primary, ButtonKind::Plain),
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
    /// "This and following": the ops to queue, and what to put back on
    /// Cancel (or Undo, with the toast's message, once they went).
    Split {
        ops: Vec<CalOp>,
        removal: Removal,
        message: String,
    },
}

impl GuestPrompt {
    fn question(&self) -> &'static str {
        match self.kind {
            GuestAsk::Save { before: None, .. } => "Send invitations to guests?",
            GuestAsk::Save { .. } | GuestAsk::Split { .. } => "Send updates to guests?",
            GuestAsk::Delete { .. } => "Send cancellation to guests?",
        }
    }
}

/// "Save this event?", asked when something would move the panel off a new
/// event that isn't saved yet.
#[derive(Clone, Debug)]
pub struct LeavePrompt {
    pub event_id: String,
    /// What the user was doing, carried on with once answered.
    pub then: Leave,
}

#[derive(Clone, Debug)]
pub enum Leave {
    Open {
        id: String,
        occurrence: NaiveDateTime,
        focus_title: bool,
    },
    /// An arrow key moved the selection; the panel follows it.
    Select(Selection),
    /// A click on another event.
    Click(Selection),
    /// The palette jumped to an event.
    Reveal(String),
    Create {
        start: NaiveDateTime,
        end: NaiveDateTime,
        all_day: bool,
    },
    QuickAdd(String),
    Restore(Box<Discarded>),
    /// The panel closes; nothing more to do.
    Close,
}

impl Leave {
    /// The event this goes to, when it's one.
    fn target(&self) -> Option<&str> {
        match self {
            Leave::Open { id, .. } | Leave::Reveal(id) => Some(id),
            Leave::Select(s) | Leave::Click(s) => Some(&s.event_id),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveChoice {
    Save,
    Discard,
}

const LEAVE_CHOICES: &[(LeaveChoice, &str)] = &[
    (LeaveChoice::Save, "Save"),
    (LeaveChoice::Discard, "Discard"),
];

/// A new event that was discarded, for Undo to bring back unsaved.
#[derive(Clone, Debug)]
pub struct Discarded {
    event: Event,
    meet_request: Option<String>,
    /// The delete, when the event had already gone in.
    op: Option<OpId>,
}

/// The toast's words for a discarded new event.
fn discarded_message(event: &Event) -> String {
    if event.title.trim().is_empty() {
        "Discarded new event".into()
    } else {
        format!("Discarded “{}”", event.title.trim())
    }
}

/// The series scope answers, for moves and edits.
pub(crate) const SCOPE_CHOICES: &[(SeriesScope, &str)] = &[
    (SeriesScope::This, "This Event"),
    (SeriesScope::Following, "This and Following"),
    (SeriesScope::All, "All Events"),
];

const GUEST_CHOICES: &[(SendUpdates, &str)] = &[
    (SendUpdates::All, "Send"),
    (SendUpdates::None, "Don't Send"),
];

/// The op with `send` as its `sendUpdates`.
fn with_send(mut op: CalOp, send: SendUpdates) -> CalOp {
    if let CalOp::Insert { send_updates, .. }
    | CalOp::Patch { send_updates, .. }
    | CalOp::Split { send_updates, .. } = &mut op
    {
        *send_updates = send;
    }
    op
}

/// "This and following" as Google takes it: one op, which ends the series
/// (`If-Match` on its etag) and only then puts in the new one (CAL-11 id).
fn split_ops(before: &Event, old: &Event, new: &Event) -> Vec<CalOp> {
    vec![CalOp::Split {
        id: old.id.clone(),
        before: before.clone(),
        after: old.clone(),
        new: new.clone(),
        send_updates: SendUpdates::None,
    }]
}

/// What a delete or a series split took out, to put back on undo.
#[derive(Clone, Debug, Default)]
pub struct Removal {
    /// Events removed, each with the index it had, by index.
    removed: Vec<(usize, Event)>,
    /// The series as it was, when the change kept it rather than removing
    /// it (an exception, or an earlier end).
    series_before: Option<Event>,
    /// Events the change made (a split's new series), which undo deletes.
    added: Vec<String>,
    select: Option<Selection>,
    /// The delete, while the backend holds it back.
    op: Option<OpId>,
}

impl Removal {
    /// What Undo calls it: "Undo Delete Event", or "Undo Change Event" for
    /// a "This and following" change, which adds a series.
    pub fn undo_title(&self) -> &'static str {
        if self.added.is_empty() {
            "Delete Event"
        } else {
            "Change Event"
        }
    }
}

/// An event moved or resized, for the app's undo stack (MA-071). Emitted
/// beside `CalendarEvent`, so the app subscribes to it on its own.
pub struct Retimed {
    /// "Move Event" or "Resize Event".
    pub title: &'static str,
    pub retime: Retime,
}

impl EventEmitter<Retimed> for CalendarView {}

pub enum CalendarEvent {
    /// A delete or a series split: the app shows a toast that can undo it.
    Undoable {
        message: String,
        removal: Box<Removal>,
    },
    /// W6: a drag, edit or delete of an event the user can't change was
    /// refused; the toast says why (CAL-22).
    Refused(String),
    /// W6: a guest was clicked: their contact card (CON-5).
    /// MB-069: at the click on the guest.
    ShowContact(crate::data::Contact, Option<Point<Pixels>>),
    /// A new event was discarded: the app shows a toast that brings it back.
    Discarded {
        message: String,
        discarded: Box<Discarded>,
    },
}

/// Calendar settings changed in Pigeon; `None` follows Google (CAL-41).
#[derive(Clone, Debug, Default)]
pub struct CalendarPrefs {
    pub event_minutes: Option<u32>,
    /// The id of the calendar new events go to; `None` picks one
    /// (`calendar::new_event_calendar`).
    pub default_calendar: Option<String>,
}

/// "Add Google Meet" for the open event (CAL-34).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeetState {
    /// The calendar takes Meet, and the event has none.
    Offer,
    Adding,
    /// Google didn't make it; asking again uses the same request id.
    Failed,
}

/// The panel sliding shut, drawn from what it showed: its event may already
/// be gone (MD-002). Read-only; nothing writes back from it.
struct ClosingPanel {
    panel: EventPanel,
    event: Event,
    was_new: bool,
}

/// The grid's scroll gliding from `from` to `to` (MD-040); `last` is where
/// it set it, so a scroll by hand shows.
#[derive(Clone, Copy)]
struct ScrollGlide {
    from: Pixels,
    to: Pixels,
    last: Option<Pixels>,
}

/// A deleted event's block as it was, fading out (MD-038). Drawn only;
/// nothing reads from it.
#[derive(Clone)]
pub(crate) struct RemovedBlock {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub title: SharedString,
    pub color: gpui::Hsla,
    /// Drawn once at full opacity, so its spring starts from there.
    pub started: bool,
    pub fade: f32,
}

/// The parts of the calendar drawn as views of their own (MC-036, MD-004).
/// Each animates on its own frames, so the panel's slide, the header's
/// controls and the prompts' entrances don't redraw the grid. The grid is
/// cached: it redraws when the calendar notifies, when its own elements
/// (hover, scroll, its springs) do, when its bounds change, or when the
/// window refreshes (theme). `GridKey` covers what it reads from outside.
#[derive(Clone)]
struct Parts {
    grid: Entity<CalendarPart>,
    header: Entity<CalendarPart>,
    panel: Entity<CalendarPart>,
    move_prompt: Entity<CalendarPart>,
    modal: Entity<CalendarPart>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Grid,
    Header,
    Panel,
    MovePrompt,
    Modal,
}

pub struct CalendarPart {
    calendar: WeakEntity<CalendarView>,
    part: Part,
    _observe: Option<Subscription>,
}

impl Render for CalendarPart {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        self.calendar
            .update(cx, |calendar, cx| calendar.render_part(part, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// What the grid reads that changes outside the calendar without telling
/// it: the text size, the clock, the week's first day and the account.
#[derive(Clone, PartialEq)]
struct GridKey {
    rem: Pixels,
    clock_24: bool,
    first_weekday: chrono::Weekday,
    me: String,
}

pub struct CalendarView {
    pub data: CalendarData,
    pub view: ViewKind,
    /// The selected day, which the views are built around: the day shown,
    /// or one in the week or month shown. The selected event is on it.
    pub anchor: NaiveDate,
    pub selected: Option<Selection>,
    pub panel: Option<EventPanel>,
    /// The panel's event was reloaded; its fields catch up on render.
    panel_stale: bool,
    /// The panel's event, and the occurrence shown, as they were when the
    /// panel opened on it: what Discard puts back. After "This event" or
    /// "This and following" it's the new event as it was before the change.
    discard_base: Option<(Event, NaiveDateTime)>,
    /// The panel sliding shut, drawn until its width spring reaches 0.
    closing_panel: Option<ClosingPanel>,
    /// The parts drawn as views of their own (MC-036), made on first render.
    parts: Option<Parts>,
    /// What the grid reads from outside the calendar, as last drawn.
    grid_key: Option<GridKey>,
    /// The grid's width as last laid out: beside the rail, less a docked
    /// panel while one shows.
    grid_width: f32,
    /// The Day view shows its month and list (MD-003, with hysteresis).
    day_side: bool,
    /// The side the floating panel moved to while open, so it fades in
    /// there (MD-006).
    panel_flip: Option<bool>,
    panel_side: Option<bool>,
    pub menu: Option<Menu>,
    pub prompt: Option<SeriesPrompt>,
    pub show_week_numbers: bool,
    pub focus_handle: FocusHandle,
    pub grid_scroll: ScrollHandle,
    /// The mouse drag in the time grid, if one is on.
    pub drag: Option<GridDrag>,
    /// A cancelled drag's event, its ghost gliding home (MD-036).
    returning: Option<Occurrence>,
    /// How far a drag has lifted its event (`settle_ghost`).
    lift: f32,
    /// The grid gliding to Today or to an event (MD-040).
    scroll_glide: Option<ScrollGlide>,
    /// A two-finger swipe across the grid: how far it went, and whether it
    /// turned the page yet (MA-080).
    swipe: Option<(Point<Pixels>, bool)>,
    /// The event made or brought back last, whose block fades in once
    /// (MD-038); cleared when the period or view changes.
    just_added: Option<String>,
    /// A deleted event's block, fading out where it was (MD-038).
    removed_block: Option<RemovedBlock>,
    /// Where the day columns of the time grid were last painted, in the
    /// window, scrolled with them: maps the mouse to a day and a time.
    grid_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The month the mini months show, after their arrows moved it away
    /// from the selected day's.
    mini_month: Option<NaiveDate>,
    date_input: Entity<InputState>,
    book: SharedContactBook,
    backend: Entity<Backend>,
    /// The event made last, while it's open and not yet saved: Save keeps
    /// it, Discard (X, Escape) drops it, and it's dropped for having no
    /// title.
    new_event: Option<String>,
    /// Edits settling (`flush_edit`).
    pending: Option<PendingEdit>,
    /// A new event that goes in once it has a title and the panel leaves
    /// it.
    unsent: Option<String>,
    _edit_timer: Option<Task<()>>,
    /// An answer to an invite, settling.
    pending_rsvp: Option<(String, RsvpStatus)>,
    _rsvp_timer: Option<Task<()>>,
    pub guest_prompt: Option<GuestPrompt>,
    /// Guest questions asked while one was open: each waits its turn
    /// rather than answering the open one (MB-041).
    queued_guest_prompts: Vec<GuestPrompt>,
    pub leave_prompt: Option<LeavePrompt>,
    /// The all-day lane shows every lane, not the first few (ME-040).
    all_day_expanded: bool,
    /// The date the go-to-date field reads as, kept as it's typed
    /// (MC-056).
    typed: Option<NaiveDate>,
    /// The header's view switch: ←/→ change the view while it has focus.
    view_switch_focus: FocusHandle,
    /// The calendars button: arrows move through its open menu (ME-045).
    calendars_focus: FocusHandle,
    calendars_highlight: Option<usize>,
    /// The user's Google Calendar settings (CAL-41).
    pub google: GoogleSettings,
    pub prefs: CalendarPrefs,
    /// Meet requests asked for, by event id: the id stays the same on a
    /// retry (CAL-34).
    meet_requests: std::collections::HashMap<String, String>,
    /// Repaints just past each minute so the current-time line moves.
    /// Reminders are app-wide (`app::today`).
    _tick: Task<()>,
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

/// 32 hex digits, which Google takes as an event id (CAL-11).
fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
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
        // On the minute, as the menu bar clock turns (MD-034).
        let tick = cx.spawn(async |this, cx| {
            loop {
                let second = chrono::Timelike::second(&Local::now()) as u64;
                cx.background_executor()
                    .timer(StdDuration::from_secs(61 - second.min(60)))
                    .await;
                let ticked = this.update(cx, |_, cx| cx.notify());
                if ticked.is_err() {
                    break;
                }
            }
        });
        let date_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Try nov 12, next fri"));
        let date_events = cx.subscribe_in(&date_input, window, Self::on_date_input_event);
        // Google's settings are read with the calendars (CAL-41).
        let backend_events = cx.subscribe(&backend, |this, _, event: &BackendEvent, cx| {
            if matches!(
                event,
                BackendEvent::CalendarsChanged | BackendEvent::SignedIn
            ) {
                this.load_google(cx);
            }
        });
        let google = backend.read(cx).calendar_settings();
        let mut this = Self {
            data,
            view: ViewKind::Week,
            anchor: today(),
            selected: None,
            panel: None,
            panel_stale: false,
            discard_base: None,
            closing_panel: None,
            parts: None,
            grid_key: None,
            grid_width: 0.,
            day_side: false,
            panel_flip: None,
            panel_side: None,
            menu: None,
            prompt: None,
            show_week_numbers: false,
            focus_handle: cx.focus_handle(),
            grid_scroll: ScrollHandle::new(),
            drag: None,
            returning: None,
            lift: 0.,
            scroll_glide: None,
            swipe: None,
            just_added: None,
            removed_block: None,
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
            queued_guest_prompts: Vec::new(),
            leave_prompt: None,
            all_day_expanded: false,
            typed: None,
            view_switch_focus: cx.focus_handle().tab_stop(true),
            calendars_focus: cx.focus_handle().tab_stop(true),
            calendars_highlight: None,
            google,
            prefs: CalendarPrefs::default(),
            meet_requests: Default::default(),
            _tick: tick,
            _autoscroll: None,
            _subscriptions: vec![date_events, backend_events],
        };
        this.apply_display();
        this.scroll_to_start();
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

    /// Where the grid opens: a little before now when today is on screen
    /// (ME-043), else at the morning. The grid clamps it at the bottom.
    fn scroll_to_start(&mut self) {
        self.scroll_glide = None;
        let (from, to) = self.visible_range();
        let today = today();
        let hour = if (from..to).contains(&today) {
            let now = time::minutes_into(today, now()) as f32 / 60.;
            (now - NOW_LEAD_HOURS).max(0.)
        } else {
            FIRST_VISIBLE_HOUR
        };
        self.grid_scroll
            .set_offset(point(px(0.), px(-hour * HOUR_HEIGHT)));
    }

    /// Scroll the time grid so `[start, end)` minutes into the day show.
    fn reveal_minutes(&mut self, start: i64, end: i64) {
        self.scroll_glide = None;
        if let Some(y) = self.reveal_offset(start, end) {
            let x = self.grid_scroll.offset().x;
            self.grid_scroll.set_offset(point(x, y));
        }
    }

    /// `reveal_minutes`, gliding there on `SPRING_SCROLL` (MD-040); the
    /// wheel or the next key stops it.
    fn glide_to_minutes(&mut self, start: i64, end: i64) {
        self.scroll_glide = self.reveal_offset(start, end).map(|to| ScrollGlide {
            from: self.grid_scroll.offset().y,
            to,
            last: None,
        });
    }

    /// The grid's scroll offset that shows `[start, end)` minutes into the
    /// day, if it has to move.
    fn reveal_offset(&self, start: i64, end: i64) -> Option<Pixels> {
        let viewport = self.grid_scroll.bounds().size.height;
        if viewport <= px(0.) {
            return None;
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
            return None;
        };
        let max = self.grid_scroll.max_offset().y;
        Some(-y.clamp(px(0.), max.max(px(0.))))
    }

    /// Moves a glide on by a frame, before the grid draws; scrolling by
    /// hand meanwhile stops it.
    fn step_glide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(glide) = &mut self.scroll_glide else {
            return;
        };
        let offset = self.grid_scroll.offset();
        if glide
            .last
            .is_some_and(|last| (offset.y - last).abs() > px(0.5))
        {
            self.scroll_glide = None;
            return;
        }
        let started = glide.last.is_some();
        let key = gpui::ElementId::from(("grid-glide", f32::from(glide.to) as i32 as usize));
        let y = spring(
            key,
            f32::from(if started { glide.to } else { glide.from }),
            SPRING_SCROLL,
            window,
            cx,
        );
        let y = px(y);
        glide.last = Some(y);
        self.grid_scroll.set_offset(point(offset.x, y));
        if !started {
            window.request_animation_frame();
        } else if (y - glide.to).abs() < px(0.5) {
            self.scroll_glide = None;
        }
    }

    pub fn set_view(&mut self, view: ViewKind, cx: &mut Context<Self>) {
        self.drag = None;
        self.just_added = None;
        if self.view != view {
            let from_month = self.view == ViewKind::Month;
            self.view = view;
            if from_month {
                self.scroll_to_start();
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
        self.just_added = None;
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
        self.just_added = None;
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
                    if self.ask_to_leave(Leave::Close, cx) {
                        return;
                    }
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
        if self.panel.is_some() && self.ask_to_leave(Leave::Select(selection.clone()), cx) {
            return;
        }
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
            let end = time::minutes_into(day, end).min(drag::DAY_MINUTES);
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
        self.click_selection(selection, window, cx);
    }

    fn click_selection(
        &mut self,
        selection: Selection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ask_to_leave(Leave::Click(selection.clone()), cx) {
            return;
        }
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
            let then = Leave::Open {
                id: id.to_string(),
                occurrence,
                focus_title,
            };
            if self.ask_to_leave(then, cx) {
                return;
            }
            self.leave_panel(window, cx);
            let Some(event) = self.event(id).cloned() else {
                return;
            };
            self.menu = None;
            self.closing_panel = None;
            self.panel = Some(EventPanel::new(&event, occurrence, window, cx));
            self.discard_base = Some((event, occurrence));
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
        self.discard_base = None;
        self.leave_prompt = None;
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
            let removed = self.data.events.remove(ix);
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
                self.start_closing(Some(removed), true);
            }
        }
    }

    /// The panel slides shut from what it shows now, even once its event
    /// is gone (MD-002). `event` is the event as it was; without one the
    /// live event is used.
    fn start_closing(&mut self, event: Option<Event>, was_new: bool) {
        let Some(panel) = self.panel.take() else {
            return;
        };
        let event = event.or_else(|| self.event(&panel.event_id).cloned());
        self.closing_panel = event.map(|event| ClosingPanel {
            panel,
            event,
            was_new,
        });
    }

    /// Whether the panel shows a new event that hasn't been saved yet.
    pub(crate) fn panel_is_new(&self) -> bool {
        self.panel
            .as_ref()
            .is_some_and(|p| self.new_event.as_deref() == Some(p.event_id.as_str()))
    }

    /// Save (⌘S): a new event goes in, through the same path as before
    /// (asking about guests); edits to others already went as typed. Either
    /// way the panel closes. A new event needs a title first.
    fn save_event(&mut self, _: &SaveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.leave_prompt.is_some() {
            return self.answer_leave(Some(LeaveChoice::Save), window, cx);
        }
        if self.panel.is_none() {
            cx.propagate();
            return;
        }
        self.save_panel(window, cx);
    }

    /// `save_event` on the open panel; whether it closed.
    fn save_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(panel) = &self.panel else {
            return false;
        };
        if self.panel_is_new() && panel.title.read(cx).value().trim().is_empty() {
            let title = panel.title.clone();
            title.update(cx, |input, cx| input.focus(window, cx));
            cx.emit(CalendarEvent::Refused(
                "Add a title to save the event".into(),
            ));
            return false;
        }
        // The question about a repeating event's change stays to be answered.
        if self.asking_scope() {
            return false;
        }
        self.close_panel(window, cx);
        true
    }

    /// The unsaved new event the panel shows, if it has anything filled in:
    /// one without is nothing to ask about.
    fn unsaved_new_event(&self) -> Option<String> {
        let panel = self.panel.as_ref().filter(|_| self.panel_is_new())?;
        self.event(&panel.event_id)
            .filter(|e| e.has_content())
            .map(|e| e.id.clone())
    }

    /// Before something moves the panel off an unsaved new event: ask "Save
    /// this event?" and return true, so the caller stops. `then` carries on
    /// once it's answered (`answer_leave`).
    pub(crate) fn ask_to_leave(&mut self, then: Leave, cx: &mut Context<Self>) -> bool {
        let Some(event_id) = self.unsaved_new_event() else {
            return false;
        };
        if then.target() == Some(event_id.as_str()) {
            return false;
        }
        self.leave_prompt = Some(LeavePrompt { event_id, then });
        cx.notify();
        true
    }

    /// Save or Discard, then what the user was doing; `None` is Cancel,
    /// which stays on the new event.
    pub fn answer_leave(
        &mut self,
        choice: Option<LeaveChoice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.leave_prompt.take() else {
            return;
        };
        match choice {
            None => {}
            Some(LeaveChoice::Discard) => {
                self.discard_new_event(window, cx);
                self.carry_on(prompt.then, window, cx);
            }
            Some(LeaveChoice::Save) => {
                // Without a title it stays, asking for one.
                if !self.save_panel(window, cx) {
                    return cx.notify();
                }
                // A question about the guests is for that event; the rest
                // waits for the user to do it again.
                if self.guest_prompt.is_none() {
                    self.carry_on(prompt.then, window, cx);
                }
            }
        }
        cx.notify();
    }

    fn carry_on(&mut self, then: Leave, window: &mut Window, cx: &mut Context<Self>) {
        match then {
            Leave::Open {
                id,
                occurrence,
                focus_title,
            } => self.open_event(&id, occurrence, focus_title, window, cx),
            Leave::Select(selection) => {
                self.select(selection.clone(), window, cx);
                self.open_event(&selection.event_id, selection.start, false, window, cx);
            }
            Leave::Click(selection) => self.click_selection(selection, window, cx),
            Leave::Reveal(id) => {
                if let Some(ix) = self.data.event_ix(&id) {
                    self.reveal_event(ix, window, cx);
                }
            }
            Leave::Create {
                start,
                end,
                all_day,
            } => {
                self.create_event(start, end, all_day, window, cx);
            }
            Leave::QuickAdd(text) => self.quick_add(&text, window, cx),
            Leave::Restore(discarded) => self.restore_discarded(*discarded, window, cx),
            Leave::Close => {}
        }
    }

    /// Discard (and X or Escape on a new event): it goes without being
    /// saved, and nothing of it reaches the backend. A toast offers Undo.
    pub(crate) fn discard_new_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.new_event.take() else {
            return self.close_panel(window, cx);
        };
        let never_went = self.unsent.as_ref() == Some(&id);
        if never_went {
            self.unsent = None;
        }
        // Edits to other events still settling go.
        self.flush_edit(cx);
        let event = self.event(&id).cloned();
        self.data.events.retain(|e| e.id != id);
        let meet_request = self.meet_requests.remove(&id);
        let op = (!never_went).then(|| {
            self.queue(
                CalOp::Delete {
                    id: id.clone(),
                    occurrence: None,
                    scope: SeriesScope::All,
                    send_updates: SendUpdates::None,
                },
                cx,
            )
        });
        if self.selected.as_ref().is_some_and(|s| s.event_id == id) {
            self.selected = None;
        }
        self.start_closing(event.clone(), true);
        self.close_panel(window, cx);
        // An empty one is nothing to bring back.
        if let Some(event) = event.filter(|e| e.has_content()) {
            cx.emit(CalendarEvent::Discarded {
                message: discarded_message(&event),
                discarded: Box::new(Discarded {
                    event,
                    meet_request,
                    op,
                }),
            });
        }
    }

    /// Undo a discard: the new event and its panel are back, still unsaved.
    pub fn restore_discarded(
        &mut self,
        discarded: Discarded,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Discarded {
            event,
            meet_request,
            op,
        } = discarded;
        let (id, start) = (event.id.clone(), event.start);
        if self.event(&id).is_some() {
            return;
        }
        if self.ask_to_leave(
            Leave::Restore(Box::new(Discarded {
                event: event.clone(),
                meet_request: meet_request.clone(),
                op,
            })),
            cx,
        ) {
            return;
        }
        // Inside the undo window a delete that was held back is cancelled
        // and the event still stands in Google; past it, or if it never
        // went, it goes in when saved.
        let still_there = op.is_some_and(|op| self.backend.update(cx, |b, _| b.cancel(op)));
        self.data.events.push(event);
        if let Some(request) = meet_request {
            self.meet_requests.insert(id.clone(), request);
        }
        self.select_day_in_view(start.date());
        self.selected = Some(Selection {
            event_id: id.clone(),
            start,
        });
        self.open_event(&id, start, true, window, cx);
        if !still_there {
            self.unsent = Some(id.clone());
        }
        self.just_added = Some(id.clone());
        self.new_event = Some(id);
        cx.notify();
    }

    pub fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_new = self.panel_is_new();
        self.leave_panel(window, cx);
        // Slides shut, from a snapshot when its event just went; the panel
        // part drops it once the width spring is at 0.
        self.start_closing(None, was_new);
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

    /// Whether `id` has a change not yet handed to the backend: settling,
    /// waiting for its scope or for "Send updates to guests?".
    pub fn has_local_edit(&self, id: &str) -> bool {
        self.pending.as_ref().is_some_and(|p| p.id == id)
            || self.unsent.as_deref() == Some(id)
            || self
                .panel
                .as_ref()
                .is_some_and(|p| p.event_id == id && p.edit_scope.asking())
            || self
                .guest_prompt
                .iter()
                .chain(&self.queued_guest_prompts)
                .any(|p| p.event_id == id)
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

    /// `can_edit` for the event at `ix`, without finding it by id.
    pub fn can_edit_ix(&self, ix: usize) -> bool {
        self.data
            .events
            .get(ix)
            .is_some_and(|e| self.access_of(e) == Access::Edit)
    }

    /// Why `event` can't be changed (CAL-22); `None` when it can.
    pub fn refusal_of(&self, event: &Event) -> Option<String> {
        let calendar = self.data.calendar(&event.calendar);
        refusal(
            self.access_of(event),
            calendar.map(|c| c.access_role).unwrap_or_default(),
            calendar.map_or("", |c| c.name.as_str()),
        )
    }

    /// Say why the change to `id` didn't happen, in a toast.
    pub(crate) fn refuse(&self, id: &str, cx: &mut Context<Self>) {
        if let Some(why) = self.event(id).and_then(|e| self.refusal_of(e)) {
            cx.emit(CalendarEvent::Refused(why));
        }
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
        // A new event still open waits for Save (or the panel leaving it).
        let unsent = self
            .unsent
            .as_ref()
            .filter(|id| self.new_event.as_ref() != Some(*id))
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
        // No longer repeating: its occurrences changed on their own go.
        if before.recurrence.is_some() && after.recurrence.is_none() {
            self.data
                .events
                .retain(|e| e.recurring_event_id.as_deref() != Some(id.as_str()));
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
    /// earlier change) keeps it; this one waits for its turn (MB-041).
    fn ask_guests(&mut self, prompt: GuestPrompt, cx: &mut Context<Self>) {
        if self.guest_prompt.is_some() {
            self.queued_guest_prompts.push(prompt);
        } else {
            self.guest_prompt = Some(prompt);
        }
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
        if !self.queued_guest_prompts.is_empty() {
            self.guest_prompt = Some(self.queued_guest_prompts.remove(0));
        }
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
                // Cancel after Save: the new event opens again, unsaved.
                None if self.panel.is_none()
                    && let Some(start) = self.event(&id).map(|e| e.start) =>
                {
                    self.open_event(&id, start, true, window, cx);
                    self.unsent = Some(id.clone());
                    self.new_event = Some(id);
                    cx.notify();
                    return;
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
            (
                GuestAsk::Split {
                    ops,
                    removal,
                    message,
                },
                Some(send),
            ) => self.commit_split(ops, removal, message, send, cx),
            // Nothing went: the series as it was, the new one gone.
            (GuestAsk::Split { removal, .. }, None) => self.put_back(removal, false, window, cx),
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Make an event over `[start, end)` and open it with its title focused.
    /// All-day events take whole days. Returns the new event's id; `None`
    /// when an unsaved new event is open, and asking about it comes first.
    pub fn create_event(
        &mut self,
        start: NaiveDateTime,
        end: NaiveDateTime,
        all_day: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let then = Leave::Create {
            start,
            end,
            all_day,
        };
        if self.ask_to_leave(then, cx) {
            return None;
        }
        // The default calendar (Settings), else the primary one. Before the
        // first sync there may be none.
        let calendar = crate::calendar::new_event_calendar(
            &self.data.calendars,
            self.prefs.default_calendar.as_deref(),
        );
        // The calendar's default reminders until the user picks one, as in
        // Google Calendar.
        let reminder_minutes = calendar.and_then(|c| c.default_reminder);
        // Google's "Automatically add Google Meet" (CAL-34).
        let auto_meet = self.google.auto_add_meet && calendar.is_some_and(|c| c.allows_meet());
        let hidden = calendar.is_some_and(|c| !c.visible);
        let calendar = calendar.map(|c| c.id.clone()).unwrap_or_default();
        // A new event on a hidden calendar would vanish from the grid: show
        // the calendar, as jumping to one of its events does.
        if hidden && let Some(ix) = self.data.calendar_ix(&calendar) {
            self.toggle_calendar(ix, cx);
        }
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
        let id = new_id();
        let meet_request = auto_meet.then(|| {
            let request = new_id();
            self.meet_requests.insert(id.clone(), request.clone());
            request
        });
        let event = Event {
            id: id.clone(),
            calendar,
            meet_request,
            start,
            end,
            all_day,
            organizer: self.data.me(),
            organizer_self: true,
            reminder_minutes,
            reminder_use_default: true,
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
                time::minutes_into(day, end).min(drag::DAY_MINUTES),
            );
        }
        self.select_day_in_view(start.date());
        self.selected = Some(Selection {
            event_id: id.clone(),
            start,
        });
        self.open_event(&id, start, true, window, cx);
        self.new_event = Some(id.clone());
        self.just_added = Some(id.clone());
        Some(id)
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
        let Some(before) = self.event(id).cloned() else {
            return;
        };
        let end = end.max(
            start
                + Duration::minutes(if before.all_day {
                    drag::DAY_MINUTES
                } else {
                    drag::SNAP_MINUTES
                }),
        );
        if self.access_of(&before) != Access::Edit {
            if (start, end) != (occurrence, occurrence + (before.end - before.start)) {
                self.refuse(id, cx);
            }
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

    /// Offer Undo for a move or resize from `before` to `after` (MA-071).
    pub(crate) fn emit_retimed(
        &self,
        id: &str,
        before: (NaiveDateTime, NaiveDateTime),
        after: (NaiveDateTime, NaiveDateTime),
        cx: &mut Context<Self>,
    ) {
        if before == after || !self.can_edit(id) {
            return;
        }
        let title = if before.0 == after.0 {
            "Resize Event"
        } else {
            "Move Event"
        };
        cx.emit(Retimed {
            title,
            retime: Retime {
                event_id: id.to_string(),
                before,
                after,
            },
        });
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
            id = new_id();
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

    /// "This and following" (CAL-6): the series as it was (`before`) ends
    /// just before the occurrence its rule put at `origin`, and a new series
    /// starts at `start` with `edited`'s fields and rule; its occurrences
    /// changed on their own from `origin` on go. Undo puts it all back.
    /// Returns the new series' id; `None` (nothing done) at the series'
    /// first occurrence or for someone else's series, where it's "All
    /// events".
    fn split_series(
        &mut self,
        before: Event,
        origin: NaiveDateTime,
        start: NaiveDateTime,
        edited: Event,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if !is_organizer(&before, self.me()) {
            return None;
        }
        let new_id = new_id();
        let (old, new) = before.split(origin, start, &edited, new_id.clone())?;
        let id = before.id.clone();
        // What was settling for the series is this change.
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            self.pending = None;
            self._edit_timer = None;
        }
        *self.event_mut(&id)? = old.clone();
        let mut removal = Removal {
            series_before: Some(before.clone()),
            added: vec![new_id.clone()],
            select: Some(Selection {
                event_id: id.clone(),
                start: origin,
            }),
            ..Removal::default()
        };
        let mut ix = 0;
        while ix < self.data.events.len() {
            let event = &self.data.events[ix];
            if event.recurring_event_id.as_deref() == Some(id.as_str())
                && event.original_start >= Some(origin)
            {
                removal
                    .removed
                    .push((ix + removal.removed.len(), self.data.events.remove(ix)));
            } else {
                ix += 1;
            }
        }
        self.data.events.push(new.clone());
        let ops = split_ops(&before, &old, &new);
        let day = time::date_label(origin.date(), today());
        let message = format!("Changed “{}” from {day} on", new.display_title());
        if asks_guests(Some(&before), &edited, self.me()) {
            let kind = GuestAsk::Split {
                ops,
                removal,
                message,
            };
            self.ask_guests(
                GuestPrompt {
                    event_id: new_id.clone(),
                    kind,
                },
                cx,
            );
        } else {
            self.commit_split(ops, removal, message, SendUpdates::None, cx);
        }
        cx.notify();
        Some(new_id)
    }

    /// Queue a split's ops and offer Undo.
    fn commit_split(
        &mut self,
        ops: Vec<CalOp>,
        removal: Removal,
        message: String,
        send: SendUpdates,
        cx: &mut Context<Self>,
    ) {
        for op in ops {
            self.queue(with_send(op, send), cx);
        }
        cx.emit(CalendarEvent::Undoable {
            message,
            removal: Box::new(removal),
        });
    }

    /// Answer an invite for the user: their own attendee entry takes
    /// `status`. Does nothing when they aren't invited.
    pub fn set_rsvp(&mut self, id: &str, status: RsvpStatus, cx: &mut Context<Self>) {
        let me = self.data.account.clone();
        if let Some(event) = self.event_mut(id)
            && let Some(attendee) = event.attendees.iter_mut().find(|a| me.is_me(&a.email))
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

    /// What the event shortcuts act on: the selected occurrence, else the
    /// one the panel shows.
    fn target(&self) -> Option<(String, NaiveDateTime)> {
        self.selected
            .as_ref()
            .map(|s| (s.event_id.clone(), s.start))
            .or_else(|| {
                self.panel
                    .as_ref()
                    .map(|p| (p.event_id.clone(), p.occurrence))
            })
    }

    /// ⇧Y, ⇧M, ⇧N: answer the selected invite (ME-034).
    fn answer_selected(&mut self, status: RsvpStatus, cx: &mut Context<Self>) {
        let Some((id, _)) = self.target() else {
            return cx.propagate();
        };
        if self
            .event(&id)
            .is_none_or(|e| e.my_status(&self.data.account).is_none())
        {
            return cx.propagate();
        }
        self.set_rsvp(&id, status, cx);
    }

    fn accept_event(&mut self, _: &AcceptEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.answer_selected(RsvpStatus::Accepted, cx);
    }

    fn maybe_event(&mut self, _: &MaybeEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.answer_selected(RsvpStatus::Tentative, cx);
    }

    fn decline_event(&mut self, _: &DeclineEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.answer_selected(RsvpStatus::Declined, cx);
    }

    /// ⌘J: join the selected event's video call (ME-096).
    fn join_event(&mut self, _: &JoinEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((id, _)) = self.target() else {
            return cx.propagate();
        };
        match self.event(&id).and_then(policy::video_link) {
            Some(link) => cx.open_url(link),
            None => cx.emit(CalendarEvent::Refused(
                "This event has no video call to join".into(),
            )),
        }
    }

    /// ⌥ and an arrow (MA-092): the selected event 15 minutes earlier or
    /// later, or a day either way, with Undo. A repeating one asks which
    /// occurrences, as a drop does.
    fn nudge(&mut self, by: Duration, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal() {
            return;
        }
        let Some((id, occurrence)) = self.target() else {
            return cx.propagate();
        };
        let Some(event) = self.event(&id) else {
            return;
        };
        // An all-day event moves by days only.
        if event.all_day && by.num_days() == 0 {
            return;
        }
        if self.access_of(event) != Access::Edit {
            return self.refuse(&id, cx);
        }
        let length = event.duration();
        let spanning = event.is_spanning();
        let (start, end) = (occurrence + by, occurrence + by + length);
        if event.is_series() {
            self.prompt = Some(SeriesPrompt {
                event_id: id,
                occurrence,
                kind: PromptKind::Move { start, end },
            });
            return cx.notify();
        }
        self.retime_occurrence(&id, occurrence, start, end, window, cx);
        self.flush_edit(cx);
        self.emit_retimed(&id, (occurrence, occurrence + length), (start, end), cx);
        // The view follows it to its new day.
        self.anchor = start.date();
        if !spanning && self.view != ViewKind::Month {
            let day = start.date();
            self.reveal_minutes(
                time::minutes_into(day, start),
                time::minutes_into(day, end).min(drag::DAY_MINUTES),
            );
        }
        cx.notify();
    }

    fn nudge_earlier(
        &mut self,
        _: &NudgeEventEarlier,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge(Duration::minutes(-drag::SNAP_MINUTES), window, cx);
    }

    fn nudge_later(&mut self, _: &NudgeEventLater, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge(Duration::minutes(drag::SNAP_MINUTES), window, cx);
    }

    fn nudge_day_back(
        &mut self,
        _: &NudgeEventDayBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge(Duration::days(-1), window, cx);
    }

    fn nudge_day_forward(
        &mut self,
        _: &NudgeEventDayForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge(Duration::days(1), window, cx);
    }

    /// A copy of the event at `ix` over `[start, end)`, as an event of its
    /// own, selected (⌥-drag, MA-092; Duplicate, ME-097). It goes in at
    /// once, asking about the guests first.
    pub(crate) fn copy_event(
        &mut self,
        ix: usize,
        start: NaiveDateTime,
        end: NaiveDateTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let source = self.data.events.get(ix)?;
        if self.access_of(source) == Access::ReadOnly {
            self.refuse(&source.id.clone(), cx);
            return None;
        }
        let id = new_id();
        let copy = Event {
            id: id.clone(),
            start,
            end,
            recurrence: None,
            exceptions: Vec::new(),
            recurring_event_id: None,
            original_start: None,
            etag: String::new(),
            ical_uid: String::new(),
            html_link: String::new(),
            // A call of its own, if it wants one.
            conference_uri: String::new(),
            meet_request: None,
            organizer: self.data.me(),
            organizer_self: true,
            ..source.clone()
        };
        self.data.events.push(copy.clone());
        self.save(
            vec![CalOp::Insert {
                event: copy.clone(),
                send_updates: SendUpdates::None,
            }],
            None,
            &copy,
            cx,
        );
        self.select_day_in_view(start.date());
        self.selected = Some(Selection {
            event_id: id.clone(),
            start,
        });
        self.just_added = Some(id.clone());
        if self.panel.is_some() {
            self.open_event(&id, start, false, window, cx);
        }
        cx.notify();
        Some(id)
    }

    /// Right-click on an event (MB-010): it's selected, and a menu offers
    /// what can be done with it.
    pub(crate) fn event_menu(
        &mut self,
        occurrence: Occurrence,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_grid(window, cx);
        let selection = self.selection_of(&occurrence);
        if self.ask_to_leave(Leave::Click(selection.clone()), cx) {
            return;
        }
        if self.panel.is_some() {
            self.click_selection(selection, window, cx);
        } else {
            self.select_day_in_view(selection.start.date());
            self.selected = Some(selection);
        }
        cx.notify();
        let Some(event) = self.data.events.get(occurrence.event) else {
            return;
        };
        let access = self.access_of(event);
        let mut menu = NativeMenu::new().menu("Open", Box::new(OpenEvent));
        if let Some(status) = event.my_status(&self.data.account) {
            menu = menu
                .separator()
                .menu_with_check(
                    "Accept",
                    status == RsvpStatus::Accepted,
                    Box::new(AcceptEvent),
                )
                .menu_with_check(
                    "Maybe",
                    status == RsvpStatus::Tentative,
                    Box::new(MaybeEvent),
                )
                .menu_with_check(
                    "Decline",
                    status == RsvpStatus::Declined,
                    Box::new(DeclineEvent),
                );
        }
        let video = policy::video_link(event).is_some();
        let located = !event.location.trim().is_empty();
        if video || located {
            menu = menu.separator();
        }
        if video {
            menu = menu.menu("Join Meeting", Box::new(JoinEvent));
        }
        if located {
            menu = menu.menu("Open Location in Maps", Box::new(OpenLocation));
        }
        if !event.html_link.trim().is_empty() {
            menu = menu
                .separator()
                .menu("Copy Link to Event", Box::new(CopyLink))
                .menu("Open in Google Calendar", Box::new(OpenInGoogleCalendar));
        }
        if access == Access::Edit {
            menu = menu.separator().menu("Duplicate", Box::new(DuplicateEvent));
            if !video {
                menu = menu.menu("Add Google Meet", Box::new(AddGoogleMeet));
            }
            menu = menu
                .menu("Repeat…", Box::new(SetRepeat))
                .menu("Add Reminder…", Box::new(AddReminder));
        }
        if access != Access::ReadOnly {
            let delete = if is_organizer(event, self.me()) {
                if event.is_series() {
                    "Delete…"
                } else {
                    "Delete"
                }
            } else {
                "Decline and Remove"
            };
            menu = menu.separator().menu(delete, Box::new(DeleteEvent));
        }
        menu.show(position, window, cx);
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
            self.refuse(id, cx);
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
        // The block fades out where it was (MD-038).
        if let Some(event) = self.event(id).filter(|e| !e.is_spanning()) {
            self.removed_block = Some(RemovedBlock {
                start: occurrence,
                end: occurrence + event.duration(),
                title: event.display_title().to_string().into(),
                color: self.data.color_of(event),
                started: false,
                fade: 1.,
            });
        }
        // What the panel shows, for it to slide shut from (MD-002).
        let shown = self
            .panel
            .as_ref()
            .and_then(|p| self.event(&p.event_id).cloned());
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
            let was_new = self.panel_is_new();
            self.start_closing(shown, was_new);
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
        cx.emit(CalendarEvent::Undoable {
            message,
            removal: Box::new(removal),
        });
        cx.notify();
    }

    /// Undo a delete: the events and the series come back as they were,
    /// the occurrence selected.
    pub fn restore(&mut self, removal: Removal, window: &mut Window, cx: &mut Context<Self>) {
        self.put_back(removal, true, window, cx);
    }

    /// `restore`; with `queue` false, only here, as nothing went yet.
    fn put_back(
        &mut self,
        removal: Removal,
        queue: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Inside the undo window the delete never went; past it, put the
        // events back.
        let cancelled = !queue
            || removal
                .op
                .is_some_and(|op| self.backend.update(cx, |b, _| b.cancel(op)));
        if !cancelled {
            // The series first: the occurrences changed on their own are
            // found among its instances.
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
            for id in &removal.added {
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
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| removal.added.contains(&p.id))
        {
            self.pending = None;
        }
        self.data.events.retain(|e| !removal.added.contains(&e.id));
        if let Some(before) = &removal.series_before
            && let Some(series) = self.event_mut(&before.id)
        {
            *series = before.clone();
        }
        for (index, event) in removal.removed {
            let index = index.min(self.data.events.len());
            self.data.events.insert(index, event);
        }
        // A panel on the new series shows the old one again.
        if let (Some(panel), Some(before), Some(select)) =
            (&self.panel, &removal.series_before, &removal.select)
            && removal.added.contains(&panel.event_id)
        {
            self.panel = Some(EventPanel::new(before, select.start, window, cx));
            self.discard_base = Some((before.clone(), select.start));
        }
        if let Some(select) = removal.select {
            self.select_day_in_view(select.start.date());
            self.just_added = Some(select.event_id.clone());
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
        let Some(id) = self.data.events.get(ix).map(|e| e.id.clone()) else {
            return;
        };
        if self.ask_to_leave(Leave::Reveal(id.clone()), cx) {
            return;
        }
        let event = &self.data.events[ix];
        let start = next_start(event);
        let (from, to) = self.visible_range();
        let on_screen = self.view != ViewKind::Month && (from..to).contains(&start.date());
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
            let end = minutes + event.duration().num_minutes();
            if on_screen {
                self.glide_to_minutes(minutes, end);
            } else {
                self.reveal_minutes(minutes, end);
            }
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
                let split = match scope {
                    SeriesScope::Following => self.event(id).cloned().and_then(|series| {
                        let edited = Event {
                            start,
                            end,
                            ..series.clone()
                        };
                        self.split_series(series, prompt.occurrence, start, edited, cx)
                    }),
                    _ => None,
                };
                let id = match split {
                    Some(new_id) => new_id,
                    None => {
                        let length = self.event(id).map(|e| e.duration());
                        self.retime_occurrence(id, prompt.occurrence, start, end, window, cx);
                        self.flush_edit(cx);
                        if let Some(length) = length {
                            let before = (prompt.occurrence, prompt.occurrence + length);
                            self.emit_retimed(id, before, (start, end), cx);
                        }
                        id.to_string()
                    }
                };
                if self.panel.is_some() {
                    self.open_event(&id, start, false, window, cx);
                }
                self.selected = Some(Selection {
                    event_id: id,
                    start,
                });
            }
        }
        cx.notify();
    }

    // Google's settings, reminders and quick add (W5).

    fn load_google(&mut self, cx: &mut Context<Self>) {
        let google = self.backend.read(cx).calendar_settings();
        if google != self.google {
            self.google = google;
            self.apply_display();
            cx.notify();
        }
    }

    /// Saved settings, at launch: the day shown last (CAL-23) and the
    /// calendar settings changed in Pigeon.
    pub fn apply_saved(&mut self, saved: &crate::settings::Saved) {
        if let Some(date) = saved.calendar_date {
            self.anchor = date;
            self.scroll_to_start();
        }
        self.prefs = CalendarPrefs {
            event_minutes: saved.event_minutes,
            default_calendar: saved.default_calendar.clone(),
        };
        self.apply_display();
    }

    /// The week's first day and a new event's length follow the settings.
    /// The clock is app-wide (`shell::set_time_format`).
    pub fn apply_display(&mut self) {
        time::set_first_weekday(self.google.first_weekday());
        quick_add::set_default_minutes(self.event_minutes());
    }

    /// A new event's length.
    pub fn event_minutes(&self) -> u32 {
        self.prefs
            .event_minutes
            .or(self.google.default_event_minutes)
            .unwrap_or(60)
    }

    /// The calendars new events can go to.
    pub fn writable_calendars(&self) -> impl Iterator<Item = &crate::calendar::Calendar> {
        self.data.calendars.iter().filter(|c| c.writable())
    }

    /// This Mac's zone, and Google's when it differs.
    pub fn time_zone_note(&self) -> String {
        let mac = LocalZone::system().name;
        match self.zone_mismatch() {
            Some(google) => format!(
                "{mac}. Google Calendar is set to {google}; events show at this Mac's times."
            ),
            None => mac.to_string(),
        }
    }

    /// Google's zone, when it isn't this Mac's.
    pub fn zone_mismatch(&self) -> Option<&str> {
        self.google
            .time_zone
            .as_deref()
            .filter(|&z| z != LocalZone::system().name)
    }

    /// "New event: Lunch with Sam Fri 1pm" from ⌘K (CAL-39): the event,
    /// open in the panel to confirm.
    pub fn quick_add(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask_to_leave(Leave::QuickAdd(text.to_string()), cx) {
            return;
        }
        let quick = quick_add::parse(text, now(), self.event_minutes());
        self.anchor = quick.start.date();
        self.mini_month = None;
        let Some(id) = self.create_event(quick.start, quick.end, quick.all_day, window, cx) else {
            return;
        };
        let me = self.me().to_string();
        let guests: Vec<crate::calendar::Attendee> = {
            let book = self.book.borrow();
            quick
                .people
                .iter()
                .filter_map(|name| {
                    let first = |n: &str| n.split_whitespace().next().map(str::to_lowercase);
                    book.suggest(name, |c| c.email.eq_ignore_ascii_case(&me))
                        .into_iter()
                        .map(|ix| book.contact(ix))
                        .find(|c| first(&c.name) == first(name))
                })
                .map(|c| crate::calendar::Attendee {
                    name: c.name.clone(),
                    email: c.email.clone(),
                    status: RsvpStatus::NeedsAction,
                })
                .collect()
        };
        let location = quick.location.clone().unwrap_or_default();
        if let Some(event) = self.event_mut(&id) {
            event.title = quick.title.clone();
            event.location = location.clone();
            for guest in guests {
                if !event
                    .attendees
                    .iter()
                    .any(|a| a.email.eq_ignore_ascii_case(&guest.email))
                {
                    event.attendees.push(guest);
                }
            }
        }
        if let Some(panel) = &self.panel {
            let (title, place) = (panel.title.clone(), panel.location.clone());
            title.update(cx, |input, cx| input.set_value(quick.title, window, cx));
            if !location.is_empty() {
                place.update(cx, |input, cx| input.set_value(location, window, cx));
            }
        }
        self.settle_edit(cx);
        cx.notify();
    }

    // Action handlers.

    /// The prompt not shown in the panel, if any: over the grid.
    fn floating_prompt(&self) -> Option<&SeriesPrompt> {
        self.prompt.as_ref().filter(|p| {
            !(p.kind == PromptKind::Delete
                && self
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.event_id == p.event_id))
        })
    }

    /// A question is up, dimming the calendar: only its answers, Enter and
    /// Escape do anything (MB-041). A drop's move question stays inline.
    fn modal(&self) -> bool {
        self.leave_prompt.is_some()
            || self.guest_prompt.is_some()
            || self
                .floating_prompt()
                .is_some_and(|p| p.kind == PromptKind::Delete)
    }

    fn show_day(&mut self, _: &ShowDayView, _: &mut Window, cx: &mut Context<Self>) {
        if self.modal() {
            return;
        }
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
        if !self.modal() {
            self.set_view(ViewKind::Week, cx);
        }
    }

    fn show_month(&mut self, _: &ShowMonthView, _: &mut Window, cx: &mut Context<Self>) {
        if !self.modal() {
            self.set_view(ViewKind::Month, cx);
        }
    }

    /// ←/→ on the focused view switch: the view beside (MA-005).
    fn step_view(&mut self, forward: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.view_switch_focus.is_focused(window) {
            return false;
        }
        let views = [ViewKind::Day, ViewKind::Week, ViewKind::Month];
        let at = views.iter().position(|&v| v == self.view).unwrap_or(1);
        let next = if forward {
            (at + 1).min(2)
        } else {
            at.saturating_sub(1)
        };
        self.set_view(views[next], cx);
        true
    }

    fn go_to_today(&mut self, _: &GoToToday, _: &mut Window, cx: &mut Context<Self>) {
        if self.modal() {
            return;
        }
        let (from, to) = self.visible_range();
        let on_screen = (from..to).contains(&today());
        self.anchor = today();
        self.mini_month = None;
        if self.view != ViewKind::Month {
            let minutes = time::minutes_into(today(), now());
            // A glide only when today was already in view (MD-040).
            if on_screen {
                self.glide_to_minutes(minutes, minutes + 60);
            } else {
                self.reveal_minutes(minutes, minutes + 60);
            }
        }
        cx.notify();
    }

    /// Two fingers across the trackpad page like `]` and `[`, once per
    /// swipe, when it's mostly sideways (MA-080). Mouse wheels, whose steps
    /// are lines, and the momentum after a swipe don't page.
    fn on_swipe(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let ScrollDelta::Pixels(delta) = event.delta else {
            return;
        };
        match event.touch_phase {
            TouchPhase::Started => self.swipe = Some((delta, false)),
            TouchPhase::Moved => {
                let Some((total, paged)) = &mut self.swipe else {
                    return;
                };
                *total += delta;
                let (dx, dy) = (f32::from(total.x), f32::from(total.y));
                if !*paged && dx.abs() > SWIPE_DISTANCE && dx.abs() > 2. * dy.abs() {
                    *paged = true;
                    if !self.modal() {
                        // Fingers to the left bring the next period in.
                        self.step(dx < 0., cx);
                    }
                }
            }
            _ => self.swipe = None,
        }
    }

    fn next_period(&mut self, _: &NextPeriod, _: &mut Window, cx: &mut Context<Self>) {
        if !self.modal() {
            self.step(true, cx);
        }
    }

    fn prev_period(&mut self, _: &PrevPeriod, _: &mut Window, cx: &mut Context<Self>) {
        if !self.modal() {
            self.step(false, cx);
        }
    }

    fn next_day(&mut self, _: &NextDay, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal() && !self.step_view(true, window, cx) {
            self.step_day(1, window, cx);
        }
    }

    fn prev_day(&mut self, _: &PrevDay, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal() && !self.step_view(false, window, cx) {
            self.step_day(-1, window, cx);
        }
    }

    fn select_next(&mut self, _: &SelectNextEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal() {
            self.select_adjacent(true, window, cx);
        }
    }

    fn select_prev(&mut self, _: &SelectPrevEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal() {
            self.select_adjacent(false, window, cx);
        }
    }

    fn toggle_go_to_date(&mut self, _: &GoToDate, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal() {
            self.toggle_date_picker(window, cx);
        }
    }

    pub fn toggle_date_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu == Some(Menu::GoToDate) {
            self.menu = None;
            window.focus(&self.focus_handle, cx);
        } else {
            self.menu = Some(Menu::GoToDate);
            self.mini_month = None;
            self.typed = None;
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
                self.typed = time::parse_date(self.date_input.read(cx).value().as_ref(), today());
                self.mini_month = self.typed.map(time::month_start);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                if let Some(date) = self.typed {
                    self.go_to_date(date, window, cx);
                }
            }
            _ => {}
        }
    }

    /// The date typed in the go-to-date field, if it reads as one.
    pub fn typed_date(&self) -> Option<NaiveDate> {
        self.typed
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
    pub(crate) fn new_event(&mut self, _: &NewEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal() {
            return;
        }
        let today = today();
        let day = self.anchor;
        let start = if day == today {
            next_half_hour(now())
        } else {
            time::at(day, 9, 0)
        };
        let length = Duration::minutes(self.event_minutes() as i64);
        self.create_event(start, start + length, false, window, cx);
    }

    /// Enter: answers an open prompt with its first choice, or opens the
    /// selected event.
    fn open_selected(&mut self, _: &OpenEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A focused button (the header's, a prompt's) takes Enter itself
        // (MA-005).
        if !self.focus_handle.is_focused(window) {
            return cx.propagate();
        }
        if self.leave_prompt.is_some() {
            return self.answer_leave(Some(LeaveChoice::Save), window, cx);
        }
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
        if self.modal() {
            return;
        }
        let Some((id, start)) = self.target() else {
            return;
        };
        if self
            .event(&id)
            .is_none_or(|e| self.access_of(e) == Access::ReadOnly)
        {
            self.refuse(&id, cx);
            return;
        }
        if self.event(&id).is_some_and(|e| e.is_series()) {
            self.ask_delete(&id, start, cx);
        } else {
            self.delete_event(&id, window, cx);
        }
    }

    /// Escape: the innermost open thing closes: a drag, a prompt, a dropdown
    /// or picker, then the panel (discarding a new event), then the
    /// selection.
    fn close_event(&mut self, _: &CloseEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.cancel_drag(cx) {
            return;
        }
        // From a focused header button, back to the grid (MA-005).
        let in_header = self.view_switch_focus.is_focused(window)
            || (self.calendars_focus.is_focused(window) && self.menu.is_none());
        if in_header && !self.modal() {
            window.focus(&self.focus_handle, cx);
            return cx.notify();
        }
        if self.leave_prompt.is_some() {
            self.answer_leave(None, window, cx);
        } else if self.guest_prompt.is_some() {
            self.answer_guests(None, window, cx);
        } else if let Some(prompt) = self.prompt.take() {
            self.cancel_move_prompt(&prompt);
            cx.notify();
        } else if let Some(menu) = self.menu.take() {
            if menu == Menu::GoToDate {
                window.focus(&self.focus_handle, cx);
            }
            if menu == Menu::Calendars {
                window.focus(&self.calendars_focus, cx);
            }
            cx.notify();
        } else if self.dismiss_panel_popup(cx) {
            cx.notify();
        } else if self.asking_scope() {
            // From any field: the change waiting for its scope is taken
            // back, and the panel stays on the event as it was.
            self.cancel_unscoped_edit(window, cx);
            window.focus(&self.focus_handle, cx);
            cx.notify();
        } else if self.panel_is_new() {
            self.discard_new_event(window, cx);
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
        if self.modal() {
            return;
        }
        self.toggle_menu(Menu::Calendars, cx);
        // Arrows and Space work its rows from the button (ME-045).
        if self.menu == Some(Menu::Calendars) {
            self.calendars_highlight = Some(0);
            window.focus(&self.calendars_focus, cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
    }

    /// The calendars in the menu's order: the user's own, then the others.
    pub(crate) fn calendars_menu_order(&self) -> Vec<usize> {
        let calendars = &self.data.calendars;
        let owner = |ix: &usize| calendars[*ix].access_role == crate::calendar::AccessRole::Owner;
        let (mut mine, others): (Vec<usize>, Vec<usize>) = (0..calendars.len()).partition(owner);
        mine.extend(others);
        mine
    }

    fn calendars_menu_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.menu != Some(Menu::Calendars) {
            self.toggle_menu(Menu::Calendars, cx);
            self.calendars_highlight = Some(0);
            return;
        }
        let count = self.data.calendars.len().max(1) as isize;
        let at = self.calendars_highlight.map_or(-1, |h| h as isize);
        let next = if at < 0 && delta < 0 {
            count - 1
        } else {
            (at + delta).rem_euclid(count)
        };
        self.calendars_highlight = Some(next as usize);
        cx.notify();
    }

    fn calendars_menu_next(
        &mut self,
        _: &event_panel::MenuNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.calendars_menu_step(1, cx);
    }

    fn calendars_menu_prev(
        &mut self,
        _: &event_panel::MenuPrev,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.calendars_menu_step(-1, cx);
    }

    /// Enter or Space on the calendars button: open the menu, or show or
    /// hide the highlighted calendar.
    fn calendars_menu_activate(
        &mut self,
        _: &event_panel::Activate,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu != Some(Menu::Calendars) {
            self.toggle_menu(Menu::Calendars, cx);
            self.calendars_highlight = Some(0);
            return;
        }
        let order = self.calendars_menu_order();
        if let Some(&ix) = self.calendars_highlight.and_then(|h| order.get(h)) {
            self.toggle_calendar(ix, cx);
        }
    }

    /// ⌥-click on a calendar: show only it; again, show them all.
    pub(crate) fn show_only_calendar(&mut self, ix: usize, cx: &mut Context<Self>) {
        let only = self
            .data
            .calendars
            .iter()
            .enumerate()
            .all(|(i, c)| c.visible == (i == ix));
        for i in 0..self.data.calendars.len() {
            let wanted = only || i == ix;
            if self.data.calendars[i].visible != wanted {
                self.toggle_calendar(i, cx);
            }
        }
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
    pub(crate) fn focus_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            ViewKind::Week => (week_view::gutter_width(window), (day - from).num_days()),
            ViewKind::Month => (0., (day - from).num_days() % 7),
        };
        let width = Self::content_width(window);
        let column = (width - left) / 7.;
        let center = left + (ix as f32 + 0.5) * column;
        center > width - event_panel::PANEL_WIDTH
    }

    /// What VoiceOver reads for an occurrence: title, day and time, its
    /// calendar and the user's answer (MA-048).
    pub fn occurrence_label(&self, occurrence: &Occurrence) -> String {
        let event = &self.data.events[occurrence.event];
        let day = time::date_label(occurrence.start.date(), today());
        let when = if event.all_day {
            "all day".to_string()
        } else {
            time::time_range_label(occurrence.start.time(), occurrence.end.time())
        };
        let mut label = format!("{}, {day}, {when}", event.display_title());
        let calendar = self.calendar_name(event);
        if !calendar.is_empty() {
            label.push_str(", ");
            label.push_str(&calendar);
        }
        let answer = match event.my_status(&self.data.account) {
            Some(RsvpStatus::NeedsAction) => Some("not answered"),
            Some(RsvpStatus::Tentative) => Some("maybe"),
            Some(RsvpStatus::Declined) => Some("declined"),
            _ => None,
        };
        if let Some(answer) = answer {
            label.push_str(", ");
            label.push_str(answer);
        }
        label
    }

    pub fn calendar_name(&self, event: &Event) -> SharedString {
        self.data
            .calendar(&event.calendar)
            .map(|c| c.name.clone().into())
            .unwrap_or_default()
    }

    fn render_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let occurrences = self.visible_occurrences();
        // One clock for the whole frame (MC-059).
        let now = now();
        match self.view {
            ViewKind::Day => self.render_day(&occurrences, now, window, cx),
            ViewKind::Week => {
                self.render_time_grid(self.grid_days(), &occurrences, now, window, cx)
            }
            ViewKind::Month => self.render_month(&occurrences, now, window, cx),
        }
    }

    /// The buttons answering a prompt about a repeating event.
    pub(crate) fn render_prompt_choices(
        &self,
        kind: PromptKind,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.render_choices(
            kind.choices(),
            true,
            kind.tones(),
            theme,
            cx,
            Self::answer_prompt,
        )
    }

    /// A row of buttons, one per choice; the first is the default (which
    /// enter takes, with `enter_hint`), drawn last (MB-027).
    pub(crate) fn render_scope_buttons<T: Copy + 'static>(
        &self,
        choices: &'static [(T, &'static str)],
        enter_hint: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
        pick: fn(&mut Self, T, &mut Window, &mut Context<Self>),
    ) -> gpui::Div {
        let tones = (ButtonKind::Primary, ButtonKind::Plain);
        self.render_choices(choices, enter_hint, tones, theme, cx, pick)
    }

    /// `render_scope_buttons`, with how the default and the other choices
    /// look: a delete's are red (MB-028).
    fn render_choices<T: Copy + 'static>(
        &self,
        choices: &'static [(T, &'static str)],
        enter_hint: bool,
        (default, other): (ButtonKind, ButtonKind),
        theme: &Theme,
        cx: &mut Context<Self>,
        pick: fn(&mut Self, T, &mut Window, &mut Context<Self>),
    ) -> gpui::Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap(SPACE_XS)
            .children(
                choices
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(ix, &(choice, label))| {
                        let first = ix == 0;
                        let kind = if first { default } else { other };
                        push_button(("prompt-choice", ix), label, kind, theme)
                            .when(first && enter_hint, |d| d.child(enter_cap(kind, theme)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                pick(this, choice, window, cx)
                            }))
                    }),
            )
    }

    /// A drop's "Move this repeating event?" over the bottom of the grid;
    /// a delete's asks over the whole calendar.
    fn render_series_prompt(
        &self,
        prompt: &SeriesPrompt,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let title = self
            .event(&prompt.event_id)
            .map(|e| e.display_title().to_string())
            .unwrap_or_default();
        let buttons = self.render_prompt_choices(prompt.kind, theme, cx);
        self.render_prompt_card(
            prompt.kind.question(),
            title,
            buttons,
            theme,
            cx,
            |this, _, cx| {
                if let Some(prompt) = this.prompt.take() {
                    this.cancel_move_prompt(&prompt);
                }
                cx.notify();
            },
        )
    }

    /// "Save this event?", before leaving a new event that isn't saved yet.
    fn render_leave_prompt(
        &self,
        prompt: &LeavePrompt,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let title = self
            .event(&prompt.event_id)
            .map(|e| e.display_title().to_string())
            .unwrap_or_default();
        let buttons = self.render_choices(
            LEAVE_CHOICES,
            true,
            (ButtonKind::Primary, ButtonKind::Danger),
            theme,
            cx,
            |this, choice, window, cx| this.answer_leave(Some(choice), window, cx),
        );
        self.render_prompt_card(
            "Save this event?",
            title,
            buttons,
            theme,
            cx,
            |this, window, cx| this.answer_leave(None, window, cx),
        )
    }

    /// "Send updates to guests?" (CAL-13).
    fn render_guest_prompt(
        &self,
        prompt: &GuestPrompt,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
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

    /// A question card: the question, the event's title, then Cancel and
    /// the answers, the default last.
    fn render_prompt_card(
        &self,
        question: &'static str,
        title: String,
        buttons: gpui::Div,
        theme: &Theme,
        cx: &mut Context<Self>,
        cancel: fn(&mut Self, &mut Window, &mut Context<Self>),
    ) -> gpui::Stateful<gpui::Div> {
        popover_surface(theme)
            .id("prompt-card")
            .occlude()
            .role(gpui::Role::AlertDialog)
            .aria_label(question)
            .w_full()
            .max_w(PROMPT_WIDTH)
            .p(SPACE_LG)
            .flex()
            .flex_col()
            .gap(SPACE_MD)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .child(
                        crate::ui::label("prompt-question", question)
                            .role(gpui::Role::Heading)
                            .text_size(TEXT_BASE)
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .when(!title.is_empty(), |d| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_muted)
                                .child(title),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_end()
                    .gap(SPACE_XS)
                    .child(
                        push_button("prompt-cancel", "Cancel", ButtonKind::Plain, theme)
                            .child(keys_caps("escape", theme))
                            .on_click(
                                cx.listener(move |this, _, window, cx| cancel(this, window, cx)),
                            ),
                    )
                    .child(buttons),
            )
    }
}

/// The widest a prompt card gets.
const PROMPT_WIDTH: Pixels = px(480.);

/// The ↩ on a default button, in its text color.
fn enter_cap(kind: ButtonKind, theme: &Theme) -> gpui::Div {
    crate::ui::keys_caps_with("enter", theme, |cap| match kind {
        ButtonKind::Primary => crate::ui::keycap_on_accent(cap, theme),
        ButtonKind::Destructive => div()
            .text_size(TEXT_XS)
            .text_color(theme.danger_text.alpha(0.8))
            .child(cap),
        _ => crate::ui::keycap(cap, theme),
    })
}

impl CalendarView {
    /// The parts, made on first render, when the calendar has an entity to
    /// point back to. The grid follows every notify of the calendar.
    fn parts(&mut self, cx: &mut Context<Self>) -> Parts {
        if let Some(parts) = &self.parts {
            return parts.clone();
        }
        let calendar = cx.entity();
        let mut make = |part: Part| {
            let weak = calendar.downgrade();
            let observed = calendar.clone();
            cx.new(|cx| CalendarPart {
                calendar: weak,
                part,
                _observe: (part == Part::Grid)
                    .then(|| cx.observe(&observed, |_, _, cx| cx.notify())),
            })
        };
        let parts = Parts {
            grid: make(Part::Grid),
            header: make(Part::Header),
            panel: make(Part::Panel),
            move_prompt: make(Part::MovePrompt),
            modal: make(Part::Modal),
        };
        self.parts = Some(parts.clone());
        parts
    }

    fn render_part(
        &mut self,
        part: Part,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match part {
            Part::Grid => {
                self.settle_ghost(window, cx);
                self.step_glide(window, cx);
                self.render_body(window, cx)
            }
            Part::Header => self.render_header(window, cx),
            Part::Panel => self.render_panel_slot(window, cx),
            Part::MovePrompt => self.render_move_prompt(window, cx),
            Part::Modal => self.render_modal(cx),
        }
    }

    /// Whether a docked panel takes room from the grid: from the moment it
    /// opens until it starts to close. The grid lays out once each way,
    /// not on every frame of the slide; shutting, the panel slides out over
    /// the grid, which has its room back already (MD-003, MD-004).
    fn panel_reserves(&self, window: &Window) -> bool {
        self.panel.is_some() && self.panel_docked(window)
    }

    /// The grid's width and whether the Day view's side shows, once per
    /// frame; the side keeps its state within `SIDE_HYSTERESIS` of the
    /// threshold, so a width near it doesn't flicker.
    fn lay_out_grid(&mut self, window: &Window) {
        let panel = if self.panel_reserves(window) {
            event_panel::PANEL_WIDTH
        } else {
            0.
        };
        self.grid_width = Self::content_width(window) - panel;
        let room = self.grid_width
            - week_view::gutter_width(window)
            - agenda::DAY_COLUMN_MAX
            - agenda::SIDE_MIN;
        let margin = if self.day_side {
            -SIDE_HYSTERESIS
        } else {
            SIDE_HYSTERESIS
        };
        self.day_side = room >= margin;
    }

    /// The panel's slot: slides open from the edge and shut into it, on one
    /// value-held spring (MD-001), here so its frames don't redraw the grid.
    fn render_panel_slot(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        if self.panel_stale {
            self.reload_panel(window, cx);
        }
        let open = self.panel.is_some();
        let shown = spring(
            ("event-panel", "shown"),
            if open { 1f32 } else { 0. },
            if open { SPRING_PANE } else { SPRING_EXIT },
            window,
            cx,
        );
        if !open && shown <= 0.001 {
            self.closing_panel = None;
        }
        let docked = self.panel_docked(window);
        let on_left = !docked && self.panel_on_left(window);
        // Moving sides while open fades in at the new one; the width
        // doesn't replay (MD-006).
        if open && !docked {
            if self.panel_side.is_some_and(|side| side != on_left) {
                self.panel_flip = Some(on_left);
            }
            self.panel_side = Some(on_left);
        } else if !open {
            self.panel_side = None;
            self.panel_flip = None;
        }
        let flipped = self.panel_flip == Some(on_left) && !docked;
        let panel = match (&self.panel, &self.closing_panel) {
            (Some(panel), _) => self
                .event(&panel.event_id)
                .map(|event| self.render_panel(panel, event, self.panel_is_new(), window, cx)),
            (None, Some(c)) => Some(self.render_panel(&c.panel, &c.event, c.was_new, window, cx)),
            (None, None) => None,
        };
        let Some(panel) = panel else {
            return div().into_any_element();
        };
        let width = event_panel::PANEL_WIDTH;
        let slot = div()
            .h_full()
            .flex_none()
            .overflow_hidden()
            .w(px(width * shown.clamp(0., 1.)))
            .when(docked && !open, |d| {
                // Shutting, docked: out over the grid, which has its room
                // back; opaque, so the grid doesn't show through.
                d.absolute().top_0().right_0().bg(theme.overlay_bg)
            })
            .when(!docked, |d| {
                // Opaque, so the grid doesn't show through.
                d.absolute()
                    .top_0()
                    .map(|d| {
                        if on_left {
                            // Its content travels with the growing edge
                            // (MD-005).
                            d.left_0()
                                .flex()
                                .justify_end()
                                .border_r_1()
                                .border_color(theme.border)
                        } else {
                            d.right_0()
                        }
                    })
                    .when(open, |d| d.occlude())
                    .bg(theme.overlay_bg)
                    .shadow(crate::ui::popover_shadow(&theme))
            })
            .child(panel)
            // Shutting, it takes no clicks (MD-002).
            .when(!open, |d| d.child(crate::ui::motion::inert_cover()));
        if flipped {
            crate::ui::motion::enter(("event-panel-side", on_left as usize), Rise::None, slot)
                .into_any_element()
        } else {
            slot.into_any_element()
        }
    }

    /// A drop's "Move this repeating event?" over the bottom of the grid.
    fn render_move_prompt(&self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let Some(prompt) = self
            .floating_prompt()
            .filter(|p| matches!(p.kind, PromptKind::Move { .. }))
        else {
            return div().into_any_element();
        };
        div()
            .absolute()
            .bottom(SPACE_XL)
            .left_0()
            .right_0()
            .px(SPACE_LG)
            .flex()
            .justify_center()
            .child(crate::ui::motion::enter(
                "move-prompt",
                Rise::Up,
                self.render_series_prompt(prompt, &theme, cx),
            ))
            .into_any_element()
    }

    /// Questions dim the calendar and ask near the top (MB-041).
    fn render_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let card = match (&self.leave_prompt, &self.guest_prompt) {
            (Some(p), _) => Some(self.render_leave_prompt(p, &theme, cx)),
            (None, Some(p)) => Some(self.render_guest_prompt(p, &theme, cx)),
            (None, None) => self
                .floating_prompt()
                .filter(|p| p.kind == PromptKind::Delete)
                .map(|p| self.render_series_prompt(p, &theme, cx)),
        };
        let Some(card) = card else {
            return div().into_any_element();
        };
        div()
            .id("prompt-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme.backdrop)
            .px(SPACE_LG)
            .pt(crate::ui::HEADER_HEIGHT + SPACE_LG)
            .flex()
            .flex_col()
            .items_center()
            .child(crate::ui::motion::enter("prompt-modal", Rise::Down, card))
            .into_any_element()
    }
}

/// How far two fingers go sideways before the grid pages (MA-080).
const SWIPE_DISTANCE: f32 = 80.;

/// How far past the Day view's side threshold the width must go to show or
/// hide it (MD-003).
const SIDE_HYSTERESIS: f32 = 16.;

impl Render for CalendarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let parts = self.parts(cx);
        self.lay_out_grid(window);
        // What the grid reads from outside changed: draw it afresh this
        // frame; it's cached again from the next.
        let key = GridKey {
            rem: window.rem_size(),
            clock_24: time::is_24_hour(),
            first_weekday: time::first_weekday(),
            me: self.data.account.email.clone(),
        };
        // VoiceOver reads each frame's drawing: a reused grid would drop
        // out of it.
        let fresh = self.grid_key.as_ref() != Some(&key) || window.is_a11y_active();
        self.grid_key = Some(key);
        let header_height = crate::ui::header_height(window);
        let reserve = if self.panel_reserves(window) {
            event_panel::PANEL_WIDTH
        } else {
            0.
        };
        let grid: AnyElement = if fresh {
            parts.grid.clone().into_any_element()
        } else {
            parts
                .grid
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element()
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
            .on_action(cx.listener(Self::save_event))
            .on_action(cx.listener(Self::discard_event_changes))
            .on_action(cx.listener(Self::toggle_calendars_menu))
            .on_action(cx.listener(Self::add_google_meet))
            .on_action(cx.listener(Self::set_repeat))
            .on_action(cx.listener(Self::add_reminder))
            .on_action(cx.listener(Self::accept_event))
            .on_action(cx.listener(Self::maybe_event))
            .on_action(cx.listener(Self::decline_event))
            .on_action(cx.listener(Self::join_event))
            .on_action(cx.listener(Self::nudge_earlier))
            .on_action(cx.listener(Self::nudge_later))
            .on_action(cx.listener(Self::nudge_day_back))
            .on_action(cx.listener(Self::nudge_day_forward))
            // The panel's, from the grid and ⌘K too (W2-CALP).
            .on_action(cx.listener(Self::copy_link))
            .on_action(cx.listener(Self::open_in_google_calendar))
            .on_action(cx.listener(Self::open_location))
            .on_action(cx.listener(Self::duplicate_event))
            .on_action(cx.listener(Self::email_guests))
            .on_modifiers_changed(cx.listener(
                |this, event: &gpui::ModifiersChangedEvent, _, cx| {
                    this.set_drag_copy(event.modifiers.alt, cx)
                },
            ))
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
                    .child(parts.header.clone()),
            )
            // Under the header, as wide as the grid's settled width, so the
            // panel's slide neither moves nor resizes it.
            .child(
                div()
                    .absolute()
                    .top(header_height)
                    .left_0()
                    .bottom_0()
                    .right(px(reserve))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.focus_grid(window, cx)),
                    )
                    .on_scroll_wheel(cx.listener(|this, event, _, cx| this.on_swipe(event, cx)))
                    .child(grid),
            )
            .child(
                div()
                    .absolute()
                    .top(header_height)
                    .left_0()
                    .bottom_0()
                    .right(px(reserve))
                    .child(parts.move_prompt.clone()),
            )
            .child(parts.panel.clone())
            .child(parts.modal.clone())
            .when(self.drag.is_some(), |this| {
                this.child(self.render_drag_layer(window, cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Rule;

    #[test]
    fn the_discard_toast_names_the_event() {
        let event = |title: &str| Event {
            title: title.into(),
            ..Event::default()
        };
        assert_eq!(discarded_message(&event(" Lunch ")), "Discarded “Lunch”");
        assert_eq!(discarded_message(&event("")), "Discarded new event");
    }

    #[test]
    fn leaving_for_the_event_already_open_is_not_leaving() {
        let selection = Selection {
            event_id: "n1".into(),
            start: time::at(NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(), 9, 0),
        };
        assert_eq!(Leave::Click(selection.clone()).target(), Some("n1"));
        assert_eq!(Leave::Select(selection).target(), Some("n1"));
        assert_eq!(Leave::Reveal("n1".into()).target(), Some("n1"));
        // Making another event or closing the panel goes to no event.
        assert_eq!(Leave::Close.target(), None);
        assert_eq!(Leave::QuickAdd("lunch".into()).target(), None);
    }

    #[test]
    fn a_split_is_one_op_that_ends_the_series_then_inserts() {
        let day = |n| time::at(NaiveDate::from_ymd_opt(2026, 11, n).unwrap(), 9, 0);
        let before = Event {
            id: "s1".into(),
            title: "Sync".into(),
            start: day(2),
            end: day(2) + Duration::minutes(30),
            recurrence: Some(Rule::weekly()),
            ..Event::default()
        };
        let edited = Event {
            title: "Sync (moved)".into(),
            ..before.clone()
        };
        let (old, new) = before
            .split(day(16), day(17), &edited, "n1".into())
            .unwrap();
        let ops: Vec<CalOp> = split_ops(&before, &old, &new)
            .into_iter()
            .map(|op| with_send(op, SendUpdates::All))
            .collect();
        assert_eq!(
            ops,
            [CalOp::Split {
                id: "s1".into(),
                before,
                after: old,
                new,
                send_updates: SendUpdates::All,
            }]
        );
    }
}

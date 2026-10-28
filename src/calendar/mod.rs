//! Calendar model, built from the store by the backend (demo mode seeds it
//! from the bundled placeholder data, which it also deserializes), plus the pure
//! logic the views run on: recurrence expansion (`recurrence`), overlap
//! layout (`layout`) and date math and parsing (`time`).
//!
//! Times are wall-clock times (`NaiveDateTime`): the JSON's offsets are
//! dropped and every time is shown as written. See CAL-12 in
//! `docs/ROADMAP.md`.

pub mod layout;
pub mod policy;
pub mod quick_add;
pub mod recurrence;
pub mod reminders;
pub mod time;

use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime};
use gpui::Hsla;
use serde::{Deserialize, Deserializer, Serialize};

use crate::data::{Contact, hex_color};

pub use policy::{Reminder, ReminderMethod};
pub use recurrence::{Occurrence, Rule, occurrences};

/// The user's Google Calendar settings (`settings.list`, CAL-41) that
/// Pigeon follows. Defaults until the user changes the same thing in
/// Pigeon.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleSettings {
    /// 0 Sunday, 1 Monday, 6 Saturday, as Google writes it.
    pub week_start: Option<u8>,
    pub default_event_minutes: Option<u32>,
    /// Add Google Meet to new events.
    pub auto_add_meet: bool,
    /// Remind only of events the user answered (or organizes).
    pub remind_responded_only: bool,
    /// Google's IANA zone for the user.
    pub time_zone: Option<String>,
}

impl GoogleSettings {
    /// From `settings.list`'s items (`{"id": …, "value": …}`).
    pub fn from_items(items: &[(String, String)]) -> Self {
        let mut out = Self::default();
        for (id, value) in items {
            match id.as_str() {
                "weekStart" => out.week_start = value.parse().ok().filter(|d| *d <= 6),
                "defaultEventLength" => {
                    out.default_event_minutes = value.parse().ok().filter(|m| *m > 0)
                }
                "autoAddHangouts" => out.auto_add_meet = value == "true",
                "remindOnRespondedEventsOnly" => out.remind_responded_only = value == "true",
                "timezone" => out.time_zone = Some(value.clone()).filter(|z| !z.is_empty()),
                _ => {}
            }
        }
        out
    }

    pub fn first_weekday(&self) -> chrono::Weekday {
        match self.week_start {
            Some(0) => chrono::Weekday::Sun,
            Some(6) => chrono::Weekday::Sat,
            _ => chrono::Weekday::Mon,
        }
    }
}

/// Which occurrences of a series a change or a delete applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeriesScope {
    This,
    Following,
    All,
}

#[derive(Debug, Deserialize)]
pub struct CalendarData {
    pub account: crate::data::Account,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
}

#[derive(Debug, Deserialize)]
pub struct Calendar {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "hex_color")]
    pub color: Hsla,
    /// Google's `calendarList.selected`.
    pub visible: bool,
    #[serde(default)]
    pub access_role: AccessRole,
    #[serde(default)]
    pub primary: bool,
    /// The calendar's IANA zone.
    #[serde(default)]
    pub time_zone: Option<String>,
    /// Minutes before, for events that use the calendar's default: the
    /// earliest of `default_reminders`.
    #[serde(default)]
    pub default_reminder: Option<u32>,
    /// Google's `defaultReminders`, all of them (CAL-33). Empty in demo
    /// mode, where `default_reminder` is the one.
    #[serde(default)]
    pub default_reminders: Vec<Reminder>,
    /// `conferenceProperties.allowedConferenceSolutionTypes` (CAL-34).
    #[serde(default)]
    pub conference_types: Vec<String>,
}

impl Calendar {
    /// The reminders of events that use the calendar's defaults.
    pub fn defaults(&self) -> Vec<Reminder> {
        if self.default_reminders.is_empty() {
            self.default_reminder
                .map(Reminder::popup)
                .into_iter()
                .collect()
        } else {
            self.default_reminders.clone()
        }
    }

    /// "Add Google Meet" is offered (CAL-34).
    pub fn allows_meet(&self) -> bool {
        self.conference_types.iter().any(|t| t == "hangoutsMeet")
    }

    /// Events can be added to it.
    pub fn writable(&self) -> bool {
        matches!(self.access_role, AccessRole::Owner | AccessRole::Writer)
    }
}

/// The calendar a new event goes to: the user's default, when it is still
/// there and writable, even if hidden. Otherwise the primary calendar when
/// shown, else the first shown one the user can write to, else any they can
/// write to. Before the first sync there may be none.
pub fn new_event_calendar<'a>(
    calendars: &'a [Calendar],
    default: Option<&str>,
) -> Option<&'a Calendar> {
    default
        .and_then(|id| calendars.iter().find(|c| c.id == id && c.writable()))
        .or_else(|| calendars.iter().find(|c| c.primary && c.visible))
        .or_else(|| calendars.iter().find(|c| c.writable() && c.visible))
        .or_else(|| calendars.iter().find(|c| c.writable()))
        .or(calendars.first())
}

/// What the user may do with a calendar's events (Google's `accessRole`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessRole {
    FreeBusyReader,
    Reader,
    Writer,
    #[default]
    Owner,
}

impl AccessRole {
    pub fn key(self) -> &'static str {
        match self {
            AccessRole::FreeBusyReader => "freeBusyReader",
            AccessRole::Reader => "reader",
            AccessRole::Writer => "writer",
            AccessRole::Owner => "owner",
        }
    }

    pub fn from_key(key: &str) -> AccessRole {
        match key {
            "freeBusyReader" => AccessRole::FreeBusyReader,
            "reader" => AccessRole::Reader,
            "writer" => AccessRole::Writer,
            _ => AccessRole::Owner,
        }
    }
}

/// Google's event `status`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventStatus {
    #[default]
    Confirmed,
    Tentative,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RsvpStatus {
    Accepted,
    Tentative,
    Declined,
    NeedsAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attendee {
    pub name: String,
    pub email: String,
    pub status: RsvpStatus,
}

impl Attendee {
    pub fn contact(&self) -> Contact {
        Contact {
            name: self.name.clone(),
            email: self.email.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Stable across edits and deletes, unlike the event's index.
    pub id: String,
    /// `Calendar::id`.
    pub calendar: String,
    pub title: String,
    /// The first occurrence. All-day events start at midnight.
    #[serde(deserialize_with = "wall_time")]
    pub start: NaiveDateTime,
    /// Exclusive. All-day events end at midnight after their last day.
    #[serde(deserialize_with = "wall_time")]
    pub end: NaiveDateTime,
    pub all_day: bool,
    #[serde(deserialize_with = "or_empty")]
    pub location: String,
    #[serde(deserialize_with = "or_empty")]
    pub description: String,
    pub attendees: Vec<Attendee>,
    pub organizer: Contact,
    /// The series' rule; its `until` is the series' end. The placeholder
    /// data writes it as a keyword ("weekly").
    #[serde(default, deserialize_with = "recurrence::rule_or_keyword")]
    pub recurrence: Option<Rule>,
    pub reminder_minutes: Option<u32>,
    #[serde(deserialize_with = "or_empty")]
    pub video_link: String,
    pub busy: bool,
    /// A series' occurrences that no longer show where the rule puts them,
    /// by their original start: deleted, or changed on their own (an event
    /// of their own then points back with `recurring_event_id`).
    #[serde(default)]
    pub exceptions: Vec<NaiveDateTime>,
    /// One occurrence of a series changed on its own: the series' id.
    #[serde(default)]
    pub recurring_event_id: Option<String>,
    /// With `recurring_event_id`: where the rule put this occurrence.
    #[serde(default)]
    pub original_start: Option<NaiveDateTime>,
    // From Google; empty in demo mode.
    #[serde(default)]
    pub etag: String,
    #[serde(default)]
    pub ical_uid: String,
    #[serde(default)]
    pub status: EventStatus,
    /// The event's own IANA zone, kept for writes (CAL-12).
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub html_link: String,
    #[serde(default)]
    pub organizer_self: bool,
    #[serde(default)]
    pub guests_can_modify: bool,
    /// Google left the guest list out, so it mustn't be written back.
    #[serde(default)]
    pub attendees_omitted: bool,
    /// Google's `eventType`: "default", "outOfOffice", "birthday" and so on.
    #[serde(default)]
    pub event_type: String,
    /// The Join link from Google's `conferenceData` (CAL-17); read-only.
    #[serde(default)]
    pub conference_uri: String,
    #[serde(default)]
    pub reminder_use_default: bool,
    /// The event's own reminders, when it has several or an email one
    /// (CAL-33); `reminder_minutes` is the earliest. Empty: just
    /// `reminder_minutes`, as a notification. Set both with
    /// `set_reminders`.
    #[serde(default)]
    pub reminders: Vec<Reminder>,
    /// "Add Google Meet" asked for, not yet answered by Google: the
    /// `createRequest`'s `requestId`, the same across retries (CAL-34).
    #[serde(default)]
    pub meet_request: Option<String>,
}

impl Event {
    /// Its own reminders (not the calendar's defaults).
    pub fn own_reminders(&self) -> Vec<Reminder> {
        if self.reminders.is_empty() {
            self.reminder_minutes
                .map(Reminder::popup)
                .into_iter()
                .collect()
        } else {
            self.reminders.clone()
        }
    }

    /// Its own reminders become `reminders`, sorted, without repeats.
    pub fn set_reminders(&mut self, mut reminders: Vec<Reminder>) {
        reminders.sort_by_key(|r| (r.minutes, r.method.key()));
        reminders.dedup();
        reminders.truncate(policy::MAX_REMINDERS);
        self.reminder_use_default = false;
        self.reminder_minutes = reminders.iter().map(|r| r.minutes).max();
        self.reminders = reminders;
    }

    pub fn duration(&self) -> Duration {
        self.end - self.start
    }

    /// Drawn as a bar across days: all-day, or timed over more than a day.
    pub fn is_spanning(&self) -> bool {
        self.all_day || self.duration() >= Duration::hours(24)
    }

    /// A repeating series (not one occurrence changed on its own).
    pub fn is_series(&self) -> bool {
        self.recurrence.is_some()
    }

    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            "(No title)"
        } else {
            &self.title
        }
    }

    /// "This and following" for a change (CAL-6): this series ends just
    /// before the occurrence its rule puts at `origin`, and a new series
    /// (`new_id`) starts at `start` with `edited`'s fields and rule, a COUNT
    /// counting only what's left. Returns both. Occurrences changed on their
    /// own from `origin` on belong to neither. `None` from the first
    /// occurrence, where "This and following" is the whole series. A new
    /// rule in `edited` (CAL-29) is the new series' as set; no rule makes
    /// it a single event.
    pub fn split(
        &self,
        origin: NaiveDateTime,
        start: NaiveDateTime,
        edited: &Event,
        new_id: String,
    ) -> Option<(Event, Event)> {
        let rule = self.recurrence.as_ref()?;
        if origin <= self.start {
            return None;
        }
        let mut old = self.clone();
        let old_rule = old.recurrence.as_mut()?;
        old_rule.until = Some(origin);
        old_rule.count = None;
        old.exceptions.retain(|&e| e < origin);
        let mut new_rule = edited.recurrence.clone();
        if let Some(new_rule) = &mut new_rule
            && new_rule.same_pattern(rule)
            && new_rule.count == rule.count
            && let Some(count) = rule.count
        {
            // COUNT counts the occurrences skipped too.
            let plain = Event {
                exceptions: Vec::new(),
                ..self.clone()
            };
            let before = occurrences(std::slice::from_ref(&plain), |_| true, self.start, origin)
                .iter()
                .filter(|o| o.start < origin)
                .count() as u32;
            new_rule.count = Some(count.saturating_sub(before).max(1));
            new_rule.until = None;
        }
        let new = Event {
            id: new_id,
            start,
            end: start + edited.duration(),
            recurrence: new_rule,
            meet_request: None,
            exceptions: Vec::new(),
            recurring_event_id: None,
            original_start: None,
            etag: String::new(),
            ical_uid: String::new(),
            html_link: String::new(),
            ..edited.clone()
        };
        Some((old, new))
    }

    /// Whether this occurrence record really differs from its `series`.
    /// Google lists an instance for occurrences that were never changed
    /// too, so having a `recurring_event_id` says nothing: it counts only
    /// when it moved, was resized, or has fields of its own.
    pub fn differs_from_series(&self, series: &Event) -> bool {
        let rule_start = self.original_start.unwrap_or(self.start);
        self.start != rule_start
            || self.duration() != series.duration()
            || (self.all_day, &self.title, &self.location, &self.description)
                != (
                    series.all_day,
                    &series.title,
                    &series.location,
                    &series.description,
                )
            || (&self.video_link, &self.conference_uri, self.busy)
                != (&series.video_link, &series.conference_uri, series.busy)
            || self.attendees != series.attendees
            || (
                self.reminder_use_default,
                self.reminder_minutes,
                &self.reminders,
            ) != (
                series.reminder_use_default,
                series.reminder_minutes,
                &series.reminders,
            )
    }

    /// Whether the fields the panel edits, and Discard puts back, are the
    /// same as `other`'s. The Meet isn't one: Google keeps a conference once
    /// it made it.
    pub fn same_edits(&self, other: &Event) -> bool {
        (
            &self.calendar,
            &self.title,
            &self.location,
            &self.description,
            &self.video_link,
            &self.attendees,
        ) == (
            &other.calendar,
            &other.title,
            &other.location,
            &other.description,
            &other.video_link,
            &other.attendees,
        ) && (
            self.start,
            self.end,
            self.all_day,
            &self.recurrence,
            &self.exceptions,
        ) == (
            other.start,
            other.end,
            other.all_day,
            &other.recurrence,
            &other.exceptions,
        ) && (
            self.busy,
            self.reminder_use_default,
            self.reminder_minutes,
            &self.reminders,
        ) == (
            other.busy,
            other.reminder_use_default,
            other.reminder_minutes,
            &other.reminders,
        )
    }

    /// Put the fields `same_edits` compares back to `from`'s, keeping the
    /// rest (the id, Google's etag and what it made) as it is now.
    pub fn restore_edits(&mut self, from: &Event) {
        self.calendar = from.calendar.clone();
        self.title = from.title.clone();
        self.location = from.location.clone();
        self.description = from.description.clone();
        self.video_link = from.video_link.clone();
        self.attendees = from.attendees.clone();
        self.start = from.start;
        self.end = from.end;
        self.all_day = from.all_day;
        self.recurrence = from.recurrence.clone();
        self.exceptions = from.exceptions.clone();
        self.busy = from.busy;
        self.reminder_use_default = from.reminder_use_default;
        self.reminder_minutes = from.reminder_minutes;
        self.reminders = from.reminders.clone();
    }

    /// My answer to an invite, when someone else organizes it. Any of my
    /// addresses is me (ACC-2).
    pub fn my_status(&self, me: &crate::data::Account) -> Option<RsvpStatus> {
        if me.is_me(&self.organizer.email) {
            return None;
        }
        self.attendees
            .iter()
            .find(|a| me.is_me(&a.email))
            .map(|a| a.status)
    }
}

/// The time as written, without its offset. The store writes it without one.
fn wall_time<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDateTime, D::Error> {
    let s = String::deserialize(d)?;
    DateTime::<FixedOffset>::parse_from_rfc3339(&s)
        .map(|t| t.naive_local())
        .or_else(|_| s.parse::<NaiveDateTime>())
        .map_err(serde::de::Error::custom)
}

fn or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// A calendar list read from the store, with the show/hide the user made
/// that hasn't committed yet kept as made: the store still holds the state
/// before it (or before the last one that did commit), which would flip the
/// checkbox back until the next commit catches up.
pub fn keep_pending_visibility(
    fresh: &mut [Calendar],
    current: &[Calendar],
    pending: &std::collections::HashSet<String>,
) {
    for calendar in fresh.iter_mut().filter(|c| pending.contains(&c.id)) {
        if let Some(local) = current.iter().find(|c| c.id == calendar.id) {
            calendar.visible = local.visible;
        }
    }
}

impl CalendarData {
    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    pub fn calendar_ix(&self, id: &str) -> Option<usize> {
        self.calendars.iter().position(|c| c.id == id)
    }

    pub fn event_ix(&self, id: &str) -> Option<usize> {
        self.events.iter().position(|e| e.id == id)
    }

    pub fn color_of(&self, event: &Event) -> Hsla {
        self.calendar(&event.calendar)
            .map(|c| c.color)
            .unwrap_or(gpui::hsla(0.6, 0.1, 0.5, 1.))
    }

    pub fn is_visible(&self, event: &Event) -> bool {
        self.calendar(&event.calendar).is_some_and(|c| c.visible)
    }

    pub fn me(&self) -> Contact {
        Contact {
            name: self.account.name.clone(),
            email: self.account.email.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_calendar_loads() {
        let data = crate::backend::demo::calendar_data();
        assert_eq!(data.calendars.len(), 8);
        assert_eq!(data.events.len(), 167);
        let standup = &data.events[1];
        assert_eq!(standup.recurrence, Some(Rule::weekdays()));
        assert_eq!(standup.start.format("%H:%M").to_string(), "09:30");
        assert!(standup.description.is_empty());
    }

    fn listed(id: &str, visible: bool) -> Calendar {
        Calendar {
            id: id.into(),
            name: id.into(),
            color: gpui::hsla(0., 0., 0.5, 1.),
            visible,
            access_role: AccessRole::default(),
            primary: false,
            time_zone: None,
            default_reminder: None,
            default_reminders: Vec::new(),
            conference_types: Vec::new(),
        }
    }

    #[test]
    fn new_events_go_to_the_default_calendar_when_it_can_take_them() {
        let mut primary = listed("me", true);
        primary.primary = true;
        let mut team = listed("team", false);
        team.access_role = AccessRole::Writer;
        let mut holidays = listed("holidays", true);
        holidays.access_role = AccessRole::Reader;
        let calendars = [primary, team, holidays];
        let pick = |default| new_event_calendar(&calendars, default).map(|c| c.id.as_str());

        // Automatic: the primary calendar.
        assert_eq!(pick(None), Some("me"));
        // A chosen one wins, even hidden.
        assert_eq!(pick(Some("team")), Some("team"));
        // Gone, read-only, or not loaded yet: automatic again.
        assert_eq!(pick(Some("deleted")), Some("me"));
        assert_eq!(pick(Some("holidays")), Some("me"));
        assert_eq!(new_event_calendar(&[], Some("team")).map(|c| &c.id), None);
    }

    #[test]
    fn automatic_falls_back_to_shown_then_any_writable_calendar() {
        let mut team = listed("team", true);
        team.access_role = AccessRole::Writer;
        let mut hidden = listed("hidden", false);
        hidden.primary = true;
        let read_only = || {
            let mut ro = listed("ro", true);
            ro.access_role = AccessRole::Reader;
            ro
        };
        let calendars = [read_only(), hidden, team];
        assert_eq!(new_event_calendar(&calendars, None).unwrap().id, "team");
        let mut hidden = listed("hidden", false);
        hidden.primary = true;
        let calendars = [read_only(), hidden];
        assert_eq!(new_event_calendar(&calendars, None).unwrap().id, "hidden");
    }

    /// Toggling a calendar on, off, on while the first commits: the store
    /// reloads in between with the first change, and the list stays as the
    /// user left it.
    #[test]
    fn a_reload_keeps_calendars_whose_show_hide_is_still_queued() {
        let current = [listed("a", true), listed("b", false), listed("c", true)];
        // The store holds a's first change (hidden), b's old state, c's new one.
        let mut fresh = vec![listed("a", false), listed("b", true), listed("c", false)];
        let pending = std::collections::HashSet::from(["a".to_string(), "b".to_string()]);
        keep_pending_visibility(&mut fresh, &current, &pending);
        let shown: Vec<bool> = fresh.iter().map(|c| c.visible).collect();
        // c has nothing queued: the store wins (a failed change rolls back).
        assert_eq!(shown, [true, false, false]);

        // Nothing queued: the store as it is.
        let mut fresh = vec![listed("a", false)];
        keep_pending_visibility(&mut fresh, &current, &Default::default());
        assert!(!fresh[0].visible);
    }

    fn monday(n: u32) -> NaiveDateTime {
        time::at(chrono::NaiveDate::from_ymd_opt(2026, 11, n).unwrap(), 9, 0)
    }

    /// Weekly on Mondays from Nov 2, with the Nov 9 and Nov 23 ones changed
    /// or skipped.
    fn weekly() -> Event {
        Event {
            id: "s1".into(),
            title: "Sync".into(),
            start: monday(2),
            end: monday(2) + Duration::minutes(30),
            recurrence: Some(Rule::weekly()),
            exceptions: vec![monday(9), monday(23)],
            etag: "\"s\"".into(),
            ..Event::default()
        }
    }

    #[test]
    fn split_ends_the_series_before_the_occurrence_and_starts_a_new_one() {
        let series = weekly();
        let edited = Event {
            title: "Sync (new room)".into(),
            ..series.clone()
        };
        let at = monday(16) + Duration::hours(1);
        let (old, new) = series.split(monday(16), at, &edited, "n1".into()).unwrap();
        assert_eq!(old.recurrence.as_ref().unwrap().until, Some(monday(16)));
        assert_eq!(old.title, "Sync");
        assert_eq!(old.exceptions, [monday(9)]);
        assert_eq!(new.id, "n1");
        assert_eq!(new.title, "Sync (new room)");
        assert_eq!((new.start, new.end), (at, at + Duration::minutes(30)));
        assert_eq!(new.recurrence, Some(Rule::weekly()));
        assert!(new.exceptions.is_empty() && new.etag.is_empty());
        assert!(new.recurring_event_id.is_none());
    }

    #[test]
    fn split_counts_only_the_occurrences_left() {
        let mut series = weekly();
        series.recurrence.as_mut().unwrap().count = Some(6);
        let (old, new) = series
            .split(monday(16), monday(16), &series, "n1".into())
            .unwrap();
        // Nov 2 and 9 (skipped, still counted) came before.
        let old_rule = old.recurrence.unwrap();
        assert_eq!((old_rule.until, old_rule.count), (Some(monday(16)), None));
        let new_rule = new.recurrence.unwrap();
        assert_eq!((new_rule.count, new_rule.until), (Some(4), None));
    }

    #[test]
    fn split_with_a_new_repeat_takes_it_whole() {
        let mut series = weekly();
        series.recurrence.as_mut().unwrap().count = Some(6);
        series.meet_request = Some("req".into());
        let edited = Event {
            recurrence: Some(Rule::daily()),
            ..series.clone()
        };
        let (old, new) = series
            .split(monday(16), monday(16), &edited, "n1".into())
            .unwrap();
        assert_eq!(old.recurrence.unwrap().until, Some(monday(16)));
        // Not the old COUNT, adjusted: the new rule as set.
        assert_eq!(new.recurrence, Some(Rule::daily()));
        assert!(new.meet_request.is_none());
        // No repeat: from the occurrence on, a single event.
        let single = Event {
            recurrence: None,
            ..series.clone()
        };
        let (_, new) = series
            .split(monday(16), monday(16), &single, "n2".into())
            .unwrap();
        assert!(new.recurrence.is_none() && !new.is_series());
    }

    /// The record Google lists for an occurrence nobody changed.
    fn instance_of(series: &Event, at: NaiveDateTime) -> Event {
        Event {
            id: format!("{}_{at}", series.id),
            start: at,
            end: at + series.duration(),
            recurrence: None,
            exceptions: Vec::new(),
            recurring_event_id: Some(series.id.clone()),
            original_start: Some(at),
            ..series.clone()
        }
    }

    #[test]
    fn an_unchanged_instance_does_not_differ_from_its_series() {
        let series = weekly();
        assert!(!instance_of(&series, monday(16)).differs_from_series(&series));
    }

    #[test]
    fn an_instance_differs_when_moved_resized_or_edited() {
        let series = weekly();
        let same = || instance_of(&series, monday(16));
        let moved = Event {
            start: monday(16) + Duration::hours(1),
            end: monday(16) + Duration::hours(1) + series.duration(),
            ..same()
        };
        let resized = Event {
            end: monday(16) + Duration::hours(2),
            ..same()
        };
        let renamed = Event {
            title: "Sync (new room)".into(),
            ..same()
        };
        let guest = Event {
            attendees: vec![Attendee {
                name: String::new(),
                email: "a@b.c".into(),
                status: RsvpStatus::Accepted,
            }],
            ..same()
        };
        let free = Event {
            busy: !series.busy,
            ..same()
        };
        for changed in [moved, resized, renamed, guest, free] {
            assert!(changed.differs_from_series(&series), "{changed:?}");
        }
    }

    #[test]
    fn restoring_edits_keeps_what_google_made() {
        let base = weekly();
        let mut event = Event {
            title: "Renamed".into(),
            location: "Room 2".into(),
            start: monday(3),
            end: monday(3) + Duration::hours(1),
            busy: !base.busy,
            exceptions: vec![monday(10)],
            recurrence: None,
            calendar: "other".into(),
            etag: "\"newer\"".into(),
            conference_uri: "https://meet.google.com/abc-defg-hij".into(),
            ..base.clone()
        };
        assert!(!event.same_edits(&base));
        event.restore_edits(&base);
        assert!(event.same_edits(&base));
        assert_eq!(event.etag, "\"newer\"");
        assert_eq!(event.conference_uri, "https://meet.google.com/abc-defg-hij");
        assert_eq!(
            (event.start, &event.exceptions),
            (base.start, &base.exceptions)
        );
        // Only the Meet differs: nothing to discard.
        assert!(event.same_edits(&Event {
            conference_uri: String::new(),
            meet_request: Some("req".into()),
            ..base
        }));
    }

    #[test]
    fn split_at_the_first_occurrence_is_the_whole_series() {
        let series = weekly();
        assert!(
            series
                .split(monday(2), monday(2), &series, "n1".into())
                .is_none()
        );
    }
}

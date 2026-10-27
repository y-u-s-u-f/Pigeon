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
    pub format_24h: Option<bool>,
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
                "format24HourTime" => out.format_24h = value.parse().ok(),
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

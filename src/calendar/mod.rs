//! Calendar model, loaded from the bundled placeholder data, plus the pure
//! logic the views run on: recurrence expansion (`recurrence`), overlap
//! layout (`layout`) and date math and parsing (`time`).
//!
//! Times are wall-clock times (`NaiveDateTime`): the JSON's offsets are
//! dropped and every time is shown as written. See CAL-12 in
//! `docs/BACKEND_REQUIREMENTS.md`.

pub mod layout;
pub mod recurrence;
pub mod time;

use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime};
use gpui::Hsla;
use serde::{Deserialize, Deserializer};

use crate::data::{Contact, hex_color};

pub use recurrence::{Occurrence, occurrences};

/// Which occurrences of a series a change or a delete applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Recurrence {
    Daily,
    Weekdays,
    Weekly,
    Biweekly,
    Monthly,
    Yearly,
}

impl Recurrence {
    pub fn label(self) -> &'static str {
        match self {
            Recurrence::Daily => "Every day",
            Recurrence::Weekdays => "Every weekday",
            Recurrence::Weekly => "Every week",
            Recurrence::Biweekly => "Every 2 weeks",
            Recurrence::Monthly => "Every month",
            Recurrence::Yearly => "Every year",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RsvpStatus {
    Accepted,
    Tentative,
    Declined,
    NeedsAction,
}

#[derive(Clone, Debug, Deserialize)]
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

#[derive(Clone, Debug, Deserialize)]
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
    pub recurrence: Option<Recurrence>,
    pub reminder_minutes: Option<u32>,
    #[serde(deserialize_with = "or_empty")]
    pub video_link: String,
    pub busy: bool,
    /// A series' occurrences that no longer show where the rule puts them,
    /// by their original start: deleted, or changed on their own (an event
    /// of their own then points back with `recurring_event_id`).
    #[serde(default)]
    pub exceptions: Vec<NaiveDateTime>,
    /// A series' end: no occurrence starts at or after it.
    #[serde(default)]
    pub until: Option<NaiveDateTime>,
    /// One occurrence of a series changed on its own: the series' id.
    #[serde(default)]
    pub recurring_event_id: Option<String>,
    /// With `recurring_event_id`: where the rule put this occurrence.
    #[serde(default)]
    pub original_start: Option<NaiveDateTime>,
}

impl Event {
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

    /// My answer to an invite, when someone else organizes it.
    pub fn my_status(&self, me: &str) -> Option<RsvpStatus> {
        if self.organizer.email.eq_ignore_ascii_case(me) {
            return None;
        }
        self.attendees
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(me))
            .map(|a| a.status)
    }
}

/// The time as written, without its offset.
fn wall_time<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDateTime, D::Error> {
    Ok(DateTime::<FixedOffset>::deserialize(d)?.naive_local())
}

fn or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

impl CalendarData {
    pub fn load() -> anyhow::Result<Self> {
        Ok(serde_json::from_str(include_str!(
            "../../assets/calendar.json"
        ))?)
    }

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
        let data = CalendarData::load().unwrap();
        assert_eq!(data.calendars.len(), 8);
        assert_eq!(data.events.len(), 167);
        let standup = &data.events[1];
        assert_eq!(standup.recurrence, Some(Recurrence::Weekdays));
        assert_eq!(standup.start.format("%H:%M").to_string(), "09:30");
        assert!(standup.description.is_empty());
    }
}

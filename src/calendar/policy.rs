//! The shared event policy (docs/BACKEND_SPRINT_2.md, W5): which events
//! count for the user, their reminders, their video links, and the meeting
//! to join next. The menu bar, quick join and reminder notifications all
//! decide the same way through here.

use chrono::{Duration, NaiveDateTime};
use serde::{Deserialize, Serialize};

use super::{Calendar, CalendarData, Event, EventStatus, Occurrence, RsvpStatus, occurrences};

/// How Google delivers a reminder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderMethod {
    /// A notification on the user's devices; Pigeon shows these.
    #[default]
    Popup,
    Email,
}

impl ReminderMethod {
    pub fn key(self) -> &'static str {
        match self {
            ReminderMethod::Popup => "popup",
            ReminderMethod::Email => "email",
        }
    }

    pub fn from_key(key: &str) -> Self {
        if key == "email" {
            ReminderMethod::Email
        } else {
            ReminderMethod::Popup
        }
    }
}

/// One reminder: how long before the event, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Reminder {
    pub minutes: u32,
    pub method: ReminderMethod,
}

impl Reminder {
    pub fn popup(minutes: u32) -> Self {
        Self {
            minutes,
            method: ReminderMethod::Popup,
        }
    }
}

/// `list`, or when it's empty, a popup `fallback` minutes ahead.
pub fn reminders_or(list: &[Reminder], fallback: Option<u32>) -> Vec<Reminder> {
    if list.is_empty() {
        fallback.map(Reminder::popup).into_iter().collect()
    } else {
        list.to_vec()
    }
}

/// The minutes of `reminders_or`, without building it.
pub fn reminder_minutes(
    list: &[Reminder],
    fallback: Option<u32>,
) -> impl Iterator<Item = u32> + '_ {
    list.iter()
        .map(|r| r.minutes)
        .chain(fallback.filter(|_| list.is_empty()))
}

/// Google takes at most this many overrides per event.
pub const MAX_REMINDERS: usize = 5;
/// Google's longest reminder: four weeks.
pub const MAX_REMINDER_MINUTES: u32 = 40320;

/// The reminders that apply to `event` on `calendar`: the calendar's
/// defaults when the event uses them, else its own.
pub fn reminders_for(event: &Event, calendar: &Calendar) -> Vec<Reminder> {
    if event.reminder_use_default {
        calendar.defaults()
    } else {
        event.own_reminders()
    }
}

/// The user's own answer, when someone else organizes the event; aliases
/// count as the user.
pub fn my_answer(data: &CalendarData, event: &Event) -> Option<RsvpStatus> {
    event.my_status(&data.account)
}

/// Whether an event counts for the user: on a calendar they show, not
/// cancelled, and not declined.
pub fn counts(data: &CalendarData, event: &Event) -> bool {
    data.is_visible(event)
        && event.status != EventStatus::Cancelled
        && my_answer(data, event) != Some(RsvpStatus::Declined)
}

/// How far ahead `joinable` looks.
pub const JOIN_AHEAD: Duration = Duration::hours(24);

/// The meetings that can be joined at `now` or within `JOIN_AHEAD`: events
/// that count, aren't all day, and have a video link. Best first: the ones
/// under way, the one that started most recently first, then the ones to
/// come, soonest first. `next_joinable` is the first; the tests check the
/// whole order.
#[cfg(test)]
pub fn joinable(data: &CalendarData, now: NaiveDateTime) -> Vec<Occurrence> {
    let mut found = joinable_unsorted(data, now);
    found.sort_by_key(|o| join_rank(o, now));
    found
}

/// The meeting to join: see `joinable`.
pub fn next_joinable(data: &CalendarData, now: NaiveDateTime) -> Option<Occurrence> {
    joinable_unsorted(data, now)
        .into_iter()
        .min_by_key(|o| join_rank(o, now))
}

/// `joinable`'s meetings in any order. The video link is looked for only
/// in occurrences already in the window (MC-061).
fn joinable_unsorted(data: &CalendarData, now: NaiveDateTime) -> Vec<Occurrence> {
    occurrences(
        &data.events,
        |e| !e.all_day && counts(data, e),
        now - Duration::days(1),
        now + JOIN_AHEAD,
    )
    .into_iter()
    .filter(|o| o.end > now && video_link(&data.events[o.event]).is_some())
    .collect()
}

/// Under way first, the latest start first; then the soonest to come.
fn join_rank(o: &Occurrence, now: NaiveDateTime) -> (u8, Duration, usize) {
    if o.start <= now {
        (0, now - o.start, o.event)
    } else {
        (1, o.start - now, o.event)
    }
}

/// The event's video link: Google's conference (CAL-17), the one typed in
/// Pigeon, or a Meet, Zoom, Teams or Webex link in its location or notes.
pub fn video_link(event: &Event) -> Option<&str> {
    [event.conference_uri.as_str(), event.video_link.as_str()]
        .into_iter()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .or_else(|| find_video_link(&event.location))
        .or_else(|| find_video_link(&event.description))
}

/// The first Meet, Zoom, Teams or Webex link in `text`.
pub fn find_video_link(text: &str) -> Option<&str> {
    text.split(|c: char| c.is_whitespace() || "<>\"'()[]".contains(c))
        .map(|t| t.trim_end_matches(['.', ',', ';', ':', '!', '?']))
        .find(|t| is_video_url(t))
}

fn is_video_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = &rest[host.len()..];
    let on = |domain: &str| {
        host.strip_suffix(domain)
            .is_some_and(|sub| sub.is_empty() || sub.ends_with('.'))
    };
    (host == "meet.google.com" && path.len() > 1)
        || (on("zoom.us") && (path.starts_with("/j/") || path.starts_with("/my/")))
        || ((host == "teams.microsoft.com" || host == "teams.live.com") && path.len() > 1)
        || (on("webex.com") && path.len() > 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calendar::{AccessRole, Attendee, time::at},
        data::{Account, Contact},
    };
    use chrono::NaiveDate;

    const ME: &str = "me@example.com";

    fn d(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, n).unwrap()
    }

    fn calendar(id: &str, visible: bool) -> Calendar {
        Calendar {
            id: id.into(),
            name: id.into(),
            color: gpui::hsla(0., 0., 0., 1.),
            visible,
            access_role: AccessRole::Owner,
            primary: false,
            time_zone: None,
            default_reminder: Some(10),
            default_reminders: Vec::new(),
            conference_types: Vec::new(),
        }
    }

    fn meeting(id: &str, start: NaiveDateTime, minutes: i64) -> Event {
        Event {
            id: id.into(),
            calendar: "work".into(),
            title: id.into(),
            start,
            end: start + Duration::minutes(minutes),
            conference_uri: format!("https://meet.google.com/{id}"),
            organizer_self: true,
            ..Event::default()
        }
    }

    fn data(events: Vec<Event>) -> CalendarData {
        CalendarData {
            account: Account {
                name: "Me".into(),
                email: ME.into(),
                aliases: vec!["alias@example.com".into()],
                send_as: Vec::new(),
            },
            calendars: vec![calendar("work", true), calendar("hidden", false)],
            events,
        }
    }

    #[test]
    fn video_links_from_conference_typed_or_text() {
        let mut e = Event::default();
        assert_eq!(video_link(&e), None);
        e.location = "Room 4, or https://acme.zoom.us/j/123?pwd=x.".into();
        assert_eq!(video_link(&e), Some("https://acme.zoom.us/j/123?pwd=x"));
        e.description = "Join: https://teams.microsoft.com/l/meetup-join/abc".into();
        e.location.clear();
        assert_eq!(
            video_link(&e),
            Some("https://teams.microsoft.com/l/meetup-join/abc")
        );
        e.description = "<a href=\"https://acme.webex.com/meet/sam\">Webex</a>".into();
        assert_eq!(video_link(&e), Some("https://acme.webex.com/meet/sam"));
        e.video_link = "https://example.com/call".into();
        assert_eq!(video_link(&e), Some("https://example.com/call"));
        e.conference_uri = "https://meet.google.com/abc-defg-hij".into();
        assert_eq!(video_link(&e), Some("https://meet.google.com/abc-defg-hij"));
        // A bare Zoom link in the notes, with a query and a fragment.
        let zoom =
            "https://acme.zoom.us/j/12345678901?pwd=AbCdEf123456.1#success";
        let e = Event {
            description: zoom.into(),
            ..Event::default()
        };
        assert_eq!(video_link(&e), Some(zoom));
        assert_eq!(find_video_link(&format!("Join:\n{zoom}\n")), Some(zoom));
        // Not video calls.
        assert_eq!(find_video_link("https://zoom.us/pricing"), None);
        assert_eq!(find_video_link("https://notzoom.us/j/1"), None);
        assert_eq!(find_video_link("meet.google.com/abc"), None);
    }

    #[test]
    fn next_joinable_prefers_the_latest_started_then_the_soonest() {
        let now = at(d(26), 10, 5);
        let events = vec![
            meeting("early", at(d(26), 9, 30), 60),
            meeting("late", at(d(26), 10, 0), 60),
            meeting("soon", at(d(26), 10, 30), 30),
            meeting("later", at(d(26), 11, 0), 30),
            meeting("over", at(d(26), 9, 0), 30),
        ];
        let data = data(events);
        let order: Vec<&str> = joinable(&data, now)
            .iter()
            .map(|o| data.events[o.event].id.as_str())
            .collect();
        assert_eq!(order, ["late", "early", "soon", "later"]);
        let next = next_joinable(&data, now).unwrap();
        assert_eq!(data.events[next.event].id, "late");
        // Before any started: the soonest.
        let next = next_joinable(&data, at(d(26), 8, 0)).unwrap();
        assert_eq!(data.events[next.event].id, "over");
    }

    #[test]
    fn only_events_that_count_are_joinable() {
        let now = at(d(26), 9, 0);
        let start = at(d(26), 9, 30);
        let mut hidden = meeting("hidden", start, 30);
        hidden.calendar = "hidden".into();
        let mut cancelled = meeting("cancelled", start, 30);
        cancelled.status = EventStatus::Cancelled;
        let mut declined = meeting("declined", start, 30);
        declined.organizer_self = false;
        declined.organizer = Contact {
            name: String::new(),
            email: "boss@example.com".into(),
        };
        declined.attendees = vec![Attendee {
            name: String::new(),
            email: "ALIAS@example.com".into(),
            status: RsvpStatus::Declined,
        }];
        let mut all_day = meeting("all-day", at(d(26), 0, 0), 24 * 60);
        all_day.all_day = true;
        let mut no_link = meeting("no-link", start, 30);
        no_link.conference_uri.clear();
        let mut maybe = declined.clone();
        maybe.id = "maybe".into();
        maybe.attendees[0].status = RsvpStatus::Tentative;
        let data = data(vec![hidden, cancelled, declined, all_day, no_link, maybe]);
        let found: Vec<&str> = joinable(&data, now)
            .iter()
            .map(|o| data.events[o.event].id.as_str())
            .collect();
        assert_eq!(found, ["maybe"]);
    }

    #[test]
    fn reminders_come_from_the_event_or_the_calendar() {
        let mut cal = calendar("work", true);
        let mut e = Event {
            reminder_minutes: Some(5),
            ..Event::default()
        };
        assert_eq!(reminders_for(&e, &cal), [Reminder::popup(5)]);
        e.reminders = vec![
            Reminder::popup(5),
            Reminder {
                minutes: 60,
                method: ReminderMethod::Email,
            },
        ];
        assert_eq!(reminders_for(&e, &cal).len(), 2);
        e.reminder_use_default = true;
        assert_eq!(reminders_for(&e, &cal), [Reminder::popup(10)]);
        cal.default_reminders = vec![Reminder::popup(10), Reminder::popup(1440)];
        assert_eq!(reminders_for(&e, &cal), cal.default_reminders);
    }
}

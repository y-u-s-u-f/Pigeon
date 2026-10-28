//! Reminder notifications (W5): which popup reminders are due, best effort
//! while the Mac is awake and Pigeon runs.
//!
//! `Scheduler::tick` is called with the wall clock and a clock that stops
//! while the Mac sleeps (`Instant`). Between two ticks it fires what fell
//! due. When the two clocks disagree (a wake, or the clock was changed),
//! and on the first tick at launch, it only starts counting again: what was
//! missed is never fired. Fired reminders are keyed by (calendar, event,
//! occurrence start, minutes before), which the store keeps, so none fires
//! twice.

use std::collections::HashSet;

use chrono::{Duration, NaiveDateTime};

use super::{
    CalendarData, Event, ReminderMethod, RsvpStatus, occurrences,
    policy::{self, MAX_REMINDER_MINUTES},
};

/// One reminder of one occurrence.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReminderKey {
    pub calendar: String,
    pub event: String,
    /// The occurrence's start.
    pub start: NaiveDateTime,
    pub minutes: u32,
}

/// A reminder that fell due, with what the notification says.
#[derive(Clone, Debug, PartialEq)]
pub struct DueReminder {
    pub key: ReminderKey,
    /// When it was due.
    pub at: NaiveDateTime,
    pub title: String,
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub location: String,
    /// The event's video link, for a Join action.
    pub join_url: Option<String>,
}

/// The clocks may drift this much between ticks before it counts as a jump.
const JUMP: Duration = Duration::seconds(90);
/// A reminder this late isn't shown any more (a slow tick).
const STALE: Duration = Duration::minutes(5);

/// Whether the user gets reminders of `event`: it counts (`policy`), and
/// with Google's `remindOnRespondedEventsOnly`, they answered it.
fn reminds(data: &CalendarData, event: &Event, responded_only: bool) -> bool {
    policy::counts(data, event)
        && !(responded_only
            && policy::my_answer(data, event).is_none_or(|a| a == RsvpStatus::NeedsAction)
            && !event.organizer_self
            && !data.account.is_me(&event.organizer.email)
            && !event.organizer.email.is_empty())
}

/// The popup reminders due in `(after, until]`, by when they're due.
pub fn due_between(
    data: &CalendarData,
    responded_only: bool,
    after: NaiveDateTime,
    until: NaiveDateTime,
) -> Vec<DueReminder> {
    // The earliest any reminder comes, without copying them (MC-064).
    let lead = data
        .calendars
        .iter()
        .filter_map(|c| c.default_minutes().max())
        .chain(data.events.iter().filter_map(|e| e.own_minutes().max()))
        .map(|m| m.min(MAX_REMINDER_MINUTES))
        .max()
        .unwrap_or(0);
    let mut out = Vec::new();
    for o in occurrences(
        &data.events,
        |e| reminds(data, e, responded_only),
        after,
        until + Duration::minutes(lead as i64 + 1),
    ) {
        let event = &data.events[o.event];
        let Some(calendar) = data.calendar(&event.calendar) else {
            continue;
        };
        for r in policy::reminders_for(event, calendar) {
            if r.method != ReminderMethod::Popup {
                continue;
            }
            let at = o.start - Duration::minutes(r.minutes as i64);
            if at <= after || at > until {
                continue;
            }
            out.push(DueReminder {
                key: ReminderKey {
                    calendar: event.calendar.clone(),
                    event: event.id.clone(),
                    start: o.start,
                    minutes: r.minutes,
                },
                at,
                title: event.display_title().to_string(),
                end: o.end,
                all_day: event.all_day,
                location: event.location.clone(),
                join_url: policy::video_link(event).map(str::to_string),
            });
        }
    }
    out.sort_by_key(|d| d.at);
    out.dedup_by(|a, b| a.key == b.key);
    out
}

#[derive(Default)]
pub struct Scheduler {
    /// The last tick: the wall clock, and the clock that stops in sleep.
    last: Option<(NaiveDateTime, std::time::Duration)>,
    fired: HashSet<ReminderKey>,
}

impl Scheduler {
    /// With the reminders fired before (from the store).
    pub fn new(fired: HashSet<ReminderKey>) -> Self {
        Self { last: None, fired }
    }

    /// The reminders due since the last tick and not fired yet, now marked
    /// fired. `mono` is a clock that stops while the Mac sleeps.
    pub fn tick(
        &mut self,
        data: &CalendarData,
        responded_only: bool,
        wall: NaiveDateTime,
        mono: std::time::Duration,
    ) -> Vec<DueReminder> {
        let after = self.last.and_then(|(last_wall, last_mono)| {
            let slept = mono.checked_sub(last_mono)?;
            let drift = (wall - last_wall) - Duration::from_std(slept).ok()?;
            (drift.abs() <= JUMP && wall > last_wall).then_some(last_wall)
        });
        self.last = Some((wall, mono));
        let Some(after) = after else {
            return Vec::new();
        };
        let due: Vec<DueReminder> = due_between(data, responded_only, after, wall)
            .into_iter()
            .filter(|d| wall - d.at <= STALE && !self.fired.contains(&d.key))
            .collect();
        self.fired.extend(due.iter().map(|d| d.key.clone()));
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calendar::{AccessRole, Attendee, Calendar, Reminder, Rule, time::at},
        data::{Account, Contact},
    };
    use chrono::NaiveDate;
    use std::time::Duration as Mono;

    fn d(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, n).unwrap()
    }

    fn data(events: Vec<Event>) -> CalendarData {
        CalendarData {
            account: Account {
                name: "Me".into(),
                email: "me@example.com".into(),
                aliases: Vec::new(),
                send_as: Vec::new(),
            },
            calendars: vec![Calendar {
                id: "work".into(),
                name: "Work".into(),
                color: gpui::hsla(0., 0., 0., 1.),
                visible: true,
                access_role: AccessRole::Owner,
                primary: true,
                time_zone: None,
                default_reminder: Some(10),
                default_reminders: vec![Reminder::popup(10), Reminder::popup(60)],
                conference_types: Vec::new(),
            }],
            events,
        }
    }

    fn event(id: &str, start: NaiveDateTime) -> Event {
        Event {
            id: id.into(),
            calendar: "work".into(),
            title: id.into(),
            start,
            end: start + Duration::minutes(30),
            reminder_use_default: true,
            organizer_self: true,
            ..Event::default()
        }
    }

    /// Ticks once a minute from `from` for `minutes`, the clocks in step.
    fn run(
        s: &mut Scheduler,
        data: &CalendarData,
        from: NaiveDateTime,
        minutes: i64,
    ) -> Vec<(String, u32)> {
        let mut fired = Vec::new();
        for m in 0..=minutes {
            let wall = from + Duration::minutes(m);
            let mono = Mono::from_secs(1_000 + 60 * m as u64);
            fired.extend(
                s.tick(data, false, wall, mono)
                    .into_iter()
                    .map(|r| (r.key.event, r.key.minutes)),
            );
        }
        fired
    }

    #[test]
    fn fires_each_reminder_once_when_due() {
        let data = data(vec![event("standup", at(d(26), 10, 0))]);
        let mut s = Scheduler::default();
        let fired = run(&mut s, &data, at(d(26), 8, 30), 120);
        assert_eq!(
            fired,
            [("standup".to_string(), 60), ("standup".to_string(), 10)]
        );
        // The same minutes again: nothing twice.
        s.last = None;
        assert!(run(&mut s, &data, at(d(26), 8, 30), 120).is_empty());
        assert_eq!(
            due_between(&data, false, at(d(26), 8, 0), at(d(27), 8, 0))
                .first()
                .map(|d| d.at),
            Some(at(d(26), 9, 0))
        );
    }

    #[test]
    fn past_reminders_never_fire_at_launch_or_wake() {
        let data = data(vec![
            event("a", at(d(26), 10, 0)),
            event("b", at(d(26), 12, 0)),
        ]);
        let mut s = Scheduler::default();
        // Launched at 9:55: the 9:00 and 9:50 reminders of "a" are past.
        assert!(
            s.tick(&data, false, at(d(26), 9, 55), Mono::from_secs(100))
                .is_empty()
        );
        assert!(
            s.tick(&data, false, at(d(26), 9, 56), Mono::from_secs(160))
                .is_empty()
        );
        // Asleep from 9:56 to 11:55: the wall clock moved two hours, the
        // other a few seconds. "b"'s 11:00 and "a"'s are missed, not fired.
        assert!(
            s.tick(&data, false, at(d(26), 11, 55), Mono::from_secs(165))
                .is_empty()
        );
        // Then on as usual; "b"'s 11:50 reminder was past already.
        let after = s.tick(&data, false, at(d(26), 11, 56), Mono::from_secs(225));
        assert!(after.is_empty());
        // The clock set back to 8:00: nothing at once; what falls due from
        // there on (never fired before) does.
        assert!(
            s.tick(&data, false, at(d(26), 8, 0), Mono::from_secs(285))
                .is_empty()
        );
        let fired = s.tick(&data, false, at(d(26), 9, 1), Mono::from_secs(3_945));
        assert_eq!(fired.len(), 1);
        assert_eq!(
            (fired[0].key.event.as_str(), fired[0].key.minutes),
            ("a", 60)
        );
    }

    #[test]
    fn wake_then_due_fires() {
        let data = data(vec![event("c", at(d(26), 14, 0))]);
        let mut s = Scheduler::default();
        s.tick(&data, false, at(d(26), 9, 0), Mono::from_secs(10));
        // Woke at 13:40; the 13:00 reminder is missed, the 13:50 one fires.
        s.tick(&data, false, at(d(26), 13, 40), Mono::from_secs(20));
        let fired = run(&mut s, &data, at(d(26), 13, 41), 15);
        assert_eq!(fired, [("c".to_string(), 10)]);
    }

    #[test]
    fn fired_keys_from_the_store_and_who_is_reminded() {
        let start = at(d(26), 10, 0);
        let mut series = event("series", start);
        series.recurrence = Some(Rule::daily());
        let mut email = event("email", start);
        email.set_reminders(vec![Reminder {
            minutes: 10,
            method: ReminderMethod::Email,
        }]);
        let mut invite = event("invite", start);
        invite.organizer_self = false;
        invite.organizer = Contact {
            name: String::new(),
            email: "boss@example.com".into(),
        };
        invite.attendees = vec![Attendee {
            name: String::new(),
            email: "me@example.com".into(),
            status: RsvpStatus::NeedsAction,
        }];
        let data = data(vec![series, email, invite]);
        let key = ReminderKey {
            calendar: "work".into(),
            event: "series".into(),
            start,
            minutes: 10,
        };
        let all = due_between(&data, false, at(d(26), 9, 40), at(d(26), 9, 55));
        let names: Vec<&str> = all.iter().map(|r| r.key.event.as_str()).collect();
        assert_eq!(names, ["series", "invite"], "no email reminders");
        // remindOnRespondedEventsOnly: not the unanswered invite.
        let answered = due_between(&data, true, at(d(26), 9, 40), at(d(26), 9, 55));
        assert_eq!(answered.len(), 1);
        // Fired before (the store says): only the next day's.
        let mut s = Scheduler::new([key].into_iter().collect());
        let fired = run(&mut s, &data, at(d(26), 9, 45), 24 * 60);
        assert_eq!(
            fired,
            [("invite".to_string(), 10), ("series".to_string(), 60)]
        );
    }
}

//! Recurring events expanded into occurrences, only for the range on screen.
//! Series have no end date in the data, so nothing is materialized ahead.

use chrono::{Datelike, Duration, NaiveDateTime, Weekday};

use super::{Event, Recurrence, time::add_months};

/// One appearance of an event on the calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    /// Index into `CalendarData::events`.
    pub event: usize,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

/// Occurrences of `events` that `include` lets through and that overlap
/// `[from, to)`, by start time, longer first on ties.
pub fn occurrences(
    events: &[Event],
    include: impl Fn(&Event) -> bool,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for (ix, event) in events.iter().enumerate() {
        if include(event) {
            let duration = event.duration();
            out.extend(
                starts_in(event, from, to)
                    .into_iter()
                    .map(|start| Occurrence {
                        event: ix,
                        start,
                        end: start + duration,
                    }),
            );
        }
    }
    out.sort_by_key(|o| (o.start, std::cmp::Reverse(o.end), o.event));
    out
}

/// Starts of the event's occurrences overlapping `[from, to)`. A series
/// skips its exceptions and stops at its end.
fn starts_in(event: &Event, from: NaiveDateTime, to: NaiveDateTime) -> Vec<NaiveDateTime> {
    let duration = event.duration().max(Duration::zero());
    let Some(recurrence) = event.recurrence else {
        let overlaps = |start: NaiveDateTime| {
            start < to && (start + duration > from || (duration.is_zero() && start >= from))
        };
        return [event.start].into_iter().filter(|&s| overlaps(s)).collect();
    };
    let to = event.until.map_or(to, |until| to.min(until));
    // Zero-length events still show at their start.
    let overlaps = |start: NaiveDateTime| {
        start < to
            && (start + duration > from || (duration.is_zero() && start >= from))
            && !event.exceptions.contains(&start)
    };
    // The earliest start whose occurrence could still reach `from`.
    let earliest = from - duration;
    let mut out = Vec::new();
    match recurrence {
        Recurrence::Daily | Recurrence::Weekdays | Recurrence::Weekly | Recurrence::Biweekly => {
            let step = match recurrence {
                Recurrence::Weekly => 7,
                Recurrence::Biweekly => 14,
                _ => 1,
            };
            let skip = ((earliest - event.start).num_days() / step).max(0);
            let mut start = event.start + Duration::days(skip * step);
            while start < to {
                let weekend = matches!(start.weekday(), Weekday::Sat | Weekday::Sun);
                if overlaps(start) && !(recurrence == Recurrence::Weekdays && weekend) {
                    out.push(start);
                }
                start += Duration::days(step);
            }
        }
        Recurrence::Monthly | Recurrence::Yearly => {
            let step = if recurrence == Recurrence::Yearly {
                12
            } else {
                1
            };
            let months_between = |a: NaiveDateTime, b: NaiveDateTime| {
                (b.year() - a.year()) * 12 + b.month() as i32 - a.month() as i32
            };
            let mut n = (months_between(event.start, earliest) / step - 1).max(0) * step;
            loop {
                // Months without the series' day (the 31st, Feb 29) are skipped.
                let date = add_months(event.start.date(), n);
                let start = date.and_time(event.start.time());
                if start >= to {
                    break;
                }
                if date.day() == event.start.day() && overlaps(start) {
                    out.push(start);
                }
                n += step;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::{calendar::time::at, data::Contact};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn event(start: NaiveDateTime, minutes: i64, recurrence: Option<Recurrence>) -> Event {
        Event {
            id: "e".into(),
            calendar: "work".into(),
            title: "Test".into(),
            start,
            end: start + Duration::minutes(minutes),
            all_day: false,
            location: String::new(),
            description: String::new(),
            attendees: Vec::new(),
            organizer: Contact {
                name: String::new(),
                email: String::new(),
            },
            recurrence,
            reminder_minutes: None,
            video_link: String::new(),
            busy: true,
            exceptions: Vec::new(),
            until: None,
            recurring_event_id: None,
            original_start: None,
        }
    }

    #[test]
    fn exceptions_and_end_are_skipped() {
        let mut e = event(at(d(2026, 10, 26), 9, 30), 15, Some(Recurrence::Weekdays));
        e.exceptions.push(at(d(2026, 10, 28), 9, 30));
        e.until = Some(at(d(2026, 10, 30), 9, 30));
        assert_eq!(
            starts(&e, d(2026, 10, 26), 7),
            vec![
                at(d(2026, 10, 26), 9, 30),
                at(d(2026, 10, 27), 9, 30),
                at(d(2026, 10, 29), 9, 30),
            ]
        );
        // Monthly too.
        let mut e = event(at(d(2026, 1, 15), 12, 0), 60, Some(Recurrence::Monthly));
        e.exceptions.push(at(d(2026, 3, 15), 12, 0));
        e.until = Some(at(d(2026, 6, 1), 0, 0));
        assert_eq!(starts(&e, d(2026, 1, 1), 365).len(), 4);
    }

    fn starts(event: &Event, from: NaiveDate, days: i64) -> Vec<NaiveDateTime> {
        let from = at(from, 0, 0);
        occurrences(
            std::slice::from_ref(event),
            |_| true,
            from,
            from + Duration::days(days),
        )
        .into_iter()
        .map(|o| o.start)
        .collect()
    }

    #[test]
    fn single_event_only_when_overlapping() {
        let e = event(at(d(2026, 10, 26), 9, 0), 60, None);
        assert_eq!(starts(&e, d(2026, 10, 26), 1), vec![e.start]);
        assert!(starts(&e, d(2026, 10, 27), 7).is_empty());
        assert!(starts(&e, d(2026, 10, 19), 7).is_empty());
    }

    #[test]
    fn weekly_and_biweekly() {
        let e = event(at(d(2026, 9, 28), 6, 30), 60, Some(Recurrence::Weekly));
        assert_eq!(
            starts(&e, d(2026, 10, 26), 7),
            vec![at(d(2026, 10, 26), 6, 30)]
        );
        // Nothing before the series starts.
        assert!(starts(&e, d(2026, 9, 21), 7).is_empty());
        // Far ahead, without walking every week in between.
        assert_eq!(starts(&e, d(2030, 1, 7), 7), vec![at(d(2030, 1, 7), 6, 30)]);

        let e = event(at(d(2026, 9, 28), 10, 0), 30, Some(Recurrence::Biweekly));
        assert_eq!(
            starts(&e, d(2026, 10, 1), 31),
            vec![at(d(2026, 10, 12), 10, 0), at(d(2026, 10, 26), 10, 0)]
        );
    }

    #[test]
    fn weekdays_skip_weekends() {
        let e = event(at(d(2026, 9, 28), 9, 30), 15, Some(Recurrence::Weekdays));
        let week = starts(&e, d(2026, 10, 26), 7);
        assert_eq!(week.len(), 5);
        assert!(week.iter().all(|s| s.weekday().num_days_from_monday() < 5));
    }

    #[test]
    fn daily_includes_one_that_started_the_day_before() {
        // 23:00 to 01:00 reaches into the next day.
        let e = event(at(d(2026, 10, 1), 23, 0), 120, Some(Recurrence::Daily));
        assert_eq!(
            starts(&e, d(2026, 10, 26), 1),
            vec![at(d(2026, 10, 25), 23, 0), at(d(2026, 10, 26), 23, 0)]
        );
    }

    #[test]
    fn monthly_skips_short_months() {
        let e = event(at(d(2026, 1, 31), 12, 0), 60, Some(Recurrence::Monthly));
        let year = starts(&e, d(2026, 1, 1), 365);
        assert_eq!(year.len(), 7);
        assert!(year.iter().all(|s| s.day() == 31));
    }

    #[test]
    fn yearly() {
        let e = event(at(d(2020, 11, 12), 0, 0), 24 * 60, Some(Recurrence::Yearly));
        assert_eq!(
            starts(&e, d(2026, 11, 1), 30),
            vec![at(d(2026, 11, 12), 0, 0)]
        );
        assert!(starts(&e, d(2026, 12, 1), 30).is_empty());
    }

    #[test]
    fn sorted_by_start_then_longer_first() {
        let a = event(at(d(2026, 10, 26), 9, 0), 30, None);
        let b = event(at(d(2026, 10, 26), 9, 0), 90, None);
        let c = event(at(d(2026, 10, 26), 8, 0), 30, None);
        let from = at(d(2026, 10, 26), 0, 0);
        let order: Vec<usize> = occurrences(&[a, b, c], |_| true, from, from + Duration::days(1))
            .iter()
            .map(|o| o.event)
            .collect();
        assert_eq!(order, vec![2, 1, 0]);
    }
}

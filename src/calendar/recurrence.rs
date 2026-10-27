//! Repeat rules, and recurring events expanded into occurrences, only for
//! the range on screen. Nothing is materialized ahead.
//!
//! `Rule` follows RFC 5545's RRULE, as Google stores it. Expansion covers
//! FREQ, INTERVAL, BYDAY without ordinals and UNTIL; ordinals, BYMONTHDAY
//! and COUNT are kept but not applied yet (CAL-5).

use chrono::{Datelike, Duration, NaiveDateTime, Weekday};
use serde::{Deserialize, Deserializer, Serialize};

use super::{Event, time::add_months};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// A repeat rule: an RRULE.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    /// Weekdays, each with an optional ordinal ("2TU", "-1FR").
    pub by_day: Vec<(Option<i8>, Weekday)>,
    pub by_month_day: Vec<i8>,
    pub count: Option<u32>,
    /// The series' end: no occurrence starts at or after it.
    pub until: Option<NaiveDateTime>,
}

const WEEKDAYS: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];

impl Rule {
    pub fn new(freq: Freq, interval: u32) -> Self {
        Self {
            freq,
            interval,
            by_day: Vec::new(),
            by_month_day: Vec::new(),
            count: None,
            until: None,
        }
    }

    pub fn daily() -> Self {
        Self::new(Freq::Daily, 1)
    }

    pub fn weekdays() -> Self {
        Self {
            by_day: WEEKDAYS.iter().map(|&d| (None, d)).collect(),
            ..Self::new(Freq::Weekly, 1)
        }
    }

    pub fn weekly() -> Self {
        Self::new(Freq::Weekly, 1)
    }

    pub fn biweekly() -> Self {
        Self::new(Freq::Weekly, 2)
    }

    pub fn monthly() -> Self {
        Self::new(Freq::Monthly, 1)
    }

    pub fn yearly() -> Self {
        Self::new(Freq::Yearly, 1)
    }

    /// The placeholder data's keywords.
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Some(match keyword {
            "daily" => Self::daily(),
            "weekdays" => Self::weekdays(),
            "weekly" => Self::weekly(),
            "biweekly" => Self::biweekly(),
            "monthly" => Self::monthly(),
            "yearly" => Self::yearly(),
            _ => return None,
        })
    }

    fn is_weekdays(&self) -> bool {
        self.by_day.len() == 5
            && WEEKDAYS
                .iter()
                .all(|&d| self.by_day.iter().any(|&(n, w)| n.is_none() && w == d))
    }

    /// "Every weekday", "Every 2 weeks", "Every week on Mon, Wed".
    pub fn label(&self) -> String {
        let n = self.interval.max(1);
        let unit = match self.freq {
            Freq::Daily => "day",
            Freq::Weekly => "week",
            Freq::Monthly => "month",
            Freq::Yearly => "year",
        };
        if self.freq == Freq::Weekly && n == 1 && self.is_weekdays() {
            return "Every weekday".into();
        }
        let every = if n == 1 {
            format!("Every {unit}")
        } else {
            format!("Every {n} {unit}s")
        };
        if self.freq == Freq::Weekly && self.by_day.len() > 1 {
            let days: Vec<String> = self.by_day.iter().map(|(_, d)| format!("{d:?}")).collect();
            return format!("{every} on {}", days.join(", "));
        }
        every
    }
}

/// An event's `recurrence`: a keyword in the placeholder data, a `Rule`
/// in the store.
pub(super) fn rule_or_keyword<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Rule>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Keyword(String),
        Rule(Rule),
    }
    match Option::<Repr>::deserialize(d)? {
        None => Ok(None),
        Some(Repr::Rule(rule)) => Ok(Some(rule)),
        Some(Repr::Keyword(k)) => Rule::from_keyword(&k)
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown recurrence {k}"))),
    }
}

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
    let Some(rule) = &event.recurrence else {
        let overlaps = |start: NaiveDateTime| {
            start < to && (start + duration > from || (duration.is_zero() && start >= from))
        };
        return [event.start].into_iter().filter(|&s| overlaps(s)).collect();
    };
    let to = rule.until.map_or(to, |until| to.min(until));
    // Zero-length events still show at their start.
    let overlaps = |start: NaiveDateTime| {
        start < to
            && (start + duration > from || (duration.is_zero() && start >= from))
            && !event.exceptions.contains(&start)
    };
    // The earliest start whose occurrence could still reach `from`.
    let earliest = from - duration;
    let interval = rule.interval.max(1) as i64;
    let days: Vec<Weekday> = rule.by_day.iter().map(|&(_, d)| d).collect();
    let on_day = |start: NaiveDateTime| days.is_empty() || days.contains(&start.weekday());
    let mut out = Vec::new();
    match rule.freq {
        Freq::Weekly if !days.is_empty() => {
            // Week by week from the series' first week (weeks start Monday),
            // each day the rule names.
            let first = event.start.date();
            let week0 = first - Duration::days(first.weekday().num_days_from_monday() as i64);
            let skip = ((earliest.date() - week0).num_days() / (7 * interval)).max(0);
            let mut week = week0 + Duration::days(skip * 7 * interval);
            while week.and_time(event.start.time()) < to {
                for offset in 0..7 {
                    let date = week + Duration::days(offset);
                    let start = date.and_time(event.start.time());
                    if date >= first && on_day(start) && overlaps(start) {
                        out.push(start);
                    }
                }
                week += Duration::days(7 * interval);
            }
        }
        Freq::Daily | Freq::Weekly => {
            let step = if rule.freq == Freq::Weekly { 7 } else { 1 } * interval;
            let skip = ((earliest - event.start).num_days() / step).max(0);
            let mut start = event.start + Duration::days(skip * step);
            while start < to {
                if overlaps(start) && on_day(start) {
                    out.push(start);
                }
                start += Duration::days(step);
            }
        }
        Freq::Monthly | Freq::Yearly => {
            let step = interval as i32 * if rule.freq == Freq::Yearly { 12 } else { 1 };
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

    fn event(start: NaiveDateTime, minutes: i64, recurrence: Option<Rule>) -> Event {
        Event {
            id: "e".into(),
            calendar: "work".into(),
            title: "Test".into(),
            start,
            end: start + Duration::minutes(minutes),
            organizer: Contact::default(),
            recurrence,
            busy: true,
            ..Event::default()
        }
    }

    #[test]
    fn exceptions_and_end_are_skipped() {
        let mut e = event(at(d(2026, 10, 26), 9, 30), 15, Some(Rule::weekdays()));
        e.exceptions.push(at(d(2026, 10, 28), 9, 30));
        e.recurrence.as_mut().unwrap().until = Some(at(d(2026, 10, 30), 9, 30));
        assert_eq!(
            starts(&e, d(2026, 10, 26), 7),
            vec![
                at(d(2026, 10, 26), 9, 30),
                at(d(2026, 10, 27), 9, 30),
                at(d(2026, 10, 29), 9, 30),
            ]
        );
        // Monthly too.
        let mut e = event(at(d(2026, 1, 15), 12, 0), 60, Some(Rule::monthly()));
        e.exceptions.push(at(d(2026, 3, 15), 12, 0));
        e.recurrence.as_mut().unwrap().until = Some(at(d(2026, 6, 1), 0, 0));
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
        let e = event(at(d(2026, 9, 28), 6, 30), 60, Some(Rule::weekly()));
        assert_eq!(
            starts(&e, d(2026, 10, 26), 7),
            vec![at(d(2026, 10, 26), 6, 30)]
        );
        // Nothing before the series starts.
        assert!(starts(&e, d(2026, 9, 21), 7).is_empty());
        // Far ahead, without walking every week in between.
        assert_eq!(starts(&e, d(2030, 1, 7), 7), vec![at(d(2030, 1, 7), 6, 30)]);

        let e = event(at(d(2026, 9, 28), 10, 0), 30, Some(Rule::biweekly()));
        assert_eq!(
            starts(&e, d(2026, 10, 1), 31),
            vec![at(d(2026, 10, 12), 10, 0), at(d(2026, 10, 26), 10, 0)]
        );
    }

    #[test]
    fn weekdays_skip_weekends() {
        let e = event(at(d(2026, 9, 28), 9, 30), 15, Some(Rule::weekdays()));
        let week = starts(&e, d(2026, 10, 26), 7);
        assert_eq!(week.len(), 5);
        assert!(week.iter().all(|s| s.weekday().num_days_from_monday() < 5));
    }

    #[test]
    fn daily_includes_one_that_started_the_day_before() {
        // 23:00 to 01:00 reaches into the next day.
        let e = event(at(d(2026, 10, 1), 23, 0), 120, Some(Rule::daily()));
        assert_eq!(
            starts(&e, d(2026, 10, 26), 1),
            vec![at(d(2026, 10, 25), 23, 0), at(d(2026, 10, 26), 23, 0)]
        );
    }

    #[test]
    fn monthly_skips_short_months() {
        let e = event(at(d(2026, 1, 31), 12, 0), 60, Some(Rule::monthly()));
        let year = starts(&e, d(2026, 1, 1), 365);
        assert_eq!(year.len(), 7);
        assert!(year.iter().all(|s| s.day() == 31));
    }

    #[test]
    fn yearly() {
        let e = event(at(d(2020, 11, 12), 0, 0), 24 * 60, Some(Rule::yearly()));
        assert_eq!(
            starts(&e, d(2026, 11, 1), 30),
            vec![at(d(2026, 11, 12), 0, 0)]
        );
        assert!(starts(&e, d(2026, 12, 1), 30).is_empty());
    }

    #[test]
    fn keyword_labels() {
        let labels: Vec<String> = [
            "daily", "weekdays", "weekly", "biweekly", "monthly", "yearly",
        ]
        .iter()
        .map(|k| Rule::from_keyword(k).unwrap().label())
        .collect();
        assert_eq!(
            labels,
            [
                "Every day",
                "Every weekday",
                "Every week",
                "Every 2 weeks",
                "Every month",
                "Every year"
            ]
        );
        assert!(Rule::from_keyword("hourly").is_none());
        let mut rule = Rule::weekly();
        rule.by_day = vec![(None, Weekday::Mon), (None, Weekday::Wed)];
        assert_eq!(rule.label(), "Every week on Mon, Wed");
    }

    #[test]
    fn weekly_on_several_days() {
        let mut rule = Rule::biweekly();
        rule.by_day = vec![(None, Weekday::Mon), (None, Weekday::Wed)];
        // Wednesday Oct 28, 2026 starts it.
        let e = event(at(d(2026, 10, 28), 10, 0), 30, Some(rule));
        assert_eq!(
            starts(&e, d(2026, 10, 26), 21),
            vec![
                at(d(2026, 10, 28), 10, 0),
                at(d(2026, 11, 9), 10, 0),
                at(d(2026, 11, 11), 10, 0),
            ]
        );
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

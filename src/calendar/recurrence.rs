//! Repeat rules, and recurring events expanded into occurrences, only for
//! the range on screen. Nothing is materialized ahead.
//!
//! `Rule` follows RFC 5545's RRULE, as Google stores it. Supported: FREQ
//! (daily to yearly), INTERVAL, BYDAY (with ordinals for monthly rules),
//! BYMONTHDAY, COUNT and UNTIL, plus EXDATE (`parse_recurrence`). Anything
//! else marks the rule `unsupported`: it isn't expanded, and the backend
//! stores the series' occurrences from Google instead (CAL-5).

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Weekday};
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    Event,
    time::{LocalZone, add_months, midnight, monday_of as week_start, month_start},
};

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
    /// The series' end: no occurrence starts at or after it. (RRULE's
    /// UNTIL is inclusive; `parse` and `to_rrule` convert.)
    pub until: Option<NaiveDateTime>,
    /// Not expanded here (the rule is outside the subset, or the series
    /// needs a time zone database); Google's own occurrences are stored
    /// instead.
    #[serde(default)]
    pub unsupported: bool,
    /// The RRULE has parts this type doesn't hold; `raw` says it.
    #[serde(default)]
    pub custom: bool,
    /// A custom rule's RRULE value as Google wrote it, for `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
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
            unsupported: false,
            custom: false,
            raw: None,
        }
    }

    /// Repeats the same way, whatever its end.
    pub fn same_pattern(&self, other: &Rule) -> bool {
        (
            self.freq,
            self.interval.max(1),
            &self.by_day,
            &self.by_month_day,
            self.custom,
            &self.raw,
        ) == (
            other.freq,
            other.interval.max(1),
            &other.by_day,
            &other.by_month_day,
            other.custom,
            &other.raw,
        )
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

    /// The rule in words, with its end: "Every weekday", "Every 2 weeks",
    /// "Every week on Mon, Wed", "Every month on the last Fri, 10 times",
    /// "Every day, until Dec 31, 2026". Custom rules from Google are said
    /// from their RRULE.
    pub fn label(&self) -> String {
        let pattern = if self.custom {
            self.raw
                .as_deref()
                .map_or_else(|| "Custom repeat".into(), describe_rrule)
        } else {
            self.pattern_label()
        };
        match (self.count, self.until) {
            (Some(1), _) => format!("{pattern}, once"),
            (Some(n), _) => format!("{pattern}, {n} times"),
            (None, Some(until)) => format!(
                "{pattern}, until {}",
                (until - Duration::seconds(1)).format("%b %-d, %Y")
            ),
            (None, None) => pattern,
        }
    }

    fn pattern_label(&self) -> String {
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
        if self.freq == Freq::Weekly && !self.by_day.is_empty() {
            let mut days: Vec<Weekday> = self.by_day.iter().map(|&(_, d)| d).collect();
            days.sort_by_key(|d| d.num_days_from_monday());
            let days: Vec<String> = days.iter().map(|d| format!("{d:?}")).collect();
            return format!("{every} on {}", days.join(", "));
        }
        if self.freq == Freq::Monthly
            && let [(Some(n), day)] = self.by_day[..]
        {
            return format!("{every} on the {} {day:?}", ordinal(n as i32));
        }
        if self.freq == Freq::Monthly && !self.by_month_day.is_empty() {
            let days: Vec<String> = self
                .by_month_day
                .iter()
                .map(|&d| match d {
                    -1 => "the last day".to_string(),
                    d if d < 0 => format!("the {} day", ordinal(d as i32)),
                    d => format!("day {d}"),
                })
                .collect();
            return format!("{every} on {}", days.join(", "));
        }
        every
    }

    /// An RRULE's value (after "RRULE:"). `all_day` says whether UNTIL is a
    /// date; a UTC UNTIL is converted to `zone`'s wall clock.
    pub fn parse(value: &str, all_day: bool, zone: &LocalZone) -> Option<Rule> {
        let mut rule = Rule::new(Freq::Daily, 1);
        let mut freq = None;
        let mut week_start = Weekday::Mon;
        for part in value.split(';').filter(|p| !p.is_empty()) {
            let (key, val) = part.split_once('=')?;
            match key.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match val {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => {
                            rule.unsupported = true;
                            Freq::Daily
                        }
                    })
                }
                "INTERVAL" => rule.interval = val.parse().ok().filter(|&n| n > 0)?,
                "COUNT" => rule.count = Some(val.parse().ok()?),
                "UNTIL" => {
                    let until = parse_ical_time(val, zone)?;
                    // Inclusive in RRULE; exclusive here.
                    rule.until = Some(if val.len() == 8 || all_day {
                        midnight(until.date() + Duration::days(1))
                    } else {
                        until + Duration::seconds(1)
                    });
                }
                "BYDAY" => {
                    for day in val.split(',') {
                        rule.by_day.push(parse_by_day(day)?);
                    }
                }
                "BYMONTHDAY" => {
                    for day in val.split(',') {
                        let day: i8 = day.parse().ok()?;
                        if day == 0 || day.abs() > 31 {
                            return None;
                        }
                        rule.by_month_day.push(day);
                    }
                }
                "WKST" => week_start = parse_weekday(val)?,
                _ => rule.unsupported = true,
            }
        }
        rule.freq = freq?;
        let ordinals = rule.by_day.iter().any(|(n, _)| n.is_some());
        rule.unsupported |= match rule.freq {
            Freq::Daily => ordinals || !rule.by_month_day.is_empty(),
            // The week start only moves occurrences when weeks are skipped
            // and the rule names several days.
            Freq::Weekly => {
                ordinals
                    || !rule.by_month_day.is_empty()
                    || (week_start != Weekday::Mon && rule.interval > 1 && rule.by_day.len() > 1)
            }
            Freq::Monthly => !rule.by_day.is_empty() && !rule.by_month_day.is_empty(),
            Freq::Yearly => !rule.by_day.is_empty() || !rule.by_month_day.is_empty(),
        };
        rule.custom = rule.unsupported;
        if rule.custom {
            rule.raw = Some(value.to_string());
        }
        Some(rule)
    }

    /// The RRULE line for Google ("RRULE:FREQ=WEEKLY;BYDAY=MO,WE").
    #[allow(dead_code)] // Calendar writes (W7).
    pub fn to_rrule(&self, all_day: bool, zone: &LocalZone) -> String {
        let freq = match self.freq {
            Freq::Daily => "DAILY",
            Freq::Weekly => "WEEKLY",
            Freq::Monthly => "MONTHLY",
            Freq::Yearly => "YEARLY",
        };
        let mut parts = vec![format!("FREQ={freq}")];
        if self.interval > 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if !self.by_day.is_empty() {
            let days: Vec<String> = self
                .by_day
                .iter()
                .map(|&(n, d)| {
                    let code = &format!("{d:?}")[..2].to_ascii_uppercase();
                    n.map_or_else(|| code.clone(), |n| format!("{n}{code}"))
                })
                .collect();
            parts.push(format!("BYDAY={}", days.join(",")));
        }
        if !self.by_month_day.is_empty() {
            let days: Vec<String> = self.by_month_day.iter().map(|d| d.to_string()).collect();
            parts.push(format!("BYMONTHDAY={}", days.join(",")));
        }
        if let Some(count) = self.count {
            parts.push(format!("COUNT={count}"));
        }
        if let Some(until) = self.until {
            let last = until - Duration::seconds(1);
            parts.push(if all_day {
                format!("UNTIL={}", last.format("%Y%m%d"))
            } else {
                format!("UNTIL={}", (zone.to_utc)(last).format("%Y%m%dT%H%M%SZ"))
            });
        }
        format!("RRULE:{}", parts.join(";"))
    }

    /// Which period (day, week, month or year, counted from the series'
    /// first) `date` falls in.
    fn period_of(&self, first: NaiveDate, date: NaiveDate) -> i64 {
        let months = |a: NaiveDate, b: NaiveDate| {
            ((b.year() - a.year()) * 12 + b.month() as i32 - a.month() as i32) as i64
        };
        match self.freq {
            Freq::Daily => (date - first).num_days(),
            Freq::Weekly => (week_start(date) - week_start(first))
                .num_days()
                .div_euclid(7),
            Freq::Monthly => months(first, date),
            Freq::Yearly => months(first, date).div_euclid(12),
        }
    }

    /// Period `k`'s first day and the days in it the rule names, in order.
    fn period(&self, first: NaiveDate, k: i64) -> (NaiveDate, Vec<NaiveDate>) {
        let on_day = |date: &NaiveDate| {
            self.by_day.is_empty() || self.by_day.iter().any(|&(_, d)| d == date.weekday())
        };
        match self.freq {
            Freq::Daily => {
                let day = first + Duration::days(k);
                (day, Some(day).filter(on_day).into_iter().collect())
            }
            Freq::Weekly => {
                let week = week_start(first) + Duration::days(7 * k);
                let mut days: Vec<NaiveDate> = if self.by_day.is_empty() {
                    vec![first.weekday()]
                } else {
                    self.by_day.iter().map(|&(_, d)| d).collect()
                }
                .into_iter()
                .map(|d| week + Duration::days(d.num_days_from_monday() as i64))
                .collect();
                days.sort();
                days.dedup();
                (week, days)
            }
            Freq::Monthly => {
                let month = add_months(month_start(first), k as i32);
                let last = add_months(month, 1) - Duration::days(1);
                let mut days: Vec<NaiveDate> = if !self.by_month_day.is_empty() {
                    self.by_month_day
                        .iter()
                        .filter_map(|&d| {
                            let day = if d > 0 {
                                d as i64
                            } else {
                                last.day() as i64 + d as i64 + 1
                            };
                            (day >= 1).then(|| month.with_day(day as u32)).flatten()
                        })
                        .collect()
                } else if !self.by_day.is_empty() {
                    self.by_day
                        .iter()
                        .flat_map(|&(n, wd)| {
                            let all: Vec<NaiveDate> = month
                                .iter_days()
                                .take_while(|d| *d <= last)
                                .filter(|d| d.weekday() == wd)
                                .collect();
                            match n {
                                None => all,
                                Some(n) if n > 0 => {
                                    all.get(n as usize - 1).copied().into_iter().collect()
                                }
                                Some(n) => all
                                    .len()
                                    .checked_sub(n.unsigned_abs() as usize)
                                    .and_then(|ix| all.get(ix).copied())
                                    .into_iter()
                                    .collect(),
                            }
                        })
                        .collect()
                } else {
                    // Months without the series' day (the 31st) are skipped.
                    month.with_day(first.day()).into_iter().collect()
                };
                days.sort();
                days.dedup();
                (month, days)
            }
            Freq::Yearly => {
                let month = add_months(month_start(first), 12 * k as i32);
                // Years without Feb 29 are skipped.
                (month, month.with_day(first.day()).into_iter().collect())
            }
        }
    }
}

/// "1st", "2nd", "last", "2nd to last".
fn ordinal(n: i32) -> String {
    let suffix = |n: i32| match (n % 10, n % 100) {
        (1, x) if x != 11 => "st",
        (2, x) if x != 12 => "nd",
        (3, x) if x != 13 => "rd",
        _ => "th",
    };
    match n {
        -1 => "last".into(),
        n if n < 0 => format!("{}{} to last", -n, suffix(-n)),
        n => format!("{n}{}", suffix(n)),
    }
}

/// Any RRULE value in words, for rules Pigeon can't edit: "Every year in
/// Nov on the 4th Thu", "Every month on the last weekday".
fn describe_rrule(value: &str) -> String {
    let mut freq = "";
    let mut interval = 1u32;
    let (mut days, mut months, mut month_days, mut set_pos) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut other = false;
    for part in value.split(';') {
        let Some((key, val)) = part.split_once('=') else {
            continue;
        };
        let list = || val.split(',').map(str::trim);
        match key.to_ascii_uppercase().as_str() {
            "FREQ" => freq = val,
            "INTERVAL" => interval = val.parse().unwrap_or(1),
            "BYDAY" => days = list().filter_map(parse_by_day).collect(),
            "BYMONTH" => {
                months = list()
                    .filter_map(|m| m.parse::<u32>().ok())
                    .filter_map(|m| NaiveDate::from_ymd_opt(2000, m, 1))
                    .map(|d| d.format("%b").to_string())
                    .collect()
            }
            "BYMONTHDAY" => month_days = list().filter_map(|d| d.parse::<i32>().ok()).collect(),
            "BYSETPOS" => set_pos = list().filter_map(|d| d.parse::<i32>().ok()).collect(),
            "COUNT" | "UNTIL" | "WKST" => {}
            _ => other = true,
        }
    }
    let unit = match freq {
        "SECONDLY" => "second",
        "MINUTELY" => "minute",
        "HOURLY" => "hour",
        "DAILY" => "day",
        "WEEKLY" => "week",
        "MONTHLY" => "month",
        "YEARLY" => "year",
        _ => return "Custom repeat".into(),
    };
    let mut out = if interval > 1 {
        format!("Every {interval} {unit}s")
    } else {
        format!("Every {unit}")
    };
    if !months.is_empty() {
        out.push_str(&format!(" in {}", months.join(", ")));
    }
    let weekdays = days.len() == 5
        && WEEKDAYS
            .iter()
            .all(|w| days.iter().any(|&(n, d)| n.is_none() && d == *w));
    match (set_pos.as_slice(), days.is_empty()) {
        ([pos], false) => {
            let what = if weekdays {
                "weekday".to_string()
            } else {
                let names: Vec<String> = days.iter().map(|(_, d)| format!("{d:?}")).collect();
                names.join(" or ")
            };
            out.push_str(&format!(" on the {} {what}", ordinal(*pos)));
        }
        (_, false) => {
            let names: Vec<String> = days
                .iter()
                .map(|&(n, d)| match n {
                    Some(n) => format!("the {} {d:?}", ordinal(n as i32)),
                    None => format!("{d:?}"),
                })
                .collect();
            out.push_str(&format!(" on {}", names.join(", ")));
        }
        _ => {}
    }
    if !month_days.is_empty() {
        let names: Vec<String> = month_days
            .iter()
            .map(|&d| {
                if d < 0 {
                    format!("the {} day", ordinal(d))
                } else {
                    format!("day {d}")
                }
            })
            .collect();
        out.push_str(&format!(" on {}", names.join(", ")));
    }
    if other {
        out.push_str(" (custom)");
    }
    out
}

fn parse_weekday(code: &str) -> Option<Weekday> {
    Some(match code {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// "MO", "2TU", "-1FR", "+1MO".
fn parse_by_day(text: &str) -> Option<(Option<i8>, Weekday)> {
    let split = text.len().checked_sub(2)?;
    let (ordinal, day) = text.split_at(split);
    let day = parse_weekday(day)?;
    if ordinal.is_empty() {
        return Some((None, day));
    }
    let n: i8 = ordinal.trim_start_matches('+').parse().ok()?;
    (n != 0 && n.abs() <= 5).then_some((Some(n), day))
}

/// "20261028" (midnight), "20261028T100000" (wall time) or
/// "20261028T170000Z" (UTC, to `zone`'s wall time).
fn parse_ical_time(text: &str, zone: &LocalZone) -> Option<NaiveDateTime> {
    if let Some(utc) = text.strip_suffix('Z') {
        let t = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some((zone.to_local)(t.and_utc()));
    }
    if text.len() == 8 {
        return NaiveDate::parse_from_str(text, "%Y%m%d").ok().map(midnight);
    }
    NaiveDateTime::parse_from_str(text, "%Y%m%dT%H%M%S").ok()
}

/// A Google event's `recurrence` lines, read: the rule, and the EXDATEs
/// as wall times in `zone`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recurrence {
    pub rule: Option<Rule>,
    pub exdates: Vec<NaiveDateTime>,
}

/// Reads RRULE and EXDATE lines. The rule comes back `unsupported` when
/// it's outside the subset, or the lines hold something only Google can
/// expand: RDATE, EXRULE, several RRULEs, or EXDATEs in another zone.
pub fn parse_recurrence(lines: &[String], all_day: bool, zone: &LocalZone) -> Recurrence {
    let mut out = Recurrence::default();
    let mut unsupported = false;
    for line in lines {
        let Some((head, value)) = line.split_once(':') else {
            unsupported = true;
            continue;
        };
        let mut params = head.split(';');
        match params
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase()
            .as_str()
        {
            "RRULE" if out.rule.is_none() => match Rule::parse(value, all_day, zone) {
                Some(rule) => out.rule = Some(rule),
                None => {
                    out.rule = Some(Rule {
                        custom: true,
                        raw: Some(value.to_string()),
                        ..Rule::daily()
                    });
                    unsupported = true;
                }
            },
            "EXDATE" => {
                let foreign_zone = params.any(|p| {
                    p.strip_prefix("TZID=")
                        .is_some_and(|tz| !tz.eq_ignore_ascii_case(zone.name))
                });
                if foreign_zone {
                    unsupported = true;
                    continue;
                }
                for text in value.split(',') {
                    match parse_ical_time(text.trim(), zone) {
                        Some(t) => out.exdates.push(t),
                        None => unsupported = true,
                    }
                }
            }
            _ => unsupported = true,
        }
    }
    if unsupported {
        let rule = out.rule.get_or_insert_with(Rule::daily);
        rule.unsupported = true;
    }
    out
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
/// skips its exceptions and stops at its end or count. Unsupported rules
/// give none: their occurrences are events of their own.
fn starts_in(event: &Event, from: NaiveDateTime, to: NaiveDateTime) -> Vec<NaiveDateTime> {
    let duration = event.duration().max(Duration::zero());
    // Zero-length events still show at their start.
    let overlaps = |start: NaiveDateTime| {
        start < to && (start + duration > from || (duration.is_zero() && start >= from))
    };
    let Some(rule) = &event.recurrence else {
        return [event.start].into_iter().filter(|&s| overlaps(s)).collect();
    };
    if rule.unsupported {
        return Vec::new();
    }
    let end = rule.until.map_or(to, |until| to.min(until));
    let first = event.start.date();
    let time = event.start.time();
    let interval = rule.interval.max(1) as i64;
    // COUNT counts from the first occurrence; otherwise skip to the period
    // whose occurrences could still reach `from`.
    let mut period = match rule.count {
        Some(_) => 0,
        None => {
            let earliest = (from - duration).date();
            (rule.period_of(first, earliest) / interval).max(0) * interval
        }
    };
    let mut seen = 0;
    let mut out = Vec::new();
    'periods: loop {
        let (period_start, days) = rule.period(first, period);
        if period_start.and_time(time) >= end {
            break;
        }
        for date in days {
            if date < first {
                continue;
            }
            let start = date.and_time(time);
            if start >= end {
                break 'periods;
            }
            seen += 1;
            if rule.count.is_some_and(|count| seen > count) {
                break 'periods;
            }
            if overlaps(start) && !event.exceptions.contains(&start) {
                out.push(start);
            }
        }
        period += interval;
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

    /// UTC−7 all year, so tests don't depend on the machine's zone.
    fn zone() -> LocalZone {
        LocalZone {
            name: "America/Los_Angeles",
            to_local: |t| t.naive_utc() - Duration::hours(7),
            to_utc: |t| (t + Duration::hours(7)).and_utc(),
        }
    }

    fn series(start: NaiveDateTime, lines: &[&str]) -> Event {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        let parsed = parse_recurrence(&lines, false, &zone());
        let mut e = event(start, 60, parsed.rule);
        e.exceptions = parsed.exdates;
        e
    }

    #[test]
    fn rrule_weekly_on_days() {
        // Mon Oct 26, 2026.
        let e = series(
            at(d(2026, 10, 26), 10, 0),
            &["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"],
        );
        assert_eq!(
            starts(&e, d(2026, 10, 26), 14),
            vec![
                at(d(2026, 10, 26), 10, 0),
                at(d(2026, 10, 28), 10, 0),
                at(d(2026, 11, 2), 10, 0),
                at(d(2026, 11, 4), 10, 0),
            ]
        );
    }

    #[test]
    fn rrule_monthly_ordinals() {
        // Second Tuesday: Oct 13, Nov 10, Dec 8.
        let e = series(at(d(2026, 10, 13), 9, 0), &["RRULE:FREQ=MONTHLY;BYDAY=2TU"]);
        assert_eq!(
            starts(&e, d(2026, 10, 1), 92),
            vec![
                at(d(2026, 10, 13), 9, 0),
                at(d(2026, 11, 10), 9, 0),
                at(d(2026, 12, 8), 9, 0),
            ]
        );
        assert_eq!(
            e.recurrence.as_ref().unwrap().label(),
            "Every month on the 2nd Tue"
        );
        // Last Friday: Oct 30, Nov 27, Dec 25.
        let e = series(
            at(d(2026, 10, 30), 16, 0),
            &["RRULE:FREQ=MONTHLY;BYDAY=-1FR"],
        );
        assert_eq!(
            starts(&e, d(2026, 10, 1), 92),
            vec![
                at(d(2026, 10, 30), 16, 0),
                at(d(2026, 11, 27), 16, 0),
                at(d(2026, 12, 25), 16, 0),
            ]
        );
        // The last day of each month.
        let e = series(
            at(d(2026, 1, 31), 8, 0),
            &["RRULE:FREQ=MONTHLY;BYMONTHDAY=-1"],
        );
        let days: Vec<u32> = starts(&e, d(2026, 1, 1), 90)
            .iter()
            .map(|s| s.day())
            .collect();
        assert_eq!(days, [31, 28, 31]);
    }

    #[test]
    fn rrule_count_until_interval_exdate() {
        // COUNT counts from the first, even when the range starts later.
        let e = series(
            at(d(2026, 10, 26), 9, 0),
            &["RRULE:FREQ=WEEKLY;WKST=MO;COUNT=3;BYDAY=MO,FR"],
        );
        assert_eq!(starts(&e, d(2026, 10, 26), 60).len(), 3);
        assert_eq!(
            starts(&e, d(2026, 11, 1), 60),
            vec![at(d(2026, 11, 2), 9, 0)]
        );
        // UNTIL is inclusive, in UTC: 06:59:59Z on Nov 4 is 23:59:59 on
        // Nov 3 here, so Tuesday the 3rd is the last.
        let e = series(
            at(d(2026, 10, 20), 19, 30),
            &["RRULE:FREQ=WEEKLY;UNTIL=20261104T065959Z;BYDAY=TU"],
        );
        assert_eq!(starts(&e, d(2026, 10, 1), 60).len(), 3);
        // An UNTIL exactly on an occurrence keeps it.
        let e = series(
            at(d(2026, 10, 20), 10, 0),
            &["RRULE:FREQ=DAILY;UNTIL=20261022T170000Z"],
        );
        assert_eq!(starts(&e, d(2026, 10, 1), 60).len(), 3);
        // Every other week, minus an EXDATE in the local zone.
        let e = series(
            at(d(2026, 10, 7), 15, 0),
            &[
                "EXDATE;TZID=America/Los_Angeles:20261021T150000",
                "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=WE",
            ],
        );
        assert_eq!(
            starts(&e, d(2026, 10, 1), 42),
            vec![at(d(2026, 10, 7), 15, 0), at(d(2026, 11, 4), 15, 0)]
        );
        // An all-day EXDATE.
        let lines = vec![
            "RRULE:FREQ=DAILY;COUNT=3".to_string(),
            "EXDATE;VALUE=DATE:20261027".to_string(),
        ];
        let parsed = parse_recurrence(&lines, true, &zone());
        assert_eq!(parsed.exdates, [at(d(2026, 10, 27), 0, 0)]);
    }

    #[test]
    fn unsupported_rules_are_flagged_and_not_expanded() {
        let flagged = |lines: &[&str]| {
            let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
            parse_recurrence(&lines, false, &zone())
                .rule
                .is_some_and(|r| r.unsupported)
        };
        assert!(flagged(&["RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=4TH"]));
        assert!(flagged(&[
            "RRULE:FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU,WE,TH,FR"
        ]));
        assert!(flagged(&["RRULE:FREQ=HOURLY"]));
        assert!(flagged(&["RRULE:FREQ=WEEKLY", "RDATE:20261101T100000Z"]));
        assert!(flagged(&[
            "EXDATE;TZID=Asia/Riyadh:20260429T074500",
            "RRULE:FREQ=WEEKLY;BYDAY=SU"
        ]));
        assert!(flagged(&[
            "RRULE:FREQ=WEEKLY;WKST=SU;INTERVAL=2;BYDAY=SU,MO"
        ]));
        assert!(!flagged(&["RRULE:FREQ=WEEKLY;WKST=SU;BYDAY=TU,WE"]));
        assert!(!flagged(&["RRULE:FREQ=YEARLY"]));
        let e = series(
            at(d(2026, 10, 26), 9, 0),
            &["RRULE:FREQ=YEARLY;BYWEEKNO=20"],
        );
        assert!(starts(&e, d(2026, 10, 1), 400).is_empty());
    }

    #[test]
    fn rrule_round_trips() {
        let z = zone();
        for (line, all_day) in [
            ("RRULE:FREQ=WEEKLY;BYDAY=MO,WE", false),
            ("RRULE:FREQ=MONTHLY;BYDAY=-1FR;COUNT=4", false),
            (
                "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=TU;UNTIL=20261104T065959Z",
                false,
            ),
            ("RRULE:FREQ=MONTHLY;BYMONTHDAY=15", false),
            ("RRULE:FREQ=YEARLY;UNTIL=20301231", true),
        ] {
            let rule = Rule::parse(line.strip_prefix("RRULE:").unwrap(), all_day, &z).unwrap();
            assert!(!rule.unsupported, "{line}");
            assert_eq!(rule.to_rrule(all_day, &z), line);
        }
    }

    #[test]
    fn rules_in_words() {
        let z = zone();
        let words = |value: &str| Rule::parse(value, false, &z).unwrap().label();
        assert_eq!(words("FREQ=WEEKLY;BYDAY=TU"), "Every week on Tue");
        assert_eq!(
            words("FREQ=MONTHLY;BYDAY=-1FR;COUNT=4"),
            "Every month on the last Fri, 4 times"
        );
        assert_eq!(
            words("FREQ=DAILY;UNTIL=20261231T075959Z"),
            "Every day, until Dec 31, 2026"
        );
        assert_eq!(
            words("FREQ=MONTHLY;BYMONTHDAY=15;INTERVAL=2"),
            "Every 2 months on day 15"
        );
        // Ones Pigeon can't edit, from their RRULE.
        assert_eq!(
            words("FREQ=YEARLY;BYMONTH=11;BYDAY=4TH"),
            "Every year in Nov on the 4th Thu"
        );
        assert_eq!(
            words("FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU,WE,TH,FR;COUNT=3"),
            "Every month on the last weekday, 3 times"
        );
        assert_eq!(words("FREQ=YEARLY;BYWEEKNO=20"), "Every year (custom)");
        assert_eq!(words("FREQ=HOURLY;INTERVAL=4"), "Every 4 hours");
        assert_eq!(ordinal(-2), "2nd to last");
        assert_eq!(ordinal(11), "11th");
    }
}

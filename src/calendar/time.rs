//! Date math, formatting and lenient parsing of typed dates and times.

use chrono::{
    DateTime, Datelike, Duration, Local, Months, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone as _, Timelike, Utc, Weekday,
};

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// The first day of the week shown, as days from Monday (Google's
/// `weekStart`, CAL-41), and whether times show on a 24-hour clock. Set
/// by the calendar from the settings; read by the formatting here.
static FIRST_WEEKDAY: AtomicU8 = AtomicU8::new(0);
static CLOCK_24H: AtomicBool = AtomicBool::new(false);

pub fn set_first_weekday(day: Weekday) {
    FIRST_WEEKDAY.store(day.num_days_from_monday() as u8, Ordering::Relaxed);
}

pub fn first_weekday() -> Weekday {
    Weekday::try_from(FIRST_WEEKDAY.load(Ordering::Relaxed)).unwrap_or(Weekday::Mon)
}

pub fn set_24_hour(on: bool) {
    CLOCK_24H.store(on, Ordering::Relaxed);
}

pub fn is_24_hour() -> bool {
    CLOCK_24H.load(Ordering::Relaxed)
}

/// Weeks start on the user's first day (Monday unless Google says
/// otherwise).
pub fn week_start(date: NaiveDate) -> NaiveDate {
    week_start_on(date, first_weekday())
}

pub fn week_start_on(date: NaiveDate, first: Weekday) -> NaiveDate {
    let back = (date.weekday().num_days_from_monday() + 7 - first.num_days_from_monday()) % 7;
    date - Duration::days(back as i64)
}

/// The Monday of `date`'s week: RRULE's default week start.
pub fn monday_of(date: NaiveDate) -> NaiveDate {
    week_start_on(date, Weekday::Mon)
}

/// The one-letter weekday heads of a week row, from the first day.
pub fn weekday_letters() -> [&'static str; 7] {
    const LETTERS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
    let first = first_weekday().num_days_from_monday() as usize;
    std::array::from_fn(|i| LETTERS[(first + i) % 7])
}

pub fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("every month has a first")
}

/// `date` moved by `months`, with the day clamped to the target month.
pub fn add_months(date: NaiveDate, months: i32) -> NaiveDate {
    let shifted = if months >= 0 {
        date.checked_add_months(Months::new(months as u32))
    } else {
        date.checked_sub_months(Months::new(months.unsigned_abs()))
    };
    shifted.expect("dates stay in range")
}

/// The first day shown in a month grid and how many weeks it has: whole
/// weeks, Monday first, covering every day of the month.
pub fn month_grid(date: NaiveDate) -> (NaiveDate, usize) {
    let first = month_start(date);
    let start = week_start(first);
    let last = add_months(first, 1) - Duration::days(1);
    let days = (last - start).num_days() + 1;
    (start, ((days + 6) / 7) as usize)
}

pub fn is_weekend(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

pub fn at(date: NaiveDate, h: u32, m: u32) -> NaiveDateTime {
    date.and_hms_opt(h, m, 0).expect("valid time")
}

pub fn midnight(date: NaiveDate) -> NaiveDateTime {
    at(date, 0, 0)
}

/// Minutes since the start of `day`; may be negative or past 1440.
pub fn minutes_into(day: NaiveDate, time: NaiveDateTime) -> i64 {
    (time - midnight(day)).num_minutes()
}

/// The next half hour strictly after `now`.
pub fn next_half_hour(now: NaiveDateTime) -> NaiveDateTime {
    let minute = now.minute();
    let base = now
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap();
    let base = base.with_minute(0).unwrap();
    if minute < 30 {
        base + Duration::minutes(30)
    } else {
        base + Duration::hours(1)
    }
}

/// "9:30 AM", or "09:30" on a 24-hour clock.
pub fn time_label(time: NaiveTime) -> String {
    if is_24_hour() {
        time.format("%H:%M").to_string()
    } else {
        time.format("%-I:%M %p").to_string()
    }
}

/// "9", "9:30": the hour and, when not on the hour, the minutes.
fn short_time(time: NaiveTime) -> String {
    if time.minute() == 0 {
        time.format("%-I").to_string()
    } else {
        time.format("%-I:%M").to_string()
    }
}

fn meridiem(time: NaiveTime) -> &'static str {
    if time.hour() < 12 { "AM" } else { "PM" }
}

/// "9 – 10:30 AM", or "11 AM – 1 PM" across noon; "09:00 – 10:30" on a
/// 24-hour clock.
pub fn time_range_label(start: NaiveTime, end: NaiveTime) -> String {
    if is_24_hour() {
        return format!("{} – {}", time_label(start), time_label(end));
    }
    if meridiem(start) == meridiem(end) {
        format!(
            "{} – {} {}",
            short_time(start),
            short_time(end),
            meridiem(end)
        )
    } else {
        format!(
            "{} {} – {} {}",
            short_time(start),
            meridiem(start),
            short_time(end),
            meridiem(end)
        )
    }
}

/// "9 AM", "9:30 AM"; "09:30" on a 24-hour clock.
pub fn compact_time_label(time: NaiveTime) -> String {
    if is_24_hour() {
        return time_label(time);
    }
    format!("{} {}", short_time(time), meridiem(time))
}

/// "Mon, Oct 26", with the year when it isn't `today`'s.
pub fn date_label(date: NaiveDate, today: NaiveDate) -> String {
    if date.year() == today.year() {
        date.format("%a, %b %-d").to_string()
    } else {
        date.format("%a, %b %-d, %Y").to_string()
    }
}

/// "30 min", "1 hr", "1.5 hrs", "1 day".
pub fn duration_label(duration: Duration) -> String {
    let minutes = duration.num_minutes();
    if minutes < 60 {
        format!("{minutes} min")
    } else if minutes % (24 * 60) == 0 {
        let days = minutes / (24 * 60);
        format!("{days} day{}", if days == 1 { "" } else { "s" })
    } else if minutes % 60 == 0 {
        let hours = minutes / 60;
        format!("{hours} hr{}", if hours == 1 { "" } else { "s" })
    } else if minutes % 60 == 30 {
        format!("{}.5 hrs", minutes / 60)
    } else {
        format!("{} hr {} min", minutes / 60, minutes % 60)
    }
}

/// A typed time: "9", "9am", "9:30 pm", "930", "21:30", "noon".
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let text: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .collect();
    match text.as_str() {
        "noon" => return NaiveTime::from_hms_opt(12, 0, 0),
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        _ => {}
    }
    let (digits, pm) = if let Some(rest) = text.strip_suffix("am").or(text.strip_suffix('a')) {
        (rest, Some(false))
    } else if let Some(rest) = text.strip_suffix("pm").or(text.strip_suffix('p')) {
        (rest, Some(true))
    } else {
        (text.as_str(), None)
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None if digits.chars().all(|c| c.is_ascii_digit()) => match digits.len() {
            1 | 2 => (digits.parse().ok()?, 0),
            3 | 4 => {
                let (h, m) = digits.split_at(digits.len() - 2);
                (h.parse().ok()?, m.parse().ok()?)
            }
            _ => return None,
        },
        None => return None,
    };
    let h = match pm {
        Some(_) if !(1..=12).contains(&h) => return None,
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// A typed date: "today", "tomorrow", "2026-10-27", "10/27", "Oct 27",
/// "October 27 2026", "27 Oct", "Tue, Oct 27, 2026", a month ("Dec", "Dec
/// 2027": its first day), or relative to `today` (see `parse_relative`).
/// Without a year, the one of `today`.
pub fn parse_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let lower = text.trim().to_lowercase();
    if let Some(date) = parse_relative(&lower, today) {
        return Some(date);
    }
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty() && w.parse::<Weekday>().is_err())
        .collect();
    let joined = words.join(" ");
    let with_year = |fmt: &str| NaiveDate::parse_from_str(&joined, fmt).ok();
    let without_year = |fmt: &str, sep: &str| {
        NaiveDate::parse_from_str(&format!("{joined}{sep}{}", today.year()), fmt).ok()
    };
    // A month alone, with or without its year: its first day. First, since
    // "dec 2026" would otherwise read as December 20, year 26.
    let month_alone = || {
        let (month, year) = match words.as_slice() {
            [month] => (*month, today.year()),
            [month, year] if year.len() == 4 => (*month, year.parse().ok()?),
            _ => return None,
        };
        NaiveDate::parse_from_str(&format!("1 {month} {year}"), "%d %B %Y").ok()
    };
    month_alone()
        .or_else(|| with_year("%Y-%m-%d"))
        .or_else(|| with_year("%m/%d/%Y"))
        .or_else(|| with_year("%b %d %Y"))
        .or_else(|| with_year("%B %d %Y"))
        .or_else(|| with_year("%d %b %Y"))
        .or_else(|| with_year("%d %B %Y"))
        .or_else(|| without_year("%m/%d/%Y", "/"))
        .or_else(|| without_year("%b %d %Y", " "))
        .or_else(|| without_year("%B %d %Y", " "))
        .or_else(|| without_year("%d %b %Y", " "))
        .or_else(|| without_year("%d %B %Y", " "))
}

/// Dates said relative to `today`: "today", "tomorrow", "yesterday", "fri"
/// (the next Friday, or today on a Friday), "next fri" and "last fri" (the
/// Friday of next or last week), "next week" (its Monday), "next month",
/// "next year" (their first days), "last week", and "in 3 days", "in 2
/// weeks", "in 1 month".
fn parse_relative(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let weekday = |word: &str| word.parse::<Weekday>().ok();
    let in_week = |monday: NaiveDate, day: Weekday| {
        monday + Duration::days(day.num_days_from_monday() as i64)
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    Some(match words.as_slice() {
        ["today"] => today,
        ["tomorrow" | "tmr" | "tom"] => today + Duration::days(1),
        ["yesterday"] => today - Duration::days(1),
        [day] | ["this", day] => {
            let day = weekday(day)?;
            let ahead = (day.num_days_from_monday() as i64
                - today.weekday().num_days_from_monday() as i64)
                .rem_euclid(7);
            today + Duration::days(ahead)
        }
        ["next" | "last", "week"] | ["next" | "last", "month"] | ["next" | "last", "year"] => {
            let sign = if words[0] == "next" { 1 } else { -1 };
            match words[1] {
                "week" => week_start(today) + Duration::days(7 * sign),
                "month" => month_start(add_months(today, sign as i32)),
                _ => NaiveDate::from_ymd_opt(today.year() + sign as i32, 1, 1)?,
            }
        }
        [which @ ("next" | "last"), day] => {
            let sign = if *which == "next" { 1 } else { -1 };
            in_week(week_start(today) + Duration::days(7 * sign), weekday(day)?)
        }
        ["in", count, unit] => {
            let count: i64 = count.parse().ok()?;
            match unit.trim_end_matches('s') {
                "day" => today + Duration::days(count),
                "week" => today + Duration::weeks(count),
                "month" => add_months(today, count as i32),
                "year" => add_months(today, 12 * count as i32),
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// The user's zone: its IANA name, and conversions between UTC and its
/// wall clock. Tests pass a fixed one, so they don't depend on the machine.
#[derive(Clone, Copy, Debug)]
pub struct LocalZone {
    pub name: &'static str,
    pub to_local: fn(DateTime<Utc>) -> NaiveDateTime,
    #[allow(dead_code)] // Calendar writes (W7).
    pub to_utc: fn(NaiveDateTime) -> DateTime<Utc>,
}

impl LocalZone {
    /// The system's zone (`iana-time-zone`, converted with `chrono::Local`).
    pub fn system() -> Self {
        static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let name =
            NAME.get_or_init(|| iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into()));
        Self {
            name,
            to_local: |t| t.with_timezone(&Local).naive_local(),
            to_utc: |t| {
                Local
                    .from_local_datetime(&t)
                    .earliest()
                    // A wall time skipped by a DST change: an hour on.
                    .or_else(|| {
                        Local
                            .from_local_datetime(&(t + Duration::hours(1)))
                            .earliest()
                    })
                    .map_or_else(|| t.and_utc(), |t| t.to_utc())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn weeks_start_on_monday() {
        assert_eq!(week_start(d(2026, 10, 26)), d(2026, 10, 26));
        assert_eq!(week_start(d(2026, 11, 1)), d(2026, 10, 26));
        assert_eq!(week_start(d(2026, 10, 31)), d(2026, 10, 26));
        // Google's weekStart: Sunday or Saturday.
        assert_eq!(
            week_start_on(d(2026, 10, 31), Weekday::Sun),
            d(2026, 10, 25)
        );
        assert_eq!(week_start_on(d(2026, 11, 1), Weekday::Sun), d(2026, 11, 1));
        assert_eq!(
            week_start_on(d(2026, 10, 30), Weekday::Sat),
            d(2026, 10, 24)
        );
        assert_eq!(monday_of(d(2026, 11, 1)), d(2026, 10, 26));
    }

    #[test]
    fn month_grid_covers_the_month_in_whole_weeks() {
        // October 2026 starts on a Thursday and ends on a Saturday.
        assert_eq!(month_grid(d(2026, 10, 15)), (d(2026, 9, 28), 5));
        // November 2026 starts on a Sunday: six rows.
        assert_eq!(month_grid(d(2026, 11, 3)), (d(2026, 10, 26), 6));
        // February 2027 starts on a Monday and has exactly four weeks.
        assert_eq!(month_grid(d(2027, 2, 10)), (d(2027, 2, 1), 4));
    }

    #[test]
    fn add_months_clamps_the_day() {
        assert_eq!(add_months(d(2026, 1, 31), 1), d(2026, 2, 28));
        assert_eq!(add_months(d(2026, 3, 31), -1), d(2026, 2, 28));
        assert_eq!(add_months(d(2026, 12, 15), 1), d(2027, 1, 15));
    }

    #[test]
    fn next_half_hour_rounds_up() {
        assert_eq!(
            next_half_hour(at(d(2026, 10, 26), 9, 0)),
            at(d(2026, 10, 26), 9, 30)
        );
        assert_eq!(
            next_half_hour(at(d(2026, 10, 26), 9, 41)),
            at(d(2026, 10, 26), 10, 0)
        );
        assert_eq!(
            next_half_hour(at(d(2026, 10, 26), 23, 45)),
            at(d(2026, 10, 27), 0, 0)
        );
    }

    #[test]
    fn labels() {
        assert_eq!(time_range_label(t(9, 0), t(10, 30)), "9 – 10:30 AM");
        assert_eq!(time_range_label(t(11, 0), t(13, 0)), "11 AM – 1 PM");
        assert_eq!(compact_time_label(t(0, 15)), "12:15 AM");
        assert_eq!(duration_label(Duration::minutes(30)), "30 min");
        assert_eq!(duration_label(Duration::minutes(60)), "1 hr");
        assert_eq!(duration_label(Duration::minutes(90)), "1.5 hrs");
        assert_eq!(duration_label(Duration::minutes(120)), "2 hrs");
        assert_eq!(duration_label(Duration::days(1)), "1 day");
        assert_eq!(date_label(d(2026, 10, 26), d(2026, 10, 1)), "Mon, Oct 26");
        assert_eq!(
            date_label(d(2027, 1, 4), d(2026, 10, 1)),
            "Mon, Jan 4, 2027"
        );
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("9"), Some(t(9, 0)));
        assert_eq!(parse_time("9am"), Some(t(9, 0)));
        assert_eq!(parse_time("9:30 PM"), Some(t(21, 30)));
        assert_eq!(parse_time("930"), Some(t(9, 30)));
        assert_eq!(parse_time("1745"), Some(t(17, 45)));
        assert_eq!(parse_time("12am"), Some(t(0, 0)));
        assert_eq!(parse_time("12 p.m."), Some(t(12, 0)));
        assert_eq!(parse_time("noon"), Some(t(12, 0)));
        assert_eq!(parse_time("2p"), Some(t(14, 0)));
        assert_eq!(parse_time("13pm"), None);
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn parses_dates() {
        let today = d(2026, 10, 26);
        assert_eq!(parse_date("today", today), Some(today));
        assert_eq!(parse_date("Tomorrow", today), Some(d(2026, 10, 27)));
        assert_eq!(parse_date("2026-11-03", today), Some(d(2026, 11, 3)));
        assert_eq!(parse_date("11/3", today), Some(d(2026, 11, 3)));
        assert_eq!(parse_date("11/3/2027", today), Some(d(2027, 11, 3)));
        assert_eq!(parse_date("nov 3", today), Some(d(2026, 11, 3)));
        assert_eq!(parse_date("November 3 2027", today), Some(d(2027, 11, 3)));
        assert_eq!(parse_date("3 Nov", today), Some(d(2026, 11, 3)));
        assert_eq!(parse_date("Tue, Nov 3", today), Some(d(2026, 11, 3)));
        assert_eq!(parse_date("Wed, Jan 6, 2027", today), Some(d(2027, 1, 6)));
        assert_eq!(parse_date("someday", today), None);
    }

    #[test]
    fn parses_relative_dates() {
        // A Monday.
        let today = d(2026, 10, 26);
        assert_eq!(parse_date("yesterday", today), Some(d(2026, 10, 25)));
        assert_eq!(parse_date("fri", today), Some(d(2026, 10, 30)));
        assert_eq!(parse_date("Friday", today), Some(d(2026, 10, 30)));
        assert_eq!(parse_date("mon", today), Some(today));
        assert_eq!(parse_date("sun", today), Some(d(2026, 11, 1)));
        assert_eq!(parse_date("next fri", today), Some(d(2026, 11, 6)));
        assert_eq!(parse_date("last fri", today), Some(d(2026, 10, 23)));
        assert_eq!(parse_date("next week", today), Some(d(2026, 11, 2)));
        assert_eq!(parse_date("next month", today), Some(d(2026, 11, 1)));
        assert_eq!(parse_date("next year", today), Some(d(2027, 1, 1)));
        assert_eq!(parse_date("in 3 days", today), Some(d(2026, 10, 29)));
        assert_eq!(parse_date("in 2 weeks", today), Some(d(2026, 11, 9)));
        assert_eq!(parse_date("in 1 month", today), Some(d(2026, 11, 26)));
        assert_eq!(parse_date("nov 12", today), Some(d(2026, 11, 12)));
        assert_eq!(parse_date("dec", today), Some(d(2026, 12, 1)));
        assert_eq!(parse_date("March 2027", today), Some(d(2027, 3, 1)));
        assert_eq!(parse_date("next", today), None);
        assert_eq!(parse_date("in 3 parsecs", today), None);
    }
}

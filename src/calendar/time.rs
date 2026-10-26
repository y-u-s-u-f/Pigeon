//! Date math, formatting and lenient parsing of typed dates and times.

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};

/// Weeks start on Monday.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
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

/// "9:30 AM".
pub fn time_label(time: NaiveTime) -> String {
    time.format("%-I:%M %p").to_string()
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

/// "9 – 10:30 AM", or "11 AM – 1 PM" across noon.
pub fn time_range_label(start: NaiveTime, end: NaiveTime) -> String {
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

/// "9 AM", "9:30 AM".
pub fn compact_time_label(time: NaiveTime) -> String {
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
/// "October 27 2026", "27 Oct", "Tue, Oct 27, 2026". Without a year, the
/// one of `today`.
pub fn parse_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let lower = text.trim().to_lowercase();
    match lower.as_str() {
        "today" => return Some(today),
        "tomorrow" => return Some(today + Duration::days(1)),
        "yesterday" => return Some(today - Duration::days(1)),
        _ => {}
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
    with_year("%Y-%m-%d")
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
}

//! Quick add (CAL-39): "New event: Lunch with Sam Fri 1pm" read locally,
//! with the date and time parsers the panel uses, into a title and times
//! for the event panel to confirm. No `events.quickAdd` round trip.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use super::time::{self, midnight, next_half_hour, parse_date, parse_time};

use std::sync::atomic::{AtomicU32, Ordering};

/// A new event's length in minutes, set by the calendar from the settings;
/// the palette's quick add row reads it.
static DEFAULT_MINUTES: AtomicU32 = AtomicU32::new(60);

pub fn set_default_minutes(minutes: u32) {
    DEFAULT_MINUTES.store(minutes, Ordering::Relaxed);
}

pub fn default_minutes() -> u32 {
    DEFAULT_MINUTES.load(Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickEvent {
    pub title: String,
    pub start: NaiveDateTime,
    /// Exclusive; midnight after the day, all day.
    pub end: NaiveDateTime,
    pub all_day: bool,
}

impl QuickEvent {
    /// "Fri, Oct 30 · 1 – 2 PM", "Fri, Oct 30 · All day".
    pub fn when_label(&self, today: NaiveDate) -> String {
        let day = time::date_label(self.start.date(), today);
        if self.all_day {
            format!("{day} · All day")
        } else {
            format!(
                "{day} · {}",
                time::time_range_label(self.start.time(), self.end.time())
            )
        }
    }
}

/// The text after "New event:" (or "new event "), if `query` starts so.
pub fn command_text(query: &str) -> Option<&str> {
    let trimmed = query.trim_start();
    let head = trimmed.get(..9)?;
    if !head.eq_ignore_ascii_case("new event") {
        return None;
    }
    let rest = &trimmed[9..];
    if let Some(rest) = rest.strip_prefix(':') {
        Some(rest.trim())
    } else if rest.is_empty() || rest.starts_with(' ') {
        Some(rest.trim())
    } else {
        None
    }
}

/// Words that join a date or time to the title, dropped with it.
const JOINERS: [&str; 5] = ["at", "on", "@", "from", "for"];

/// Reads `text` at `now`. Without a time it's all day on the date given;
/// with neither, today at the next half hour. A time without a date is
/// today, or tomorrow once it's past. Without an end or a length ("for 30
/// min"), it lasts `default_minutes`.
pub fn parse(text: &str, now: NaiveDateTime, default_minutes: u32) -> QuickEvent {
    let words: Vec<&str> = text.split_whitespace().collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut used = vec![false; words.len()];
    let today = now.date();
    let mut date: Option<NaiveDate> = None;
    let mut start: Option<NaiveTime> = None;
    let mut end: Option<NaiveTime> = None;
    let mut length: Option<Duration> = None;
    let mut all_day = false;

    let mut i = 0;
    while i < words.len() {
        let after_joiner = i > 0 && !used[i - 1] && JOINERS.contains(&lower[i - 1].as_str());
        // "all day"
        if lower[i] == "all-day"
            || (lower[i] == "all" && lower.get(i + 1).is_some_and(|w| w == "day"))
        {
            let n = if lower[i] == "all" { 2 } else { 1 };
            mark(&mut used, i, n, false);
            all_day = true;
            i += n;
            continue;
        }
        // "for 30 min", "for 2 hours", "for 1h"
        if lower[i] == "for"
            && let Some((d, n)) = duration_at(&lower[i + 1..])
        {
            length = Some(d);
            mark(&mut used, i, n + 1, false);
            i += n + 1;
            continue;
        }
        // A time or a range: "1pm", "1-2pm", "1pm to 2pm", "at 3".
        if start.is_none()
            && let Some((s, e, n)) = time_at(&lower[i..], after_joiner)
        {
            start = Some(s);
            end = e;
            mark(&mut used, i, n, after_joiner);
            i += n;
            continue;
        }
        // A date, of up to three words: "next fri", "nov 12 2026".
        if date.is_none()
            && let Some((d, n)) = (1..=3).rev().find_map(|n| {
                let span = lower.get(i..i + n)?;
                if n == 1 && span[0].chars().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                parse_date(&span.join(" "), today).map(|d| (d, n))
            })
        {
            date = Some(d);
            mark(&mut used, i, n, after_joiner);
            i += n;
            continue;
        }
        i += 1;
    }

    let title = words
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|(w, _)| *w)
        .collect::<Vec<_>>()
        .join(" ");
    let minutes = Duration::minutes(default_minutes.max(5) as i64);
    if all_day || (start.is_none() && date.is_some()) {
        let day = date.unwrap_or(today);
        return QuickEvent {
            title,
            start: midnight(day),
            end: midnight(day + Duration::days(1)),
            all_day: true,
        };
    }
    let start = match (date, start) {
        (Some(day), Some(t)) => day.and_time(t),
        (None, Some(t)) if today.and_time(t) > now => today.and_time(t),
        (None, Some(t)) => (today + Duration::days(1)).and_time(t),
        _ => next_half_hour(now),
    };
    let end = match end {
        Some(t) => {
            let mut end = start.date().and_time(t);
            if end <= start {
                end += Duration::days(1);
            }
            end
        }
        None => start + length.unwrap_or(minutes),
    };
    QuickEvent {
        title,
        start,
        end,
        all_day: false,
    }
}

/// Marks `n` words from `i` taken, and the joiner before them.
fn mark(used: &mut [bool], i: usize, n: usize, joiner: bool) {
    let end = (i + n).min(used.len());
    for u in &mut used[i..end] {
        *u = true;
    }
    if joiner {
        used[i - 1] = true;
    }
}

/// A time said clearly: with am/pm or minutes, "noon", or any hour after a
/// joiner ("at 3").
fn clear_time(word: &str, after_joiner: bool) -> Option<NaiveTime> {
    let mut t = parse_time(word)?;
    // "at 3" is in the afternoon.
    if after_joiner && word.chars().all(|c| c.is_ascii_digit()) && (1..=7).contains(&t.hour()) {
        t += Duration::hours(12);
    }
    let clear = word.contains(':')
        || word.ends_with('m')
        || word == "noon"
        || word == "midnight"
        || after_joiner;
    clear.then_some(t)
}

/// A time or a range at the start of `words`: start, end, words taken.
fn time_at(words: &[String], after_joiner: bool) -> Option<(NaiveTime, Option<NaiveTime>, usize)> {
    let first = words.first()?;
    // "1-2pm", "10:30–11"
    if let Some((a, b)) = first.split_once(['-', '–']) {
        let end = parse_time(b)?;
        let start = parse_time(a)?;
        return Some((with_meridiem_of(start, a, end), Some(end), 1));
    }
    // "1pm to 2pm", "1 - 2pm"
    if let (Some(sep), Some(b)) = (words.get(1), words.get(2))
        && matches!(sep.as_str(), "to" | "-" | "–" | "until")
        && let (Some(start), Some(end)) = (parse_time(first), clear_time(b, true))
        && (clear_time(first, after_joiner).is_some() || b.ends_with('m'))
    {
        return Some((with_meridiem_of(start, first, end), Some(end), 3));
    }
    clear_time(first, after_joiner).map(|t| (t, None, 1))
}

/// "1-2pm": a start without am/pm takes the end's afternoon when that fits.
fn with_meridiem_of(start: NaiveTime, text: &str, end: NaiveTime) -> NaiveTime {
    let bare =
        !text.ends_with('m') && !text.contains(':') || text.chars().all(|c| c.is_ascii_digit());
    if bare && end.hour() >= 12 && start.hour() < 12 && start.hour() + 12 <= end.hour() {
        start + Duration::hours(12)
    } else {
        start
    }
}

/// "30 min", "2 hours", "1h", "90m": the length and words taken.
fn duration_at(words: &[String]) -> Option<(Duration, usize)> {
    let first = words.first()?;
    let unit_of = |unit: &str| -> Option<i64> {
        match unit.trim_end_matches('s') {
            "m" | "min" | "minute" | "mins" => Some(1),
            "h" | "hr" | "hour" => Some(60),
            _ => None,
        }
    };
    let digits: String = first.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: i64 = digits.parse().ok()?;
    let rest = &first[digits.len()..];
    if !rest.is_empty() {
        return Some((Duration::minutes(n * unit_of(rest)?), 1));
    }
    let unit = unit_of(words.get(1)?)?;
    Some((Duration::minutes(n * unit), 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::time::at;

    /// Monday Oct 26, 2026, 10:12.
    fn now() -> NaiveDateTime {
        at(day(26), 10, 12)
    }

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, n).unwrap()
    }

    fn quick(text: &str) -> QuickEvent {
        parse(text, now(), 60)
    }

    #[test]
    fn the_command_prefix() {
        assert_eq!(command_text("New event: Lunch"), Some("Lunch"));
        assert_eq!(command_text("  new event lunch fri"), Some("lunch fri"));
        assert_eq!(command_text("new event"), Some(""));
        assert_eq!(command_text("new events"), None);
        assert_eq!(command_text("new"), None);
        assert_eq!(command_text("Archive"), None);
    }

    #[test]
    fn title_day_and_time() {
        let e = quick("Lunch with Sam Fri 1pm");
        assert_eq!(e.title, "Lunch with Sam");
        assert_eq!((e.start, e.end), (at(day(30), 13, 0), at(day(30), 14, 0)));
        assert!(!e.all_day);
        let e = quick("Dentist tomorrow at 9:30 for 45 min");
        assert_eq!(e.title, "Dentist");
        assert_eq!((e.start, e.end), (at(day(27), 9, 30), at(day(27), 10, 15)));
        let e = quick("Review on nov 3 from 2-3:30pm");
        assert_eq!(e.title, "Review");
        assert_eq!(
            e.start,
            at(NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(), 14, 0)
        );
        assert_eq!(e.end - e.start, Duration::minutes(90));
        let e = quick("Call Ann 4pm to 5pm next wed");
        assert_eq!(e.title, "Call Ann");
        assert_eq!(
            (e.start, e.end),
            (
                at(day(28) + Duration::days(7), 16, 0),
                at(day(28) + Duration::days(7), 17, 0)
            )
        );
    }

    #[test]
    fn defaults() {
        // A time already past today: tomorrow.
        let e = quick("Standup 9am");
        assert_eq!(e.start, at(day(27), 9, 0));
        let e = quick("Gym 6pm");
        assert_eq!(e.start, at(day(26), 18, 0));
        // A date alone: all day.
        let e = quick("Offsite fri");
        assert!(e.all_day);
        assert_eq!((e.start, e.end), (at(day(30), 0, 0), at(day(31), 0, 0)));
        let e = quick("Holiday all day tomorrow");
        assert!(e.all_day && e.title == "Holiday");
        // Neither: the next half hour.
        let e = quick("Think");
        assert_eq!((e.start, e.end), (at(day(26), 10, 30), at(day(26), 11, 30)));
        // Numbers in the title stay.
        let e = quick("Read 3 papers at 3");
        assert_eq!(e.title, "Read 3 papers");
        assert_eq!(e.start, at(day(26), 15, 0));
        assert_eq!(parse("Sync 2pm", now(), 30).end, at(day(26), 14, 30));
    }
}

//! Quick add (CAL-39): "New event: Lunch with Sam Fri 1pm" read locally,
//! with the date and time parsers the panel uses, into a title and times
//! for the event panel to confirm. No `events.quickAdd` round trip.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};

use super::time::{self, midnight, next_half_hour, parse_date, parse_time};
use super::{Calendar, new_event_calendar};

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
    /// "at Nopa": a place said after the times, in capitals.
    pub location: Option<String>,
    /// "with Sam and Ann": names to look up as guests. They stay in the
    /// title.
    pub people: Vec<String>,
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

/// What the scan of a text found, by word.
struct Scan<'a> {
    words: Vec<&'a str>,
    lower: Vec<String>,
    used: Vec<bool>,
    date: Option<NaiveDate>,
    /// The words the date was read from.
    date_words: std::ops::Range<usize>,
    start: Option<NaiveTime>,
    end: Option<NaiveTime>,
    /// The words the time was read from.
    time_words: std::ops::Range<usize>,
    length: Option<Duration>,
    all_day: bool,
}

/// Reads a date, a time or a range, a length and "all day" from `text`'s
/// words, dates relative to `today`.
fn scan(text: &str, today: NaiveDate) -> Scan<'_> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut s = Scan {
        used: vec![false; words.len()],
        words,
        lower,
        date: None,
        date_words: 0..0,
        start: None,
        end: None,
        time_words: 0..0,
        length: None,
        all_day: false,
    };
    let lower = &s.lower;
    let used = &mut s.used;

    let mut i = 0;
    while i < lower.len() {
        let after_joiner = i > 0 && !used[i - 1] && JOINERS.contains(&lower[i - 1].as_str());
        // "all day"
        if lower[i] == "all-day"
            || (lower[i] == "all" && lower.get(i + 1).is_some_and(|w| w == "day"))
        {
            let n = if lower[i] == "all" { 2 } else { 1 };
            mark(used, i, n, false);
            s.all_day = true;
            i += n;
            continue;
        }
        // "for 30 min", "for 2 hours", "for 1h"
        if lower[i] == "for"
            && let Some((d, n)) = duration_at(&lower[i + 1..])
        {
            s.length = Some(d);
            mark(used, i, n + 1, false);
            i += n + 1;
            continue;
        }
        // A time or a range: "1pm", "1-2pm", "1pm to 2pm", "at 3".
        if s.start.is_none()
            && let Some((start, end, n)) = time_at(&lower[i..], after_joiner)
        {
            s.start = Some(start);
            s.end = end;
            s.time_words = i..i + n;
            mark(used, i, n, after_joiner);
            i += n;
            continue;
        }
        // A date, of up to three words: "next fri", "nov 12 2026".
        if s.date.is_none()
            && let Some((d, n)) = (1..=3).rev().find_map(|n| {
                let span = lower.get(i..i + n)?;
                if n == 1 && span[0].chars().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                parse_date(&span.join(" "), today).map(|d| (d, n))
            })
        {
            s.date = Some(d);
            s.date_words = i..i + n;
            mark(used, i, n, after_joiner);
            i += n;
            continue;
        }
        i += 1;
    }
    s
}

/// Reads `text` at `now`. Without a time it's all day on the date given;
/// with neither, today at the next half hour. A time without a date is
/// today, or tomorrow once it's past. Without an end or a length ("for 30
/// min"), it lasts `default_minutes`.
pub fn parse(text: &str, now: NaiveDateTime, default_minutes: u32) -> QuickEvent {
    let today = now.date();
    let Scan {
        words,
        lower,
        mut used,
        date,
        start,
        end,
        length,
        all_day,
        ..
    } = scan(text, today);

    let location = location_at(&words, &lower, &mut used);
    let people = people_at(&words, &lower, &used);
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
            location,
            people,
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
        location,
        people,
    }
}

/// A date or time found in mail (AI-16), as `parse_when` read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedWhen {
    pub start: NaiveDateTime,
    pub end: Option<NaiveDateTime>,
    pub all_day: bool,
}

/// Words a date in mail may have around it that change nothing.
const FILLER: [&str; 14] = [
    "the", "of", "this", "on", "at", "@", "from", "between", "and", "to", "until", "-", "–", "by",
];

/// Reads `span`, a date the detector found in mail sent at `sent` (local),
/// strictly: `None` when it holds a word it doesn't know ("the day after
/// tomorrow"). Unlike `parse` it never makes up a time: without one it's
/// all day. A weekday is the one after `sent`; a date without a year the
/// next one on or after it. A time alone is on the day sent, or the next
/// once past. A bare hour from 1 to 7 is in the afternoon.
pub fn parse_when(span: &str, sent: NaiveDateTime) -> Option<ParsedWhen> {
    // "Jan 5th" → "Jan 5".
    let text: String = span
        .split_whitespace()
        .map(|w| {
            let bare = w.trim_end_matches([',', '.', ';', '!', '?']);
            let digits = bare.trim_end_matches(char::is_alphabetic);
            let suffix = bare[digits.len()..].to_ascii_lowercase();
            if !digits.is_empty()
                && digits.chars().all(|c| c.is_ascii_digit())
                && matches!(suffix.as_str(), "st" | "nd" | "rd" | "th")
            {
                digits
            } else {
                bare
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let today = sent.date();
    let s = scan(&text, today);
    if s.date.is_none() && s.start.is_none() {
        return None;
    }
    // Anything left must be filler or a zone ("PST", "CET").
    let known = s.words.iter().zip(&s.used).all(|(w, used)| {
        let w = w.trim_matches(|c: char| ",.;:()!?".contains(c));
        *used
            || w.is_empty()
            || FILLER.contains(&w.to_lowercase().as_str())
            || ((2..=5).contains(&w.len()) && w.chars().all(|c| c.is_ascii_uppercase()))
    });
    if !known {
        return None;
    }
    let date = s.date.map(|d| {
        let said = s.lower[s.date_words.clone()].join(" ");
        let said = said.trim_start_matches("this ");
        let year = said
            .split(|c: char| !c.is_ascii_digit())
            .any(|n| n.len() == 4);
        if d == today && said.trim_end_matches(',').parse::<Weekday>().is_ok() {
            d + Duration::days(7)
        } else if d < today && !year && !said.starts_with("last") && said != "yesterday" {
            time::add_months(d, 12)
        } else {
            d
        }
    });
    let Some(start) = s.start.filter(|_| !s.all_day) else {
        return Some(ParsedWhen {
            start: midnight(date?),
            end: None,
            all_day: true,
        });
    };
    let said = s.lower[s.time_words.clone()].join(" ");
    let bare = !said.contains(['a', 'p']);
    let shift = |t: NaiveTime| {
        if bare && (1..=7).contains(&t.hour()) {
            t + Duration::hours(12)
        } else {
            t
        }
    };
    let start = match date {
        Some(day) => day.and_time(shift(start)),
        None if today.and_time(shift(start)) > sent => today.and_time(shift(start)),
        None => (today + Duration::days(1)).and_time(shift(start)),
    };
    let end = s
        .end
        .map(|t| {
            let mut end = start.date().and_time(shift(t));
            if end <= start {
                end += Duration::days(1);
            }
            end
        })
        .or(s.length.map(|l| start + l));
    Some(ParsedWhen {
        start,
        end,
        all_day: false,
    })
}

/// CAL-39: what a "#work" tag in a quick add names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    /// The calendar's id.
    Found(String),
    Ambiguous,
    Unknown,
    None,
}

/// The first "#tag" in `text`: a word of `#` and letters, digits, `-` and
/// `_`. Its byte range and the word without `#`.
pub fn tag_word(text: &str) -> Option<(std::ops::Range<usize>, &str)> {
    text.split_whitespace().find_map(|word| {
        let name = word.strip_prefix('#')?;
        let ok = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let at = word.as_ptr() as usize - text.as_ptr() as usize;
        ok.then_some((at..at + word.len(), name))
    })
}

/// `text` without its tag.
pub fn without_tag(text: &str) -> String {
    match tag_word(text) {
        Some((range, _)) => format!(
            "{} {}",
            text[..range.start].trim_end(),
            text[range.end..].trim_start()
        )
        .trim()
        .to_string(),
        None => text.to_string(),
    }
}

/// Lowercase letters and digits only: "Work Projects" → "workprojects".
fn squashed(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The calendar `text`'s tag names among `calendars` (writable ones, as
/// id and name): an exact name first, else the one name it starts.
pub fn resolve_tag(text: &str, calendars: &[(String, String)]) -> Tag {
    let Some((_, word)) = tag_word(text) else {
        return Tag::None;
    };
    let tag = squashed(word);
    if tag.is_empty() {
        return Tag::Unknown;
    }
    let names: Vec<(&str, String)> = calendars
        .iter()
        .map(|(id, name)| (id.as_str(), squashed(name)))
        .collect();
    let exact: Vec<&str> = names
        .iter()
        .filter(|(_, n)| *n == tag)
        .map(|(id, _)| *id)
        .collect();
    let found = if exact.is_empty() {
        names
            .iter()
            .filter(|(_, n)| n.starts_with(&tag))
            .map(|(id, _)| *id)
            .collect()
    } else {
        exact
    };
    match found.as_slice() {
        [] => Tag::Unknown,
        [id] => Tag::Found(id.to_string()),
        _ => Tag::Ambiguous,
    }
}

/// The calendar a quick add goes to: the tagged one while it's writable,
/// else the one any new event goes to.
pub fn pick_calendar(tag: &Tag, calendars: &[Calendar], default: Option<&str>) -> Option<String> {
    let tagged = match tag {
        Tag::Found(id) => calendars.iter().find(|c| c.id == *id && c.writable()),
        _ => None,
    };
    tagged
        .or_else(|| new_event_calendar(calendars, default))
        .map(|c| c.id.clone())
}

fn capitalized(word: &str) -> bool {
    word.chars().next().is_some_and(char::is_uppercase)
}

/// The last unused "at" or "@" followed by a capitalized word not taken by
/// a date or time: the words up to the next taken word or "with". "Meet at
/// noon" never gets here, since times are read first.
fn location_at(words: &[&str], lower: &[String], used: &mut [bool]) -> Option<String> {
    let at = (0..words.len().saturating_sub(1)).rev().find(|&i| {
        !used[i]
            && !used[i + 1]
            && matches!(lower[i].as_str(), "at" | "@")
            && capitalized(words[i + 1])
    })?;
    let end = (at + 1..words.len())
        .find(|&i| used[i] || lower[i] == "with")
        .unwrap_or(words.len());
    for u in &mut used[at..end] {
        *u = true;
    }
    Some(words[at + 1..end].join(" "))
}

/// Capitalized names after an unused "with", joined by "and" or "&".
fn people_at(words: &[&str], lower: &[String], used: &[bool]) -> Vec<String> {
    let mut people = Vec::new();
    for i in (0..words.len()).filter(|&i| !used[i] && lower[i] == "with") {
        let mut j = i + 1;
        while j < words.len() && !used[j] && capitalized(words[j]) {
            people.push(words[j].trim_end_matches(',').to_string());
            j += 1;
            if j + 1 < words.len() && matches!(lower[j].as_str(), "and" | "&") {
                j += 1;
            }
        }
    }
    people
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
        assert_eq!(e.people, ["Sam"]);
        assert_eq!(e.location, None);
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

    #[test]
    fn places_and_people() {
        let e = quick("Lunch with Sam at Nopa Fri 1pm");
        assert_eq!(e.title, "Lunch with Sam");
        assert_eq!(e.location.as_deref(), Some("Nopa"));
        assert_eq!(e.start, at(day(30), 13, 0));
        let e = quick("Coffee @ Blue Bottle with Ann and Raj 9am");
        assert_eq!(e.title, "Coffee with Ann and Raj");
        assert_eq!(e.location.as_deref(), Some("Blue Bottle"));
        assert_eq!(e.people, ["Ann", "Raj"]);
        let e = quick("Meet at noon");
        assert_eq!((e.title.as_str(), e.location), ("Meet", None));
        let e = quick("Look at it with team");
        assert_eq!(e.title, "Look at it with team");
        assert!(e.location.is_none() && e.people.is_empty());
    }

    fn cals(names: &[(&str, &str)]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect()
    }

    #[test]
    fn tags_name_calendars() {
        let list = cals(&[
            ("w", "Work"),
            ("wp", "Work Projects"),
            ("home", "Home"),
            ("f1", "Family"),
            ("f2", "family"),
        ]);
        let found = |id: &str| Tag::Found(id.into());
        assert_eq!(resolve_tag("Lunch #work fri", &list), found("w"));
        assert_eq!(resolve_tag("Lunch #WorkProjects", &list), found("wp"));
        assert_eq!(resolve_tag("Lunch #work-projects", &list), found("wp"));
        assert_eq!(resolve_tag("Lunch #ho", &list), found("home"));
        assert_eq!(resolve_tag("Lunch #wor", &list), Tag::Ambiguous);
        assert_eq!(resolve_tag("Lunch #family", &list), Tag::Ambiguous);
        assert_eq!(resolve_tag("Lunch #gym", &list), Tag::Unknown);
        assert_eq!(resolve_tag("Lunch #", &list), Tag::None);
        assert_eq!(resolve_tag("Lunch C#", &list), Tag::None);
        assert_eq!(resolve_tag("Lunch #wo!k", &list), Tag::None);
        assert_eq!(resolve_tag("Lunch fri", &list), Tag::None);
        assert_eq!(without_tag("Lunch #work fri 1pm"), "Lunch fri 1pm");
        assert_eq!(without_tag("#work Lunch"), "Lunch");
        assert_eq!(tag_word("Lunch #gym").map(|(_, w)| w), Some("gym"));
    }

    fn calendar(id: &str, name: &str, primary: bool) -> Calendar {
        Calendar {
            id: id.into(),
            name: name.into(),
            color: gpui::hsla(0., 0., 0., 1.),
            visible: true,
            access_role: crate::calendar::AccessRole::Owner,
            primary,
            time_zone: None,
            default_reminder: None,
            default_reminders: Vec::new(),
            conference_types: Vec::new(),
        }
    }

    #[test]
    fn a_tag_picks_its_calendar() {
        let mut list = vec![
            calendar("me", "Me", true),
            calendar("w", "Work", false),
            calendar("h", "Holidays", false),
        ];
        list[2].access_role = crate::calendar::AccessRole::Reader;
        let named: Vec<(String, String)> = list
            .iter()
            .filter(|c| c.writable())
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        let tag = resolve_tag("Standup #work", &named);
        assert_eq!(pick_calendar(&tag, &list, Some("me")), Some("w".into()));
        assert_eq!(
            pick_calendar(&Tag::None, &list, Some("w")),
            Some("w".into())
        );
        assert_eq!(pick_calendar(&Tag::Unknown, &list, None), Some("me".into()));
        // Read-only: the default.
        assert_eq!(
            pick_calendar(&Tag::Found("h".into()), &list, Some("me")),
            Some("me".into())
        );
    }

    #[test]
    fn dates_in_mail_are_read_strictly() {
        let when = |text| parse_when(text, now());
        // Monday Oct 26, 10:12.
        let w = when("Friday at 3").unwrap();
        assert_eq!((w.start, w.all_day), (at(day(30), 15, 0), false));
        // On the day sent: the next week's.
        let w = when("Monday").unwrap();
        assert_eq!(
            (w.start, w.all_day),
            (at(day(26) + Duration::days(7), 0, 0), true)
        );
        let w = when("Oct 1").unwrap();
        assert_eq!(
            w.start.date(),
            NaiveDate::from_ymd_opt(2027, 10, 1).unwrap()
        );
        let w = when("Oct 1, 2026").unwrap();
        assert_eq!(w.start.date(), day(1));
        let w = when("Thursday, October 29th").unwrap();
        assert_eq!((w.start, w.all_day), (at(day(29), 0, 0), true));
        assert_eq!(when("noon").unwrap().start, at(day(26), 12, 0));
        assert_eq!(when("9am").unwrap().start, at(day(27), 9, 0));
        let w = when("3:30–4:15").unwrap();
        assert_eq!(
            (w.start, w.end),
            (at(day(26), 15, 30), Some(at(day(26), 16, 15)))
        );
        let w = when("Oct 30 at 2pm PST").unwrap();
        assert_eq!(w.start, at(day(30), 14, 0));
        assert_eq!(when("the day after tomorrow"), None);
        assert_eq!(when("sometime"), None);
    }
}

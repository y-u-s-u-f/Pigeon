//! AI-16, dates in mail: NSDataDetector finds them, the Quick Add parser
//! reads them again against the day the mail was sent. Nothing is sent
//! anywhere; Jev's questions about them are the pass's.

use std::ops::Range;

use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone as _, Timelike, Utc};

use super::EventCandidate;
use crate::calendar::{quick_add, time::midnight};
use crate::native::{self, Detected};

/// Seconds east of UTC at an instant, in the zone named, else this Mac's.
type Offsets<'a> = &'a dyn Fn(Option<&str>, DateTime<Utc>) -> Option<i32>;

/// The dates in a message, titled after its subject. At most 3, earliest
/// first.
pub fn for_message(message: &super::text::Message) -> Vec<EventCandidate> {
    let mut found = candidates(&message.body, message.date.with_timezone(&Utc));
    let title = title_of(&message.subject);
    for c in &mut found {
        c.title = title.clone();
    }
    found
}

/// The dates in a message's `text`, sent at `sent`, untitled (see
/// `for_message`). At most 3, earliest first.
pub fn candidates(text: &str, sent: DateTime<Utc>) -> Vec<EventCandidate> {
    let offsets = |zone: Option<&str>, at: DateTime<Utc>| match zone {
        Some(name) => native::zone_offset(name, at),
        None => Some(
            Local
                .offset_from_utc_datetime(&at.naive_utc())
                .local_minus_utc(),
        ),
    };
    from_detected("", text, sent, &native::detect_dates(text), &offsets)
}

fn from_detected(
    subject: &str,
    text: &str,
    sent: DateTime<Utc>,
    detected: &[Detected],
    offsets: Offsets,
) -> Vec<EventCandidate> {
    let Some(sent_local) =
        offsets(None, sent).map(|o| sent.naive_utc() + Duration::seconds(o as i64))
    else {
        return Vec::new();
    };
    let location = detected.iter().find_map(|d| match d {
        Detected::Address { range } => Some(squash(&text[range.clone()])),
        _ => None,
    });
    let title = title_of(subject);
    let mut out: Vec<EventCandidate> = detected
        .iter()
        .filter_map(|d| {
            let Detected::Date {
                range,
                wall,
                zone,
                seconds,
            } = d
            else {
                return None;
            };
            let span = &text[range.clone()];
            let (start, end, all_day) = match read(text, range.clone(), sent_local) {
                Some(when) => {
                    let end = when.end.or_else(|| {
                        let seconds = (*seconds).filter(|_| !when.all_day)?;
                        Some(when.start + Duration::seconds(seconds))
                    });
                    (when.start, end, when.all_day)
                }
                // Another language: the detector's reading, when it has a
                // year to go by.
                None if has_year(span) => {
                    // Without a time the detector says noon.
                    let all_day = !span.contains(':')
                        && matches!((wall.hour(), wall.minute()), (0, 0) | (12, 0));
                    if all_day {
                        (midnight(wall.date()), None, true)
                    } else {
                        let end = seconds.map(|s| *wall + Duration::seconds(s));
                        (*wall, end, false)
                    }
                }
                None => return None,
            };
            let zone = zone.as_deref();
            let start = to_utc(start, |at| offsets(zone, at))?;
            let end = match end {
                Some(end) => Some(to_utc(end, |at| offsets(zone, at))?),
                None => None,
            };
            Some(EventCandidate {
                title: title.clone(),
                start,
                end,
                all_day,
                zone: zone.map(str::to_string),
                location: location.clone(),
                sentence: sentence_around(text, range.clone()),
            })
        })
        .collect();
    out.sort_by_key(|c| c.start);
    out.dedup_by_key(|c| c.start);
    out.truncate(3);
    out
}

/// The span, or the span with an "at 3" after it that the detector left
/// out.
fn read(text: &str, range: Range<usize>, sent: NaiveDateTime) -> Option<quick_add::ParsedWhen> {
    let span = &text[range.clone()];
    let mut after = text[range.end..].split_whitespace();
    if let (Some(at), Some(word)) = (after.next(), after.next())
        && matches!(at.to_lowercase().as_str(), "at" | "@")
        && let Some(when) = quick_add::parse_when(&format!("{span} {at} {word}"), sent)
        && !when.all_day
    {
        return Some(when);
    }
    quick_add::parse_when(span, sent)
}

/// A wall clock as UTC, by the zone's offsets; `None` when the time is
/// skipped or happens twice (a daylight saving change).
fn to_utc(
    wall: NaiveDateTime,
    offset_at: impl Fn(DateTime<Utc>) -> Option<i32>,
) -> Option<DateTime<Utc>> {
    let mut offsets = [
        offset_at((wall - Duration::days(1)).and_utc())?,
        offset_at((wall + Duration::days(1)).and_utc())?,
    ];
    offsets.sort();
    let mut found = Vec::new();
    for o in offsets {
        let utc = (wall - Duration::seconds(o as i64)).and_utc();
        if offset_at(utc)? == o && !found.contains(&utc) {
            found.push(utc);
        }
    }
    match found.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// Any four-digit number.
fn has_year(span: &str) -> bool {
    span.split(|c: char| !c.is_ascii_digit())
        .any(|n| n.len() == 4)
}

/// The subject without "Re:", "Fwd:" and the like in front.
pub fn title_of(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        let Some(n) = ["re:", "fwd:", "fw:"]
            .iter()
            .find(|p| lower.starts_with(**p))
            .map(|p| p.len())
        else {
            return s.to_string();
        };
        s = s[n..].trim_start();
    }
}

/// Whitespace runs as one space.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The sentence holding `range`: from the end of the one before (". ",
/// "! ", "? " or a line break) to the end of its own; at most 300 bytes.
fn sentence_around(text: &str, range: Range<usize>) -> String {
    let ends = |at: usize, c: char| {
        c == '\n' || matches!(c, '.' | '!' | '?') && text[at + 1..].starts_with(char::is_whitespace)
    };
    let start = text[..range.start]
        .char_indices()
        .rev()
        .find(|&(at, c)| ends(at, c))
        .map_or(0, |(at, c)| at + c.len_utf8());
    let end = text[range.end..]
        .char_indices()
        .find(|&(at, c)| ends(range.end + at, c))
        .map_or(text.len(), |(at, c)| {
            range.end + at + if c == '\n' { 0 } else { c.len_utf8() }
        });
    let mut sentence = squash(&text[start..end]);
    if sentence.len() > 300 {
        let cut = (0..=300)
            .rev()
            .find(|&i| sentence.is_char_boundary(i))
            .unwrap_or(0);
        sentence.truncate(cut);
        sentence.push('…');
    }
    sentence
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, NaiveDate, Weekday};

    /// US Eastern, with its daylight saving rules since 2007.
    fn eastern(at: DateTime<Utc>) -> i32 {
        us_offset(at, -5)
    }

    fn us_offset(at: DateTime<Utc>, standard: i32) -> i32 {
        let year = at.year();
        let nth_sunday = |month: u32, n: u8| {
            NaiveDate::from_weekday_of_month_opt(year, month, Weekday::Sun, n).unwrap()
        };
        // 2 AM local: in UTC, standard time at the start, daylight at the end.
        let start = (midnight(nth_sunday(3, 2)) + Duration::hours(2 - standard as i64)).and_utc();
        let end =
            (midnight(nth_sunday(11, 1)) + Duration::hours(2 - (standard as i64 + 1))).and_utc();
        let hours = if at >= start && at < end {
            standard + 1
        } else {
            standard
        };
        hours * 3600
    }

    /// The Mac in New York; a zone the text names is US Pacific.
    fn offsets(zone: Option<&str>, at: DateTime<Utc>) -> Option<i32> {
        Some(match zone {
            None => eastern(at),
            Some(_) => us_offset(at, -8),
        })
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
            .and_utc()
    }

    fn find(text: &str, sent: DateTime<Utc>) -> Vec<EventCandidate> {
        from_detected(
            "Re: Fwd: Planning",
            text,
            sent,
            &native::detect_dates(text),
            &offsets,
        )
    }

    fn one(text: &str, sent: DateTime<Utc>) -> EventCandidate {
        let found = find(text, sent);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        found.into_iter().next().unwrap()
    }

    #[test]
    fn last_years_friday_at_3() {
        // Wednesday Oct 22, 2025, 10 AM in New York.
        let c = one("Can we meet Friday at 3?", utc(2025, 10, 22, 14, 0));
        assert_eq!(c.start, utc(2025, 10, 24, 19, 0));
        assert!(!c.all_day);
        assert_eq!(c.title, "Planning");
        assert_eq!(c.sentence, "Can we meet Friday at 3?");
    }

    #[test]
    fn a_time_alone_is_on_the_day_sent() {
        // Last week's mail, sent Tuesday Oct 20, 9 AM in New York.
        let sent = utc(2026, 10, 20, 13, 0);
        let c = one("Let's talk at 3pm.", sent);
        assert_eq!(c.start, utc(2026, 10, 20, 19, 0));
        assert!(!c.all_day);
        let c = one("Lunch at noon works for me.", sent);
        assert_eq!(c.start, utc(2026, 10, 20, 16, 0));
        // Midnight already passed that day: the next one.
        let c = one("The release goes out at midnight.", sent);
        assert_eq!(c.start, utc(2026, 10, 21, 4, 0));
    }

    #[test]
    fn a_date_without_a_year_is_the_next_one() {
        // December 10, 2026.
        let c = one("The kickoff is on Jan 5.", utc(2026, 12, 10, 15, 0));
        assert!(c.all_day);
        // Midnight in New York, in standard time.
        assert_eq!(c.start, utc(2027, 1, 5, 5, 0));
    }

    #[test]
    fn a_date_alone_is_all_day() {
        let c = one(
            "The offsite is on November 12, 2026.",
            utc(2026, 10, 20, 13, 0),
        );
        assert!(c.all_day);
        assert_eq!(c.start, utc(2026, 11, 12, 5, 0));
        assert_eq!(c.end, None);
    }

    #[test]
    fn a_named_zone_wins() {
        let c = one(
            "The call is on December 3, 2026 at 3pm PST.",
            utc(2026, 11, 20, 13, 0),
        );
        assert!(c.zone.is_some());
        assert_eq!(c.start, utc(2026, 12, 3, 23, 0));
    }

    #[test]
    fn a_range() {
        let c = one(
            "Review on November 4, 2026 from 3:30–4:15.",
            utc(2026, 10, 28, 13, 0),
        );
        assert_eq!(c.start, utc(2026, 11, 4, 20, 30));
        assert_eq!(c.end, Some(utc(2026, 11, 4, 21, 15)));
    }

    #[test]
    fn two_dates_earliest_first() {
        let found = find(
            "Either November 12, 2026 at 2pm or November 10, 2026 at 10am. See you there.",
            utc(2026, 10, 28, 13, 0),
        );
        let starts: Vec<_> = found.iter().map(|c| c.start).collect();
        assert_eq!(starts, [utc(2026, 11, 10, 15, 0), utc(2026, 11, 12, 19, 0)]);
    }

    #[test]
    fn german_needs_a_year() {
        let sent = utc(2026, 12, 1, 13, 0);
        let c = one("Wir treffen uns am 5. Januar 2027 um 15 Uhr.", sent);
        assert_eq!(c.start, utc(2027, 1, 5, 20, 0));
        assert!(!c.all_day);
        assert!(find("Wir treffen uns am 5. Januar um 15 Uhr.", sent).is_empty());
    }

    #[test]
    fn daylight_saving_gaps_and_folds_drop_it() {
        let sent = utc(2026, 2, 20, 13, 0);
        assert!(find("Maintenance on March 8, 2026 at 2:30am.", sent).is_empty());
        let sent = utc(2026, 10, 20, 13, 0);
        assert!(find("Maintenance on November 1, 2026 at 1:30am.", sent).is_empty());
        assert!(find("Maintenance on November 1, 2026 at 3:30am.", sent).len() == 1);
    }

    #[test]
    fn an_unreadable_phrase_is_dropped() {
        let sent = utc(2026, 10, 20, 13, 0);
        assert!(find("Treffen wir uns übermorgen um 10 Uhr?", sent).is_empty());
    }

    #[test]
    fn wall_clocks_to_utc() {
        let wall = |m, d, h, min| {
            NaiveDate::from_ymd_opt(2026, m, d)
                .unwrap()
                .and_hms_opt(h, min, 0)
                .unwrap()
        };
        let east = |at| Some(eastern(at));
        assert_eq!(to_utc(wall(7, 1, 9, 0), east), Some(utc(2026, 7, 1, 13, 0)));
        assert_eq!(to_utc(wall(1, 1, 9, 0), east), Some(utc(2026, 1, 1, 14, 0)));
        assert_eq!(to_utc(wall(3, 8, 2, 30), east), None);
        assert_eq!(to_utc(wall(11, 1, 1, 30), east), None);
    }

    #[test]
    fn titles_and_sentences() {
        assert_eq!(title_of("RE: fwd: Fw: Dinner"), "Dinner");
        assert_eq!(title_of("Return policy"), "Return policy");
        let text = "Hi Ann. We meet Friday at 3! Bring snacks.";
        let at = text.find("Friday").unwrap();
        assert_eq!(sentence_around(text, at..at + 6), "We meet Friday at 3!");
    }
}

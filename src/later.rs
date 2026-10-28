//! Snooze (ACT-11) and Send Later (CMP-17) times: the picker's presets, a
//! typed time read with the calendar's quick-add parsers, and the labels
//! that say when. Wall-clock times are local; `to_ms` turns one into an
//! instant across DST.

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Timelike,
    Weekday,
};

use crate::calendar::{quick_add, time};

/// The hour Tomorrow, This Weekend and Next Week start at, and This
/// Evening's.
const MORNING: u32 = 8;
const EVENING: u32 = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    LaterToday,
    ThisEvening,
    Tomorrow,
    ThisWeekend,
    NextWeek,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::LaterToday,
        Preset::ThisEvening,
        Preset::Tomorrow,
        Preset::ThisWeekend,
        Preset::NextWeek,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Preset::LaterToday => "Later Today",
            Preset::ThisEvening => "This Evening",
            Preset::Tomorrow => "Tomorrow",
            Preset::ThisWeekend => "This Weekend",
            Preset::NextWeek => "Next Week",
        }
    }

    /// When it falls, seen from `now`; `None` where it doesn't apply
    /// (Later Today in the evening, This Weekend on a Sunday).
    fn at(self, now: NaiveDateTime) -> Option<NaiveDateTime> {
        let today = now.date();
        let morning = |day: NaiveDate| time::at(day, MORNING, 0);
        match self {
            // Three hours on, up to the next half hour; until the evening.
            Preset::LaterToday => {
                let at = time::next_half_hour(now + Duration::hours(3) - Duration::seconds(1));
                (at.date() == today && at.hour() < EVENING).then_some(at)
            }
            Preset::ThisEvening => Some(time::at(today, EVENING, 0)),
            Preset::Tomorrow => Some(morning(today + Duration::days(1))),
            Preset::ThisWeekend => {
                if today.weekday() == Weekday::Sun {
                    return None;
                }
                let days = 5 - today.weekday().num_days_from_monday() as i64;
                Some(morning(today + Duration::days(days)))
            }
            Preset::NextWeek => {
                let days = 7 - today.weekday().num_days_from_monday() as i64;
                Some(morning(today + Duration::days(days)))
            }
        }
    }
}

/// The presets still ahead of `now`, each at its time; one that falls with
/// an earlier one (Tomorrow on a Friday is This Weekend) shows once.
pub fn presets(now: NaiveDateTime) -> Vec<(Preset, NaiveDateTime)> {
    let mut out: Vec<(Preset, NaiveDateTime)> = Vec::new();
    for preset in Preset::ALL {
        if let Some(at) = preset.at(now)
            && at > now
            && !out.iter().any(|(_, t)| *t == at)
        {
            out.push((preset, at));
        }
    }
    out
}

/// A typed time, as the field reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    Empty,
    At(NaiveDateTime),
    /// Reads as a time, but it's gone.
    Past(NaiveDateTime),
    Unknown,
}

/// "Fri 9am", "tomorrow", "nov 12 3pm", "6pm", "in 2 hours", "tonight". A
/// date alone is its morning; a time alone is today's, or tomorrow's once
/// past.
pub fn parse(text: &str, now: NaiveDateTime) -> Typed {
    let lower = text.trim().to_lowercase();
    if lower.is_empty() {
        return Typed::Empty;
    }
    let now = now
        .with_second(0)
        .unwrap_or(now)
        .with_nanosecond(0)
        .unwrap_or(now);
    let at = match relative(&lower) {
        Some(d) => now + d,
        None => {
            let words: Vec<&str> = lower
                .split_whitespace()
                .filter(|w| *w != "this")
                .map(|w| match w {
                    "morning" => "8am",
                    "afternoon" => "2pm",
                    "evening" | "tonight" => "6pm",
                    w => w,
                })
                .collect();
            let event = quick_add::parse(&words.join(" "), now, 60);
            if !event.title.trim().is_empty() || event.location.is_some() {
                return Typed::Unknown;
            }
            if event.all_day {
                time::at(event.start.date(), MORNING, 0)
            } else {
                event.start
            }
        }
    };
    if at <= now {
        Typed::Past(at)
    } else {
        Typed::At(at)
    }
}

/// "in 2 hours", "in 30 min", "in a day", "in 3 weeks".
fn relative(text: &str) -> Option<Duration> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (n, unit) = match words.as_slice() {
        ["in", n, unit] => (*n, *unit),
        ["in", one] => {
            // "in 2h", "in 30m"
            let split = one.find(|c: char| !c.is_ascii_digit())?;
            (&one[..split], &one[split..])
        }
        _ => return None,
    };
    let n: i64 = match n {
        "a" | "an" | "one" => 1,
        n => n.parse().ok().filter(|n| *n > 0)?,
    };
    let minutes = match unit.trim_end_matches('s') {
        "m" | "min" | "minute" => 1,
        "h" | "hr" | "hour" => 60,
        "d" | "day" => 24 * 60,
        "w" | "wk" | "week" => 7 * 24 * 60,
        _ => return None,
    };
    Some(Duration::minutes(n * minutes))
}

/// The instant of a wall-clock time in `zone`, in Unix ms. A time a DST
/// gap skips moves on by the gap; one a fall-back repeats is the first.
pub fn to_ms<Tz: TimeZone>(zone: &Tz, at: NaiveDateTime) -> i64 {
    match zone.from_local_datetime(&at) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t.timestamp_millis(),
        LocalResult::None => zone
            .from_local_datetime(&(at + Duration::hours(1)))
            .earliest()
            .map_or_else(|| at.and_utc().timestamp_millis(), |t| t.timestamp_millis()),
    }
}

/// `to_ms` in the Mac's zone.
pub fn local_ms(at: NaiveDateTime) -> i64 {
    to_ms(&Local, at)
}

/// Unix ms as the Mac's wall clock.
pub fn local_time(ms: i64) -> NaiveDateTime {
    DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .with_timezone(&Local)
        .naive_local()
}

pub fn now_local() -> NaiveDateTime {
    Local::now().naive_local()
}

/// "Today, 6:00 PM", "Tomorrow, 8:00 AM", "Sat, 8:00 AM" within the week,
/// else "Mon, Nov 9, 8:00 AM".
pub fn when_label(at: NaiveDateTime, now: NaiveDateTime) -> String {
    format!(
        "{}, {}",
        day_label(at.date(), now.date()),
        time::time_label(at.time())
    )
}

/// `when_label` inside a sentence: "until today, 6:00 PM".
pub fn in_sentence(at: NaiveDateTime, now: NaiveDateTime) -> String {
    let label = when_label(at, now);
    for day in ["Today", "Tomorrow"] {
        if let Some(rest) = label.strip_prefix(day) {
            return format!("{}{rest}", day.to_lowercase());
        }
    }
    label
}

/// The list's date column: "6:00 PM" today, else "Tomorrow 8:00 AM", "Sat
/// 8:00 AM", "Nov 9 8:00 AM".
pub fn short_label(at: NaiveDateTime, now: NaiveDateTime) -> String {
    let clock = time::time_label(at.time());
    if at.date() == now.date() {
        return clock;
    }
    format!(
        "{} {clock}",
        day_label(at.date(), now.date()).replace(',', "")
    )
}

fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    let days = (day - today).num_days();
    match days {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        2..=6 => day.format("%a").to_string(),
        _ if day.year() == today.year() => day.format("%a, %b %-d").to_string(),
        _ => day.format("%a, %b %-d, %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveTime};

    fn t(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    fn named(now: NaiveDateTime) -> Vec<(&'static str, NaiveDateTime)> {
        presets(now)
            .into_iter()
            .map(|(p, at)| (p.title(), at))
            .collect()
    }

    #[test]
    fn a_weekday_morning_offers_all_five() {
        // Wed Oct 28 2026, 10:20.
        assert_eq!(
            named(t(2026, 10, 28, 10, 20)),
            [
                ("Later Today", t(2026, 10, 28, 13, 30)),
                ("This Evening", t(2026, 10, 28, 18, 0)),
                ("Tomorrow", t(2026, 10, 29, 8, 0)),
                ("This Weekend", t(2026, 10, 31, 8, 0)),
                ("Next Week", t(2026, 11, 2, 8, 0)),
            ]
        );
        // On the half hour it stays there.
        assert_eq!(
            presets(t(2026, 10, 28, 10, 30))[0].1,
            t(2026, 10, 28, 13, 30)
        );
    }

    #[test]
    fn past_and_doubled_presets_go() {
        // 4 PM: Later Today would be 7 PM, after This Evening.
        let afternoon = named(t(2026, 10, 28, 16, 0));
        assert_eq!(afternoon[0], ("This Evening", t(2026, 10, 28, 18, 0)));
        // 9 PM: no evening left.
        assert_eq!(named(t(2026, 10, 28, 21, 0))[0].0, "Tomorrow");
        // Late at night, Later Today would cross midnight.
        assert!(
            !named(t(2026, 10, 28, 23, 0))
                .iter()
                .any(|(n, _)| *n == "Later Today")
        );
        // Friday: Tomorrow is the weekend, shown once.
        let friday = named(t(2026, 10, 30, 9, 0));
        assert!(friday.contains(&("Tomorrow", t(2026, 10, 31, 8, 0))));
        assert!(!friday.iter().any(|(n, _)| *n == "This Weekend"));
        // Saturday after 8: this weekend's morning is gone.
        let saturday = named(t(2026, 10, 31, 9, 0));
        assert!(!saturday.iter().any(|(n, _)| *n == "This Weekend"));
        assert!(saturday.contains(&("Next Week", t(2026, 11, 2, 8, 0))));
        // Saturday at 7: still ahead.
        assert!(named(t(2026, 10, 31, 7, 0)).contains(&("This Weekend", t(2026, 10, 31, 8, 0))));
        // Sunday: no weekend, and Next Week is tomorrow.
        let sunday = named(t(2026, 11, 1, 10, 0));
        assert!(!sunday.iter().any(|(n, _)| *n == "This Weekend"));
        assert!(sunday.contains(&("Tomorrow", t(2026, 11, 2, 8, 0))));
        assert!(!sunday.iter().any(|(n, _)| *n == "Next Week"));
        // Across a month and a year.
        assert!(named(t(2026, 12, 31, 20, 0)).contains(&("Tomorrow", t(2027, 1, 1, 8, 0))));
    }

    /// New York in 2026: clocks go forward at 2 AM on March 8 and back at
    /// 2 AM on November 1.
    #[derive(Clone, Copy, Debug)]
    struct NewYork;

    impl TimeZone for NewYork {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            NewYork
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let (est, edt) = (zone(-5), zone(-4));
            let spring = t(2026, 3, 8, 2, 0);
            let fall = t(2026, 11, 1, 1, 0);
            if *local < spring {
                LocalResult::Single(est)
            } else if *local < spring + Duration::hours(1) {
                LocalResult::None
            } else if *local < fall {
                LocalResult::Single(edt)
            } else if *local < fall + Duration::hours(1) {
                LocalResult::Ambiguous(edt, est)
            } else {
                LocalResult::Single(est)
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            if *utc >= t(2026, 3, 8, 7, 0) && *utc < t(2026, 11, 1, 6, 0) {
                zone(-4)
            } else {
                zone(-5)
            }
        }
    }

    fn zone(hours: i32) -> FixedOffset {
        FixedOffset::east_opt(hours * 3600).unwrap()
    }

    fn utc(at: NaiveDateTime) -> i64 {
        at.and_utc().timestamp_millis()
    }

    #[test]
    fn presets_land_on_the_wall_clock_across_dst() {
        // Saturday before spring forward: Next Week's Monday 8 AM is EDT.
        let now = t(2026, 3, 7, 10, 0);
        let (_, monday) = *presets(now).last().unwrap();
        assert_eq!(monday, t(2026, 3, 9, 8, 0));
        assert_eq!(to_ms(&NewYork, monday), utc(t(2026, 3, 9, 12, 0)));
        // Tomorrow from the Saturday is Sunday 8 AM, after the jump: EDT.
        assert_eq!(
            to_ms(&NewYork, t(2026, 3, 8, 8, 0)),
            utc(t(2026, 3, 8, 12, 0))
        );
        // Saturday before fall back: Sunday 8 AM is EST.
        assert_eq!(
            to_ms(&NewYork, t(2026, 11, 1, 8, 0)),
            utc(t(2026, 11, 1, 13, 0))
        );
        // A time the gap skips moves past it; a repeated one is the first.
        assert_eq!(
            to_ms(&NewYork, t(2026, 3, 8, 2, 30)),
            utc(t(2026, 3, 8, 7, 30))
        );
        assert_eq!(
            to_ms(&NewYork, t(2026, 11, 1, 1, 30)),
            utc(t(2026, 11, 1, 5, 30))
        );
    }

    #[test]
    fn typed_times_read_like_quick_add() {
        let now = t(2026, 10, 28, 10, 20); // Wednesday
        assert_eq!(parse("  ", now), Typed::Empty);
        assert_eq!(parse("fri 9am", now), Typed::At(t(2026, 10, 30, 9, 0)));
        assert_eq!(parse("tomorrow", now), Typed::At(t(2026, 10, 29, 8, 0)));
        assert_eq!(
            parse("tomorrow 3pm", now),
            Typed::At(t(2026, 10, 29, 15, 0))
        );
        assert_eq!(parse("6pm", now), Typed::At(t(2026, 10, 28, 18, 0)));
        // A time already gone today is tomorrow's.
        assert_eq!(parse("9am", now), Typed::At(t(2026, 10, 29, 9, 0)));
        assert_eq!(
            parse("nov 12 2:30pm", now),
            Typed::At(t(2026, 11, 12, 14, 30))
        );
        assert_eq!(parse("tonight", now), Typed::At(t(2026, 10, 28, 18, 0)));
        assert_eq!(
            parse("tomorrow morning", now),
            Typed::At(t(2026, 10, 29, 8, 0))
        );
        assert_eq!(parse("in 2 hours", now), Typed::At(t(2026, 10, 28, 12, 20)));
        assert_eq!(parse("in 30m", now), Typed::At(t(2026, 10, 28, 10, 50)));
        assert_eq!(parse("in a week", now), Typed::At(t(2026, 11, 4, 10, 20)));
        assert_eq!(parse("today", now), Typed::Past(t(2026, 10, 28, 8, 0)));
        assert_eq!(parse("whenever", now), Typed::Unknown);
        assert_eq!(parse("fri at Nopa", now), Typed::Unknown);
    }

    #[test]
    fn labels_say_the_day_and_time() {
        // The clock follows the 12/24-hour setting, which other tests set.
        let six = time::time_label(NaiveTime::from_hms_opt(18, 0, 0).unwrap());
        let eight = time::time_label(NaiveTime::from_hms_opt(8, 0, 0).unwrap());
        let now = t(2026, 10, 28, 10, 20);
        assert_eq!(
            when_label(t(2026, 10, 28, 18, 0), now),
            format!("Today, {six}")
        );
        assert_eq!(
            when_label(t(2026, 10, 29, 8, 0), now),
            format!("Tomorrow, {eight}")
        );
        assert_eq!(
            when_label(t(2026, 10, 31, 8, 0), now),
            format!("Sat, {eight}")
        );
        assert_eq!(
            when_label(t(2026, 11, 9, 8, 0), now),
            format!("Mon, Nov 9, {eight}")
        );
        assert_eq!(
            in_sentence(t(2026, 10, 29, 8, 0), now),
            format!("tomorrow, {eight}")
        );
        assert_eq!(
            in_sentence(t(2026, 10, 31, 8, 0), now),
            format!("Sat, {eight}")
        );
        assert_eq!(short_label(t(2026, 10, 28, 18, 0), now), six);
        assert_eq!(
            short_label(t(2026, 11, 9, 8, 0), now),
            format!("Mon Nov 9 {eight}")
        );
    }
}

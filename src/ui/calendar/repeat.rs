//! The repeat editor's rules (CAL-29): the presets offered for an
//! occurrence, and the custom editor's draft. Both only make rules the
//! expander in `calendar::recurrence` supports.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

use crate::calendar::{Rule, recurrence::Freq, time};

/// The weekday's place in its month: 1 to 4, or -1 (last) for a 5th.
fn nth_weekday(date: NaiveDate) -> i8 {
    match date.day0() / 7 + 1 {
        5 => -1,
        n => n as i8,
    }
}

fn weekly_on(day: Weekday) -> Rule {
    Rule {
        by_day: vec![(None, day)],
        ..Rule::weekly()
    }
}

pub(super) fn monthly_on_weekday(start: NaiveDate) -> Rule {
    Rule {
        by_day: vec![(Some(nth_weekday(start)), start.weekday())],
        ..Rule::monthly()
    }
}

pub(super) fn monthly_on_date(start: NaiveDate) -> Rule {
    Rule {
        by_month_day: vec![start.day() as i8],
        ..Rule::monthly()
    }
}

/// The repeat menu for an occurrence on `start`: "Does not repeat", then
/// the presets. "Custom…" follows them in the menu.
pub fn presets(start: NaiveDate) -> Vec<(Option<Rule>, String)> {
    let mut out = vec![(None, "Does not repeat".to_string())];
    for rule in [
        Rule::daily(),
        Rule::weekdays(),
        weekly_on(start.weekday()),
        monthly_on_weekday(start),
        monthly_on_date(start),
    ] {
        let label = rule.label();
        out.push((Some(rule), label));
    }
    out.push((
        Some(Rule::yearly()),
        format!("Every year on {}", start.format("%b %-d")),
    ));
    out
}

/// `rule` with what it leaves to the start said outright, so equal
/// repeats compare equal: a weekly rule's day, a monthly rule's date.
fn spelled_out(rule: &Rule, start: NaiveDate) -> Rule {
    let mut rule = rule.clone();
    match rule.freq {
        Freq::Weekly if rule.by_day.is_empty() => rule.by_day = vec![(None, start.weekday())],
        Freq::Monthly if rule.by_day.is_empty() && rule.by_month_day.is_empty() => {
            rule.by_month_day = vec![start.day() as i8]
        }
        _ => {}
    }
    rule
}

/// Whether `a` and `b` repeat alike, from `start`, whatever their ends.
fn same_repeat(a: &Rule, b: &Rule, start: NaiveDate) -> bool {
    spelled_out(a, start).same_pattern(&spelled_out(b, start))
}

/// The preset in `presets` (from `presets(start)`) matching `rule`, if one
/// does (`0`: no rule).
pub fn preset_index(
    presets: &[(Option<Rule>, String)],
    rule: Option<&Rule>,
    start: NaiveDate,
) -> Option<usize> {
    presets.iter().position(|(preset, _)| match (preset, rule) {
        (None, None) => true,
        (Some(p), Some(r)) => same_repeat(p, r, start),
        _ => false,
    })
}

/// Where a custom repeat ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatEnd {
    Never,
    /// The last day an occurrence may start on.
    On(NaiveDate),
    After(u32),
}

/// The custom repeat editor's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepeatDraft {
    pub freq: Freq,
    pub interval: u32,
    /// Weekly: the days, Monday first.
    pub days: Vec<Weekday>,
    /// Monthly: on the nth weekday rather than on the date.
    pub by_weekday: bool,
    pub end: RepeatEnd,
    /// The occurrence it's set from.
    pub start: NaiveDate,
}

pub const MAX_INTERVAL: u32 = 99;
pub const MAX_COUNT: u32 = 730;

impl RepeatDraft {
    /// From the event's rule; a rule the editor can't show starts it over,
    /// weekly on the occurrence's day.
    pub fn new(rule: Option<&Rule>, start: NaiveDate) -> Self {
        let mut draft = Self {
            freq: Freq::Weekly,
            interval: 1,
            days: vec![start.weekday()],
            by_weekday: false,
            end: RepeatEnd::Never,
            start,
        };
        let Some(rule) = rule.filter(|r| !r.custom && !r.unsupported) else {
            return draft;
        };
        draft.freq = rule.freq;
        draft.interval = rule.interval.clamp(1, MAX_INTERVAL);
        if rule.freq == Freq::Weekly && !rule.by_day.is_empty() {
            draft.days = rule.by_day.iter().map(|&(_, d)| d).collect();
            draft.days.sort_by_key(|d| d.num_days_from_monday());
            draft.days.dedup();
        }
        draft.by_weekday = rule.freq == Freq::Monthly && !rule.by_day.is_empty();
        draft.end = match (rule.count, rule.until) {
            (Some(n), _) => RepeatEnd::After(n.clamp(1, MAX_COUNT)),
            (None, Some(until)) => RepeatEnd::On((until - Duration::seconds(1)).date()),
            (None, None) => RepeatEnd::Never,
        };
        draft
    }

    pub fn to_rule(&self) -> Rule {
        let mut rule = Rule::new(self.freq, self.interval.clamp(1, MAX_INTERVAL));
        match self.freq {
            Freq::Weekly => {
                let mut days = self.days.clone();
                if days.is_empty() {
                    days.push(self.start.weekday());
                }
                days.sort_by_key(|d| d.num_days_from_monday());
                days.dedup();
                rule.by_day = days.into_iter().map(|d| (None, d)).collect();
            }
            Freq::Monthly if self.by_weekday => {
                rule.by_day = vec![(Some(nth_weekday(self.start)), self.start.weekday())];
            }
            Freq::Monthly => rule.by_month_day = vec![self.start.day() as i8],
            Freq::Daily | Freq::Yearly => {}
        }
        match self.end {
            RepeatEnd::Never => {}
            RepeatEnd::After(n) => rule.count = Some(n.clamp(1, MAX_COUNT)),
            RepeatEnd::On(day) => {
                rule.until = Some(time::midnight(day.max(self.start) + Duration::days(1)))
            }
        }
        rule
    }

    pub fn toggle_day(&mut self, day: Weekday) {
        if let Some(ix) = self.days.iter().position(|&d| d == day) {
            // One day at least.
            if self.days.len() > 1 {
                self.days.remove(ix);
            }
        } else {
            self.days.push(day);
            self.days.sort_by_key(|d| d.num_days_from_monday());
        }
    }

    pub fn step_interval(&mut self, delta: i32) {
        self.set_interval(self.interval as i64 + delta as i64);
    }

    /// A typed or stepped interval, kept within 1…`MAX_INTERVAL`.
    pub fn set_interval(&mut self, n: i64) {
        self.interval = n.clamp(1, MAX_INTERVAL as i64) as u32;
    }

    pub fn step_count(&mut self, delta: i32) {
        if let RepeatEnd::After(n) = self.end {
            self.set_count(n as i64 + delta as i64);
        }
    }

    /// A typed or stepped count, kept within 1…`MAX_COUNT`.
    pub fn set_count(&mut self, n: i64) {
        if let RepeatEnd::After(_) = self.end {
            self.end = RepeatEnd::After(n.clamp(1, MAX_COUNT as i64) as u32);
        }
    }

    /// "Never", "On" or "After": the end picked, at its default.
    pub fn set_end_kind(&mut self, kind: usize) {
        self.end = match kind {
            1 => RepeatEnd::On(match self.end {
                RepeatEnd::On(day) => day,
                _ => self.start + Duration::days(90),
            }),
            2 => RepeatEnd::After(match self.end {
                RepeatEnd::After(n) => n,
                _ => 10,
            }),
            _ => RepeatEnd::Never,
        };
    }

    pub fn end_kind(&self) -> usize {
        match self.end {
            RepeatEnd::Never => 0,
            RepeatEnd::On(_) => 1,
            RepeatEnd::After(_) => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::{Event, occurrences, time::LocalZone};

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    fn zone() -> LocalZone {
        LocalZone {
            name: "UTC",
            to_local: |t| t.naive_utc(),
            to_utc: |t| t.and_utc(),
        }
    }

    #[test]
    fn presets_follow_the_occurrence() {
        // Tue Oct 27 2026: the 4th Tuesday; Tue Sep 29: the 5th, so "last".
        let labels: Vec<String> = presets(d(10, 27)).into_iter().map(|(_, l)| l).collect();
        assert_eq!(
            labels,
            [
                "Does not repeat",
                "Every day",
                "Every weekday",
                "Every week on Tue",
                "Every month on the 4th Tue",
                "Every month on day 27",
                "Every year on Oct 27",
            ]
        );
        assert_eq!(presets(d(9, 29))[4].1, "Every month on the last Tue");
    }

    #[test]
    fn the_rule_in_use_finds_its_preset() {
        let start = d(10, 27);
        let list = presets(start);
        assert_eq!(preset_index(&list, None, start), Some(0));
        // The placeholder data's "weekly" leaves the day to the start.
        assert_eq!(preset_index(&list, Some(&Rule::weekly()), start), Some(3));
        assert_eq!(preset_index(&list, Some(&Rule::monthly()), start), Some(5));
        let until = Rule {
            until: Some(time::midnight(d(12, 1))),
            ..Rule::daily()
        };
        assert_eq!(preset_index(&list, Some(&until), start), Some(1));
        assert_eq!(preset_index(&list, Some(&Rule::biweekly()), start), None);
    }

    #[test]
    fn the_monthly_choices_label_their_rules() {
        let start = d(10, 27);
        assert_eq!(
            monthly_on_weekday(start).label(),
            "Every month on the 4th Tue"
        );
        assert_eq!(monthly_on_date(start).label(), "Every month on day 27");
    }

    #[test]
    fn typed_values_are_kept_in_bounds() {
        let mut draft = RepeatDraft::new(None, d(10, 27));
        draft.set_interval(0);
        assert_eq!(draft.interval, 1);
        draft.set_interval(500);
        assert_eq!(draft.interval, MAX_INTERVAL);
        // No count without "After".
        draft.set_count(5);
        assert_eq!(draft.end, RepeatEnd::Never);
        draft.set_end_kind(2);
        draft.set_count(10_000);
        assert_eq!(draft.end, RepeatEnd::After(MAX_COUNT));
        draft.step_count(-10);
        assert_eq!(draft.end, RepeatEnd::After(MAX_COUNT - 10));
    }

    #[test]
    fn a_draft_round_trips_its_rule() {
        let start = d(10, 27);
        let rule = Rule {
            by_day: vec![(None, Weekday::Mon), (None, Weekday::Thu)],
            count: Some(8),
            ..Rule::biweekly()
        };
        let draft = RepeatDraft::new(Some(&rule), start);
        assert_eq!(draft.interval, 2);
        assert_eq!(draft.days, [Weekday::Mon, Weekday::Thu]);
        assert_eq!(draft.end, RepeatEnd::After(8));
        assert_eq!(draft.to_rule(), rule);

        let monthly = Rule {
            until: Some(time::midnight(d(12, 31))),
            ..monthly_on_weekday(start)
        };
        let draft = RepeatDraft::new(Some(&monthly), start);
        assert!(draft.by_weekday);
        assert_eq!(draft.end, RepeatEnd::On(d(12, 30)));
        assert_eq!(draft.to_rule(), monthly);
    }

    #[test]
    fn the_draft_edits_within_bounds() {
        let mut draft = RepeatDraft::new(None, d(10, 27));
        assert_eq!(draft.days, [Weekday::Tue]);
        draft.toggle_day(Weekday::Tue);
        assert_eq!(draft.days, [Weekday::Tue], "one day at least");
        draft.toggle_day(Weekday::Fri);
        draft.toggle_day(Weekday::Mon);
        assert_eq!(draft.days, [Weekday::Mon, Weekday::Tue, Weekday::Fri]);
        draft.step_interval(-3);
        assert_eq!(draft.interval, 1);
        draft.set_end_kind(2);
        draft.step_count(-20);
        assert_eq!(draft.end, RepeatEnd::After(1));
        // An end before the start is the start.
        draft.end = RepeatEnd::On(d(10, 1));
        assert_eq!(draft.to_rule().until, Some(time::midnight(d(10, 28))));
        // A custom rule from Google starts over.
        let custom = Rule {
            custom: true,
            raw: Some("FREQ=YEARLY;BYMONTH=11;BYDAY=4TH".into()),
            ..Rule::yearly()
        };
        assert_eq!(
            RepeatDraft::new(Some(&custom), d(10, 27)).freq,
            Freq::Weekly
        );
    }

    #[test]
    fn every_rule_made_here_expands_and_writes() {
        let start = d(10, 27);
        let mut rules: Vec<Rule> = presets(start).into_iter().filter_map(|(r, _)| r).collect();
        let mut draft = RepeatDraft::new(None, start);
        draft.by_weekday = true;
        for freq in [Freq::Daily, Freq::Weekly, Freq::Monthly, Freq::Yearly] {
            draft.freq = freq;
            draft.interval = 3;
            draft.set_end_kind(2);
            rules.push(draft.to_rule());
        }
        for rule in rules {
            let event = Event {
                id: "e".into(),
                start: time::at(start, 9, 0),
                end: time::at(start, 10, 0),
                recurrence: Some(rule.clone()),
                ..Event::default()
            };
            let found = occurrences(
                std::slice::from_ref(&event),
                |_| true,
                time::midnight(start),
                time::midnight(start + Duration::days(2000)),
            );
            assert!(found.len() > 1, "{rule:?} expands");
            let written = rule.to_rrule(false, &zone());
            let value = written.strip_prefix("RRULE:").unwrap_or(&written);
            let back = Rule::parse(value, false, &zone()).unwrap();
            assert!(!back.custom && !back.unsupported, "{written} reads back");
            assert!(back.same_pattern(&rule), "{written}");
        }
    }
}

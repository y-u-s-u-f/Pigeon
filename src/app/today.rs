//! What the menu bar shows, and the reminders (sprint 2 W7b, NTF-4,
//! CAL-15): kept app-wide, so both work with the window closed. The
//! calendar and the latest Primary Inbox threads are read from the backend
//! when it reports a change (debounced), not on every tick. A tick at each
//! minute boundary fires the reminders that fell due and lets the menu bar
//! redraw its countdown.
//!
//! The decisions themselves are plain functions, tested below: what the
//! panel lists (`agenda`), the title beside the icon (`status_title`), the
//! new-mail dot (`has_new_mail`) and quick join (`quick_join`).

use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDateTime, Timelike as _};
use gpui::{App, AppContext as _, Context, Entity, Global, Task};

use crate::{
    backend::{
        Backend, BackendEvent,
        categories::{Category, in_category},
    },
    calendar::{
        CalendarData, GoogleSettings, Occurrence, occurrences,
        policy::{self, next_joinable},
        reminders::{DueReminder, ReminderKey, Scheduler},
        time,
    },
    data::{Account, Folder, Thread},
    logging,
    notify::{self, Notice, NoticeTarget},
};

/// How far ahead the next meeting shows beside the icon.
pub const TITLE_AHEAD: Duration = Duration::hours(3);
/// Characters of the meeting's title beside the icon.
pub const TITLE_MAX: usize = 20;
/// Quick join opens a meeting this soon, or one under way.
pub const QUICK_JOIN_AHEAD: Duration = Duration::minutes(10);
/// Changes reported this close together are read once.
const RELOAD_DEBOUNCE: StdDuration = StdDuration::from_millis(250);

/// One Primary Inbox conversation, as the panel lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct MailRow {
    pub id: String,
    pub sender: String,
    pub subject: String,
    pub date: DateTime<FixedOffset>,
    pub internal_date: i64,
    pub unread: bool,
}

/// The Inbox's Primary conversations, newest first.
pub fn mail_rows(threads: &[Thread], me: &Account) -> Vec<MailRow> {
    let mut rows: Vec<MailRow> = threads
        .iter()
        .filter(|t| t.folder == Folder::Inbox && !t.messages.is_empty())
        .filter(|t| in_category(t, Category::Primary))
        .map(|t| {
            let last = t.last_message();
            // The latest sender who isn't the user, as the list shows it.
            let from = t
                .messages
                .iter()
                .rev()
                .map(|m| &m.from)
                .find(|c| !me.is_me(&c.email))
                .unwrap_or(&last.from);
            MailRow {
                id: t.id.clone(),
                sender: from.display_name().to_string(),
                subject: if t.subject.trim().is_empty() {
                    "(no subject)".into()
                } else {
                    t.subject.clone()
                },
                date: last.date,
                internal_date: last.internal_date,
                unread: t.unread,
            }
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.internal_date));
    rows
}

/// The newest mail's time, for `has_new_mail`.
pub fn newest(rows: &[MailRow]) -> i64 {
    rows.iter().map(|r| r.internal_date).max().unwrap_or(0)
}

/// The dot on the icon: unread Primary mail newer than what the panel
/// showed when it was last opened.
pub fn has_new_mail(rows: &[MailRow], seen: i64) -> bool {
    rows.iter().any(|r| r.unread && r.internal_date > seen)
}

/// What the panel lists: the meeting to join on top, then the rest of
/// today, or tomorrow's first event when nothing else is left today.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agenda {
    pub next: Option<Occurrence>,
    /// All-day events first, then the timed ones by start.
    pub later: Vec<Occurrence>,
    pub tomorrow: Option<Occurrence>,
}

pub fn agenda(data: &CalendarData, now: NaiveDateTime) -> Agenda {
    let today = now.date();
    let day_end = time::midnight(today + Duration::days(1));
    let next = next_joinable(data, now);
    let is_next = |o: &Occurrence| next.is_some_and(|n| n.event == o.event && n.start == o.start);
    let all_day = |o: &Occurrence| data.events[o.event].all_day;
    let mut later: Vec<Occurrence> =
        occurrences(&data.events, |e| policy::counts(data, e), now, day_end)
            .into_iter()
            .filter(|o| !is_next(o))
            // An all-day event from an earlier day still running isn't today's.
            .filter(|o| !all_day(o) || o.start.date() == today)
            .collect();
    later.sort_by_key(|o| (!all_day(o), o.start));
    let next_today = next.is_some_and(|n| n.start < day_end);
    let tomorrow = if later.iter().any(|o| !all_day(o)) || next_today {
        None
    } else {
        let mut candidates = occurrences(
            &data.events,
            |e| policy::counts(data, e),
            day_end,
            day_end + Duration::days(1),
        );
        candidates.retain(|o| o.start >= day_end);
        candidates.sort_by_key(|o| (all_day(o), o.start));
        candidates.into_iter().next().filter(|o| !is_next(o))
    };
    Agenda {
        next,
        later,
        tomorrow,
    }
}

/// "12m", "1h", "2h 5m": whole minutes from `now` (to the minute) to `at`.
pub fn countdown(now: NaiveDateTime, at: NaiveDateTime) -> String {
    let minutes = (at - floor_minute(now)).num_minutes().max(1);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

fn floor_minute(t: NaiveDateTime) -> NaiveDateTime {
    t.with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}

/// `text` cut to `max` characters, with an ellipsis when it was longer.
pub fn truncate(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Beside the icon: "Design review in 12m", "Join · Design review" while
/// it runs, nothing without a meeting in the next `TITLE_AHEAD`.
pub fn status_title(data: &CalendarData, now: NaiveDateTime) -> Option<String> {
    let next = next_joinable(data, now)?;
    let title = truncate(data.events[next.event].display_title(), TITLE_MAX);
    if next.start <= now {
        Some(format!("Join · {title}"))
    } else if next.start - now <= TITLE_AHEAD {
        Some(format!("{title} in {}", countdown(now, next.start)))
    } else {
        None
    }
}

/// The meeting quick join opens, with its link: under way, or starting
/// within `QUICK_JOIN_AHEAD`.
pub fn quick_join(data: &CalendarData, now: NaiveDateTime) -> Option<(Occurrence, String)> {
    let next = next_joinable(data, now)?;
    if next.start - now > QUICK_JOIN_AHEAD {
        return None;
    }
    let url = policy::video_link(&data.events[next.event])?;
    Some((next, url.to_string()))
}

/// The video service a link goes to, for the panel's card.
pub fn video_service(url: &str) -> &'static str {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host == "meet.google.com" {
        "Google Meet"
    } else if host.ends_with("zoom.us") {
        "Zoom"
    } else if host.starts_with("teams.") {
        "Microsoft Teams"
    } else if host.ends_with("webex.com") {
        "Webex"
    } else {
        "Video call"
    }
}

/// The notification for a reminder that fell due. A newer reminder of the
/// same occurrence replaces it.
pub fn reminder_notice(reminder: &DueReminder) -> Notice {
    let key = &reminder.key;
    let start = key.start;
    let body = if reminder.all_day {
        format!(
            "All day · {}",
            time::date_label(start.date(), reminder.at.date())
        )
    } else {
        let when = if key.minutes == 0 {
            "Now".to_string()
        } else {
            format!(
                "In {}",
                time::duration_label(Duration::minutes(key.minutes as i64))
            )
        };
        format!(
            "{when} · {}",
            time::time_range_label(start.time(), reminder.end.time())
        )
    };
    let body = if reminder.location.trim().is_empty() {
        body
    } else {
        format!("{body}\n{}", reminder.location.trim())
    };
    Notice {
        tag: format!(
            "event-{}-{}-{}",
            key.calendar,
            key.event,
            start.format("%Y%m%dT%H%M")
        ),
        title: reminder.title.clone(),
        body,
        target: NoticeTarget::Event {
            calendar: key.calendar.clone(),
            id: key.event.clone(),
            start,
        },
        join_url: reminder.join_url.clone(),
    }
}

/// The reminder scheduler and its sleep-aware clock (CAL-15).
pub struct Reminders {
    scheduler: Scheduler,
    started: Instant,
}

impl Reminders {
    pub fn new(fired: std::collections::HashSet<ReminderKey>) -> Self {
        Self {
            scheduler: Scheduler::new(fired),
            started: Instant::now(),
        }
    }

    /// Hand each reminder due since the last tick to `sink`. `mono` stops
    /// while the Mac sleeps.
    pub fn tick(
        &mut self,
        data: &CalendarData,
        responded_only: bool,
        wall: NaiveDateTime,
        mono: StdDuration,
        sink: &mut dyn FnMut(&ReminderKey, Notice),
    ) {
        for due in self.scheduler.tick(data, responded_only, wall, mono) {
            sink(&due.key, reminder_notice(&due));
        }
    }
}

pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// The app-wide model the menu bar reads; it notifies on every change and
/// at each minute boundary.
pub struct Today {
    backend: Entity<Backend>,
    pub calendar: CalendarData,
    pub mail: Vec<MailRow>,
    google: GoogleSettings,
    reminders: Reminders,
    /// The newest mail when the panel was last opened; none until the
    /// first read, which counts as seen.
    seen: Option<i64>,
    /// Reads asked for and not done yet.
    mail_due: bool,
    calendar_due: bool,
    _reload: Option<Task<()>>,
    _load: Option<Task<()>>,
    _tick: Task<()>,
}

struct TodayModel(Entity<Today>);

impl Global for TodayModel {}

/// Start the model on `backend`. Once, at launch.
pub fn init(backend: Entity<Backend>, cx: &mut App) -> Entity<Today> {
    let today = cx.new(|cx| Today::new(backend.clone(), cx));
    cx.subscribe(&backend, {
        let today = today.downgrade();
        move |_, event: &BackendEvent, cx| {
            today.update(cx, |t, cx| t.on_backend_event(event, cx)).ok();
        }
    })
    .detach();
    cx.set_global(TodayModel(today.clone()));
    today
}

pub fn model(cx: &App) -> Option<Entity<Today>> {
    cx.try_global::<TodayModel>().map(|m| m.0.clone())
}

impl Today {
    fn new(backend: Entity<Backend>, cx: &mut Context<Self>) -> Self {
        let (calendar, google, fired) = {
            let b = backend.read(cx);
            (
                b.load_calendar(),
                b.calendar_settings(),
                b.fired_reminders(),
            )
        };
        let tick = cx.spawn(async |this, cx| {
            loop {
                // Just past each minute boundary, when reminders fall due.
                let into = Local::now().second() as u64;
                cx.background_executor()
                    .timer(StdDuration::from_secs(61 - into.min(60)))
                    .await;
                if this.update(cx, |this, cx| this.minute(cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            backend,
            calendar,
            mail: Vec::new(),
            google,
            reminders: Reminders::new(fired),
            seen: None,
            mail_due: true,
            calendar_due: false,
            _reload: None,
            _load: None,
            _tick: tick,
        };
        // The first tick only starts the clock: nothing missed fires.
        this.check_reminders(cx);
        this.reload(cx);
        this
    }

    fn on_backend_event(&mut self, event: &BackendEvent, cx: &mut Context<Self>) {
        let (mail, calendar) = match event {
            BackendEvent::ThreadsChanged(_)
            | BackendEvent::ThreadsRemoved(_)
            | BackendEvent::IdReplaced { .. }
            | BackendEvent::CountsChanged
            | BackendEvent::OpFailed { .. } => (true, false),
            BackendEvent::EventsChanged(_)
            | BackendEvent::EventsRemoved(_)
            | BackendEvent::CalendarsChanged => (false, true),
            // Demo commits change the store without saying what.
            BackendEvent::OpCommitted(_) => (true, true),
            BackendEvent::SignedIn | BackendEvent::SignedOut => {
                let fired = self.backend.read(cx).fired_reminders();
                self.reminders = Reminders::new(fired);
                self.check_reminders(cx);
                self.seen = None;
                (true, true)
            }
            BackendEvent::Status(_) => {
                cx.notify();
                (false, false)
            }
            _ => (false, false),
        };
        if mail || calendar {
            self.mail_due |= mail;
            self.calendar_due |= calendar;
            self._reload = Some(cx.spawn(async |this, cx| {
                cx.background_executor().timer(RELOAD_DEBOUNCE).await;
                this.update(cx, |this, cx| this.reload(cx)).ok();
            }));
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.calendar_due) {
            let b = self.backend.read(cx);
            self.calendar = b.load_calendar();
            self.google = b.calendar_settings();
            cx.notify();
        }
        if std::mem::take(&mut self.mail_due) {
            let load = self
                .backend
                .update(cx, |b, cx| b.load_threads_in_background(cx));
            self._load = Some(cx.spawn(async move |this, cx| {
                let threads = load.await;
                this.update(cx, |this, cx| {
                    let me = this.backend.read(cx).account().cloned().unwrap_or_default();
                    this.mail = mail_rows(&threads, &me);
                    this.seen.get_or_insert(newest(&this.mail));
                    cx.notify();
                })
                .ok();
            }));
        }
    }

    fn minute(&mut self, cx: &mut Context<Self>) {
        self.check_reminders(cx);
        cx.notify();
    }

    /// Notify the reminders that fell due since the last tick (CAL-15).
    fn check_reminders(&mut self, cx: &mut Context<Self>) {
        let mut due = Vec::new();
        let mono = self.reminders.started.elapsed();
        self.reminders.tick(
            &self.calendar,
            self.google.remind_responded_only,
            now(),
            mono,
            &mut |key, notice| due.push((key.clone(), notice)),
        );
        for (key, notice) in due {
            self.backend.read(cx).mark_reminder_fired(&key);
            logging::info(&format!(
                "reminder: {} at {} ({} min before)",
                notice.title, key.start, key.minutes
            ));
            notify::notify(notice, cx);
        }
    }

    pub fn backend(&self) -> &Entity<Backend> {
        &self.backend
    }

    pub fn has_new_mail(&self) -> bool {
        self.seen.is_some_and(|seen| has_new_mail(&self.mail, seen))
    }

    /// The panel opened: what it shows now is seen.
    pub fn mark_seen(&mut self, cx: &mut Context<Self>) {
        let newest = newest(&self.mail);
        if self.seen != Some(newest) {
            self.seen = Some(newest);
            cx.notify();
        }
    }

    /// A change made from the panel, shown at once (the store has it once
    /// the backend overlays the queue).
    pub fn update_mail(&mut self, id: &str, f: impl FnOnce(&mut Vec<MailRow>, usize)) {
        if let Some(ix) = self.mail.iter().position(|r| r.id == id) {
            f(&mut self.mail, ix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calendar::{AccessRole, Attendee, Calendar, Event, Reminder, RsvpStatus, time::at},
        data::{Body, Contact, Message},
    };
    use chrono::NaiveDate;

    fn d(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, n).unwrap()
    }

    fn data(events: Vec<Event>) -> CalendarData {
        CalendarData {
            account: Account {
                name: "Me".into(),
                email: "me@example.com".into(),
                aliases: Vec::new(),
                send_as: Vec::new(),
            },
            calendars: vec![Calendar {
                id: "work".into(),
                name: "Work".into(),
                color: gpui::hsla(0., 0., 0., 1.),
                visible: true,
                access_role: AccessRole::Owner,
                primary: true,
                time_zone: None,
                default_reminder: Some(10),
                default_reminders: vec![Reminder::popup(10)],
                conference_types: Vec::new(),
            }],
            events,
        }
    }

    fn event(id: &str, start: NaiveDateTime, minutes: i64) -> Event {
        Event {
            id: id.into(),
            calendar: "work".into(),
            title: id.into(),
            start,
            end: start + Duration::minutes(minutes),
            organizer_self: true,
            reminder_use_default: true,
            ..Event::default()
        }
    }

    fn meeting(id: &str, start: NaiveDateTime, minutes: i64) -> Event {
        Event {
            conference_uri: format!("https://meet.google.com/{id}"),
            ..event(id, start, minutes)
        }
    }

    fn all_day(id: &str, day: NaiveDate) -> Event {
        Event {
            all_day: true,
            ..event(id, time::midnight(day), 24 * 60)
        }
    }

    fn ids(data: &CalendarData, list: &[Occurrence]) -> Vec<String> {
        list.iter()
            .map(|o| data.events[o.event].id.clone())
            .collect()
    }

    #[test]
    fn agenda_puts_the_next_meeting_on_top_and_the_rest_of_today_below() {
        let now = at(d(27), 10, 48);
        let data = data(vec![
            meeting("design", at(d(27), 11, 0), 45),
            event("lunch", at(d(27), 12, 30), 60),
            event("over", at(d(27), 9, 0), 30),
            all_day("holiday", d(27)),
            event("tomorrow", at(d(28), 9, 0), 30),
            all_day("yesterday", d(26)),
        ]);
        let agenda = agenda(&data, now);
        assert_eq!(data.events[agenda.next.unwrap().event].id, "design");
        // All day first; neither the card's meeting nor what's over.
        assert_eq!(ids(&data, &agenda.later), ["holiday", "lunch"]);
        assert_eq!(agenda.tomorrow, None);
    }

    #[test]
    fn an_empty_day_shows_tomorrows_first_event() {
        let now = at(d(27), 18, 0);
        let data = data(vec![
            all_day("holiday", d(27)),
            all_day("trip", d(28)),
            event("standup", at(d(28), 9, 30), 15),
            event("review", at(d(28), 14, 0), 60),
        ]);
        let agenda = agenda(&data, now);
        assert_eq!(agenda.next, None);
        // Only all-day left today: the day counts as empty.
        assert_eq!(ids(&data, &agenda.later), ["holiday"]);
        let tomorrow = agenda.tomorrow.unwrap();
        assert_eq!(data.events[tomorrow.event].id, "standup", "timed first");
        // Only all-day tomorrow: that one.
        let data2 = super::tests::data(vec![all_day("trip", d(28))]);
        let tomorrow = super::agenda(&data2, now).tomorrow.unwrap();
        assert_eq!(data2.events[tomorrow.event].id, "trip");
        // The card's meeting isn't listed again as tomorrow's.
        let data3 = super::tests::data(vec![meeting("standup", at(d(28), 9, 30), 15)]);
        let agenda = super::agenda(&data3, now);
        assert!(agenda.next.is_some() && agenda.tomorrow.is_none());
    }

    #[test]
    fn status_title_counts_down_then_offers_join() {
        let mut long = meeting("x", at(d(27), 11, 0), 45);
        long.title = "Quarterly planning with the whole design team".into();
        let data = data(vec![long]);
        assert_eq!(
            status_title(&data, at(d(27), 10, 48)).as_deref(),
            Some("Quarterly planning… in 12m")
        );
        // Seconds don't count: still 12 minutes at 10:48:40.
        let now = at(d(27), 10, 48) + Duration::seconds(40);
        assert_eq!(countdown(now, at(d(27), 11, 0)), "12m");
        assert_eq!(countdown(at(d(27), 9, 0), at(d(27), 11, 0)), "2h");
        assert_eq!(countdown(at(d(27), 8, 55), at(d(27), 11, 0)), "2h 5m");
        assert_eq!(
            status_title(&data, at(d(27), 11, 5)).as_deref(),
            Some("Join · Quarterly planning…")
        );
        // More than three hours ahead: nothing.
        assert_eq!(status_title(&data, at(d(27), 7, 59)), None);
        assert!(status_title(&data, at(d(27), 8, 0)).is_some());
        assert_eq!(status_title(&data, at(d(27), 11, 45)), None, "over");
        assert_eq!(truncate("Standup", TITLE_MAX), "Standup");
        assert_eq!(truncate("abcdef", 4), "abc…");
    }

    #[test]
    fn quick_join_within_ten_minutes_or_running() {
        let data = data(vec![meeting("sync", at(d(27), 11, 0), 30)]);
        assert_eq!(quick_join(&data, at(d(27), 10, 49)), None);
        let (o, url) = quick_join(&data, at(d(27), 10, 50)).unwrap();
        assert_eq!(url, "https://meet.google.com/sync");
        assert_eq!(o.start, at(d(27), 11, 0));
        assert!(quick_join(&data, at(d(27), 11, 20)).is_some(), "running");
        assert_eq!(quick_join(&data, at(d(27), 11, 30)), None, "over");
        assert_eq!(super::tests::data(Vec::new()).events.len(), 0);
        assert_eq!(
            quick_join(&super::tests::data(Vec::new()), at(d(27), 11, 0)),
            None
        );
        assert_eq!(video_service(&url), "Google Meet");
        assert_eq!(video_service("https://acme.zoom.us/j/1"), "Zoom");
    }

    fn thread(id: &str, minutes: i64, unread: bool, labels: &[&str]) -> Thread {
        let date = DateTime::parse_from_rfc3339("2026-10-27T10:00:00-07:00").unwrap()
            + Duration::minutes(minutes);
        let mut t: Thread = serde_json::from_value(serde_json::json!({
            "id": id, "subject": format!("Subject {id}"), "folder": "inbox",
            "labels": [], "starred": false, "unread": unread, "messages": [],
        }))
        .unwrap();
        t.messages.push(Message {
            id: format!("m-{id}"),
            label_ids: labels.iter().map(|l| l.to_string()).collect(),
            from: Contact {
                name: format!("Sender {id}"),
                email: format!("{id}@example.com"),
            },
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            date: date.fixed_offset(),
            internal_date: date.timestamp_millis(),
            snippet: String::new(),
            body: Body::NotLoaded,
            attachments: Vec::new(),
            invite: None,
            message_id: String::new(),
            in_reply_to: String::new(),
            references: Vec::new(),
            reply_to: Vec::new(),
            size_estimate: 0,
        });
        t
    }

    #[test]
    fn recent_mail_is_primary_inbox_and_the_dot_is_for_newer_unread() {
        let mut archived = thread("archived", 5, true, &["INBOX"]);
        archived.folder = Folder::Archive;
        let threads = vec![
            thread("old", 0, false, &["INBOX"]),
            thread("social", 10, true, &["INBOX", "CATEGORY_SOCIAL"]),
            thread("new", 20, true, &["INBOX", "UNREAD"]),
            archived,
        ];
        let rows = mail_rows(&threads, &Account::default());
        let names: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(names, ["new", "old"]);
        assert_eq!(rows[0].sender, "Sender new");
        // Seen up to "old": "new" is news, unread.
        assert!(has_new_mail(&rows, rows[1].internal_date));
        assert!(!has_new_mail(&rows, newest(&rows)));
        // Read already: no dot.
        let mut read = rows.clone();
        read[0].unread = false;
        assert!(!has_new_mail(&read, rows[1].internal_date));
    }

    #[test]
    fn reminders_fire_through_the_app_level_path() {
        let mut design = meeting("design", at(d(27), 11, 0), 45);
        design.location = "Room 3B".into();
        let mut declined = event("declined", at(d(27), 11, 0), 30);
        declined.organizer_self = false;
        declined.organizer = Contact {
            name: String::new(),
            email: "boss@example.com".into(),
        };
        declined.attendees = vec![Attendee {
            name: String::new(),
            email: "me@example.com".into(),
            status: RsvpStatus::Declined,
        }];
        let data = data(vec![design, declined]);
        let mut reminders = Reminders::new(Default::default());
        let mut sink: Vec<(ReminderKey, Notice)> = Vec::new();
        // A fake clock: one tick a minute from 10:45, the clocks in step.
        for m in 0..=20 {
            reminders.tick(
                &data,
                false,
                at(d(27), 10, 45) + Duration::minutes(m),
                StdDuration::from_secs(500 + 60 * m as u64),
                &mut |key, notice| sink.push((key.clone(), notice)),
            );
        }
        assert_eq!(sink.len(), 1, "one reminder, not the declined one");
        let (key, notice) = &sink[0];
        assert_eq!((key.event.as_str(), key.minutes), ("design", 10));
        assert_eq!(notice.title, "design");
        assert!(notice.body.starts_with("In 10 min · "), "{}", notice.body);
        assert!(notice.body.ends_with("\nRoom 3B"));
        assert_eq!(
            notice.join_url.as_deref(),
            Some("https://meet.google.com/design")
        );
        assert_eq!(
            notice.target,
            NoticeTarget::Event {
                calendar: "work".into(),
                id: "design".into(),
                start: at(d(27), 11, 0),
            }
        );
        assert_eq!(notice.tag, "event-work-design-20261027T1100");
    }
}

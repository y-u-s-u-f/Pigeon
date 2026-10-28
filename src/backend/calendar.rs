//! Google Calendar: the API types and calls, and how Google's calendars and
//! events map onto Pigeon's model.
//!
//! Reads (W4): `sync_pass` brings the store up to date with one call, off
//! the main thread (SYN-4, docs/BACKEND_SPRINT_1.md 2.6):
//!
//! - `colors.get` once, cached in `sync_state`.
//! - `calendarList.list`, with its sync token after the first run.
//! - Per shown calendar, `events.list singleEvents=false` in the local zone:
//!   the last 180 days onward on a full sync, then by sync token. A 410
//!   wipes the calendar's events and syncs it in full again.
//! - Series keep their RRULE and are expanded locally; their exceptions are
//!   the EXDATEs plus every instance Google lists on its own (moved or
//!   cancelled). Rules outside the subset `recurrence` expands store
//!   Google's own occurrences (`events.instances`) instead.
//!
//! Times are wall-clock times in the user's zone (6.4 item 8).
//!
//! Writes (W7): `commit` sends one `CalOp` to Google, also off the main
//! thread. See its block below.

use std::collections::{BTreeSet, HashMap};

use anyhow::{Context as _, anyhow};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use super::{
    BackendEvent, CalOp, SendUpdates,
    http::{self, Request, Response, Transport, TransportError, encode as enc},
    store::Store,
};
use crate::{
    calendar::{
        AccessRole, Attendee, Calendar, Event, EventStatus, GoogleSettings, Reminder,
        ReminderMethod, RsvpStatus, Rule, SeriesScope,
        recurrence::{ical_until, parse_recurrence},
        reminders::ReminderKey,
        time::{LocalZone, midnight},
    },
    data::Contact,
};

pub const API: &str = "https://www.googleapis.com/calendar/v3";

/// Calendar's quota counts requests, not units: each call costs one.
const COST: u32 = 1;
/// How far back a full sync reaches.
const PAST_DAYS: i64 = 180;
/// How far ahead Google's own occurrences of an unsupported rule are kept.
const INSTANCES_AHEAD_DAYS: i64 = 400;

const COLORS_KEY: &str = "calendar_colors";
const LIST_TOKEN_KEY: &str = "calendar_list_sync_token";

/// What a sync pass changed, for the app to reload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CalendarChanges {
    /// The calendar list (names, colours, visibility) changed.
    pub calendars: bool,
    /// Events to reload, by id.
    pub changed: Vec<String>,
    /// Events gone, by id.
    pub removed: Vec<String>,
    /// Calendars that couldn't be synced this time, and why. The rest were.
    pub errors: Vec<String>,
}

impl CalendarChanges {
    pub fn is_empty(&self) -> bool {
        !self.calendars && self.changed.is_empty() && self.removed.is_empty()
    }

    /// The events a sync or write touched, split into those still stored
    /// (changed) and those gone (removed), in id order.
    fn split_touched(
        &mut self,
        store: &Store,
        account: &str,
        touched: BTreeSet<String>,
    ) -> anyhow::Result<()> {
        let ids: Vec<String> = touched.into_iter().collect();
        let present = store.existing_event_ids(account, &ids)?;
        for id in ids {
            if present.contains(&id) {
                self.changed.push(id);
            } else {
                self.removed.push(id);
            }
        }
        Ok(())
    }

    /// The events to emit, in order: calendars first, so events find theirs.
    pub fn backend_events(&self) -> Vec<BackendEvent> {
        let mut out = Vec::new();
        if self.calendars {
            out.push(BackendEvent::CalendarsChanged);
        }
        if !self.changed.is_empty() {
            out.push(BackendEvent::EventsChanged(self.changed.clone()));
        }
        if !self.removed.is_empty() {
            out.push(BackendEvent::EventsRemoved(self.removed.clone()));
        }
        out
    }
}

// API types: only the fields Pigeon reads. Events are kept whole as JSON
// too, for writes.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default)]
    next_sync_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalendarListEntry {
    pub id: String,
    pub summary: String,
    pub summary_override: Option<String>,
    pub color_id: Option<String>,
    pub background_color: Option<String>,
    pub selected: bool,
    pub hidden: bool,
    pub deleted: bool,
    pub access_role: String,
    pub primary: bool,
    pub time_zone: Option<String>,
    pub default_reminders: Vec<GoogleReminder>,
    pub conference_properties: ConferenceProperties,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConferenceProperties {
    pub allowed_conference_solution_types: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleReminder {
    pub method: String,
    pub minutes: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleEvent {
    pub id: String,
    pub etag: String,
    pub status: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub html_link: Option<String>,
    #[serde(rename = "iCalUID")]
    pub ical_uid: Option<String>,
    pub start: Option<GoogleTime>,
    pub end: Option<GoogleTime>,
    pub original_start_time: Option<GoogleTime>,
    pub recurring_event_id: Option<String>,
    pub recurrence: Vec<String>,
    pub attendees: Vec<GoogleAttendee>,
    pub attendees_omitted: bool,
    pub organizer: Option<GooglePerson>,
    pub guests_can_modify: bool,
    pub transparency: Option<String>,
    pub event_type: Option<String>,
    pub hangout_link: Option<String>,
    pub conference_data: Option<GoogleConference>,
    pub reminders: Option<GoogleReminders>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleTime {
    pub date: Option<String>,
    pub date_time: Option<String>,
    pub time_zone: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleAttendee {
    pub email: String,
    pub display_name: Option<String>,
    pub response_status: String,
    pub resource: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GooglePerson {
    pub email: String,
    pub display_name: Option<String>,
    #[serde(rename = "self")]
    pub is_self: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleConference {
    pub entry_points: Vec<GoogleEntryPoint>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleEntryPoint {
    pub entry_point_type: String,
    pub uri: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleReminders {
    pub use_default: bool,
    pub overrides: Vec<GoogleReminder>,
}

// Mapping.

/// A calendar list entry as a `Calendar`. The colour is the entry's own,
/// else its palette colour from `colors.get`.
pub fn calendar_from_entry(
    entry: &CalendarListEntry,
    palette: &HashMap<String, String>,
) -> Calendar {
    let color = entry
        .background_color
        .as_deref()
        .or_else(|| {
            entry
                .color_id
                .as_ref()
                .and_then(|id| palette.get(id))
                .map(String::as_str)
        })
        .and_then(crate::data::from_hex)
        .unwrap_or(gpui::hsla(0.6, 0.1, 0.5, 1.));
    Calendar {
        id: entry.id.clone(),
        name: entry
            .summary_override
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| entry.summary.clone()),
        color,
        visible: entry.selected,
        access_role: AccessRole::from_key(&entry.access_role),
        primary: entry.primary,
        time_zone: entry.time_zone.clone(),
        default_reminder: earliest_reminder(&entry.default_reminders),
        default_reminders: reminders(&entry.default_reminders),
        conference_types: entry
            .conference_properties
            .allowed_conference_solution_types
            .clone(),
    }
}

/// The reminder furthest ahead of the event, in minutes.
fn earliest_reminder(reminders: &[GoogleReminder]) -> Option<u32> {
    reminders.iter().map(|r| r.minutes).max()
}

/// Google's reminders, all of them (CAL-33).
fn reminders(reminders: &[GoogleReminder]) -> Vec<Reminder> {
    reminders
        .iter()
        .map(|r| Reminder {
            minutes: r.minutes,
            method: ReminderMethod::from_key(&r.method),
        })
        .collect()
}

/// One item of an event listing.
#[allow(clippy::large_enum_variant)] // Short-lived, one at a time.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// An event, a series, or one instance of a series changed or
    /// cancelled on its own (`status` says which).
    Event(Event),
    /// An event or series that's gone (or one Pigeon doesn't show).
    Deleted(String),
}

/// A Google event in `calendar` as Pigeon's. `default_reminder` is the
/// calendar's, for events that use it.
pub fn event_from_google(
    raw: &Value,
    calendar: &str,
    default_reminder: Option<u32>,
    zone: &LocalZone,
) -> Option<Item> {
    let g = GoogleEvent::deserialize(raw).ok()?;
    let status = match g.status.as_str() {
        "cancelled" => EventStatus::Cancelled,
        "tentative" => EventStatus::Tentative,
        _ => EventStatus::Confirmed,
    };
    let original_start = g
        .original_start_time
        .as_ref()
        .and_then(|t| wall_time(t, zone))
        .map(|(t, _)| t);
    // Working locations aren't events in Google Calendar's own views.
    if g.event_type.as_deref() == Some("workingLocation") {
        return Some(Item::Deleted(g.id));
    }
    if status == EventStatus::Cancelled {
        // A cancelled instance stays as an exception of its series; the
        // rest are gone. Google sends only the ids for those.
        return Some(match (g.recurring_event_id, original_start) {
            (Some(series), Some(at)) => Item::Event(Event {
                id: g.id,
                calendar: calendar.to_string(),
                start: at,
                end: at,
                recurring_event_id: Some(series),
                original_start: Some(at),
                status,
                etag: g.etag,
                ..Event::default()
            }),
            _ => Item::Deleted(g.id),
        });
    }
    let (start, all_day) = wall_time(g.start.as_ref()?, zone)?;
    let (end, _) = wall_time(g.end.as_ref()?, zone)?;
    let mut recurrence = parse_recurrence(&g.recurrence, all_day, zone);
    // A timed series kept in another zone moves against this one's wall
    // clock across DST changes, and its days can fall on other dates
    // here: Google's own occurrences are right, local expansion isn't.
    if let Some(rule) = &mut recurrence.rule
        && !all_day
        && g.start
            .as_ref()
            .and_then(|t| t.time_zone.as_deref())
            .is_some_and(|tz| tz != zone.name)
    {
        rule.unsupported = true;
    }
    let conference_uri = g
        .conference_data
        .as_ref()
        .and_then(|c| {
            c.entry_points
                .iter()
                .find(|e| e.entry_point_type == "video")
        })
        .map(|e| e.uri.clone())
        .or(g.hangout_link)
        .unwrap_or_default();
    let reminders = g.reminders.unwrap_or_default();
    let organizer = g.organizer.unwrap_or_default();
    Some(Item::Event(Event {
        id: g.id,
        calendar: calendar.to_string(),
        title: g.summary.unwrap_or_default(),
        start,
        end: end.max(start),
        all_day,
        location: g.location.unwrap_or_default(),
        description: plain_description(g.description.as_deref().unwrap_or_default()),
        attendees: g
            .attendees
            .iter()
            .filter(|a| !a.resource)
            .map(|a| Attendee {
                name: a.display_name.clone().unwrap_or_default(),
                email: a.email.clone(),
                status: match a.response_status.as_str() {
                    "accepted" => RsvpStatus::Accepted,
                    "tentative" => RsvpStatus::Tentative,
                    "declined" => RsvpStatus::Declined,
                    _ => RsvpStatus::NeedsAction,
                },
            })
            .collect(),
        organizer: Contact {
            name: organizer.display_name.unwrap_or_default(),
            email: organizer.email,
        },
        recurrence: recurrence.rule,
        reminder_minutes: if reminders.use_default {
            default_reminder
        } else {
            earliest_reminder(&reminders.overrides)
        },
        video_link: conference_uri.clone(),
        busy: g.transparency.as_deref() != Some("transparent"),
        exceptions: recurrence.exdates,
        recurring_event_id: g.recurring_event_id,
        original_start,
        etag: g.etag,
        ical_uid: g.ical_uid.unwrap_or_default(),
        status,
        time_zone: g.start.and_then(|t| t.time_zone),
        html_link: g.html_link.unwrap_or_default(),
        organizer_self: organizer.is_self,
        guests_can_modify: g.guests_can_modify,
        attendees_omitted: g.attendees_omitted,
        event_type: g.event_type.unwrap_or_else(|| "default".into()),
        conference_uri,
        reminder_use_default: reminders.use_default,
        reminders: if reminders.use_default {
            Vec::new()
        } else {
            self::reminders(&reminders.overrides)
        },
        meet_request: None,
    }))
}

/// A start or end as a wall time in `zone`, and whether it's a date.
fn wall_time(t: &GoogleTime, zone: &LocalZone) -> Option<(NaiveDateTime, bool)> {
    if let Some(date_time) = &t.date_time {
        let t = DateTime::parse_from_rfc3339(date_time).ok()?;
        return Some(((zone.to_local)(t.to_utc()), false));
    }
    let date = chrono::NaiveDate::parse_from_str(t.date.as_deref()?, "%Y-%m-%d").ok()?;
    Some((midnight(date), true))
}

/// Google keeps descriptions as HTML when they were written in its editor.
fn plain_description(text: &str) -> String {
    if text.contains('<') && text.contains('>') {
        crate::html::plain_text(&crate::html::parse(text))
            .trim()
            .to_string()
    } else {
        text.to_string()
    }
}

// Calls.

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A failed call's reason, in Google's words when it gave some.
fn api_error(what: &str, response: &Response) -> anyhow::Error {
    let message = response
        .error_message()
        .unwrap_or_else(|| format!("HTTP {}", response.status));
    anyhow!("{what}: {message}")
}

fn get(transport: &dyn Transport, url: &str) -> anyhow::Result<Response> {
    Ok(transport.send(&Request::get(url, COST))?)
}

enum Listing<T> {
    Items {
        items: Vec<T>,
        sync_token: Option<String>,
    },
    /// 410: the sync token expired; list everything again.
    Gone,
}

/// Every page of a listing at `url` (which has a query already), the
/// first one `first` when it came already (in a batch, MC-013).
fn list_all<T: DeserializeOwned>(
    transport: &dyn Transport,
    url: &str,
    what: &str,
    mut first: Option<Response>,
) -> anyhow::Result<Listing<T>> {
    let mut items = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let page_url = match &page_token {
            Some(token) => format!("{url}&pageToken={}", enc(token)),
            None => url.to_string(),
        };
        let response = match first.take() {
            Some(response) => response,
            None => get(transport, &page_url)?,
        };
        if response.status == 410 {
            return Ok(Listing::Gone);
        }
        if !response.is_success() {
            return Err(api_error(what, &response));
        }
        let page: Page<T> = response
            .json()
            .with_context(|| format!("{what}: unexpected response"))?;
        items.extend(page.items);
        match page.next_page_token {
            Some(token) => page_token = Some(token),
            None => {
                return Ok(Listing::Items {
                    items,
                    sync_token: page.next_sync_token,
                });
            }
        }
    }
}

fn is_network(e: &anyhow::Error) -> bool {
    e.downcast_ref::<TransportError>()
        .is_some_and(TransportError::goes_later)
}

// The sync pass.

/// Brings `account`'s calendars and events in the store up to date with
/// Google. Blocking: run it off the main thread. A network failure stops
/// it with an error; a calendar Google refuses is skipped and named in
/// `errors`.
pub fn sync_pass(
    transport: &dyn Transport,
    store: &Store,
    account: &str,
) -> anyhow::Result<CalendarChanges> {
    sync_pass_at(transport, store, account, &LocalZone::system(), Utc::now())
}

fn sync_pass_at(
    transport: &dyn Transport,
    store: &Store,
    account: &str,
    zone: &LocalZone,
    now: DateTime<Utc>,
) -> anyhow::Result<CalendarChanges> {
    let mut changes = CalendarChanges::default();
    let mut touched = BTreeSet::new();
    let palette = colors(transport, store, account)?;
    sync_calendar_list(
        transport,
        store,
        account,
        &palette,
        &mut changes,
        &mut touched,
    )?;
    let calendars = store.calendars_to_sync(account)?;
    let passes: Vec<CalendarPass> = calendars
        .iter()
        .map(|(calendar, default_reminder)| CalendarPass {
            transport,
            store,
            account,
            calendar,
            default_reminder: *default_reminder,
            zone,
            now,
        })
        .collect();
    let mut first = first_pages(transport, &passes)?;
    for pass in &passes {
        match pass.run(first.remove(pass.calendar), &mut touched) {
            Ok(()) => {}
            Err(e) if is_network(&e) => return Err(e),
            Err(e) => changes.errors.push(format!("{e:#}")),
        }
    }
    // CAL-41: optional; the next pass tries again.
    if let Err(e) = sync_settings(transport, store, account, now, &mut changes) {
        eprintln!("Pigeon: {e:#}");
    }
    changes.split_touched(store, account, touched)?;
    Ok(changes)
}

/// Google Calendar's HTTP batch endpoint, and the most calls in one.
const BATCH_ENDPOINT: &str = "https://www.googleapis.com/batch/calendar/v3";
const BATCH_SIZE: usize = 50;

/// The first page of every calendar's listing by sync token, by calendar,
/// asked for in one HTTP batch (MC-013); a calendar left out lists on its
/// own. One calendar asks on its own anyway. No network stops the pass.
fn first_pages(
    transport: &dyn Transport,
    passes: &[CalendarPass],
) -> anyhow::Result<HashMap<String, Response>> {
    let mut calendars = Vec::new();
    let mut requests = Vec::new();
    for pass in passes {
        if let Some(token) = pass
            .store
            .calendar_sync_token(pass.account, pass.calendar)?
        {
            calendars.push(pass.calendar.to_string());
            requests.push(Request::get(pass.token_url(&token), COST));
        }
    }
    let mut out = HashMap::new();
    if requests.len() < 2 {
        return Ok(out);
    }
    let answers = http::batch(
        transport,
        &http::SystemClock,
        BATCH_ENDPOINT,
        BATCH_SIZE,
        &requests,
    );
    for (calendar, answer) in calendars.into_iter().zip(answers) {
        match answer {
            Ok(response) => {
                out.insert(calendar, response);
            }
            Err(http::BatchError::Transport(e)) if e.goes_later() => return Err(e.into()),
            Err(_) => {}
        }
    }
    Ok(out)
}

const SETTINGS_KEY: &str = "calendar_settings";
const SETTINGS_AT_KEY: &str = "calendar_settings_at";
/// How often Google's calendar settings are read again, in seconds.
const SETTINGS_EVERY: i64 = 6 * 60 * 60;

/// The user's Google Calendar settings (CAL-41), read with `settings.list`
/// every few hours. A change counts as a calendar list change, so the app
/// reloads them with the calendars.
fn sync_settings(
    transport: &dyn Transport,
    store: &Store,
    account: &str,
    now: DateTime<Utc>,
    changes: &mut CalendarChanges,
) -> anyhow::Result<()> {
    let last = store
        .sync_state(account, SETTINGS_AT_KEY)?
        .and_then(|s| s.parse::<i64>().ok());
    if last.is_some_and(|at| now.timestamp() - at < SETTINGS_EVERY) {
        return Ok(());
    }
    #[derive(Deserialize)]
    struct Setting {
        id: String,
        value: String,
    }
    let response = get(
        transport,
        &format!("{API}/users/me/settings?maxResults=250"),
    )?;
    if !response.is_success() {
        return Err(api_error("Couldn't read the calendar settings", &response));
    }
    let page: Page<Setting> = response.json()?;
    let items: Vec<(String, String)> = page.items.into_iter().map(|s| (s.id, s.value)).collect();
    let json = serde_json::to_string(&GoogleSettings::from_items(&items))?;
    if store.sync_state(account, SETTINGS_KEY)?.as_deref() != Some(json.as_str()) {
        store.set_sync_state(account, SETTINGS_KEY, &json)?;
        changes.calendars = true;
    }
    store.set_sync_state(account, SETTINGS_AT_KEY, &now.timestamp().to_string())?;
    Ok(())
}

/// Calendar colour ids to hex, fetched once.
fn colors(
    transport: &dyn Transport,
    store: &Store,
    account: &str,
) -> anyhow::Result<HashMap<String, String>> {
    if let Some(cached) = store.sync_state(account, COLORS_KEY)?
        && let Ok(palette) = serde_json::from_str(&cached)
    {
        return Ok(palette);
    }
    let response = get(transport, &format!("{API}/colors"))?;
    if !response.is_success() {
        return Err(api_error("Couldn't read calendar colours", &response));
    }
    let body: Value = response.json()?;
    let palette: HashMap<String, String> = body["calendar"]
        .as_object()
        .map(|colors| {
            colors
                .iter()
                .filter_map(|(id, c)| Some((id.clone(), c["background"].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    store.set_sync_state(account, COLORS_KEY, &serde_json::to_string(&palette)?)?;
    Ok(palette)
}

fn sync_calendar_list(
    transport: &dyn Transport,
    store: &Store,
    account: &str,
    palette: &HashMap<String, String>,
    changes: &mut CalendarChanges,
    touched: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    const WHAT: &str = "Couldn't read your calendars";
    let full_url = format!("{API}/users/me/calendarList?maxResults=250&showHidden=true");
    let token = store.sync_state(account, LIST_TOKEN_KEY)?;
    let mut full = token.is_none();
    let mut listing = match &token {
        Some(token) => list_all(
            transport,
            &format!(
                "{API}/users/me/calendarList?maxResults=250&syncToken={}",
                enc(token)
            ),
            WHAT,
            None,
        )?,
        None => list_all(transport, &full_url, WHAT, None)?,
    };
    if matches!(listing, Listing::Gone) {
        full = true;
        listing = list_all(transport, &full_url, WHAT, None)?;
    }
    let Listing::Items { items, sync_token } = listing else {
        return Err(anyhow!("{WHAT}: Google keeps asking for a full sync"));
    };
    let entries: Vec<CalendarListEntry> = items;
    if full {
        let listed: BTreeSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        for id in store.all_calendar_ids(account)? {
            if !listed.contains(id.as_str()) {
                touched.extend(store.delete_calendar(account, &id)?);
            }
        }
    }
    // A show/hide still to go isn't on Google yet: its list says the old
    // state, which would bring the old one back.
    let queued = store.queued_selections(account)?;
    let stored = if queued.is_empty() {
        Vec::new()
    } else {
        store.load_calendars(account)?
    };
    for entry in &entries {
        if entry.deleted {
            touched.extend(store.delete_calendar(account, &entry.id)?);
        } else {
            let mut calendar = calendar_from_entry(entry, palette);
            if queued.contains(&entry.id)
                && let Some(kept) = stored.iter().find(|c| c.id == entry.id)
            {
                calendar.visible = kept.visible;
            }
            store.put_google_calendar(account, &calendar, entry.hidden)?;
        }
    }
    changes.calendars |= full || !entries.is_empty();
    if let Some(token) = sync_token {
        store.set_sync_state(account, LIST_TOKEN_KEY, &token)?;
    }
    Ok(())
}

/// One calendar's events, synced.
struct CalendarPass<'a> {
    transport: &'a dyn Transport,
    store: &'a Store,
    account: &'a str,
    calendar: &'a str,
    default_reminder: Option<u32>,
    zone: &'a LocalZone,
    now: DateTime<Utc>,
}

impl CalendarPass<'_> {
    fn what(&self) -> String {
        format!("Couldn't sync calendar {}", self.calendar)
    }

    fn base_url(&self) -> String {
        format!(
            "{API}/calendars/{}/events?singleEvents=false&maxResults=2500&timeZone={}",
            enc(self.calendar),
            enc(self.zone.name)
        )
    }

    /// The listing of what changed since `token`.
    fn token_url(&self, token: &str) -> String {
        format!("{}&syncToken={}", self.base_url(), enc(token))
    }

    /// `first`: the token listing's first page, when it came in a batch.
    fn run(&self, first: Option<Response>, touched: &mut BTreeSet<String>) -> anyhow::Result<()> {
        let base = self.base_url();
        let full_url = format!(
            "{base}&timeMin={}",
            enc(&rfc3339(self.now - Duration::days(PAST_DAYS)))
        );
        let token = self
            .store
            .calendar_sync_token(self.account, self.calendar)?;
        let mut full = token.is_none();
        let mut listing = match &token {
            Some(token) => list_all(self.transport, &self.token_url(token), &self.what(), first)?,
            None => list_all(self.transport, &full_url, &self.what(), None)?,
        };
        if matches!(listing, Listing::Gone) {
            // The token expired: start this calendar over.
            touched.extend(
                self.store
                    .delete_calendar_events(self.account, self.calendar, None)?,
            );
            self.store
                .set_calendar_sync_token(self.account, self.calendar, None)?;
            full = true;
            listing = list_all(self.transport, &full_url, &self.what(), None)?;
        }
        let Listing::Items { items, sync_token } = listing else {
            return Err(anyhow!(
                "{}: Google keeps asking for a full sync",
                self.what()
            ));
        };
        self.apply(&items, full, touched)?;
        self.store
            .set_calendar_sync_token(self.account, self.calendar, sync_token.as_deref())?;
        Ok(())
    }

    /// Stores a listing's items. A full listing replaces what was stored.
    fn apply(
        &self,
        items: &[Value],
        full: bool,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<()> {
        let mut puts = Vec::new();
        let mut deletes = Vec::new();
        let mut series = BTreeSet::new();
        for raw in items {
            match event_from_google(raw, self.calendar, self.default_reminder, self.zone) {
                Some(Item::Event(event)) => {
                    if event.is_series() {
                        series.insert(event.id.clone());
                    }
                    if let Some(id) = &event.recurring_event_id {
                        series.insert(id.clone());
                    }
                    touched.insert(event.id.clone());
                    puts.push((event, raw.to_string()));
                }
                Some(Item::Deleted(id)) => deletes.push(id),
                None => {}
            }
        }
        if full {
            let listed: BTreeSet<&str> = puts.iter().map(|(e, _)| e.id.as_str()).collect();
            deletes.extend(
                self.store
                    .calendar_event_ids(self.account, self.calendar)?
                    .into_iter()
                    .filter(|id| !listed.contains(id.as_str())),
            );
        }
        if !deletes.is_empty() {
            touched.extend(self.store.delete_calendar_events(
                self.account,
                self.calendar,
                Some(&deletes),
            )?);
        }
        self.store.put_google_events(self.account, &puts)?;
        let mut refreshed = Vec::new();
        for id in series {
            refreshed.extend(self.refresh_series(&id, touched)?);
        }
        // One transaction for all of them (MC-020).
        self.store.put_google_events(self.account, &refreshed)?;
        Ok(())
    }

    /// A series' exceptions, recomputed: its EXDATEs and every instance
    /// stored on its own. For a rule Pigeon can't expand, Google's own
    /// occurrences replace the stored ones. Gives the series to store, when
    /// its exceptions are recomputed.
    fn refresh_series(
        &self,
        id: &str,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<Option<(Event, String)>> {
        let Some((mut series, json)) =
            self.store.calendar_event(self.account, self.calendar, id)?
        else {
            return Ok(None);
        };
        let Some(rule) = &series.recurrence else {
            return Ok(None);
        };
        touched.insert(series.id.clone());
        if rule.unsupported {
            let url = format!(
                "{API}/calendars/{}/events/{}/instances?maxResults=2500&timeZone={}&timeMin={}&timeMax={}",
                enc(self.calendar),
                enc(id),
                enc(self.zone.name),
                enc(&rfc3339(self.now - Duration::days(PAST_DAYS))),
                enc(&rfc3339(self.now + Duration::days(INSTANCES_AHEAD_DAYS))),
            );
            let Listing::Items { items, .. } =
                list_all::<Value>(self.transport, &url, &self.what(), None)?
            else {
                return Err(anyhow!("{}: the series' occurrences went", self.what()));
            };
            let instances: Vec<(Event, String)> = items
                .iter()
                .filter_map(|raw| {
                    match event_from_google(raw, self.calendar, self.default_reminder, self.zone)? {
                        Item::Event(mut e) if e.status != EventStatus::Cancelled => {
                            e.recurring_event_id.get_or_insert_with(|| id.to_string());
                            Some((e, raw.to_string()))
                        }
                        _ => None,
                    }
                })
                .collect();
            touched.extend(
                self.store
                    .delete_instances(self.account, self.calendar, id)?,
            );
            touched.extend(instances.iter().map(|(e, _)| e.id.clone()));
            self.store.put_google_events(self.account, &instances)?;
            return Ok(None);
        }
        // Only the rule's lines are read from the stored JSON.
        #[derive(Deserialize)]
        struct Lines {
            #[serde(default)]
            recurrence: Vec<String>,
        }
        let lines: Vec<String> = json
            .as_deref()
            .and_then(|j| serde_json::from_str::<Lines>(j).ok())
            .map(|g| g.recurrence)
            .unwrap_or_default();
        let mut exceptions = parse_recurrence(&lines, series.all_day, self.zone).exdates;
        exceptions.extend(
            self.store
                .instance_starts(self.account, self.calendar, id)?,
        );
        exceptions.sort();
        exceptions.dedup();
        series.exceptions = exceptions;
        Ok(Some((series, json.unwrap_or_default())))
    }
}

// Writes (W7) -----------------------------------------------------------
//
// `commit` sends one `CalOp` to Google (docs/BACKEND_SPRINT_1.md, W7):
//
// - Insert: `events.insert` with the client's id (CAL-11). An occurrence
//   taken out of its series ("This event") is Google's instance, found with
//   `events.instances`, patched.
// - Patch: `events.patch` with `If-Match`, only the fields that changed.
// - Delete: `events.delete`; "This event" deletes the instance, "This and
//   following" rewrites the series' UNTIL. Someone else's event is declined
//   and removed from the user's calendar instead (CAL-22).
// - Move: `events.move`, then a patch putting back the explicit reminders
//   and color, which are per calendar and dropped by the move. Rsvp: the user's own `responseStatus`. SetSelected:
//   `calendarList.patch`.
//
// A 412 (changed elsewhere) or any other failure stores Google's version
// again, which the app reloads: that's the rollback (STA-2, STA-5).

/// What the user may do with an event (CAL-22).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Edit,
    /// Only answer the invite; deleting it declines it and hides it.
    Rsvp,
    ReadOnly,
}

/// Whether the user organizes `event`: Google says so, or it names them
/// (the placeholder data does), or nobody.
pub fn is_organizer(event: &Event, me: &str) -> bool {
    event.organizer_self
        || event.organizer.email.is_empty()
        || event.organizer.email.eq_ignore_ascii_case(me)
}

/// From the calendar's `accessRole`, who organizes the event and whether
/// its guests may change it. Birthdays and events from Gmail are Google's.
pub fn access(event: &Event, role: AccessRole, me: &str) -> Access {
    if !matches!(role, AccessRole::Owner | AccessRole::Writer) {
        return Access::ReadOnly;
    }
    let googles = matches!(event.event_type.as_str(), "birthday" | "fromGmail");
    if !googles && (is_organizer(event, me) || event.guests_can_modify) {
        Access::Edit
    } else if event
        .attendees
        .iter()
        .any(|a| a.email.eq_ignore_ascii_case(me))
    {
        Access::Rsvp
    } else {
        Access::ReadOnly
    }
}

/// Why the user can't change an event with `access` on a calendar named
/// `calendar` with `role` (CAL-22): the panel's line, and the toast when a
/// drag, an edit or a delete is refused. `None` when they can.
pub fn refusal(access: Access, role: AccessRole, calendar: &str) -> Option<String> {
    match access {
        Access::Edit => None,
        Access::Rsvp => Some("Only the organizer can change this event".into()),
        Access::ReadOnly if !matches!(role, AccessRole::Owner | AccessRole::Writer) => {
            Some(if calendar.trim().is_empty() {
                "This calendar is read-only".into()
            } else {
                format!("“{}” is read-only", calendar.trim())
            })
        }
        Access::ReadOnly => Some("You can't change this event".into()),
    }
}

/// Guests besides the user and the organizer.
pub fn has_guests(event: &Event, me: &str) -> bool {
    event.attendees.iter().any(|a| {
        !a.email.eq_ignore_ascii_case(me) && !a.email.eq_ignore_ascii_case(&event.organizer.email)
    })
}

/// Whether saving `after` (new when `before` is `None`) should ask "Send
/// updates to guests?" (CAL-13): it has guests, and the change is one they
/// see, not just the user's reminder or busy/free.
pub fn asks_guests(before: Option<&Event>, after: &Event, me: &str) -> bool {
    let guests = has_guests(after, me) || before.is_some_and(|b| has_guests(b, me));
    guests
        && match before {
            None => true,
            Some(before) => patch_body(before, after, None, &LocalZone::system())
                .keys()
                .any(|k| k != "reminders" && k != "transparency"),
        }
}

fn send_updates_param(send: SendUpdates) -> &'static str {
    match send {
        SendUpdates::All => "all",
        SendUpdates::None => "none",
    }
}

/// Google's id for the occurrence of `series` the rule puts at `original`:
/// `<series>_<UTC start>`, or `<series>_<date>` all day.
pub fn instance_id(
    series: &str,
    original: NaiveDateTime,
    all_day: bool,
    zone: &LocalZone,
) -> String {
    if all_day {
        format!("{series}_{}", original.format("%Y%m%d"))
    } else {
        format!(
            "{series}_{}",
            (zone.to_utc)(original).format("%Y%m%dT%H%M%SZ")
        )
    }
}

/// An RRULE line with its end replaced: UNTIL when given, else COUNT when
/// given, else neither. Everything else in it stays as Google wrote it.
pub fn rewrite_rrule(line: &str, until: Option<&str>, count: Option<u32>) -> String {
    let value = line.strip_prefix("RRULE:").unwrap_or(line);
    let mut parts: Vec<String> = value
        .split(';')
        .filter(|p| {
            let key = p.split('=').next().unwrap_or_default().to_ascii_uppercase();
            !p.is_empty() && key != "UNTIL" && key != "COUNT"
        })
        .map(str::to_string)
        .collect();
    match (until, count) {
        (Some(until), _) => parts.push(format!("UNTIL={until}")),
        (None, Some(count)) => parts.push(format!("COUNT={count}")),
        (None, None) => {}
    }
    format!("RRULE:{}", parts.join(";"))
}

/// The event's `recurrence` lines (CAL-29): its rule as the RRULE, and
/// from `raw` the lines Pigeon doesn't model (EXDATE, RDATE), kept on
/// purpose. When only the end changed since `before`, Google's RRULE stays
/// with its end rewritten (it may hold parts Pigeon doesn't, like WKST); a
/// new rule replaces it whole. No rule: no lines.
fn recurrence_lines(
    before: Option<&Rule>,
    event: &Event,
    raw: Option<&Value>,
    zone: &LocalZone,
) -> Vec<String> {
    let Some(rule) = &event.recurrence else {
        return Vec::new();
    };
    let old: Vec<String> = raw
        .and_then(|r| r["recurrence"].as_array())
        .map(|l| {
            l.iter()
                .filter_map(|l| l.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let same = before.is_some_and(|b| b.same_pattern(rule));
    let until = rule.until.map(|u| ical_until(u, event.all_day, zone));
    let mut out = Vec::new();
    let mut has_rule = false;
    for line in old {
        if !line.to_ascii_uppercase().starts_with("RRULE:") {
            out.push(line);
        } else if !has_rule {
            has_rule = true;
            out.push(if same {
                rewrite_rrule(&line, until.as_deref(), rule.count)
            } else {
                rule.to_rrule(event.all_day, zone)
            });
        }
    }
    if !has_rule {
        out.insert(0, rule.to_rrule(event.all_day, zone));
    }
    out
}

/// Location and description with a video link typed in Pigeon folded in:
/// into the location when that's empty, else atop the description. A
/// link from Google's `conferenceData` is read-only (6.4 item 9).
fn place(event: &Event) -> (String, String) {
    let link = event.video_link.trim();
    let (location, description) = (event.location.clone(), event.description.clone());
    if link.is_empty()
        || link == event.conference_uri
        || location.contains(link)
        || description.contains(link)
    {
        (location, description)
    } else if location.trim().is_empty() {
        (link.to_string(), description)
    } else {
        let description = format!("{link}\n\n{description}").trim_end().to_string();
        (location, description)
    }
}

/// A start or end: a date all day, else the instant with the event's zone
/// (its own, else the user's; CAL-12). A patch clears the other form.
fn time_json(t: NaiveDateTime, event: &Event, zone: &LocalZone, patch: bool) -> Value {
    let tz = event
        .time_zone
        .clone()
        .unwrap_or_else(|| zone.name.to_string());
    let mut out = if event.all_day {
        json!({ "date": t.format("%Y-%m-%d").to_string(), "timeZone": tz })
    } else {
        json!({ "dateTime": rfc3339((zone.to_utc)(t)), "timeZone": tz })
    };
    if patch {
        let other = if event.all_day { "dateTime" } else { "date" };
        out[other] = Value::Null;
    }
    out
}

fn rsvp_key(status: RsvpStatus) -> &'static str {
    match status {
        RsvpStatus::Accepted => "accepted",
        RsvpStatus::Tentative => "tentative",
        RsvpStatus::Declined => "declined",
        RsvpStatus::NeedsAction => "needsAction",
    }
}

/// The guest list to write: Google's entries for the guests that stay
/// (rooms always stay; Pigeon doesn't show them), then the new guests.
fn attendees_json(event: &Event, raw: Option<&Value>) -> Value {
    let keeps = |email: &str| {
        event
            .attendees
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(email))
    };
    let old: Vec<Value> = raw
        .and_then(|r| r["attendees"].as_array().cloned())
        .unwrap_or_default();
    let mut out: Vec<Value> = old
        .iter()
        .filter(|a| {
            a["resource"].as_bool() == Some(true) || keeps(a["email"].as_str().unwrap_or(""))
        })
        .cloned()
        .collect();
    for a in &event.attendees {
        let known = old.iter().any(|o| {
            o["email"]
                .as_str()
                .is_some_and(|e| e.eq_ignore_ascii_case(&a.email))
        });
        if !known {
            let mut entry = json!({ "email": a.email, "responseStatus": rsvp_key(a.status) });
            if !a.name.is_empty() {
                entry["displayName"] = json!(a.name);
            }
            out.push(entry);
        }
    }
    Value::Array(out)
}

/// The calendar's default reminders, or the ones the user picked, up to
/// five, notifications or emails (CAL-33; none is no overrides). The
/// default sends an empty `overrides` too: a PATCH merges, and Google
/// refuses default reminders next to the event's old overrides.
fn reminders_json(event: &Event) -> Value {
    if event.reminder_use_default {
        return json!({ "useDefault": true, "overrides": [] });
    }
    let overrides: Vec<Value> = event
        .own_reminders()
        .iter()
        .map(|r| json!({ "method": r.method.key(), "minutes": r.minutes }))
        .collect();
    json!({ "useDefault": false, "overrides": overrides })
}

/// "Add Google Meet" (CAL-34): the request Google answers with a Meet,
/// with the event's request id, so a retry is the same request.
fn meet_request_json(request_id: &str) -> Value {
    json!({
        "createRequest": {
            "requestId": request_id,
            "conferenceSolutionKey": { "type": "hangoutsMeet" },
        }
    })
}

/// The fields of `after` that differ from `before`, as Google's event JSON
/// for `events.patch`. `raw` is the stored Google event, for the guest list
/// and the repeat lines. The calendar (a move) and the series' exceptions
/// (instances of their own) aren't fields here.
pub fn patch_body(
    before: &Event,
    after: &Event,
    raw: Option<&Value>,
    zone: &LocalZone,
) -> Map<String, Value> {
    let mut body = Map::new();
    if before.title != after.title {
        body.insert("summary".into(), json!(after.title));
    }
    let (old_location, old_description) = place(before);
    let (location, description) = place(after);
    if old_location != location {
        body.insert("location".into(), json!(location));
    }
    if old_description != description {
        body.insert("description".into(), json!(description));
    }
    if (before.start, before.end, before.all_day, &before.time_zone)
        != (after.start, after.end, after.all_day, &after.time_zone)
    {
        body.insert("start".into(), time_json(after.start, after, zone, true));
        body.insert("end".into(), time_json(after.end, after, zone, true));
    }
    let emails =
        |e: &Event| -> Vec<String> { e.attendees.iter().map(|a| a.email.to_lowercase()).collect() };
    if !after.attendees_omitted && emails(before) != emails(after) {
        body.insert("attendees".into(), attendees_json(after, raw));
    }
    // On the default, only which default it is can differ (another calendar).
    if before.reminder_use_default != after.reminder_use_default
        || (!after.reminder_use_default && before.own_reminders() != after.own_reminders())
    {
        body.insert("reminders".into(), reminders_json(after));
    }
    if before.busy != after.busy {
        let transparency = if after.busy { "opaque" } else { "transparent" };
        body.insert("transparency".into(), json!(transparency));
    }
    // The whole rule (CAL-29); none turns the series into one event.
    if before.recurrence != after.recurrence && after.recurring_event_id.is_none() {
        body.insert(
            "recurrence".into(),
            json!(recurrence_lines(
                before.recurrence.as_ref(),
                after,
                raw,
                zone
            )),
        );
    }
    // Only a new Meet is written; otherwise Google keeps `conferenceData`.
    if let Some(request) = &after.meet_request
        && before.meet_request.as_ref() != Some(request)
        && after.conference_uri.is_empty()
    {
        body.insert("conferenceData".into(), meet_request_json(request));
    }
    body
}

/// A new event, whole, for `events.insert` with its own id (CAL-11).
pub fn insert_body(event: &Event, zone: &LocalZone) -> Value {
    let (location, description) = place(event);
    let mut body = json!({
        "id": event.id,
        "summary": event.title,
        "location": location,
        "description": description,
        "start": time_json(event.start, event, zone, false),
        "end": time_json(event.end, event, zone, false),
        "reminders": reminders_json(event),
        "transparency": if event.busy { "opaque" } else { "transparent" },
    });
    if !event.attendees.is_empty() {
        body["attendees"] = attendees_json(event, None);
    }
    if event.recurrence.is_some() {
        body["recurrence"] = json!(recurrence_lines(None, event, None, zone));
    }
    if let Some(request) = event
        .meet_request
        .as_deref()
        .filter(|_| event.conference_uri.is_empty())
    {
        body["conferenceData"] = meet_request_json(request);
    }
    body
}

impl super::Backend {
    /// The user's Google Calendar settings as last read (CAL-41); the
    /// defaults in demo mode and before the first sync.
    pub fn calendar_settings(&self) -> GoogleSettings {
        self.store
            .sync_state(self.email(), SETTINGS_KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    /// The reminders shown already, recently (W5 reminder notifications).
    pub fn fired_reminders(&self) -> std::collections::HashSet<ReminderKey> {
        self.store
            .fired_reminders(self.email())
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// A reminder was shown; it won't be again.
    pub fn mark_reminder_fired(&self, key: &ReminderKey) {
        if let Err(e) =
            self.store
                .mark_reminder_fired(self.email(), key, Utc::now().timestamp_millis())
        {
            eprintln!("Pigeon: couldn't keep a reminder as shown: {e:#}");
        }
    }

    /// After a calendar write, sync soon, so what Google changed with it
    /// (the series' instances, the guests' copies) shows (2.6).
    pub(super) fn sync_calendar_soon(&mut self, cx: &mut gpui::Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(super::SYNC_AFTER_WRITE)
                .await;
            this.update(cx, |this, cx| {
                this.sync.calendar_due = true;
                this.run_sync(cx);
            })
            .ok();
        })
        .detach();
    }
}

/// Google answered 412: the event changed since it was read.
#[derive(Debug)]
struct Conflict;

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Changed elsewhere; reloaded")
    }
}

impl std::error::Error for Conflict {}

/// What a commit did, for the app: the events to reload, and why it
/// failed, if it did. The store already holds Google's version.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Committed {
    pub changes: CalendarChanges,
    pub error: Option<String>,
    /// It failed for want of a network or a sign-in; it goes again later,
    /// with nothing rolled back.
    pub retry: Option<TransportError>,
}

impl Committed {
    /// As the outbox reports ops: `Err` with the toast's text.
    pub fn result(&self) -> anyhow::Result<()> {
        match &self.error {
            Some(e) => Err(anyhow!("{e}")),
            None => Ok(()),
        }
    }
}

/// Sends `op` to Google for `account` and stores what Google answers.
/// Blocking: run it off the main thread.
pub fn commit(transport: &dyn Transport, store: &Store, account: &str, op: &CalOp) -> Committed {
    commit_at(
        transport,
        store,
        account,
        op,
        &LocalZone::system(),
        Utc::now(),
    )
}

fn commit_at(
    transport: &dyn Transport,
    store: &Store,
    account: &str,
    op: &CalOp,
    zone: &LocalZone,
    now: DateTime<Utc>,
) -> Committed {
    let w = Writer {
        transport,
        store,
        account,
        zone,
        now,
        pause: PAUSE,
    };
    let mut touched = BTreeSet::new();
    let mut out = Committed::default();
    if let Err(e) = w.apply(op, &mut touched) {
        out.retry = e
            .downcast_ref::<TransportError>()
            .filter(|e| e.goes_later())
            .cloned();
        out.error = Some(if e.is::<Conflict>() {
            Conflict.to_string()
        } else {
            format!("{e:#}")
        });
        // Google's version back: the rollback.
        match op {
            _ if out.retry.is_some() => {}
            CalOp::SetSelected { .. } => out.changes.calendars = true,
            CalOp::Insert { event, .. } => {
                w.reload(&event.calendar, &event.id, &mut touched).ok();
            }
            CalOp::Patch { id, after, .. } => {
                w.reload(&after.calendar, id, &mut touched).ok();
            }
            // Whichever step failed, both as Google has them: the new
            // series is gone if it never went in.
            CalOp::Split { id, after, new, .. } => {
                w.reload(&after.calendar, id, &mut touched).ok();
                w.reload(&new.calendar, &new.id, &mut touched).ok();
            }
            CalOp::Delete { id, .. } | CalOp::Move { id, .. } | CalOp::Rsvp { id, .. } => {
                if let Some((event, _)) = w.locate(id) {
                    w.reload(&event.calendar, id, &mut touched).ok();
                }
            }
        }
    }
    if let Err(e) = out.changes.split_touched(store, account, touched) {
        eprintln!("Pigeon: couldn't read the events written back: {e:#}");
    }
    out
}

struct Writer<'a> {
    transport: &'a dyn Transport,
    store: &'a Store,
    account: &'a str,
    zone: &'a LocalZone,
    now: DateTime<Utc>,
    /// Waits between polls for a new Meet; tests don't wait.
    pause: fn(std::time::Duration),
}

#[cfg(not(test))]
const PAUSE: fn(std::time::Duration) = std::thread::sleep;
#[cfg(test)]
const PAUSE: fn(std::time::Duration) = |_| {};

/// How often, and how many times, a Meet still being set up is asked
/// after (CAL-34).
const MEET_POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(1);
const MEET_POLLS: usize = 8;

/// The query that lets a write create a conference.
fn conference_param(body: &Value) -> &'static str {
    if body.get("conferenceData").is_some() {
        "&conferenceDataVersion=1"
    } else {
        ""
    }
}

impl Writer<'_> {
    fn events_url(&self, calendar: &str) -> String {
        format!("{API}/calendars/{}/events", enc(calendar))
    }

    fn event_url(&self, calendar: &str, id: &str) -> String {
        format!("{}/{}", self.events_url(calendar), enc(id))
    }

    /// Sends, and maps 412 to `Conflict` and other failures to Google's
    /// reason. Statuses in `ok` count as done (a delete of what's gone).
    fn send(
        &self,
        method: &'static str,
        url: String,
        body: Option<&Value>,
        if_match: Option<&str>,
        ok: &[u16],
    ) -> anyhow::Result<Response> {
        let mut request = match body {
            Some(body) => Request::json(method, url, body, COST),
            None => Request {
                method,
                ..Request::get(url, COST)
            },
        };
        if let Some(etag) = if_match.filter(|e| !e.is_empty()) {
            request.headers.push(("If-Match".into(), etag.to_string()));
        }
        let response = self.transport.send(&request)?;
        if response.status == 412 {
            return Err(Conflict.into());
        }
        if !response.is_success() && !ok.contains(&response.status) {
            let error = api_error("Google Calendar didn't take the change", &response);
            eprintln!("Pigeon: {method} {}: {error:#}", response.status);
            return Err(error);
        }
        Ok(response)
    }

    /// The stored event with this id, in whichever calendar, with its JSON.
    fn locate(&self, id: &str) -> Option<(Event, Option<Value>)> {
        let event = self
            .store
            .load_events(self.account, &[id.to_string()])
            .ok()?
            .into_iter()
            .next()?;
        let calendar = event.calendar.clone();
        self.stored(&calendar, id).or(Some((event, None)))
    }

    fn stored(&self, calendar: &str, id: &str) -> Option<(Event, Option<Value>)> {
        let (event, json) = self
            .store
            .calendar_event(self.account, calendar, id)
            .ok()??;
        Some((event, json.and_then(|j| serde_json::from_str(&j).ok())))
    }

    fn pass<'b>(&'b self, calendar: &'b str) -> anyhow::Result<CalendarPass<'b>> {
        let default_reminder = self
            .store
            .calendars_to_sync(self.account)?
            .into_iter()
            .find(|(id, _)| id == calendar)
            .and_then(|(_, r)| r);
        Ok(CalendarPass {
            transport: self.transport,
            store: self.store,
            account: self.account,
            calendar,
            default_reminder,
            zone: self.zone,
            now: self.now,
        })
    }

    /// Stores an event as Google returned it; a series (or an instance's
    /// series) gets its exceptions recomputed.
    fn store_raw(
        &self,
        calendar: &str,
        raw: &Value,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<()> {
        let pass = self.pass(calendar)?;
        match event_from_google(raw, calendar, pass.default_reminder, self.zone) {
            Some(Item::Event(event)) => {
                touched.insert(event.id.clone());
                let series = if event.is_series() {
                    Some(event.id.clone())
                } else {
                    event.recurring_event_id.clone()
                };
                self.store
                    .put_google_events(self.account, &[(event, raw.to_string())])?;
                if let Some(series) = series
                    && let Some(refreshed) = pass.refresh_series(&series, touched)?
                {
                    self.store.put_google_events(self.account, &[refreshed])?;
                }
            }
            Some(Item::Deleted(id)) => {
                touched.extend(self.store.delete_calendar_events(
                    self.account,
                    calendar,
                    Some(&[id]),
                )?);
            }
            None => {}
        }
        Ok(())
    }

    /// Google's version of an event, stored; gone if Google has none.
    fn reload(
        &self,
        calendar: &str,
        id: &str,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<()> {
        let url = format!(
            "{}?timeZone={}",
            self.event_url(calendar, id),
            enc(self.zone.name)
        );
        let response = get(self.transport, &url)?;
        match response.status {
            404 | 410 => {
                touched.insert(id.to_string());
                self.store.delete_calendar_events(
                    self.account,
                    calendar,
                    Some(&[id.to_string()]),
                )?;
                Ok(())
            }
            _ if response.is_success() => self.store_raw(calendar, &response.json()?, touched),
            _ => Err(api_error("Couldn't reload the event", &response)),
        }
    }

    /// The occurrence of `series` its rule puts at `original` (wall time),
    /// from `events.instances` around it.
    fn find_instance(
        &self,
        calendar: &str,
        series: &str,
        original: NaiveDateTime,
    ) -> anyhow::Result<Value> {
        let at = (self.zone.to_utc)(original);
        let url = format!(
            "{}/instances?maxResults=50&timeZone={}&timeMin={}&timeMax={}",
            self.event_url(calendar, series),
            enc(self.zone.name),
            enc(&rfc3339(at - Duration::days(1))),
            enc(&rfc3339(at + Duration::days(1))),
        );
        let Listing::Items { items, .. } =
            list_all::<Value>(self.transport, &url, "Couldn't find the occurrence", None)?
        else {
            return Err(anyhow!("Couldn't find the occurrence: the series is gone"));
        };
        items
            .into_iter()
            .find(|raw| {
                GoogleTime::deserialize(&raw["originalStartTime"])
                    .ok()
                    .and_then(|t| wall_time(&t, self.zone))
                    .is_some_and(|(t, _)| t == original)
            })
            .ok_or_else(|| anyhow!("Couldn't find that occurrence in Google Calendar"))
    }

    fn apply(&self, op: &CalOp, touched: &mut BTreeSet<String>) -> anyhow::Result<()> {
        match op {
            CalOp::Insert {
                event,
                send_updates,
            } => self.insert(event, *send_updates, touched),
            CalOp::Patch {
                id,
                before,
                after,
                scope,
                send_updates,
            } => self.patch(id, before, after, *scope, *send_updates, touched),
            // "This and following" (CAL-6): the new series goes in only
            // once the old one ended.
            CalOp::Split {
                id,
                before,
                after,
                new,
                send_updates,
            } => {
                self.patch(
                    id,
                    before,
                    after,
                    SeriesScope::Following,
                    *send_updates,
                    touched,
                )?;
                self.insert(new, *send_updates, touched)
            }
            CalOp::Delete {
                id,
                occurrence,
                scope,
                send_updates,
            } => self.delete(id, *occurrence, *scope, *send_updates, touched),
            CalOp::Move { id, to_calendar } => self.move_to(id, to_calendar),
            CalOp::Rsvp { id, status } => self.rsvp(id, *status, SendUpdates::All).map(|_| ()),
            CalOp::SetSelected { calendar, selected } => {
                self.send(
                    "PATCH",
                    format!("{API}/users/me/calendarList/{}", enc(calendar)),
                    Some(&json!({ "selected": selected })),
                    None,
                    &[],
                )?;
                self.store.apply_cal_op(self.account, self.account, op)
            }
        }
    }

    fn insert(
        &self,
        event: &Event,
        send: SendUpdates,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<()> {
        if event.recurring_event_id.is_some() {
            // One occurrence changed on its own: Google's instance, with
            // what differs from its series.
            let before = self.series_at_occurrence(event).unwrap_or_else(|| Event {
                id: event.id.clone(),
                calendar: event.calendar.clone(),
                ..Event::default()
            });
            return self.patch_instance(&before, event, send, touched);
        }
        let query = format!("?sendUpdates={}", send_updates_param(send));
        let body = insert_body(event, self.zone);
        let url = format!(
            "{}{query}{}",
            self.events_url(&event.calendar),
            conference_param(&body)
        );
        let response = self.send("POST", url, Some(&body), None, &[409])?;
        let response = if response.status == 409 {
            // Its id was deleted before (an undo after the delete went):
            // Google restores a cancelled event by confirming it again.
            self.send(
                "PATCH",
                format!("{}{query}", self.event_url(&event.calendar, &event.id)),
                Some(&json!({ "status": "confirmed" })),
                None,
                &[],
            )?
        } else {
            response
        };
        let mut stored: Value = response.json()?;
        // The app has it already, but not its Meet.
        let mut seen = BTreeSet::new();
        if body.get("conferenceData").is_some() {
            stored = self.await_meet(&event.calendar, stored, touched)?;
            touched.insert(event.id.clone());
        }
        self.store_raw(&event.calendar, &stored, &mut seen)
    }

    /// A write that asked for a Meet: Google's event once the Meet is
    /// there, asking again a few times while it's being set up (CAL-34).
    /// Fails, retryably, when Google refuses or takes too long; what it
    /// answered last is stored.
    fn await_meet(
        &self,
        calendar: &str,
        mut event: Value,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<Value> {
        const WHAT: &str = "Couldn't add Google Meet";
        for attempt in 0..=MEET_POLLS {
            match event["conferenceData"]["createRequest"]["status"]["statusCode"].as_str() {
                Some("pending") => {}
                Some("failure") => {
                    self.store_raw(calendar, &event, touched)?;
                    return Err(anyhow!("{WHAT}: Google refused it"));
                }
                _ => return Ok(event),
            }
            if attempt == MEET_POLLS {
                break;
            }
            (self.pause)(MEET_POLL_EVERY);
            let id = event["id"].as_str().unwrap_or_default().to_string();
            let url = format!(
                "{}?timeZone={}",
                self.event_url(calendar, &id),
                enc(self.zone.name)
            );
            let response = get(self.transport, &url)?;
            if !response.is_success() {
                return Err(api_error(WHAT, &response));
            }
            event = response.json()?;
        }
        self.store_raw(calendar, &event, touched)?;
        Err(anyhow!("{WHAT}: Google is still setting it up"))
    }

    /// The series of an occurrence as stored, as that one occurrence.
    fn series_at_occurrence(&self, event: &Event) -> Option<Event> {
        let series_id = event.recurring_event_id.as_deref()?;
        let original = event.original_start?;
        let (series, _) = self.stored(&event.calendar, series_id)?;
        Some(Event {
            id: event.id.clone(),
            start: original,
            end: original + series.duration(),
            recurrence: None,
            exceptions: Vec::new(),
            recurring_event_id: Some(series_id.to_string()),
            original_start: Some(original),
            ..series
        })
    }

    /// Patch Google's instance for the occurrence `after` takes out of its
    /// series. If Google's id isn't the one Pigeon gave it, the app swaps.
    fn patch_instance(
        &self,
        before: &Event,
        after: &Event,
        send: SendUpdates,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<()> {
        let series = after
            .recurring_event_id
            .as_deref()
            .ok_or_else(|| anyhow!("not an occurrence"))?;
        let original = after.original_start.unwrap_or(after.start);
        let instance = self.find_instance(&after.calendar, series, original)?;
        let instance_id = instance["id"].as_str().unwrap_or_default().to_string();
        let body = patch_body(before, after, Some(&instance), self.zone);
        let stored = if body.is_empty() {
            instance
        } else {
            let url = format!(
                "{}?sendUpdates={}",
                self.event_url(&after.calendar, &instance_id),
                send_updates_param(send)
            );
            self.send(
                "PATCH",
                url,
                Some(&Value::Object(body)),
                instance["etag"].as_str(),
                &[],
            )?
            .json()?
        };
        let mut seen = BTreeSet::new();
        self.store_raw(&after.calendar, &stored, &mut seen)?;
        // Changing an instance gives its series a new etag.
        self.reload(&after.calendar, series, &mut seen)?;
        if instance_id != after.id {
            touched.insert(after.id.clone());
            touched.insert(instance_id);
        }
        Ok(())
    }

    fn patch(
        &self,
        id: &str,
        before: &Event,
        after: &Event,
        scope: SeriesScope,
        send: SendUpdates,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<()> {
        let stored = self
            .stored(&after.calendar, id)
            .or_else(|| self.stored(&before.calendar, id));
        let Some((current, raw)) = stored else {
            if after.recurring_event_id.is_some() {
                // An occurrence just taken out of its series.
                return self.patch_instance(before, after, send, touched);
            }
            return Err(anyhow!("This event isn't in Google Calendar"));
        };
        let body = patch_body(before, after, raw.as_ref(), self.zone);
        if body.is_empty() {
            return Ok(());
        }
        let body = Value::Object(body);
        let url = format!(
            "{}?sendUpdates={}{}",
            self.event_url(&current.calendar, id),
            send_updates_param(send),
            conference_param(&body)
        );
        let etag = Some(current.etag.as_str()).filter(|e| !e.is_empty());
        let response = self.send(
            "PATCH",
            url,
            Some(&body),
            etag.or(Some(after.etag.as_str())),
            &[],
        )?;
        // "This and following" (CAL-6): the series' occurrences changed on
        // their own past its new end go, as a delete of the following does.
        if scope == SeriesScope::Following
            && let Some(until) = after.recurrence.as_ref().and_then(|r| r.until)
        {
            self.store.drop_instances_from(self.account, id, until)?;
        }
        // No longer repeating: its occurrences of their own go too.
        if before.is_series() && !after.is_series() {
            touched.extend(
                self.store
                    .delete_instances(self.account, &current.calendar, id)?,
            );
        }
        let mut stored: Value = response.json()?;
        if body.get("conferenceData").is_some() {
            stored = self.await_meet(&current.calendar, stored, touched)?;
            touched.insert(id.to_string());
        }
        let mut seen = BTreeSet::new();
        self.store_raw(&current.calendar, &stored, &mut seen)
    }

    fn delete(
        &self,
        id: &str,
        occurrence: Option<NaiveDateTime>,
        scope: SeriesScope,
        mut send: SendUpdates,
        touched: &mut BTreeSet<String>,
    ) -> anyhow::Result<()> {
        // Never stored: it never reached Google, or is gone already.
        let Some((event, raw)) = self.locate(id) else {
            return Ok(());
        };
        let calendar = event.calendar.clone();
        let scope = match (scope, occurrence) {
            (_, None) => SeriesScope::All,
            _ if !event.is_series() => SeriesScope::All,
            (SeriesScope::Following, Some(at)) if at <= event.start => SeriesScope::All,
            (SeriesScope::Following, _) if !is_organizer(&event, self.account) => SeriesScope::All,
            (scope, _) => scope,
        };
        let target = match (scope, occurrence) {
            (SeriesScope::This, Some(at)) => {
                let instance = self.find_instance(&calendar, id, at)?;
                instance["id"].as_str().unwrap_or(id).to_string()
            }
            _ => id.to_string(),
        };
        if !is_organizer(&event, self.account) {
            // Someone else's: decline it, then take it off this calendar.
            self.rsvp(&target, RsvpStatus::Declined, SendUpdates::All)
                .ok();
            send = SendUpdates::None;
        }
        let local = CalOp::Delete {
            id: id.to_string(),
            occurrence,
            scope,
            send_updates: send,
        };
        if scope == SeriesScope::Following {
            let mut series = event.clone();
            if let Some(rule) = &mut series.recurrence {
                rule.until = occurrence;
                rule.count = None;
            }
            let body = json!({ "recurrence": recurrence_lines(event.recurrence.as_ref(), &series, raw.as_ref(), self.zone) });
            let url = format!(
                "{}?sendUpdates={}",
                self.event_url(&calendar, id),
                send_updates_param(send)
            );
            let response = self.send("PATCH", url, Some(&body), Some(&event.etag), &[])?;
            self.store
                .apply_cal_op(self.account, self.account, &local)?;
            return self.store_raw(&calendar, &response.json()?, touched);
        }
        let url = format!(
            "{}?sendUpdates={}",
            self.event_url(&calendar, &target),
            send_updates_param(send)
        );
        self.send("DELETE", url, None, None, &[404, 410])?;
        self.store
            .apply_cal_op(self.account, self.account, &local)?;
        if scope == SeriesScope::All {
            touched.insert(id.to_string());
        } else {
            // The cancelled occurrence, and the series' new etag.
            let mut seen = BTreeSet::new();
            self.reload(&calendar, &target, &mut seen)?;
            self.reload(&calendar, id, &mut seen)?;
        }
        Ok(())
    }

    /// `events.move`, then the per-calendar fields it drops (the reminders,
    /// the color) patched back onto the moved event.
    fn move_to(&self, id: &str, to: &str) -> anyhow::Result<()> {
        let Some((event, raw)) = self.locate(id) else {
            return Err(anyhow!("This event isn't in Google Calendar"));
        };
        if event.calendar == to {
            return Ok(());
        }
        let url = format!(
            "{}/move?destination={}&sendUpdates=none",
            self.event_url(&event.calendar, id),
            enc(to)
        );
        let moved: Value = self.send("POST", url, None, None, &[])?.json()?;
        // Whatever the move answers, Google drops these right after (a
        // later read has neither): set them again.
        let mut body = Map::new();
        if let Some(raw) = &raw {
            if raw["reminders"]["useDefault"] == json!(false) {
                body.insert("reminders".into(), raw["reminders"].clone());
            }
            if raw["colorId"].is_string() {
                body.insert("colorId".into(), raw["colorId"].clone());
            }
        }
        self.store.delete_calendar_events(
            self.account,
            &event.calendar,
            Some(&[id.to_string()]),
        )?;
        let mut seen = BTreeSet::new();
        if body.is_empty() {
            return self.store_raw(to, &moved, &mut seen);
        }
        // Stored with the old reminders until Google has them back, so the
        // panel never shows the new calendar's default in between.
        let mut interim = moved.clone();
        for (key, value) in &body {
            interim[key] = value.clone();
        }
        self.store_raw(to, &interim, &mut seen)?;
        // No If-Match: the etag `events.move` answers is already stale
        // (Google answers 412 to it), and nobody else had it in between.
        let url = format!("{}?sendUpdates=none", self.event_url(to, id));
        let patched = self.send("PATCH", url, Some(&Value::Object(body)), None, &[])?;
        self.store_raw(to, &patched.json()?, &mut seen)
    }

    /// The user's own answer, the rest of the guest list as Google has it
    /// (CAL-14).
    fn rsvp(&self, id: &str, status: RsvpStatus, send: SendUpdates) -> anyhow::Result<()> {
        let Some((event, raw)) = self.locate(id) else {
            return Err(anyhow!("This event isn't in Google Calendar"));
        };
        let me = self.account;
        let mut attendees: Vec<Value> = raw
            .as_ref()
            .and_then(|r| r["attendees"].as_array().cloned())
            .unwrap_or_default();
        let mine = attendees.iter_mut().find(|a| {
            a["self"].as_bool() == Some(true)
                || a["email"]
                    .as_str()
                    .is_some_and(|e| e.eq_ignore_ascii_case(me))
        });
        match mine {
            Some(entry) => entry["responseStatus"] = json!(rsvp_key(status)),
            None => attendees.push(json!({ "email": me, "responseStatus": rsvp_key(status) })),
        }
        let url = format!(
            "{}?sendUpdates={}",
            self.event_url(&event.calendar, id),
            send_updates_param(send)
        );
        let response = self.send(
            "PATCH",
            url,
            Some(&json!({ "attendees": attendees })),
            None,
            &[],
        )?;
        let mut seen = BTreeSet::new();
        self.store_raw(&event.calendar, &response.json()?, &mut seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::http::FakeTransport,
        calendar::{occurrences, time::at},
    };
    use chrono::{NaiveDate, TimeZone as _};

    const ME: &str = "me@example.com";
    const LIST: &str = include_str!("../../tests/fixtures/google/calendar/calendar_list.json");
    const COLORS: &str = include_str!("../../tests/fixtures/google/calendar/colors.json");
    const EVENTS: &str = include_str!("../../tests/fixtures/google/calendar/events_full.json");
    const EVENTS_PAGE_2: &str =
        include_str!("../../tests/fixtures/google/calendar/events_full_page2.json");
    const EVENTS_DELTA: &str =
        include_str!("../../tests/fixtures/google/calendar/events_delta.json");
    const INSTANCES: &str = include_str!("../../tests/fixtures/google/calendar/instances.json");

    fn zone() -> LocalZone {
        LocalZone {
            name: "America/Los_Angeles",
            to_local: |t| t.naive_utc() - Duration::hours(7),
            to_utc: |t| (t + Duration::hours(7)).and_utc(),
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 26, 17, 0, 0).unwrap()
    }

    fn item(json: &str) -> Item {
        let raw: Value = serde_json::from_str(json).unwrap();
        event_from_google(&raw, "primary@example.com", Some(30), &zone()).unwrap()
    }

    fn event(json: &str) -> Event {
        match item(json) {
            Item::Event(e) => e,
            Item::Deleted(id) => panic!("{id} deleted"),
        }
    }

    #[test]
    fn timed_event_with_offset() {
        // 13:00 in New York is 10:00 here.
        let e = event(
            r#"{"id":"e1","etag":"\"1\"","status":"confirmed","summary":"Review",
                "start":{"dateTime":"2026-10-28T13:00:00-04:00","timeZone":"America/New_York"},
                "end":{"dateTime":"2026-10-28T14:30:00-04:00","timeZone":"America/New_York"},
                "transparency":"transparent","iCalUID":"e1@google.com",
                "organizer":{"email":"me@example.com","self":true},
                "attendees":[{"email":"a@example.com","displayName":"Ann","responseStatus":"accepted"},
                             {"email":"room@resource.example.com","resource":true,"responseStatus":"accepted"},
                             {"email":"me@example.com","self":true,"responseStatus":"needsAction"}],
                "conferenceData":{"entryPoints":[{"entryPointType":"phone","uri":"tel:+1"},
                                                 {"entryPointType":"video","uri":"https://meet.google.com/abc"}]},
                "reminders":{"useDefault":false,"overrides":[{"method":"popup","minutes":10}]},
                "description":"<b>Agenda</b>"}"#,
        );
        assert_eq!(e.start, at(d(2026, 10, 28), 10, 0));
        assert_eq!(e.end, at(d(2026, 10, 28), 11, 30));
        assert!(!e.all_day && !e.busy && e.organizer_self);
        assert_eq!(e.time_zone.as_deref(), Some("America/New_York"));
        assert_eq!(e.attendees.len(), 2);
        assert_eq!(e.attendees[0].status, RsvpStatus::Accepted);
        assert_eq!(e.attendees[1].status, RsvpStatus::NeedsAction);
        assert_eq!(e.conference_uri, "https://meet.google.com/abc");
        assert_eq!(e.video_link, e.conference_uri);
        assert_eq!(e.reminder_minutes, Some(10));
        assert_eq!(e.description, "Agenda");
        assert_eq!(e.ical_uid, "e1@google.com");
        assert_eq!(e.event_type, "default");
    }

    #[test]
    fn all_day_event_ends_exclusive_and_uses_default_reminder() {
        let e = event(
            r#"{"id":"e2","status":"confirmed","summary":"Offsite",
                "start":{"date":"2026-10-29"},"end":{"date":"2026-10-31"},
                "reminders":{"useDefault":true},"hangoutLink":"https://meet.google.com/xyz"}"#,
        );
        assert!(e.all_day);
        assert_eq!(e.start, at(d(2026, 10, 29), 0, 0));
        assert_eq!(e.end, at(d(2026, 10, 31), 0, 0));
        assert_eq!(e.reminder_minutes, Some(30));
        assert!(e.reminder_use_default);
        assert_eq!(e.conference_uri, "https://meet.google.com/xyz");
    }

    #[test]
    fn cancelled_and_modified_instances() {
        let cancelled = event(
            r#"{"id":"s1_20261028T170000Z","status":"cancelled","recurringEventId":"s1",
                "originalStartTime":{"dateTime":"2026-10-28T10:00:00-07:00"}}"#,
        );
        assert_eq!(cancelled.status, EventStatus::Cancelled);
        assert_eq!(cancelled.recurring_event_id.as_deref(), Some("s1"));
        assert_eq!(cancelled.original_start, Some(at(d(2026, 10, 28), 10, 0)));
        // A cancelled event that isn't an instance is gone.
        assert_eq!(
            item(r#"{"id":"x_R20260911T023000","status":"cancelled"}"#),
            Item::Deleted("x_R20260911T023000".into())
        );
        let moved = event(
            r#"{"id":"s1_20261030T170000Z","status":"confirmed","summary":"Standup",
                "recurringEventId":"s1",
                "originalStartTime":{"dateTime":"2026-10-30T10:00:00-07:00"},
                "start":{"dateTime":"2026-10-30T11:00:00-07:00"},
                "end":{"dateTime":"2026-10-30T11:15:00-07:00"}}"#,
        );
        assert_eq!(moved.start, at(d(2026, 10, 30), 11, 0));
        assert_eq!(moved.original_start, Some(at(d(2026, 10, 30), 10, 0)));
        assert!(!moved.is_series());
    }

    #[test]
    fn series_in_another_zone_uses_googles_occurrences() {
        let e = event(
            r#"{"id":"s","status":"confirmed","summary":"Sync",
                "start":{"dateTime":"2026-10-27T08:30:00+03:00","timeZone":"Asia/Riyadh"},
                "end":{"dateTime":"2026-10-27T09:30:00+03:00","timeZone":"Asia/Riyadh"},
                "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=TU"]}"#,
        );
        // Tuesday 08:30 there is Monday 22:30 here.
        assert_eq!(e.start, at(d(2026, 10, 26), 22, 30));
        let rule = e.recurrence.unwrap();
        assert!(rule.unsupported && !rule.custom);
        assert_eq!(rule.label(), "Every week on Tue");
    }

    fn fake() -> FakeTransport {
        FakeTransport::new()
            .on("GET", "/calendar/v3/colors", &[], &[(200, COLORS)])
            .on(
                "GET",
                "/calendar/v3/users/me/calendarList",
                &[],
                &[(200, LIST)],
            )
            .on(
                "GET",
                "/calendar/v3/calendars/primary%40example.com/events",
                &[("pageToken", "p2")],
                &[(200, EVENTS_PAGE_2)],
            )
            .on(
                "GET",
                "/calendar/v3/calendars/primary%40example.com/events",
                &[("syncToken", "sync-1")],
                &[
                    (200, EVENTS_DELTA),
                    (410, r#"{"error":{"code":410,"message":"Gone"}}"#),
                ],
            )
            .on(
                "GET",
                "/calendar/v3/calendars/primary%40example.com/events",
                &[],
                &[(200, EVENTS)],
            )
            .on(
                "GET",
                "/calendar/v3/calendars/primary%40example.com/events/odd/instances",
                &[],
                &[(200, INSTANCES)],
            )
            .on(
                "GET",
                "/calendar/v3/calendars/en.usa%23holiday%40group.v.calendar.google.com/events",
                &[],
                &[(200, r#"{"items":[],"nextSyncToken":"h1"}"#)],
            )
            .on(
                "GET",
                "/calendar/v3/calendars/team%40example.com/events",
                &[],
                &[(403, r#"{"error":{"code":403,"message":"Forbidden"}}"#)],
            )
    }

    fn week(store: &Store) -> Vec<(String, NaiveDateTime)> {
        let events = store.load_all_events(ME).unwrap();
        let from = at(d(2026, 10, 26), 0, 0);
        occurrences(&events, |_| true, from, from + Duration::days(7))
            .into_iter()
            .map(|o| (events[o.event].title.clone(), o.start))
            .collect()
    }

    /// A calendar list sync while a show/hide waits to go: Google still
    /// lists the old state, and the store keeps the user's.
    #[test]
    fn a_list_sync_keeps_a_queued_show_hide() {
        let store = Store::open_in_memory().unwrap();
        let fake = FakeTransport::new().on(
            "GET",
            "/calendar/v3/users/me/calendarList",
            &[],
            &[(200, LIST)],
        );
        let list = |store: &Store| {
            sync_calendar_list(
                &fake,
                store,
                ME,
                &HashMap::new(),
                &mut CalendarChanges::default(),
                &mut BTreeSet::new(),
            )
            .unwrap();
            store.load_calendars(ME).unwrap()[0].visible
        };
        let hide = |selected| crate::backend::CalOp::SetSelected {
            calendar: "primary@example.com".into(),
            selected,
        };
        assert!(list(&store));

        // Hidden here (the row waits in the outbox); Google still says shown.
        store.apply_cal_op(ME, ME, &hide(false)).unwrap();
        let op = crate::backend::Op::Calendar(hide(false));
        let row = store
            .outbox_insert(
                ME,
                op.kind(),
                &op.resource(),
                &serde_json::to_string(&op).unwrap(),
                0,
                0,
            )
            .unwrap();
        assert!(!list(&store), "the queued change wins over the list");

        // Committed: the list is Google's again.
        store.outbox_delete(row).unwrap();
        assert!(list(&store));
    }

    #[test]
    fn full_sync_pages_then_syncs_by_token_and_resyncs_on_410() {
        let store = Store::open_in_memory().unwrap();
        let fake = fake();
        let changes = sync_pass_at(&fake, &store, ME, &zone(), now()).unwrap();
        assert!(changes.calendars);
        assert_eq!(changes.errors.len(), 1, "the 403 calendar is skipped");

        // Calendars: hidden left out, colour from the entry or the palette.
        let calendars = store.load_calendars(ME).unwrap();
        let names: Vec<&str> = calendars.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Me", "Team", "Holidays"]);
        assert!(calendars[0].primary && calendars[0].visible);
        assert_eq!(calendars[0].default_reminder, Some(30));
        assert!(!calendars[2].visible);
        assert_eq!(calendars[2].access_role, AccessRole::Reader);
        assert_eq!(
            crate::data::to_hex(calendars[2].color),
            crate::data::to_hex(crate::data::from_hex("#16a765").unwrap())
        );

        // The first page asked for 180 days back in the local zone.
        let first = &fake.requests()[2];
        assert!(first.url.contains("singleEvents=false"));
        assert!(first.url.contains("timeZone=America%2FLos_Angeles"));
        assert!(first.url.contains("timeMin=2026-04-29T17%3A00%3A00Z"));
        assert!(
            fake.requests()
                .iter()
                .any(|r| r.url.contains("pageToken=p2"))
        );

        // The series skips its cancelled and moved occurrences; the moved
        // one shows where it went; the unsupported rule shows Google's.
        let shown = week(&store);
        let standups: Vec<NaiveDateTime> = shown
            .iter()
            .filter(|(t, _)| t == "Standup")
            .map(|(_, s)| *s)
            .collect();
        assert_eq!(
            standups,
            [
                at(d(2026, 10, 26), 9, 0),
                at(d(2026, 10, 27), 9, 0),
                at(d(2026, 10, 29), 9, 0),
                at(d(2026, 10, 30), 11, 0),
            ]
        );
        let odd: Vec<NaiveDateTime> = shown
            .iter()
            .filter(|(t, _)| t == "Odd rule")
            .map(|(_, s)| *s)
            .collect();
        assert_eq!(odd, [at(d(2026, 10, 27), 12, 0)]);
        assert!(shown.iter().any(|(t, _)| t == "Offsite"));
        assert_eq!(
            store
                .calendar_sync_token(ME, "primary@example.com")
                .unwrap()
                .as_deref(),
            Some("sync-1")
        );

        // By token: one event renamed, one deleted, one more occurrence
        // cancelled.
        let changes = sync_pass_at(&fake, &store, ME, &zone(), now()).unwrap();
        assert!(changes.removed.contains(&"offsite".to_string()));
        assert!(changes.changed.contains(&"standup".to_string()));
        let shown = week(&store);
        assert!(shown.iter().any(|(t, _)| t == "Review (moved)"));
        assert!(!shown.iter().any(|(t, _)| t == "Offsite"));
        assert_eq!(shown.iter().filter(|(t, _)| t == "Standup").count(), 3);
        let token_request = fake
            .requests()
            .into_iter()
            .rev()
            .find(|r| r.path() == "/calendar/v3/calendars/primary%40example.com/events");
        let token_request = token_request.unwrap();
        assert!(token_request.url.contains("syncToken=sync-1"));
        assert!(!token_request.url.contains("timeMin"));

        // 410: the calendar is wiped and listed in full again.
        sync_pass_at(&fake, &store, ME, &zone(), now()).unwrap();
        let shown = week(&store);
        assert!(
            shown.iter().any(|(t, _)| t == "Offsite"),
            "back from the full list"
        );
        assert_eq!(shown.iter().filter(|(t, _)| t == "Standup").count(), 4);
        // Colours were fetched once.
        let colors = fake
            .requests()
            .iter()
            .filter(|r| r.path() == "/calendar/v3/colors")
            .count();
        assert_eq!(colors, 1);
    }

    #[test]
    fn changes_become_backend_events() {
        let changes = CalendarChanges {
            calendars: true,
            changed: vec!["a".into()],
            removed: vec!["b".into()],
            errors: vec![],
        };
        assert_eq!(
            changes.backend_events(),
            [
                BackendEvent::CalendarsChanged,
                BackendEvent::EventsChanged(vec!["a".into()]),
                BackendEvent::EventsRemoved(vec!["b".into()]),
            ]
        );
        assert!(CalendarChanges::default().is_empty());
    }

    // Writes (W7).

    const CAL: &str = "primary@example.com";
    const CAL_PATH: &str = "/calendar/v3/calendars/primary%40example.com/events";
    const SERIES: &str = r#"{"id":"s1","etag":"\"s\"","status":"confirmed","summary":"Standup",
        "organizer":{"email":"me@example.com","self":true},
        "start":{"dateTime":"2026-10-26T10:00:00-07:00","timeZone":"America/Los_Angeles"},
        "end":{"dateTime":"2026-10-26T10:30:00-07:00","timeZone":"America/Los_Angeles"},
        "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE","EXDATE;TZID=America/Los_Angeles:20261102T100000"]}"#;
    const SINGLE: &str = r#"{"id":"e1","etag":"\"1\"","status":"confirmed","summary":"Review",
        "organizer":{"email":"me@example.com","self":true},
        "start":{"dateTime":"2026-10-28T10:00:00-07:00"},"end":{"dateTime":"2026-10-28T11:00:00-07:00"},
        "attendees":[{"email":"me@example.com","self":true,"responseStatus":"accepted"},
                     {"email":"room@resource.example.com","resource":true,"responseStatus":"accepted"},
                     {"email":"ann@example.com","responseStatus":"needsAction"}]}"#;

    /// A store with the primary calendar and `events` from Google.
    fn seeded(events: &[&str]) -> Store {
        let store = Store::open_in_memory().unwrap();
        let entry = CalendarListEntry {
            id: CAL.into(),
            summary: "Me".into(),
            selected: true,
            access_role: "owner".into(),
            ..Default::default()
        };
        store
            .put_google_calendar(ME, &calendar_from_entry(&entry, &HashMap::new()), false)
            .unwrap();
        for json in events {
            let raw: Value = serde_json::from_str(json).unwrap();
            let Some(Item::Event(e)) = event_from_google(&raw, CAL, None, &zone()) else {
                panic!("not an event");
            };
            store
                .put_google_events(ME, &[(e, raw.to_string())])
                .unwrap();
        }
        store
    }

    fn stored(store: &Store, id: &str) -> Event {
        store.load_events(ME, &[id.to_string()]).unwrap().remove(0)
    }

    fn body(request: &Request) -> Value {
        serde_json::from_slice(&request.body).unwrap()
    }

    fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
        request
            .headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn patch_body_carries_only_what_changed() {
        let store = seeded(&[SINGLE]);
        let before = stored(&store, "e1");
        let raw: Value = serde_json::from_str(SINGLE).unwrap();
        let patch = |f: &dyn Fn(&mut Event)| {
            let mut after = before.clone();
            f(&mut after);
            patch_body(&before, &after, Some(&raw), &zone())
        };
        assert!(patch(&|_| {}).is_empty());

        let p = patch(&|e| {
            e.title = "Plan".into();
            e.busy = false;
        });
        let keys: Vec<&String> = p.keys().collect();
        assert_eq!(keys, ["summary", "transparency"]);
        assert_eq!(p["transparency"], "transparent");

        // Timed: the instant, with the user's zone; the date cleared.
        let p = patch(&|e| {
            e.start += Duration::hours(1);
            e.end += Duration::hours(1);
        });
        assert_eq!(
            p["start"],
            json!({"dateTime":"2026-10-28T18:00:00Z","timeZone":"America/Los_Angeles","date":null})
        );
        // All day: dates, the time cleared.
        let p = patch(&|e| {
            e.all_day = true;
            e.start = at(d(2026, 10, 28), 0, 0);
            e.end = at(d(2026, 10, 29), 0, 0);
        });
        assert_eq!(p["start"]["date"], "2026-10-28");
        assert_eq!(p["end"]["date"], "2026-10-29");
        assert_eq!(p["start"]["dateTime"], Value::Null);

        let p = patch(&|e| e.reminder_minutes = Some(30));
        assert_eq!(
            p["reminders"],
            json!({"useDefault":false,"overrides":[{"method":"popup","minutes":30}]})
        );
        let reminded = Event {
            reminder_minutes: Some(30),
            ..before.clone()
        };
        assert_eq!(
            patch_body(&reminded, &before, Some(&raw), &zone())["reminders"],
            json!({"useDefault":false,"overrides":[]})
        );

        // Guests: the room stays though Pigeon doesn't show it; Ann goes,
        // Bob comes.
        let p = patch(&|e| {
            e.attendees.retain(|a| a.email != "ann@example.com");
            e.attendees.push(Attendee {
                name: "Bob".into(),
                email: "bob@example.com".into(),
                status: RsvpStatus::NeedsAction,
            });
        });
        let emails: Vec<&str> = p["attendees"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["email"].as_str().unwrap())
            .collect();
        assert_eq!(
            emails,
            [
                "me@example.com",
                "room@resource.example.com",
                "bob@example.com"
            ]
        );
        assert!(
            patch(&|e| {
                e.attendees_omitted = true;
                e.attendees.clear();
            })
            .is_empty()
        );

        // A typed video link: into the empty location, else atop the notes.
        let p = patch(&|e| e.video_link = "https://zoom.example.com/j/1".into());
        assert_eq!(p["location"], "https://zoom.example.com/j/1");
        let p = patch(&|e| {
            e.location = "Room 4".into();
            e.description = "Agenda".into();
            e.video_link = "https://zoom.example.com/j/1".into();
        });
        assert_eq!(p["description"], "https://zoom.example.com/j/1\n\nAgenda");
        // Google's Meet link isn't written.
        let mut meet = before.clone();
        meet.conference_uri = "https://meet.google.com/x".into();
        let mut after = meet.clone();
        after.video_link = meet.conference_uri.clone();
        assert!(patch_body(&meet, &after, Some(&raw), &zone()).is_empty());
    }

    #[test]
    fn guests_are_asked_about_only_what_they_see() {
        let store = seeded(&[SINGLE]);
        let with_guest = stored(&store, "e1");
        let alone = Event {
            attendees: Vec::new(),
            ..with_guest.clone()
        };
        let changed = |e: &Event, f: &dyn Fn(&mut Event)| {
            let mut after = e.clone();
            f(&mut after);
            after
        };
        let title = |e: &mut Event| e.title = "New".into();
        assert!(asks_guests(
            Some(&with_guest),
            &changed(&with_guest, &title),
            ME
        ));
        assert!(!asks_guests(Some(&alone), &changed(&alone, &title), ME));
        // The user's own reminder and busy/free don't concern the guests.
        assert!(!asks_guests(
            Some(&with_guest),
            &changed(&with_guest, &|e| e.reminder_minutes = Some(5)),
            ME
        ));
        assert!(!asks_guests(
            Some(&with_guest),
            &changed(&with_guest, &|e| e.busy = false),
            ME
        ));
        // Removing the last guest still tells them.
        assert!(asks_guests(Some(&with_guest), &alone, ME));
        // A new event: invitations, if it has guests.
        assert!(asks_guests(None, &with_guest, ME));
        assert!(!asks_guests(None, &alone, ME));
        assert_eq!(send_updates_param(SendUpdates::All), "all");
        assert_eq!(send_updates_param(SendUpdates::None), "none");
    }

    #[test]
    fn read_only_rules() {
        let mine = Event {
            organizer: Contact {
                name: String::new(),
                email: ME.into(),
            },
            ..Event::default()
        };
        let me_attending = vec![Attendee {
            name: String::new(),
            email: ME.into(),
            status: RsvpStatus::NeedsAction,
        }];
        let theirs = Event {
            organizer: Contact {
                name: String::new(),
                email: "boss@example.com".into(),
            },
            attendees: me_attending.clone(),
            ..Event::default()
        };
        let cases = [
            (&mine, AccessRole::Owner, Access::Edit),
            (&mine, AccessRole::Writer, Access::Edit),
            (&mine, AccessRole::Reader, Access::ReadOnly),
            (&mine, AccessRole::FreeBusyReader, Access::ReadOnly),
            (&theirs, AccessRole::Owner, Access::Rsvp),
            (&theirs, AccessRole::Reader, Access::ReadOnly),
        ];
        for (event, role, want) in cases {
            assert_eq!(
                access(event, role, ME),
                want,
                "{role:?} {}",
                event.organizer.email
            );
        }
        let modifiable = Event {
            guests_can_modify: true,
            ..theirs.clone()
        };
        assert_eq!(access(&modifiable, AccessRole::Owner, ME), Access::Edit);
        let on_their_calendar = Event {
            organizer: Contact {
                name: String::new(),
                email: "team@group.calendar.google.com".into(),
            },
            organizer_self: true,
            ..Event::default()
        };
        assert_eq!(
            access(&on_their_calendar, AccessRole::Writer, ME),
            Access::Edit
        );
        let birthday = Event {
            event_type: "birthday".into(),
            ..mine.clone()
        };
        assert_eq!(access(&birthday, AccessRole::Owner, ME), Access::ReadOnly);
        let not_invited = Event {
            attendees: Vec::new(),
            ..theirs
        };
        assert_eq!(
            access(&not_invited, AccessRole::Owner, ME),
            Access::ReadOnly
        );
    }

    #[test]
    fn refusals_say_why() {
        let holidays = "Holidays in United Kingdom";
        assert_eq!(refusal(Access::Edit, AccessRole::Owner, holidays), None);
        for role in [AccessRole::Reader, AccessRole::FreeBusyReader] {
            assert_eq!(
                refusal(Access::ReadOnly, role, holidays).as_deref(),
                Some("“Holidays in United Kingdom” is read-only")
            );
        }
        assert_eq!(
            refusal(Access::ReadOnly, AccessRole::Reader, "").as_deref(),
            Some("This calendar is read-only")
        );
        assert_eq!(
            refusal(Access::Rsvp, AccessRole::Owner, "Me").as_deref(),
            Some("Only the organizer can change this event")
        );
        // A birthday on the user's own calendar.
        assert_eq!(
            refusal(Access::ReadOnly, AccessRole::Owner, "Me").as_deref(),
            Some("You can't change this event")
        );
    }

    #[test]
    fn insert_uses_the_client_id() {
        let store = seeded(&[]);
        let event = Event {
            id: "0123456789abcdef0123456789abcdef".into(),
            calendar: CAL.into(),
            title: "Lunch".into(),
            start: at(d(2026, 10, 28), 12, 0),
            end: at(d(2026, 10, 28), 13, 0),
            reminder_minutes: Some(10),
            busy: true,
            ..Event::default()
        };
        let answer = r#"{"id":"0123456789abcdef0123456789abcdef","etag":"\"n\"","status":"confirmed",
            "summary":"Lunch","organizer":{"email":"me@example.com","self":true},
            "start":{"dateTime":"2026-10-28T12:00:00-07:00"},"end":{"dateTime":"2026-10-28T13:00:00-07:00"}}"#;
        let fake = FakeTransport::new().on("POST", CAL_PATH, &[], &[(200, answer)]);
        let op = CalOp::Insert {
            event: event.clone(),
            send_updates: SendUpdates::All,
        };
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        let request = &fake.requests()[0];
        assert!(request.url.ends_with("?sendUpdates=all"));
        let sent = body(request);
        assert_eq!(sent["id"], event.id.as_str());
        assert_eq!(sent["start"]["dateTime"], "2026-10-28T19:00:00Z");
        assert_eq!(sent["reminders"]["overrides"][0]["minutes"], 10);
        assert_eq!(stored(&store, &event.id).etag, "\"n\"");
    }

    /// A series from Google with `reminders`, on a calendar defaulting to 30.
    fn series_with(reminders: &str) -> Event {
        let raw: Value = serde_json::from_str(&SERIES.replace(
            r#""summary":"Standup","#,
            &format!(r#""summary":"Standup","reminders":{reminders},"#),
        ))
        .unwrap();
        let Some(Item::Event(e)) = event_from_google(&raw, CAL, Some(30), &zone()) else {
            panic!("not an event");
        };
        e
    }

    #[test]
    fn split_keeps_the_series_reminders() {
        let origin = at(d(2026, 11, 4), 10, 0);
        let split = |series: &Event| {
            let (_, new) = series.split(origin, origin, series, "n1".into()).unwrap();
            insert_body(&new, &zone())["reminders"].clone()
        };
        let default = series_with(r#"{"useDefault":true}"#);
        assert_eq!(split(&default), json!({"useDefault":true,"overrides":[]}));
        let explicit =
            series_with(r#"{"useDefault":false,"overrides":[{"method":"popup","minutes":5}]}"#);
        assert_eq!(
            split(&explicit),
            json!({"useDefault":false,"overrides":[{"method":"popup","minutes":5}]})
        );
    }

    #[test]
    fn new_event_reminders() {
        let new = Event {
            id: "n".into(),
            calendar: CAL.into(),
            start: at(d(2026, 10, 28), 12, 0),
            end: at(d(2026, 10, 28), 13, 0),
            reminder_minutes: Some(30),
            reminder_use_default: true,
            ..Event::default()
        };
        let reminders = |e: &Event| insert_body(e, &zone())["reminders"].clone();
        assert_eq!(reminders(&new), json!({"useDefault":true,"overrides":[]}));
        let picked = Event {
            reminder_minutes: Some(15),
            reminder_use_default: false,
            ..new.clone()
        };
        assert_eq!(
            reminders(&picked),
            json!({"useDefault":false,"overrides":[{"method":"popup","minutes":15}]})
        );
        let none = Event {
            reminder_minutes: None,
            reminder_use_default: false,
            ..new.clone()
        };
        assert_eq!(reminders(&none), json!({"useDefault":false,"overrides":[]}));
        // Picking the default's own value still makes it explicit; another
        // calendar's default is no write.
        let same = Event {
            reminder_use_default: false,
            ..new.clone()
        };
        assert_eq!(
            patch_body(&new, &same, None, &zone())["reminders"],
            json!({"useDefault":false,"overrides":[{"method":"popup","minutes":30}]})
        );
        let other_default = Event {
            reminder_minutes: Some(10),
            ..new.clone()
        };
        assert!(patch_body(&new, &other_default, None, &zone()).is_empty());
    }

    #[test]
    fn instance_lookup_patches_googles_occurrence() {
        let store = seeded(&[SERIES]);
        let series = stored(&store, "s1");
        let original = at(d(2026, 10, 28), 10, 0);
        let id = instance_id("s1", original, false, &zone());
        assert_eq!(id, "s1_20261028T170000Z");
        assert_eq!(
            instance_id("s1", at(d(2026, 10, 28), 0, 0), true, &zone()),
            "s1_20261028"
        );
        let instances = r#"{"items":[
            {"id":"s1_20261026T170000Z","etag":"\"i1\"","status":"confirmed","summary":"Standup",
             "recurringEventId":"s1","originalStartTime":{"dateTime":"2026-10-26T10:00:00-07:00"},
             "start":{"dateTime":"2026-10-26T10:00:00-07:00"},"end":{"dateTime":"2026-10-26T10:30:00-07:00"}},
            {"id":"s1_20261028T170000Z","etag":"\"i2\"","status":"confirmed","summary":"Standup",
             "recurringEventId":"s1","originalStartTime":{"dateTime":"2026-10-28T10:00:00-07:00"},
             "start":{"dateTime":"2026-10-28T10:00:00-07:00"},"end":{"dateTime":"2026-10-28T10:30:00-07:00"}}]}"#;
        let patched = r#"{"id":"s1_20261028T170000Z","etag":"\"i3\"","status":"confirmed","summary":"Standup (late)",
             "recurringEventId":"s1","originalStartTime":{"dateTime":"2026-10-28T10:00:00-07:00"},
             "start":{"dateTime":"2026-10-28T11:00:00-07:00"},"end":{"dateTime":"2026-10-28T11:30:00-07:00"}}"#;
        let fake = FakeTransport::new()
            .on(
                "GET",
                &format!("{CAL_PATH}/s1/instances"),
                &[],
                &[(200, instances)],
            )
            .on(
                "PATCH",
                &format!("{CAL_PATH}/s1_20261028T170000Z"),
                &[],
                &[(200, patched)],
            )
            .on(
                "GET",
                &format!("{CAL_PATH}/s1"),
                &[],
                &[(200, &SERIES.replace(r#"\"s\""#, r#"\"s9\""#))],
            );
        let exception = Event {
            id: id.clone(),
            title: "Standup (late)".into(),
            start: at(d(2026, 10, 28), 11, 0),
            end: at(d(2026, 10, 28), 11, 30),
            recurrence: None,
            exceptions: Vec::new(),
            recurring_event_id: Some("s1".into()),
            original_start: Some(original),
            ..series
        };
        let op = CalOp::Insert {
            event: exception,
            send_updates: SendUpdates::None,
        };
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        assert!(done.changes.removed.is_empty(), "same id as Google's");
        let requests = fake.requests();
        assert!(requests[0].url.contains("timeMin=2026-10-27T17%3A00%3A00Z"));
        let patch = &requests[1];
        assert_eq!(header(patch, "If-Match"), Some("\"i2\""));
        let sent = body(patch);
        let keys: Vec<&String> = sent.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["summary", "start", "end"]);
        // The series skips it; the instance is stored on its own.
        assert!(stored(&store, "s1").exceptions.contains(&original));
        assert_eq!(stored(&store, &id).etag, "\"i3\"");
        // Changing the instance changed the series' etag; it's read again.
        assert_eq!(stored(&store, "s1").etag, "\"s9\"");
    }

    #[test]
    fn until_rewrite() {
        assert_eq!(
            rewrite_rrule(
                "RRULE:FREQ=WEEKLY;COUNT=10;BYDAY=MO",
                Some("20261104T165959Z"),
                None
            ),
            "RRULE:FREQ=WEEKLY;BYDAY=MO;UNTIL=20261104T165959Z"
        );
        assert_eq!(
            rewrite_rrule("RRULE:FREQ=DAILY;UNTIL=20261231", None, Some(3)),
            "RRULE:FREQ=DAILY;COUNT=3"
        );
        assert_eq!(
            rewrite_rrule("RRULE:FREQ=DAILY;UNTIL=20261231", None, None),
            "RRULE:FREQ=DAILY"
        );
        assert_eq!(
            ical_until(at(d(2026, 11, 4), 0, 0), true, &zone()),
            "20261103"
        );

        // "This and following": the series ends before the occurrence;
        // its EXDATE stays.
        let store = seeded(&[SERIES]);
        let answer = r#"{"id":"s1","etag":"\"s2\"","status":"confirmed","summary":"Standup",
            "organizer":{"email":"me@example.com","self":true},
            "start":{"dateTime":"2026-10-26T10:00:00-07:00","timeZone":"America/Los_Angeles"},
            "end":{"dateTime":"2026-10-26T10:30:00-07:00","timeZone":"America/Los_Angeles"},
            "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20261104T165959Z",
                          "EXDATE;TZID=America/Los_Angeles:20261102T100000"]}"#;
        let fake =
            FakeTransport::new().on("PATCH", &format!("{CAL_PATH}/s1"), &[], &[(200, answer)]);
        let op = CalOp::Delete {
            id: "s1".into(),
            occurrence: Some(at(d(2026, 11, 4), 10, 0)),
            scope: SeriesScope::Following,
            send_updates: SendUpdates::None,
        };
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        let patch = &fake.requests()[0];
        assert!(patch.url.ends_with("?sendUpdates=none"));
        assert_eq!(header(patch, "If-Match"), Some("\"s\""));
        assert_eq!(
            body(patch),
            json!({"recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20261104T165959Z",
                                 "EXDATE;TZID=America/Los_Angeles:20261102T100000"]})
        );
        let series = stored(&store, "s1");
        assert_eq!(series.etag, "\"s2\"");
        assert_eq!(
            series.recurrence.unwrap().until,
            Some(at(d(2026, 11, 4), 10, 0))
        );
        assert!(done.changes.changed.contains(&"s1".to_string()));
    }

    #[test]
    fn split_patches_the_series_end_then_inserts_the_new_series() {
        let store = seeded(&[SERIES]);
        let before = stored(&store, "s1");
        let origin = at(d(2026, 11, 4), 10, 0);
        // An occurrence changed on its own after the split goes.
        let moved = Event {
            id: "s1_20261109T180000Z".into(),
            recurrence: None,
            exceptions: Vec::new(),
            recurring_event_id: Some("s1".into()),
            original_start: Some(at(d(2026, 11, 9), 10, 0)),
            ..before.clone()
        };
        store.put_events(ME, std::slice::from_ref(&moved)).unwrap();
        let edited = Event {
            title: "Standup (new room)".into(),
            ..before.clone()
        };
        let (old, new) = before.split(origin, origin, &edited, "n1".into()).unwrap();
        let answer = r#"{"id":"s1","etag":"\"s2\"","status":"confirmed","summary":"Standup",
            "organizer":{"email":"me@example.com","self":true},
            "start":{"dateTime":"2026-10-26T10:00:00-07:00","timeZone":"America/Los_Angeles"},
            "end":{"dateTime":"2026-10-26T10:30:00-07:00","timeZone":"America/Los_Angeles"},
            "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20261104T165959Z",
                          "EXDATE;TZID=America/Los_Angeles:20261102T100000"]}"#;
        let inserted = r#"{"id":"n1","etag":"\"n\"","status":"confirmed","summary":"Standup (new room)",
            "organizer":{"email":"me@example.com","self":true},
            "start":{"dateTime":"2026-11-04T10:00:00-08:00","timeZone":"America/Los_Angeles"},
            "end":{"dateTime":"2026-11-04T10:30:00-08:00","timeZone":"America/Los_Angeles"},
            "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"]}"#;
        let fake = FakeTransport::new()
            .on("PATCH", &format!("{CAL_PATH}/s1"), &[], &[(200, answer)])
            .on("POST", CAL_PATH, &[], &[(200, inserted)]);
        let patch = CalOp::Patch {
            id: "s1".into(),
            before: before.clone(),
            after: old,
            scope: SeriesScope::Following,
            send_updates: SendUpdates::None,
        };
        let insert = CalOp::Insert {
            event: new,
            send_updates: SendUpdates::None,
        };
        for op in [&patch, &insert] {
            assert_eq!(commit_at(&fake, &store, ME, op, &zone(), now()).error, None);
        }
        let requests = fake.requests();
        assert_eq!(header(&requests[0], "If-Match"), Some("\"s\""));
        assert_eq!(
            body(&requests[0]),
            json!({"recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20261104T165959Z",
                                 "EXDATE;TZID=America/Los_Angeles:20261102T100000"]})
        );
        assert_eq!(requests[1].method, "POST");
        let sent = body(&requests[1]);
        assert_eq!(sent["id"], "n1");
        assert_eq!(sent["summary"], "Standup (new room)");
        assert_eq!(sent["start"]["dateTime"], "2026-11-04T17:00:00Z");
        assert_eq!(sent["recurrence"][0], "RRULE:FREQ=WEEKLY;BYDAY=MO,WE");
        assert!(
            store
                .load_events(ME, std::slice::from_ref(&moved.id))
                .unwrap()
                .is_empty()
        );
        assert_eq!(stored(&store, "n1").etag, "\"n\"");
    }

    #[test]
    fn conflict_reloads_googles_version() {
        let store = seeded(&[SINGLE]);
        let before = stored(&store, "e1");
        let theirs = r#"{"id":"e1","etag":"\"2\"","status":"confirmed","summary":"Theirs",
            "organizer":{"email":"me@example.com","self":true},
            "start":{"dateTime":"2026-10-28T10:00:00-07:00"},"end":{"dateTime":"2026-10-28T11:00:00-07:00"}}"#;
        let fake = FakeTransport::new()
            .on(
                "PATCH",
                &format!("{CAL_PATH}/e1"),
                &[],
                &[(
                    412,
                    r#"{"error":{"code":412,"message":"Precondition Failed"}}"#,
                )],
            )
            .on("GET", &format!("{CAL_PATH}/e1"), &[], &[(200, theirs)]);
        let op = CalOp::Patch {
            id: "e1".into(),
            before: before.clone(),
            after: Event {
                title: "Mine".into(),
                ..before
            },
            scope: SeriesScope::All,
            send_updates: SendUpdates::None,
        };
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error.as_deref(), Some("Changed elsewhere; reloaded"));
        assert!(done.retry.is_none());
        assert!(done.result().is_err());
        assert_eq!(done.changes.changed, ["e1"]);
        assert_eq!(header(&fake.requests()[0], "If-Match"), Some("\"1\""));
        let now_stored = stored(&store, "e1");
        assert_eq!(
            (now_stored.title.as_str(), now_stored.etag.as_str()),
            ("Theirs", "\"2\"")
        );
    }

    #[test]
    fn a_drilled_refusal_reloads_and_says_why() {
        let store = seeded(&[SINGLE]);
        let before = stored(&store, "e1");
        let fake = FakeTransport::new()
            .on(
                "PATCH",
                &format!("{CAL_PATH}/e1"),
                &[],
                &[(400, crate::backend::http::REFUSAL_BODY)],
            )
            .on("GET", &format!("{CAL_PATH}/e1"), &[], &[(200, SINGLE)]);
        let op = CalOp::Patch {
            id: "e1".into(),
            before: before.clone(),
            after: Event {
                title: "Mine".into(),
                ..before.clone()
            },
            scope: SeriesScope::All,
            send_updates: SendUpdates::None,
        };
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(
            done.error.as_deref(),
            Some("Google Calendar didn't take the change: Simulated refusal (drill)")
        );
        assert!(done.retry.is_none());
        assert_eq!(done.changes.changed, ["e1"]);
        assert_eq!(stored(&store, "e1").title, before.title);
    }

    #[test]
    fn move_keeps_explicit_reminders() {
        const OTHER_PATH: &str = "/calendar/v3/calendars/other%40example.com/events";
        let event = |reminders: &str, etag: &str| {
            format!(
                r#"{{"id":"m1","etag":"\"{etag}\"","status":"confirmed","summary":"Move me",
                "organizer":{{"email":"me@example.com","self":true}},
                "start":{{"dateTime":"2026-10-28T10:00:00-07:00"}},"end":{{"dateTime":"2026-10-28T11:00:00-07:00"}},
                "reminders":{reminders}}}"#
            )
        };
        let explicit = r#"{"useDefault":false,"overrides":[{"method":"popup","minutes":10}]}"#;
        let default = r#"{"useDefault":true}"#;
        let op = CalOp::Move {
            id: "m1".into(),
            to_calendar: "other@example.com".into(),
        };

        // Explicit: moved, then the reminders patched back.
        let store = seeded(&[&event(explicit, "1")]);
        let moved = event(default, "2");
        let patched = event(explicit, "3");
        let fake = FakeTransport::new()
            .on(
                "POST",
                &format!("{CAL_PATH}/m1/move"),
                &[],
                &[(200, moved.as_str())],
            )
            .on(
                "PATCH",
                &format!("{OTHER_PATH}/m1"),
                &[],
                &[(200, patched.as_str())],
            );
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        let requests = fake.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].method, "PATCH");
        assert_eq!(header(&requests[1], "If-Match"), None);
        assert_eq!(
            body(&requests[1]),
            json!({"reminders": serde_json::from_str::<Value>(explicit).unwrap()})
        );
        let now_stored = stored(&store, "m1");
        assert_eq!(now_stored.calendar, "other@example.com");
        assert_eq!(now_stored.reminder_minutes, Some(10));
        assert_eq!(now_stored.etag, "\"3\"");

        // The patch failing leaves the moved event, as Google has it.
        let store = seeded(&[&event(explicit, "1")]);
        let fake = FakeTransport::new()
            .on(
                "POST",
                &format!("{CAL_PATH}/m1/move"),
                &[],
                &[(200, moved.as_str())],
            )
            .on("PATCH", &format!("{OTHER_PATH}/m1"), &[], &[(412, "{}")])
            .on(
                "GET",
                &format!("{OTHER_PATH}/m1"),
                &[],
                &[(200, moved.as_str())],
            );
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert!(done.error.is_some());
        let now_stored = stored(&store, "m1");
        assert_eq!(now_stored.calendar, "other@example.com");
        assert_eq!(now_stored.etag, "\"2\"");

        // The calendar's default: the move alone.
        let store = seeded(&[&event(default, "1")]);
        let moved = event(default, "2");
        let fake = FakeTransport::new().on(
            "POST",
            &format!("{CAL_PATH}/m1/move"),
            &[],
            &[(200, moved.as_str())],
        );
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        assert_eq!(fake.requests().len(), 1);
        assert_eq!(stored(&store, "m1").calendar, "other@example.com");
    }

    #[test]
    fn someone_elses_event_is_declined_and_removed() {
        let invite = r#"{"id":"inv","etag":"\"v\"","status":"confirmed","summary":"Their meeting",
            "organizer":{"email":"boss@example.com"},
            "start":{"dateTime":"2026-10-28T10:00:00-07:00"},"end":{"dateTime":"2026-10-28T11:00:00-07:00"},
            "attendees":[{"email":"boss@example.com","organizer":true,"responseStatus":"accepted"},
                         {"email":"me@example.com","self":true,"responseStatus":"needsAction"}]}"#;
        let store = seeded(&[invite]);
        let declined = invite.replace("needsAction", "declined");
        let fake = FakeTransport::new()
            .on(
                "PATCH",
                &format!("{CAL_PATH}/inv"),
                &[],
                &[(200, declined.as_str())],
            )
            .on("DELETE", &format!("{CAL_PATH}/inv"), &[], &[(204, "")]);
        let op = CalOp::Delete {
            id: "inv".into(),
            occurrence: None,
            scope: SeriesScope::All,
            send_updates: SendUpdates::All,
        };
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        let requests = fake.requests();
        assert_eq!(requests[0].method, "PATCH");
        assert!(requests[0].url.ends_with("?sendUpdates=all"));
        assert_eq!(
            body(&requests[0])["attendees"][1]["responseStatus"],
            "declined"
        );
        assert_eq!(requests[1].method, "DELETE");
        assert!(requests[1].url.ends_with("?sendUpdates=none"));
        assert_eq!(done.changes.removed, ["inv"]);
        assert!(store.load_events(ME, &["inv".into()]).unwrap().is_empty());
    }
    // W5.

    #[test]
    fn rule_changes_replace_the_whole_rrule_and_keep_exdates() {
        let store = seeded(&[SERIES]);
        let before = stored(&store, "s1");
        let raw: Value = serde_json::from_str(SERIES).unwrap();
        // A new rule: the RRULE goes whole; EXDATE stays.
        let mut after = before.clone();
        let mut rule = Rule::monthly();
        rule.by_day = vec![(Some(-1), chrono::Weekday::Mon)];
        rule.count = Some(6);
        after.recurrence = Some(rule);
        let p = patch_body(&before, &after, Some(&raw), &zone());
        assert_eq!(
            p["recurrence"],
            json!([
                "RRULE:FREQ=MONTHLY;BYDAY=-1MO;COUNT=6",
                "EXDATE;TZID=America/Los_Angeles:20261102T100000"
            ])
        );
        // Only the end: Google's RRULE with the end rewritten.
        let mut after = before.clone();
        after.recurrence.as_mut().unwrap().count = Some(3);
        let p = patch_body(&before, &after, Some(&raw), &zone());
        assert_eq!(p["recurrence"][0], "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=3");
        // RDATE lines stay too.
        let with_rdate = json!({"recurrence": ["RRULE:FREQ=WEEKLY", "RDATE:20261101T100000Z"]});
        let mut after = before.clone();
        after.recurrence = Some(Rule::daily());
        let p = patch_body(&before, &after, Some(&with_rdate), &zone());
        assert_eq!(
            p["recurrence"],
            json!(["RRULE:FREQ=DAILY", "RDATE:20261101T100000Z"])
        );
        // No repeat: no lines at all.
        let mut after = before.clone();
        after.recurrence = None;
        let p = patch_body(&before, &after, Some(&raw), &zone());
        assert_eq!(p["recurrence"], json!([]));
        // A single event made to repeat.
        let single = stored(&seeded(&[SINGLE]), "e1");
        let mut after = single.clone();
        after.recurrence = Some(Rule::weekdays());
        let p = patch_body(&single, &after, None, &zone());
        assert_eq!(
            p["recurrence"],
            json!(["RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"])
        );
        // And back, through Google: the store drops its instances.
        let answer = SERIES.replace(
            r#""recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE","EXDATE;TZID=America/Los_Angeles:20261102T100000"]"#,
            r#""recurrence":[]"#,
        );
        let moved = Event {
            id: "s1_20261028T170000Z".into(),
            recurrence: None,
            recurring_event_id: Some("s1".into()),
            original_start: Some(at(d(2026, 10, 28), 10, 0)),
            ..before.clone()
        };
        store.put_events(ME, std::slice::from_ref(&moved)).unwrap();
        let fake =
            FakeTransport::new().on("PATCH", &format!("{CAL_PATH}/s1"), &[], &[(200, &answer)]);
        let op = CalOp::Patch {
            id: "s1".into(),
            before: before.clone(),
            after: Event {
                recurrence: None,
                exceptions: Vec::new(),
                ..before.clone()
            },
            scope: SeriesScope::All,
            send_updates: SendUpdates::None,
        };
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        assert_eq!(body(&fake.requests()[0])["recurrence"], json!([]));
        assert!(!stored(&store, "s1").is_series());
        assert!(
            store
                .load_events(ME, std::slice::from_ref(&moved.id))
                .unwrap()
                .is_empty()
        );
        assert!(done.changes.removed.contains(&moved.id));
    }

    #[test]
    fn a_split_inserts_the_new_series_only_after_the_old_one_ended() {
        let store = seeded(&[SERIES]);
        let before = stored(&store, "s1");
        let origin = at(d(2026, 11, 4), 10, 0);
        let edited = Event {
            title: "Standup (new room)".into(),
            ..before.clone()
        };
        let (old, new) = before.split(origin, origin, &edited, "n1".into()).unwrap();
        let op = CalOp::Split {
            id: "s1".into(),
            before: before.clone(),
            after: old.clone(),
            new: new.clone(),
            send_updates: SendUpdates::None,
        };
        // The series changed elsewhere (412): nothing goes in, and the new
        // series is gone again.
        let theirs = SERIES.replace(r#"\"s\""#, r#"\"s7\""#);
        let fake = FakeTransport::new()
            .on("PATCH", &format!("{CAL_PATH}/s1"), &[], &[(412, "{}")])
            .on("GET", &format!("{CAL_PATH}/s1"), &[], &[(200, &theirs)])
            .on("POST", CAL_PATH, &[], &[(200, "{}")]);
        store.put_events(ME, std::slice::from_ref(&new)).unwrap();
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error.as_deref(), Some("Changed elsewhere; reloaded"));
        assert!(fake.requests().iter().all(|r| r.method != "POST"));
        assert!(done.changes.removed.contains(&"n1".to_string()));
        assert!(store.load_events(ME, &["n1".into()]).unwrap().is_empty());
        assert_eq!(stored(&store, "s1").etag, "\"s7\"");
        // Offline at the first step: retried later, nothing rolled back.
        let fake = FakeTransport::new().on("PATCH", &format!("{CAL_PATH}/s1"), &[], &[(0, "")]);
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert!(done.retry.is_some());
        assert_eq!(fake.requests().len(), 1);
        // Both steps in order.
        let store = seeded(&[SERIES]);
        let answer = SERIES.replace(
            "RRULE:FREQ=WEEKLY;BYDAY=MO,WE",
            "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20261104T165959Z",
        );
        let inserted = r#"{"id":"n1","etag":"\"n\"","status":"confirmed","summary":"Standup (new room)",
            "organizer":{"email":"me@example.com","self":true},
            "start":{"dateTime":"2026-11-04T10:00:00-08:00"},"end":{"dateTime":"2026-11-04T10:30:00-08:00"},
            "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"]}"#;
        let fake = FakeTransport::new()
            .on("PATCH", &format!("{CAL_PATH}/s1"), &[], &[(200, &answer)])
            .on("POST", CAL_PATH, &[], &[(200, inserted)]);
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        let methods: Vec<&str> = fake.requests().iter().map(|r| r.method).collect();
        assert_eq!(methods, ["PATCH", "POST"]);
        assert_eq!(stored(&store, "n1").etag, "\"n\"");
        // Demo mode applies both at once.
        let demo = seeded(&[SERIES]);
        demo.apply_cal_op(ME, ME, &op).unwrap();
        assert_eq!(stored(&demo, "s1").recurrence.unwrap().until, Some(origin));
        assert_eq!(stored(&demo, "n1").title, "Standup (new room)");
    }

    #[test]
    fn an_edit_right_after_a_split_goes_after_it_to_the_new_series() {
        use crate::backend::{Op, outbox::Outbox};
        let store = seeded(&[SERIES]);
        let before = stored(&store, "s1");
        let origin = at(d(2026, 11, 4), 10, 0);
        let mut edited = before.clone();
        if let Some(rule) = &mut edited.recurrence {
            rule.interval = 2;
            rule.count = Some(5);
        }
        let (old, new) = before.split(origin, origin, &edited, "n1".into()).unwrap();
        // "Does not repeat" on the new series, before the split went.
        let single = Event {
            recurrence: None,
            ..new.clone()
        };
        let outbox = Outbox::new(store.clone(), ME.into());
        let split = Op::Calendar(CalOp::Split {
            id: "s1".into(),
            before: before.clone(),
            after: old.clone(),
            new: new.clone(),
            send_updates: SendUpdates::None,
        });
        let patch = Op::Calendar(CalOp::Patch {
            id: "n1".into(),
            before: new.clone(),
            after: single,
            scope: SeriesScope::All,
            send_updates: SendUpdates::None,
        });
        let split_id = outbox.push(split, 0, 0).unwrap();
        outbox.push(patch, 0, 0).unwrap();
        // The split goes alone; the edit waits for it.
        let due = outbox.take_due(0).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, split_id);
        let answer = SERIES.replace(
            "RRULE:FREQ=WEEKLY;BYDAY=MO,WE",
            "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=20261104T165959Z",
        );
        let inserted = r#"{"id":"n1","etag":"\"n\"","status":"confirmed","summary":"Standup",
            "organizer":{"email":"me@example.com","self":true},
            "start":{"dateTime":"2026-11-04T10:00:00-08:00"},"end":{"dateTime":"2026-11-04T10:30:00-08:00"},
            "recurrence":["RRULE:FREQ=WEEKLY;INTERVAL=2;COUNT=5;BYDAY=MO,WE"]}"#;
        let single_answer = r#"{"id":"n1","etag":"\"n2\"","status":"confirmed","summary":"Standup",
            "organizer":{"email":"me@example.com","self":true},
            "start":{"dateTime":"2026-11-04T10:00:00-08:00"},"end":{"dateTime":"2026-11-04T10:30:00-08:00"}}"#;
        let fake = FakeTransport::new()
            .on("PATCH", &format!("{CAL_PATH}/s1"), &[], &[(200, &answer)])
            .on("POST", CAL_PATH, &[], &[(200, inserted)])
            .on(
                "PATCH",
                &format!("{CAL_PATH}/n1"),
                &[],
                &[(200, single_answer)],
            );
        let Op::Calendar(op) = &due[0].1 else {
            panic!("a calendar op")
        };
        let done = commit_at(&fake, &store, ME, op, &zone(), now());
        assert_eq!(done.error, None);
        outbox.finish(split_id).unwrap();
        // Google's new series (stored, and read by the next sync) doesn't
        // replace the app's copy while the edit to it is queued.
        assert!(stored(&store, "n1").is_series());
        assert!(outbox.queued_events().contains("n1"));
        assert!(!outbox.queued_events().contains("s1"));
        // Then the edit goes, to the new series' id, with its etag.
        let due = outbox.take_due(0).unwrap();
        assert_eq!(due.len(), 1);
        let Op::Calendar(op) = &due[0].1 else {
            panic!("a calendar op")
        };
        let done = commit_at(&fake, &store, ME, op, &zone(), now());
        assert_eq!(done.error, None);
        let requests = fake.requests();
        let last = requests.last().unwrap();
        assert_eq!(last.method, "PATCH");
        assert!(last.url.contains("/events/n1?"));
        assert_eq!(body(last)["recurrence"], json!([]));
        assert!(
            last.headers
                .iter()
                .any(|(k, v)| k == "If-Match" && v == "\"n\"")
        );
        assert!(!stored(&store, "n1").is_series());
    }

    #[test]
    fn several_reminders_and_every_default_are_kept() {
        let e = series_with(
            r#"{"useDefault":false,"overrides":[{"method":"email","minutes":1440},{"method":"popup","minutes":10}]}"#,
        );
        assert_eq!(e.reminders.len(), 2);
        assert_eq!(e.reminder_minutes, Some(1440));
        assert_eq!(
            reminders_json(&e),
            json!({"useDefault":false,"overrides":[
                {"method":"email","minutes":1440},{"method":"popup","minutes":10}]})
        );
        let mut fewer = e.clone();
        fewer.set_reminders(vec![Reminder::popup(10)]);
        let p = patch_body(&e, &fewer, None, &zone());
        assert_eq!(
            p["reminders"],
            json!({"useDefault":false,"overrides":[{"method":"popup","minutes":10}]})
        );
        // Back to the calendar's default: the old overrides are cleared,
        // or Google's PATCH refuses both at once.
        let mut default = e.clone();
        default.reminder_use_default = true;
        let p = patch_body(&e, &default, None, &zone());
        assert_eq!(p["reminders"], json!({"useDefault":true,"overrides":[]}));
        let entry: CalendarListEntry = serde_json::from_str(
            r#"{"id":"c","summary":"C","selected":true,"accessRole":"owner",
                "defaultReminders":[{"method":"popup","minutes":10},{"method":"email","minutes":60}],
                "conferenceProperties":{"allowedConferenceSolutionTypes":["hangoutsMeet"]}}"#,
        )
        .unwrap();
        let calendar = calendar_from_entry(&entry, &HashMap::new());
        assert_eq!(calendar.default_reminder, Some(60));
        assert_eq!(calendar.default_reminders.len(), 2);
        assert_eq!(calendar.default_reminders[1].method, ReminderMethod::Email);
        assert!(calendar.allows_meet());
        // Kept in the store.
        let store = Store::open_in_memory().unwrap();
        store.put_google_calendar(ME, &calendar, false).unwrap();
        let back = &store.load_calendars(ME).unwrap()[0];
        assert_eq!(back.default_reminders, calendar.default_reminders);
        assert!(back.allows_meet());
    }

    #[test]
    fn adding_meet_asks_once_then_waits_for_it() {
        let store = seeded(&[SINGLE]);
        let before = stored(&store, "e1");
        let after = Event {
            meet_request: Some("req-1".into()),
            ..before.clone()
        };
        let pending = SINGLE.replace(
            r#""summary":"Review","#,
            r#""summary":"Review","conferenceData":{"createRequest":{"requestId":"req-1",
                "status":{"statusCode":"pending"}}},"#,
        );
        let done_json = SINGLE.replace(
            r#""summary":"Review","#,
            r#""summary":"Review","conferenceData":{"createRequest":{"requestId":"req-1",
                "status":{"statusCode":"success"}},
                "entryPoints":[{"entryPointType":"video","uri":"https://meet.google.com/abc-defg-hij"}]},"#,
        );
        let fake = FakeTransport::new()
            .on("PATCH", &format!("{CAL_PATH}/e1"), &[], &[(200, &pending)])
            .on(
                "GET",
                &format!("{CAL_PATH}/e1"),
                &[],
                &[(200, &pending), (200, &done_json)],
            );
        let op = CalOp::Patch {
            id: "e1".into(),
            before: before.clone(),
            after: after.clone(),
            scope: SeriesScope::All,
            send_updates: SendUpdates::None,
        };
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert_eq!(done.error, None);
        let requests = fake.requests();
        assert!(requests[0].url.contains("conferenceDataVersion=1"));
        assert_eq!(
            body(&requests[0]),
            json!({"conferenceData":{"createRequest":{"requestId":"req-1",
                "conferenceSolutionKey":{"type":"hangoutsMeet"}}}})
        );
        assert_eq!(requests.len(), 3, "polled until success");
        let now_stored = stored(&store, "e1");
        assert_eq!(
            now_stored.conference_uri,
            "https://meet.google.com/abc-defg-hij"
        );
        assert!(now_stored.meet_request.is_none());
        assert!(done.changes.changed.contains(&"e1".to_string()));

        // Still pending after every poll: a retryable failure, the same
        // request id the next time.
        let fake = FakeTransport::new()
            .on("PATCH", &format!("{CAL_PATH}/e1"), &[], &[(200, &pending)])
            .on("GET", &format!("{CAL_PATH}/e1"), &[], &[(200, &pending)]);
        let store = seeded(&[SINGLE]);
        let done = commit_at(&fake, &store, ME, &op, &zone(), now());
        assert!(
            done.error
                .as_deref()
                .unwrap()
                .starts_with("Couldn't add Google Meet")
        );
        assert_eq!(
            fake.requests().iter().filter(|r| r.method == "GET").count(),
            MEET_POLLS + 1,
            "bounded, plus the rollback's read"
        );
        // Other writes never touch conferenceData.
        let titled = Event {
            title: "Other".into(),
            ..now_stored.clone()
        };
        let p = patch_body(&now_stored, &titled, None, &zone());
        assert!(p.get("conferenceData").is_none());
        // A new event with Meet asks for it in the insert.
        let new = Event {
            id: "n2".into(),
            calendar: CAL.into(),
            start: at(d(2026, 10, 28), 12, 0),
            end: at(d(2026, 10, 28), 13, 0),
            meet_request: Some("req-2".into()),
            ..Event::default()
        };
        assert_eq!(
            insert_body(&new, &zone())["conferenceData"]["createRequest"]["requestId"],
            "req-2"
        );
    }

    #[test]
    fn google_calendar_settings_are_read_every_few_hours() {
        let store = Store::open_in_memory().unwrap();
        let page = r#"{"items":[{"id":"weekStart","value":"0"},{"id":"format24HourTime","value":"true"},
            {"id":"defaultEventLength","value":"30"},{"id":"autoAddHangouts","value":"true"},
            {"id":"remindOnRespondedEventsOnly","value":"true"},{"id":"timezone","value":"Europe/London"},
            {"id":"locale","value":"en"}]}"#;
        let fake =
            FakeTransport::new().on("GET", "/calendar/v3/users/me/settings", &[], &[(200, page)]);
        let mut changes = CalendarChanges::default();
        sync_settings(&fake, &store, ME, now(), &mut changes).unwrap();
        assert!(changes.calendars);
        let json = store.sync_state(ME, SETTINGS_KEY).unwrap().unwrap();
        let settings: GoogleSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            settings,
            GoogleSettings {
                week_start: Some(0),
                default_event_minutes: Some(30),
                auto_add_meet: true,
                remind_responded_only: true,
                time_zone: Some("Europe/London".into()),
            }
        );
        assert_eq!(settings.first_weekday(), chrono::Weekday::Sun);
        // Not again within six hours; then again, unchanged: no reload.
        let mut changes = CalendarChanges::default();
        sync_settings(&fake, &store, ME, now() + Duration::hours(1), &mut changes).unwrap();
        assert_eq!(fake.requests().len(), 1);
        sync_settings(&fake, &store, ME, now() + Duration::hours(7), &mut changes).unwrap();
        assert_eq!(fake.requests().len(), 2);
        assert!(!changes.calendars);
        // Without settings, the defaults.
        assert_eq!(GoogleSettings::from_items(&[]), GoogleSettings::default());
    }
}

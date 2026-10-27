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

use std::collections::{BTreeSet, HashMap};

use anyhow::{Context as _, anyhow};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

use super::{
    BackendEvent,
    http::{Request, Response, Transport, TransportError},
    store::Store,
};
use crate::{
    calendar::{
        AccessRole, Attendee, Calendar, Event, EventStatus, RsvpStatus,
        recurrence::parse_recurrence,
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
    }
}

/// The reminder furthest ahead of the event, in minutes.
fn earliest_reminder(reminders: &[GoogleReminder]) -> Option<u32> {
    reminders.iter().map(|r| r.minutes).max()
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
    let g: GoogleEvent = serde_json::from_value(raw.clone()).ok()?;
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

/// Percent-encodes a path segment or query value.
fn enc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// A failed call's reason, in Google's words when it gave some.
fn api_error(what: &str, response: &Response) -> anyhow::Error {
    let message = response
        .json::<Value>()
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
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

/// Every page of a listing at `url` (which has a query already).
fn list_all<T: DeserializeOwned>(
    transport: &dyn Transport,
    url: &str,
    what: &str,
) -> anyhow::Result<Listing<T>> {
    let mut items = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let page_url = match &page_token {
            Some(token) => format!("{url}&pageToken={}", enc(token)),
            None => url.to_string(),
        };
        let response = get(transport, &page_url)?;
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
    e.downcast_ref::<TransportError>().is_some()
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
    for (calendar, default_reminder) in store.calendars_to_sync(account)? {
        let pass = CalendarPass {
            transport,
            store,
            account,
            calendar: &calendar,
            default_reminder,
            zone,
            now,
        };
        match pass.run(&mut touched) {
            Ok(()) => {}
            Err(e) if is_network(&e) => return Err(e),
            Err(e) => changes.errors.push(format!("{e:#}")),
        }
    }
    let ids: Vec<String> = touched.into_iter().collect();
    let present: BTreeSet<String> = store
        .load_events(account, &ids)?
        .into_iter()
        .map(|e| e.id)
        .collect();
    for id in ids {
        if present.contains(&id) {
            changes.changed.push(id);
        } else {
            changes.removed.push(id);
        }
    }
    Ok(changes)
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
        )?,
        None => list_all(transport, &full_url, WHAT)?,
    };
    if matches!(listing, Listing::Gone) {
        full = true;
        listing = list_all(transport, &full_url, WHAT)?;
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
    for entry in &entries {
        if entry.deleted {
            touched.extend(store.delete_calendar(account, &entry.id)?);
        } else {
            store.put_google_calendar(
                account,
                &calendar_from_entry(entry, palette),
                entry.hidden,
            )?;
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

    fn run(&self, touched: &mut BTreeSet<String>) -> anyhow::Result<()> {
        let base = format!(
            "{API}/calendars/{}/events?singleEvents=false&maxResults=2500&timeZone={}",
            enc(self.calendar),
            enc(self.zone.name)
        );
        let full_url = format!(
            "{base}&timeMin={}",
            enc(&rfc3339(self.now - Duration::days(PAST_DAYS)))
        );
        let token = self
            .store
            .calendar_sync_token(self.account, self.calendar)?;
        let mut full = token.is_none();
        let mut listing = match &token {
            Some(token) => list_all(
                self.transport,
                &format!("{base}&syncToken={}", enc(token)),
                &self.what(),
            )?,
            None => list_all(self.transport, &full_url, &self.what())?,
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
            listing = list_all(self.transport, &full_url, &self.what())?;
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
        for id in series {
            self.refresh_series(&id, touched)?;
        }
        Ok(())
    }

    /// A series' exceptions, recomputed: its EXDATEs and every instance
    /// stored on its own. For a rule Pigeon can't expand, Google's own
    /// occurrences replace the stored ones.
    fn refresh_series(&self, id: &str, touched: &mut BTreeSet<String>) -> anyhow::Result<()> {
        let Some((mut series, json)) =
            self.store.calendar_event(self.account, self.calendar, id)?
        else {
            return Ok(());
        };
        let Some(rule) = &series.recurrence else {
            return Ok(());
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
                list_all::<Value>(self.transport, &url, &self.what())?
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
            return Ok(());
        }
        let lines: Vec<String> = json
            .as_deref()
            .and_then(|j| serde_json::from_str::<GoogleEvent>(j).ok())
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
        self.store
            .put_google_events(self.account, &[(series, json.unwrap_or_default())])?;
        Ok(())
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
        assert_eq!(rule.label(), "Every week");
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
}

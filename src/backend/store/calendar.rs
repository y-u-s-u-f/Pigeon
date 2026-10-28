//! Calendars and events in the store. Events are kept whole as JSON
//! (`model`), with the columns lookups need beside it.

use std::collections::HashSet;

use chrono::NaiveDateTime;
use rusqlite::{OptionalExtension as _, Transaction, params};

use super::Store;
use crate::{
    backend::CalOp,
    calendar::{AccessRole, Calendar, Event, SeriesScope, reminders::ReminderKey},
};

impl Store {
    pub fn put_calendars(&self, account: &str, calendars: &[Calendar]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for c in calendars {
            tx.execute(
                "INSERT INTO calendars (account, id, summary, bg, selected, access_role,
                    is_primary, time_zone, default_reminder, default_reminders, conference_types)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (account, id) DO UPDATE SET
                    summary = excluded.summary, bg = excluded.bg, selected = excluded.selected,
                    access_role = excluded.access_role, is_primary = excluded.is_primary,
                    time_zone = excluded.time_zone, default_reminder = excluded.default_reminder,
                    default_reminders = excluded.default_reminders,
                    conference_types = excluded.conference_types",
                params![
                    account,
                    c.id,
                    c.name,
                    crate::data::to_hex(c.color),
                    c.visible,
                    c.access_role.key(),
                    c.primary,
                    c.time_zone,
                    c.default_reminder,
                    super::json(&c.default_reminders),
                    super::json(&c.conference_types),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_calendars(&self, account: &str) -> anyhow::Result<Vec<Calendar>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, summary, bg, selected, access_role, is_primary, time_zone,
                default_reminder, default_reminders, conference_types
             FROM calendars WHERE account = ?1 AND hidden = 0 ORDER BY rowid",
        )?;
        let rows = stmt.query_map([account], |r| {
            Ok(Calendar {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r
                    .get::<_, Option<String>>(2)?
                    .and_then(|hex| crate::data::from_hex(&hex))
                    .unwrap_or(crate::theme::FALLBACK_COLOR),
                visible: r.get(3)?,
                access_role: AccessRole::from_key(&r.get::<_, String>(4)?),
                primary: r.get(5)?,
                time_zone: r.get(6)?,
                default_reminder: r.get(7)?,
                default_reminders: super::from_json(
                    &r.get::<_, Option<String>>(8)?.unwrap_or_default(),
                ),
                conference_types: super::from_json(
                    &r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                ),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn put_events(&self, account: &str, events: &[Event]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for event in events {
            write_event(&tx, account, event, None)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_all_events(&self, account: &str) -> anyhow::Result<Vec<Event>> {
        self.load_events_where(account, None)
    }

    pub fn load_events(&self, account: &str, ids: &[String]) -> anyhow::Result<Vec<Event>> {
        self.load_events_where(account, Some(ids))
    }

    fn load_events_where(
        &self,
        account: &str,
        ids: Option<&[String]>,
    ) -> anyhow::Result<Vec<Event>> {
        let conn = self.conn();
        // Cancelled instances are kept only as their series' exceptions.
        const LIVE: &str = "(status IS NULL OR status != 'cancelled')";
        let mut stmt;
        let mut rows = match ids {
            None => {
                stmt = conn.prepare_cached(&format!(
                    "SELECT model FROM events WHERE account = ?1 AND {LIVE} ORDER BY rowid"
                ))?;
                stmt.query([account])?
            }
            Some(ids) => {
                stmt = conn.prepare_cached(&format!(
                    "SELECT model FROM events WHERE account = ?1 AND {LIVE}
                        AND id IN (SELECT value FROM json_each(?2)) ORDER BY rowid"
                ))?;
                stmt.query(params![account, super::json(&ids)])?
            }
        };
        let mut events = Vec::new();
        while let Some(r) = rows.next()? {
            events.push(serde_json::from_str(&r.get::<_, String>(0)?)?);
        }
        Ok(events)
    }

    /// Which of `ids` are stored events, cancelled instances aside: what
    /// a sync touched that's still there (MC-002).
    pub fn existing_event_ids(
        &self,
        account: &str,
        ids: &[String],
    ) -> anyhow::Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id FROM events WHERE account = ?1
                AND (status IS NULL OR status != 'cancelled')
                AND id IN (SELECT value FROM json_each(?2))",
        )?;
        let rows = stmt.query_map(params![account, super::json(&ids)], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // Google sync (W4): calendars by their list entry, events per calendar
    // with Google's JSON beside them.

    /// A calendar from Google's list; its sync token stays.
    pub fn put_google_calendar(
        &self,
        account: &str,
        c: &Calendar,
        hidden: bool,
    ) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO calendars (account, id, summary, bg, selected, hidden, access_role,
                is_primary, time_zone, default_reminder, default_reminders, conference_types)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (account, id) DO UPDATE SET
                summary = excluded.summary, bg = excluded.bg, selected = excluded.selected,
                hidden = excluded.hidden, access_role = excluded.access_role,
                is_primary = excluded.is_primary, time_zone = excluded.time_zone,
                default_reminder = excluded.default_reminder,
                default_reminders = excluded.default_reminders,
                conference_types = excluded.conference_types",
            params![
                account,
                c.id,
                c.name,
                crate::data::to_hex(c.color),
                c.visible,
                hidden,
                c.access_role.key(),
                c.primary,
                c.time_zone,
                c.default_reminder,
                super::json(&c.default_reminders),
                super::json(&c.conference_types),
            ],
        )?;
        Ok(())
    }

    // Reminder notifications (W5).

    /// The reminders shown in the last week.
    pub fn fired_reminders(&self, account: &str) -> anyhow::Result<Vec<ReminderKey>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT calendar_id, event_id, instance_start, offset_minutes FROM fired_reminders
             WHERE account = ?1",
        )?;
        let rows = stmt.query_map([account], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (calendar, event, start, minutes) = row?;
            if let Ok(start) = NaiveDateTime::parse_from_str(&start, START_FORMAT) {
                out.push(ReminderKey {
                    calendar,
                    event,
                    start,
                    minutes,
                });
            }
        }
        Ok(out)
    }

    /// Keeps a reminder as shown at `now` (ms); ones older than a week go.
    pub fn mark_reminder_fired(
        &self,
        account: &str,
        key: &ReminderKey,
        now: i64,
    ) -> anyhow::Result<()> {
        const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO fired_reminders
                (account, calendar_id, event_id, instance_start, offset_minutes, fired_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                account,
                key.calendar,
                key.event,
                key.start.format(START_FORMAT).to_string(),
                key.minutes,
                now
            ],
        )?;
        conn.execute(
            "DELETE FROM fired_reminders WHERE account = ?1 AND fired_at < ?2",
            params![account, now - WEEK_MS],
        )?;
        Ok(())
    }

    /// Every stored calendar, hidden ones too.
    pub fn all_calendar_ids(&self, account: &str) -> anyhow::Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id FROM calendars WHERE account = ?1 ORDER BY rowid")?;
        let rows = stmt.query_map([account], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The calendars whose events to sync, with their default reminder:
    /// not hidden, and not free/busy only (Google lists no events there).
    pub fn calendars_to_sync(&self, account: &str) -> anyhow::Result<Vec<(String, Option<u32>)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, default_reminder FROM calendars
             WHERE account = ?1 AND hidden = 0 AND access_role != 'freeBusyReader'
             ORDER BY rowid",
        )?;
        let rows = stmt.query_map([account], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A calendar and its events; returns the events' ids.
    pub fn delete_calendar(&self, account: &str, id: &str) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let removed = delete_calendar_events(&tx, account, id, None)?;
        tx.execute(
            "DELETE FROM calendars WHERE account = ?1 AND id = ?2",
            params![account, id],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    pub fn calendar_sync_token(&self, account: &str, id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT sync_token FROM calendars WHERE account = ?1 AND id = ?2",
                params![account, id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn set_calendar_sync_token(
        &self,
        account: &str,
        id: &str,
        token: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE calendars SET sync_token = ?3 WHERE account = ?1 AND id = ?2",
            params![account, id, token],
        )?;
        Ok(())
    }

    /// Every event row in a calendar, cancelled instances too.
    pub fn calendar_event_ids(&self, account: &str, calendar: &str) -> anyhow::Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id FROM events WHERE account = ?1 AND calendar_id = ?2 ORDER BY rowid",
        )?;
        let rows = stmt.query_map(params![account, calendar], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The events `ids` in a calendar and their series' instances, or with
    /// `None` all of the calendar's. Returns the ids deleted.
    pub fn delete_calendar_events(
        &self,
        account: &str,
        calendar: &str,
        ids: Option<&[String]>,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let removed = delete_calendar_events(&tx, account, calendar, ids)?;
        tx.commit()?;
        Ok(removed)
    }

    /// Events from Google, each with its JSON.
    pub fn put_google_events(
        &self,
        account: &str,
        events: &[(Event, String)],
    ) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (event, json) in events {
            write_event(&tx, account, event, Some(json))?;
        }
        tx.commit()?;
        Ok(())
    }

    /// One event in a calendar, cancelled or not, with Google's JSON.
    pub fn calendar_event(
        &self,
        account: &str,
        calendar: &str,
        id: &str,
    ) -> anyhow::Result<Option<(Event, Option<String>)>> {
        let row: Option<(String, Option<String>)> = self
            .conn()
            .query_row(
                "SELECT model, json FROM events WHERE account = ?1 AND calendar_id = ?2 AND id = ?3",
                params![account, calendar, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((model, json)) => Some((serde_json::from_str(&model)?, json)),
            None => None,
        })
    }

    /// Where the rule put each instance of `series` stored on its own,
    /// moved or cancelled.
    pub fn instance_starts(
        &self,
        account: &str,
        calendar: &str,
        series: &str,
    ) -> anyhow::Result<Vec<NaiveDateTime>> {
        let conn = self.conn();
        // The column, not the model: no event is parsed (MC-020).
        let mut stmt = conn.prepare(
            "SELECT original_start FROM events
             WHERE account = ?1 AND calendar_id = ?2 AND recurring_event_id = ?3
             AND original_start IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![account, calendar, series], |r| {
            r.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for start in rows {
            out.push(NaiveDateTime::parse_from_str(
                &start?,
                "%Y-%m-%d %H:%M:%S%.f",
            )?);
        }
        Ok(out)
    }

    /// The occurrences of `series` changed on their own from `from` on go
    /// (its end moved before them).
    pub fn drop_instances_from(
        &self,
        account: &str,
        series: &str,
        from: NaiveDateTime,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        drop_instances_from(&tx, account, series, from)?;
        tx.commit()?;
        Ok(())
    }

    /// The shown instances of `series` (cancelled ones stay); returns
    /// their ids.
    pub fn delete_instances(
        &self,
        account: &str,
        calendar: &str,
        series: &str,
    ) -> anyhow::Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id FROM events WHERE account = ?1 AND calendar_id = ?2
                AND recurring_event_id = ?3 AND (status IS NULL OR status != 'cancelled')",
        )?;
        let ids: Vec<String> = stmt
            .query_map(params![account, calendar, series], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        conn.execute(
            "DELETE FROM events WHERE account = ?1 AND calendar_id = ?2
                AND recurring_event_id = ?3 AND (status IS NULL OR status != 'cancelled')",
            params![account, calendar, series],
        )?;
        Ok(ids)
    }

    /// Apply a calendar op the way Google will (or, in demo mode, instead
    /// of it). `me` is the user's address, for RSVPs.
    pub fn apply_cal_op(&self, account: &str, me: &str, op: &CalOp) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        match op {
            CalOp::Insert { event, .. } => write_event(&tx, account, event, None)?,
            // "This and following", whole: the series ends, the new one
            // starts.
            CalOp::Split { after, new, .. } => {
                write_event(&tx, account, after, None)?;
                if let Some(until) = after.recurrence.as_ref().and_then(|r| r.until) {
                    drop_instances_from(&tx, account, &after.id, until)?;
                }
                write_event(&tx, account, new, None)?;
            }
            CalOp::Patch {
                before,
                after: event,
                scope,
                ..
            } => {
                write_event(&tx, account, event, None)?;
                // No longer repeating: its occurrences of their own go.
                if before.is_series() && !event.is_series() {
                    tx.execute(
                        "DELETE FROM events WHERE account = ?1 AND recurring_event_id = ?2",
                        params![account, event.id],
                    )?;
                }
                // "This and following" for a change: the series ends, and
                // its occurrences changed on their own from there on go.
                if *scope == SeriesScope::Following
                    && let Some(until) = event.recurrence.as_ref().and_then(|r| r.until)
                {
                    drop_instances_from(&tx, account, &event.id, until)?;
                }
            }
            CalOp::Delete {
                id,
                occurrence,
                scope,
                ..
            } => {
                let series = read_event(&tx, account, id)?;
                let scope = match (series.as_ref(), occurrence) {
                    (Some(e), Some(_)) if e.is_series() => *scope,
                    _ => SeriesScope::All,
                };
                let occurrence = occurrence.unwrap_or_default();
                if let Some(mut series) = series.filter(|_| scope != SeriesScope::All) {
                    match scope {
                        SeriesScope::This => series.exceptions.push(occurrence),
                        _ => {
                            if let Some(rule) = &mut series.recurrence {
                                rule.until = Some(occurrence);
                            }
                        }
                    }
                    write_event(&tx, account, &series, None)?;
                } else {
                    tx.execute(
                        "DELETE FROM events WHERE account = ?1 AND id = ?2",
                        params![account, id],
                    )?;
                }
                // Occurrences changed on their own go with the ones deleted.
                match scope {
                    SeriesScope::All => tx.execute(
                        "DELETE FROM events WHERE account = ?1 AND recurring_event_id = ?2",
                        params![account, id],
                    )?,
                    SeriesScope::Following => drop_instances_from(&tx, account, id, occurrence)?,
                    SeriesScope::This => 0,
                };
            }
            CalOp::Move { id, to_calendar } => {
                if let Some(mut event) = read_event(&tx, account, id)? {
                    tx.execute(
                        "DELETE FROM events WHERE account = ?1 AND id = ?2",
                        params![account, id],
                    )?;
                    event.calendar = to_calendar.clone();
                    write_event(&tx, account, &event, None)?;
                }
            }
            CalOp::Rsvp { id, status } => {
                if let Some(mut event) = read_event(&tx, account, id)? {
                    for attendee in &mut event.attendees {
                        if attendee.email.eq_ignore_ascii_case(me) {
                            attendee.status = *status;
                        }
                    }
                    write_event(&tx, account, &event, None)?;
                }
            }
            CalOp::SetSelected { calendar, selected } => {
                tx.execute(
                    "UPDATE calendars SET selected = ?3 WHERE account = ?1 AND id = ?2",
                    params![account, calendar, selected],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

/// The events `ids` in a calendar and their series' instances, or with
/// `None` all of the calendar's. Returns the ids deleted.
fn delete_calendar_events(
    tx: &Transaction,
    account: &str,
    calendar: &str,
    ids: Option<&[String]>,
) -> anyhow::Result<Vec<String>> {
    let mut removed = Vec::new();
    let mut select = tx.prepare_cached(
        "SELECT id, recurring_event_id FROM events WHERE account = ?1 AND calendar_id = ?2",
    )?;
    let rows = select.query_map(params![account, calendar], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    let wanted: Option<HashSet<&str>> = ids.map(|ids| ids.iter().map(String::as_str).collect());
    for row in rows {
        let (id, series) = row?;
        let goes = wanted.as_ref().is_none_or(|ids| {
            ids.contains(id.as_str()) || series.as_deref().is_some_and(|s| ids.contains(s))
        });
        if goes {
            removed.push(id);
        }
    }
    let mut delete = tx
        .prepare_cached("DELETE FROM events WHERE account = ?1 AND calendar_id = ?2 AND id = ?3")?;
    for id in &removed {
        delete.execute(params![account, calendar, id])?;
    }
    Ok(removed)
}

fn read_event(tx: &Transaction, account: &str, id: &str) -> anyhow::Result<Option<Event>> {
    let model: Option<String> = tx
        .prepare_cached("SELECT model FROM events WHERE account = ?1 AND id = ?2")?
        .query_row(params![account, id], |r| r.get(0))
        .optional()?;
    Ok(match model {
        Some(model) => Some(serde_json::from_str(&model)?),
        None => None,
    })
}

/// Deletes the occurrences of `series` changed on their own whose rule
/// time is `from` or later.
fn drop_instances_from(
    tx: &Transaction,
    account: &str,
    series: &str,
    from: NaiveDateTime,
) -> anyhow::Result<usize> {
    Ok(tx.execute(
        "DELETE FROM events WHERE account = ?1 AND recurring_event_id = ?2
            AND original_start >= ?3",
        params![account, series, from.to_string()],
    )?)
}

/// Writes the event, with Google's JSON when given; without, the JSON
/// stored stays.
fn write_event(
    tx: &Transaction,
    account: &str,
    event: &Event,
    json: Option<&str>,
) -> anyhow::Result<()> {
    let status = serde_json::to_value(event.status)?;
    tx.prepare_cached(
        "INSERT INTO events (account, calendar_id, id, etag, ical_uid, status, start, \"end\",
            all_day, recurring_event_id, original_start, model, json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT (account, calendar_id, id) DO UPDATE SET
            etag = excluded.etag, ical_uid = excluded.ical_uid, status = excluded.status,
            start = excluded.start, \"end\" = excluded.\"end\", all_day = excluded.all_day,
            recurring_event_id = excluded.recurring_event_id,
            original_start = excluded.original_start, model = excluded.model,
            json = COALESCE(excluded.json, json)",
    )?
    .execute(params![
        account,
        event.calendar,
        event.id,
        event.etag,
        event.ical_uid,
        status.as_str(),
        event.start.to_string(),
        event.end.to_string(),
        event.all_day,
        event.recurring_event_id,
        event.original_start.map(|s| s.to_string()),
        serde_json::to_string(event)?,
        json,
    ])?;
    Ok(())
}

/// How a fired reminder's occurrence start is kept.
const START_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

#[cfg(test)]
mod tests {
    use crate::backend::{CalOp, SendUpdates, demo, store::Store};
    use crate::calendar::{RsvpStatus, SeriesScope, reminders::ReminderKey};

    #[test]
    fn fired_reminders_are_kept_for_a_week() {
        let store = Store::open_in_memory().unwrap();
        let key = |minutes| ReminderKey {
            calendar: "work".into(),
            event: "e1".into(),
            start: crate::calendar::time::at(
                chrono::NaiveDate::from_ymd_opt(2026, 10, 27).unwrap(),
                9,
                0,
            ),
            minutes,
        };
        const DAY_MS: i64 = 24 * 60 * 60 * 1000;
        store.mark_reminder_fired("me", &key(10), 0).unwrap();
        store.mark_reminder_fired("me", &key(10), 0).unwrap();
        assert_eq!(store.fired_reminders("me").unwrap(), [key(10)]);
        assert!(store.fired_reminders("other").unwrap().is_empty());
        // Eight days on, the old one goes.
        store
            .mark_reminder_fired("me", &key(60), 8 * DAY_MS)
            .unwrap();
        assert_eq!(store.fired_reminders("me").unwrap(), [key(60)]);
    }

    #[test]
    fn events_round_trip_and_ops() {
        let store = Store::open_in_memory().unwrap();
        let account = demo::seed(&store).unwrap().email;
        let original = demo::calendar_data();
        let stored = store.load_all_events(&account).unwrap();
        assert_eq!(stored, original.events);
        let calendars = store.load_calendars(&account).unwrap();
        assert_eq!(calendars.len(), original.calendars.len());

        let series = original.events.iter().find(|e| e.is_series()).unwrap();
        let second = series.start + chrono::Duration::days(7);
        store
            .apply_cal_op(
                &account,
                &account,
                &CalOp::Delete {
                    id: series.id.clone(),
                    occurrence: Some(second),
                    scope: SeriesScope::This,
                    send_updates: SendUpdates::None,
                },
            )
            .unwrap();
        let after = &store
            .load_events(&account, std::slice::from_ref(&series.id))
            .unwrap()[0];
        assert!(after.exceptions.contains(&second));

        let invite = original
            .events
            .iter()
            .find(|e| e.attendees.iter().any(|a| a.email == account))
            .unwrap();
        store
            .apply_cal_op(
                &account,
                &account,
                &CalOp::Rsvp {
                    id: invite.id.clone(),
                    status: RsvpStatus::Declined,
                },
            )
            .unwrap();
        let after = &store
            .load_events(&account, std::slice::from_ref(&invite.id))
            .unwrap()[0];
        let me = after.attendees.iter().find(|a| a.email == account).unwrap();
        assert_eq!(me.status, RsvpStatus::Declined);

        // MC-002: which ids are still stored, in one query.
        let ids = vec![invite.id.clone(), "gone".to_string()];
        let present = store.existing_event_ids(&account, &ids).unwrap();
        assert_eq!(present.len(), 1);
        assert!(present.contains(&invite.id));
        assert_eq!(store.load_events(&account, &ids).unwrap().len(), 1);
    }
}

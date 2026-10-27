//! Calendars and events in the store. Events are kept whole as JSON
//! (`model`), with the columns lookups need beside it.

use chrono::NaiveDateTime;
use rusqlite::{OptionalExtension as _, Transaction, params};

use super::Store;
use crate::{
    backend::CalOp,
    calendar::{AccessRole, Calendar, Event, SeriesScope},
};

impl Store {
    pub fn put_calendars(&self, account: &str, calendars: &[Calendar]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for c in calendars {
            tx.execute(
                "INSERT INTO calendars (account, id, summary, bg, selected, access_role,
                    is_primary, time_zone, default_reminder)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (account, id) DO UPDATE SET
                    summary = excluded.summary, bg = excluded.bg, selected = excluded.selected,
                    access_role = excluded.access_role, is_primary = excluded.is_primary,
                    time_zone = excluded.time_zone, default_reminder = excluded.default_reminder",
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
                default_reminder
             FROM calendars WHERE account = ?1 AND hidden = 0 ORDER BY rowid",
        )?;
        let rows = stmt.query_map([account], |r| {
            Ok(Calendar {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r
                    .get::<_, Option<String>>(2)?
                    .and_then(|hex| crate::data::from_hex(&hex))
                    .unwrap_or(gpui::hsla(0.6, 0.1, 0.5, 1.)),
                visible: r.get(3)?,
                access_role: AccessRole::from_key(&r.get::<_, String>(4)?),
                primary: r.get(5)?,
                time_zone: r.get(6)?,
                default_reminder: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn put_events(&self, account: &str, events: &[Event]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for event in events {
            write_event(&tx, account, event)?;
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
        let mut stmt = conn.prepare(
            "SELECT id, model FROM events
             WHERE account = ?1 AND (status IS NULL OR status != 'cancelled') ORDER BY rowid",
        )?;
        let mut rows = stmt.query([account])?;
        let mut events = Vec::new();
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            if ids.is_some_and(|ids| !ids.contains(&id)) {
                continue;
            }
            events.push(serde_json::from_str(&r.get::<_, String>(1)?)?);
        }
        Ok(events)
    }

    pub fn delete_events(&self, account: &str, ids: &[String]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute(
                "DELETE FROM events WHERE account = ?1 AND id = ?2",
                params![account, id],
            )?;
        }
        tx.commit()?;
        Ok(())
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
                is_primary, time_zone, default_reminder)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (account, id) DO UPDATE SET
                summary = excluded.summary, bg = excluded.bg, selected = excluded.selected,
                hidden = excluded.hidden, access_role = excluded.access_role,
                is_primary = excluded.is_primary, time_zone = excluded.time_zone,
                default_reminder = excluded.default_reminder",
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
            ],
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
        let removed = self.delete_calendar_events(account, id, None)?;
        self.conn().execute(
            "DELETE FROM calendars WHERE account = ?1 AND id = ?2",
            params![account, id],
        )?;
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
        let mut removed = Vec::new();
        {
            let mut select = tx.prepare(
                "SELECT id, recurring_event_id FROM events WHERE account = ?1 AND calendar_id = ?2",
            )?;
            let rows = select.query_map(params![account, calendar], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (id, series) = row?;
                let goes = ids.is_none_or(|ids| {
                    ids.contains(&id) || series.as_ref().is_some_and(|s| ids.contains(s))
                });
                if goes {
                    removed.push(id);
                }
            }
            let mut delete = tx.prepare(
                "DELETE FROM events WHERE account = ?1 AND calendar_id = ?2 AND id = ?3",
            )?;
            for id in &removed {
                delete.execute(params![account, calendar, id])?;
            }
        }
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
            write_event(&tx, account, event)?;
            tx.execute(
                "UPDATE events SET json = ?4 WHERE account = ?1 AND calendar_id = ?2 AND id = ?3",
                params![account, event.calendar, event.id, json],
            )?;
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
        let mut stmt = conn.prepare(
            "SELECT model FROM events
             WHERE account = ?1 AND calendar_id = ?2 AND recurring_event_id = ?3",
        )?;
        let rows = stmt.query_map(params![account, calendar, series], |r| {
            r.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for model in rows {
            let event: Event = serde_json::from_str(&model?)?;
            out.extend(event.original_start);
        }
        Ok(out)
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
            CalOp::Insert { event, .. } | CalOp::Patch { after: event, .. } => {
                write_event(&tx, account, event)?
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
                    write_event(&tx, account, &series)?;
                } else {
                    tx.execute(
                        "DELETE FROM events WHERE account = ?1 AND id = ?2",
                        params![account, id],
                    )?;
                }
                // Occurrences changed on their own go with the ones deleted.
                let original = occurrence.to_string();
                match scope {
                    SeriesScope::All => tx.execute(
                        "DELETE FROM events WHERE account = ?1 AND recurring_event_id = ?2",
                        params![account, id],
                    )?,
                    SeriesScope::Following => tx.execute(
                        "DELETE FROM events WHERE account = ?1 AND recurring_event_id = ?2
                            AND original_start >= ?3",
                        params![account, id, original],
                    )?,
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
                    write_event(&tx, account, &event)?;
                }
            }
            CalOp::Rsvp { id, status } => {
                if let Some(mut event) = read_event(&tx, account, id)? {
                    for attendee in &mut event.attendees {
                        if attendee.email.eq_ignore_ascii_case(me) {
                            attendee.status = *status;
                        }
                    }
                    write_event(&tx, account, &event)?;
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

fn read_event(tx: &Transaction, account: &str, id: &str) -> anyhow::Result<Option<Event>> {
    let model: Option<String> = tx
        .query_row(
            "SELECT model FROM events WHERE account = ?1 AND id = ?2",
            params![account, id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match model {
        Some(model) => Some(serde_json::from_str(&model)?),
        None => None,
    })
}

fn write_event(tx: &Transaction, account: &str, event: &Event) -> anyhow::Result<()> {
    let status = serde_json::to_value(event.status)?;
    tx.execute(
        "INSERT INTO events (account, calendar_id, id, etag, ical_uid, status, start, \"end\",
            all_day, recurring_event_id, original_start, model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT (account, calendar_id, id) DO UPDATE SET
            etag = excluded.etag, ical_uid = excluded.ical_uid, status = excluded.status,
            start = excluded.start, \"end\" = excluded.\"end\", all_day = excluded.all_day,
            recurring_event_id = excluded.recurring_event_id,
            original_start = excluded.original_start, model = excluded.model",
        params![
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
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::backend::{CalOp, SendUpdates, demo, store::Store};
    use crate::calendar::{RsvpStatus, SeriesScope};

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
    }
}

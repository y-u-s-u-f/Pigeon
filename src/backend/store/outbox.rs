//! The outbox's rows (PER-2). `backend::outbox` decides what goes in.

use std::collections::HashSet;

use rusqlite::{OptionalExtension as _, params};

use super::Store;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxRow {
    pub id: i64,
    pub kind: String,
    pub resource: String,
    /// JSON `backend::Op`.
    pub payload: String,
    /// Unix ms.
    pub not_before: i64,
    pub state: String,
    pub attempts: i64,
}

const COLUMNS: &str = "id, kind, resource, payload, not_before, state, attempts";

fn row(r: &rusqlite::Row) -> rusqlite::Result<OutboxRow> {
    Ok(OutboxRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        resource: r.get(2)?,
        payload: r.get(3)?,
        not_before: r.get(4)?,
        state: r.get(5)?,
        attempts: r.get(6)?,
    })
}

impl Store {
    pub fn outbox_insert(
        &self,
        account: &str,
        kind: &str,
        resource: &str,
        payload: &str,
        not_before: i64,
        now: i64,
    ) -> anyhow::Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO outbox (account, kind, resource, payload, not_before, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![account, kind, resource, payload, not_before, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn outbox_row(&self, id: i64) -> anyhow::Result<Option<OutboxRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM outbox WHERE id = ?1"),
                [id],
                row,
            )
            .optional()?)
    }

    /// The account's rows that are still to go, oldest first.
    pub fn outbox_pending(&self, account: &str) -> anyhow::Result<Vec<OutboxRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outbox WHERE account = ?1 AND state IN ('pending', 'inflight')
             ORDER BY id"
        ))?;
        Ok(stmt.query_map([account], row)?.collect::<Result<_, _>>()?)
    }

    /// The calendars with a show/hide still to go (`SetSelected`'s resource
    /// is `calendar:<id>`).
    pub fn queued_selections(&self, account: &str) -> anyhow::Result<HashSet<String>> {
        Ok(self
            .outbox_pending(account)?
            .into_iter()
            .filter(|r| r.kind == "calendar.set_selected")
            .filter_map(|r| r.resource.strip_prefix("calendar:").map(String::from))
            .collect())
    }

    pub fn outbox_update_payload(
        &self,
        id: i64,
        payload: &str,
        not_before: i64,
    ) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE outbox SET payload = ?2, not_before = ?3 WHERE id = ?1",
            params![id, payload, not_before],
        )?;
        Ok(())
    }

    pub fn outbox_set_state(
        &self,
        id: i64,
        state: &str,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE outbox SET state = ?2, error = ?3,
                attempts = attempts + (?2 = 'inflight') WHERE id = ?1",
            params![id, state, error],
        )?;
        Ok(())
    }

    /// Deletes the row; false when there was none.
    pub fn outbox_delete(&self, id: i64) -> anyhow::Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM outbox WHERE id = ?1", [id])?
            > 0)
    }
}

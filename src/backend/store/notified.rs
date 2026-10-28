//! Messages already notified about (NTF-1, sprint 2 W1).

use std::collections::HashSet;

use rusqlite::params;

use super::Store;

/// How many notified ids are kept per account: far more than arrive
/// between two polls.
pub const NOTIFIED_KEEP: usize = 500;

impl Store {
    /// Which of `ids` were notified about already.
    pub fn notified(&self, account: &str, ids: &[String]) -> anyhow::Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT 1 FROM notified WHERE account = ?1 AND message_id = ?2")?;
        let mut found = HashSet::new();
        for id in ids {
            if stmt.exists(params![account, id])? {
                found.insert(id.clone());
            }
        }
        Ok(found)
    }

    /// Record `ids` as notified and set the sync state `key` to `value`
    /// (the history id) together, so neither moves without the other.
    pub fn checkpoint_notified(
        &self,
        account: &str,
        ids: &[String],
        key: &str,
        value: &str,
        now: i64,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for id in ids {
            tx.prepare_cached(
                "INSERT OR IGNORE INTO notified (account, message_id, at) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![account, id, now])?;
        }
        tx.execute(
            "DELETE FROM notified WHERE account = ?1 AND rowid NOT IN
                (SELECT rowid FROM notified WHERE account = ?1
                 ORDER BY at DESC, rowid DESC LIMIT ?2)",
            params![account, NOTIFIED_KEEP as i64],
        )?;
        tx.execute(
            "INSERT INTO sync_state (account, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT (account, key) DO UPDATE SET value = excluded.value",
            params![account, key, value],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notified_ids_are_bounded_and_kept_with_the_cursor() {
        let store = Store::open_in_memory().unwrap();
        let ids: Vec<String> = (0..NOTIFIED_KEEP + 10).map(|i| format!("m{i}")).collect();
        store
            .checkpoint_notified("a", &ids[..10], "h", "5", 1)
            .unwrap();
        store
            .checkpoint_notified("a", &ids[10..], "h", "6", 2)
            .unwrap();
        assert_eq!(store.sync_state("a", "h").unwrap().as_deref(), Some("6"));
        // The oldest went; the newest stay.
        assert!(store.notified("a", &ids[..10]).unwrap().is_empty());
        assert_eq!(
            store.notified("a", &ids[10..]).unwrap().len(),
            NOTIFIED_KEEP
        );
        // Per account.
        assert!(store.notified("b", &ids).unwrap().is_empty());
    }
}

//! Snoozed conversations' wake times (ACT-11). `backend::later` decides
//! when they wake.

use rusqlite::params;

use super::Store;

/// One conversation's snooze: still asleep (`wake_at`), or woken
/// (`woke_at`). Unix ms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snooze {
    pub thread_id: String,
    pub wake_at: Option<i64>,
    pub snoozed_at: i64,
    pub woke_at: Option<i64>,
}

impl Store {
    pub fn snoozes(&self, account: &str) -> anyhow::Result<Vec<Snooze>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT thread_id, wake_at, snoozed_at, woke_at FROM snoozes WHERE account = ?1",
        )?;
        let rows = stmt.query_map([account], |r| {
            Ok(Snooze {
                thread_id: r.get(0)?,
                wake_at: r.get(1)?,
                snoozed_at: r.get(2)?,
                woke_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Write `snooze` over the thread's row.
    pub fn put_snooze(&self, account: &str, snooze: &Snooze) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO snoozes (account, thread_id, wake_at, snoozed_at, woke_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (account, thread_id) DO UPDATE SET wake_at = excluded.wake_at,
                snoozed_at = excluded.snoozed_at, woke_at = excluded.woke_at",
            params![
                account,
                snooze.thread_id,
                snooze.wake_at,
                snooze.snoozed_at,
                snooze.woke_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_snooze(&self, account: &str, thread_id: &str) -> anyhow::Result<()> {
        self.conn().execute(
            "DELETE FROM snoozes WHERE account = ?1 AND thread_id = ?2",
            params![account, thread_id],
        )?;
        Ok(())
    }

    /// Woken rows from before `before` go: they no longer change the order.
    pub fn prune_woken(&self, account: &str, before: i64) -> anyhow::Result<()> {
        self.conn().execute(
            "DELETE FROM snoozes WHERE account = ?1 AND wake_at IS NULL AND woke_at < ?2",
            params![account, before],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snoozes_round_trip_per_account() {
        let store = Store::open_in_memory().unwrap();
        let asleep = Snooze {
            thread_id: "t".into(),
            wake_at: Some(10),
            snoozed_at: 1,
            woke_at: None,
        };
        store.put_snooze("a", &asleep).unwrap();
        assert_eq!(store.snoozes("a").unwrap(), std::slice::from_ref(&asleep));
        assert!(store.snoozes("b").unwrap().is_empty());
        let woke = Snooze {
            wake_at: None,
            woke_at: Some(10),
            ..asleep.clone()
        };
        store.put_snooze("a", &woke).unwrap();
        assert_eq!(store.snoozes("a").unwrap(), [woke]);
        store.prune_woken("a", 5).unwrap();
        assert_eq!(store.snoozes("a").unwrap().len(), 1);
        store.prune_woken("a", 11).unwrap();
        assert!(store.snoozes("a").unwrap().is_empty());
        store.put_snooze("a", &asleep).unwrap();
        store.delete_snooze("a", "t").unwrap();
        assert!(store.snoozes("a").unwrap().is_empty());
    }
}

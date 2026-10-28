//! The local SQLite store: the cache of mail, calendars and contacts, the
//! outbox, and the app's settings. One connection behind a mutex; every
//! call is a short transaction, so callers off the main thread hold it
//! briefly. The tables are in `schema.sql`.

mod calendar;
mod contacts;
mod images;
mod mail;
mod notified;
mod outbox;
mod snoozes;

pub use snoozes::Snooze;

pub use images::IMAGE_CACHE_CAP;
pub use mail::modify_thread;

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::Context as _;
use rusqlite::{Connection, OptionalExtension as _, params};

const SCHEMA: &str = include_str!("schema.sql");

/// The sync state key of the account's aliases (Gmail's `sendAs`), as JSON.
pub const ALIASES_KEY: &str = "gmail_aliases";
/// W3 (MST-1): every send-as address with its name, reply-to and
/// signature, as JSON `[SendAs]`.
pub const SEND_AS_KEY: &str = "gmail_send_as";

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// The file at `path`, created (0600, WAL) and migrated as needed.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("couldn't create {}", dir.display()))?;
            set_mode(dir, 0o700);
        }
        let conn = Connection::open(path)?;
        set_mode(path, 0o600);
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::new(conn)
    }

    /// A private store that goes away with it: demo mode and tests.
    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::new(Connection::open_in_memory()?)
    }

    fn new(mut conn: Connection) -> anyhow::Result<Self> {
        migrate(&mut conn)?;
        // Room for every statement a sync step writes with (MC-004).
        conn.set_prepared_statement_cache_capacity(64);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic mid-transaction rolls it back, so the data is still sound.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // Settings (PER-3): one row per key, the value as JSON.

    pub fn load_settings(&self) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut map = serde_json::Map::new();
        for row in rows {
            let (key, value) = row?;
            if let Ok(value) = serde_json::from_str(&value) {
                map.insert(key, value);
            }
        }
        Ok(map)
    }

    pub fn save_settings(
        &self,
        settings: &serde_json::Map<String, serde_json::Value>,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (key, value) in settings {
            tx.prepare_cached(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            )?
            .execute(params![key, value.to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }

    // Sync state: history ids, sync tokens, the last sync.

    pub fn sync_state(&self, account: &str, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT value FROM sync_state WHERE account = ?1 AND key = ?2",
                params![account, key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_sync_state(&self, account: &str, key: &str, value: &str) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO sync_state (account, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT (account, key) DO UPDATE SET value = excluded.value",
            params![account, key, value],
        )?;
        Ok(())
    }

    /// The account's addresses: its own and its aliases as last synced, to
    /// tell mail the user received from mail they sent (`derive_folder`).
    pub fn me(&self, account: &str) -> crate::data::Account {
        let aliases = self
            .sync_state(account, ALIASES_KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        crate::data::Account {
            email: account.to_string(),
            aliases,
            ..Default::default()
        }
    }

    /// Everything stored for `account`: signing out.
    pub fn delete_account(&self, account: &str) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for table in [
            "labels",
            "threads",
            "messages",
            "bodies",
            "drafts",
            "calendars",
            "events",
            "contacts",
            "sync_state",
            "outbox",
            "notified",
            "fired_reminders",
            "draft_tombstones",
            "snoozes",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE account = ?1"),
                [account],
            )?;
        }
        // W10: images aren't per account; none outlive a sign-out.
        tx.execute("DELETE FROM images", [])?;
        tx.commit()?;
        Ok(())
    }
}

/// Applies the blocks of `schema.sql` newer than the file's `user_version`.
fn migrate(conn: &mut Connection) -> anyhow::Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (version, sql) in migrations() {
        if version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("migration {version:03} failed"))?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

/// The numbered blocks, in order.
fn migrations() -> Vec<(i64, &'static str)> {
    const MARK: &str = "-- migration: ";
    SCHEMA
        .split(MARK)
        .skip(1)
        .map(|block| {
            let (number, sql) = block.split_once('\n').unwrap_or((block, ""));
            let number = number
                .trim()
                .parse()
                .expect("migration numbers are numbers");
            (number, sql)
        })
        .collect()
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) {}

/// JSON for a column.
fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("model types serialize")
}

/// A JSON column, or the type's default when it doesn't parse.
fn from_json<T: serde::de::DeserializeOwned + Default>(text: &str) -> T {
    serde_json::from_str(text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_numbered_in_order() {
        let numbers: Vec<i64> = migrations().iter().map(|(n, _)| *n).collect();
        assert_eq!(numbers[0], 1);
        assert!(numbers.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn settings_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let mut map = serde_json::Map::new();
        map.insert("appearance".into(), "dark".into());
        map.insert("text_scale".into(), 1.2.into());
        store.save_settings(&map).unwrap();
        map.insert("appearance".into(), "light".into());
        store.save_settings(&map).unwrap();
        assert_eq!(store.load_settings().unwrap(), map);
    }

    #[test]
    fn sync_state_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.sync_state("a", "k").unwrap(), None);
        store.set_sync_state("a", "k", "1").unwrap();
        store.set_sync_state("a", "k", "2").unwrap();
        assert_eq!(store.sync_state("a", "k").unwrap().as_deref(), Some("2"));
    }
}

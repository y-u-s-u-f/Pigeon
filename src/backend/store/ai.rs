//! AI results (AI-2): one row per account, feature and key, replaced by
//! a newer one. Rows only go in while their conversation is stored, so
//! nothing written after `delete_account` stays.

use std::collections::HashMap;

use rusqlite::params;

use super::{Store, json};
use crate::ai::{AiRow, Feature, ThreadAi};

impl Store {
    /// Write `row` for `thread_id`; false when the conversation isn't
    /// stored (any more), and nothing was written.
    pub fn put_ai_result(
        &self,
        account: &str,
        thread_id: &str,
        row: &AiRow,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let written = tx
            .prepare_cached(
                "INSERT INTO ai_results
                    (account, feature, key, thread_id, input_hash, model, value, created)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
                 WHERE EXISTS (SELECT 1 FROM threads WHERE account = ?1 AND id = ?4)
                 ON CONFLICT (account, feature, key) DO UPDATE SET
                    thread_id = excluded.thread_id, input_hash = excluded.input_hash,
                    model = excluded.model, value = excluded.value, created = excluded.created",
            )?
            .execute(params![
                account,
                row.feature.as_str(),
                row.key,
                thread_id,
                row.input_hash,
                row.model,
                row.value,
                row.created
            ])?;
        tx.commit()?;
        Ok(written > 0)
    }

    pub fn delete_ai_result(
        &self,
        account: &str,
        feature: Feature,
        key: &str,
    ) -> anyhow::Result<()> {
        self.conn().execute(
            "DELETE FROM ai_results WHERE account = ?1 AND feature = ?2 AND key = ?3",
            params![account, feature.as_str(), key],
        )?;
        Ok(())
    }

    /// The rows of these conversations (all of them for `None`), by
    /// conversation id. Rows of features this build doesn't know are left
    /// out.
    pub fn ai_results(
        &self,
        account: &str,
        thread_ids: Option<&[String]>,
    ) -> anyhow::Result<HashMap<String, ThreadAi>> {
        let conn = self.conn();
        let columns = "SELECT thread_id, feature, key, input_hash, model, value, created
                       FROM ai_results WHERE account = ?1";
        let mut out: HashMap<String, ThreadAi> = HashMap::new();
        let mut push = |r: &rusqlite::Row| -> rusqlite::Result<()> {
            let thread_id: String = r.get(0)?;
            let Some(feature) = Feature::parse(&r.get::<_, String>(1)?) else {
                return Ok(());
            };
            out.entry(thread_id).or_default().rows.push(AiRow {
                feature,
                key: r.get(2)?,
                input_hash: r.get(3)?,
                model: r.get(4)?,
                value: r.get(5)?,
                created: r.get(6)?,
            });
            Ok(())
        };
        match thread_ids {
            None => {
                let mut stmt = conn.prepare_cached(columns)?;
                let mut rows = stmt.query([account])?;
                while let Some(r) = rows.next()? {
                    push(r)?;
                }
            }
            Some(ids) => {
                let mut stmt = conn.prepare_cached(&format!(
                    "{columns} AND thread_id IN (SELECT value FROM json_each(?2))"
                ))?;
                let mut rows = stmt.query(params![account, json(&ids)])?;
                while let Some(r) = rows.next()? {
                    push(r)?;
                }
            }
        }
        Ok(out)
    }

    /// At startup: rows whose conversation is no longer stored.
    pub fn prune_ai_results(&self) -> anyhow::Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM ai_results WHERE NOT EXISTS (SELECT 1 FROM threads
                WHERE threads.account = ai_results.account AND threads.id = ai_results.thread_id)",
            [],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::mime::ParsedBody,
        data::{Body, Contact, Folder, Message, Thread},
    };

    const ME: &str = "me@example.com";

    fn row(feature: Feature, key: &str, value: &str) -> AiRow {
        AiRow {
            feature,
            key: key.into(),
            input_hash: "h1".into(),
            model: "test-model".into(),
            value: value.into(),
            created: 1,
        }
    }

    fn store_with_thread() -> Store {
        let store = Store::open_in_memory().unwrap();
        let mut m = Message::new(Contact::default());
        m.id = "m1".into();
        m.body = Body::NotLoaded;
        let thread = Thread::local("t1".into(), "S".into(), Folder::Inbox, vec![m], None);
        store.put_threads(ME, &[thread]).unwrap();
        store
    }

    #[test]
    fn rows_round_trip_and_replace() {
        let store = store_with_thread();
        assert!(
            store
                .put_ai_result(ME, "t1", &row(Feature::Line, "t1", "a"))
                .unwrap()
        );
        assert!(
            store
                .put_ai_result(ME, "t1", &row(Feature::Line, "t1", "b"))
                .unwrap()
        );
        assert!(
            store
                .put_ai_result(ME, "t1", &row(Feature::Signals, "m1", "{}"))
                .unwrap()
        );
        let all = store.ai_results(ME, None).unwrap();
        let ai = &all["t1"];
        assert_eq!(ai.rows.len(), 2);
        assert_eq!(ai.get(Feature::Line, "t1").unwrap().value, "b");
        assert!(ai.fresh(Feature::Line, "t1", "h1").is_some());
        assert!(ai.fresh(Feature::Line, "t1", "h2").is_none());
        let some = store.ai_results(ME, Some(&["t1".to_string()])).unwrap();
        assert_eq!(some.len(), 1);
        assert!(
            store
                .ai_results(ME, Some(&["x".to_string()]))
                .unwrap()
                .is_empty()
        );
        store.delete_ai_result(ME, Feature::Line, "t1").unwrap();
        assert_eq!(store.ai_results(ME, None).unwrap()["t1"].rows.len(), 1);
    }

    #[test]
    fn results_written_after_delete_account_dont_appear() {
        let store = store_with_thread();
        assert!(
            store
                .put_ai_result(ME, "t1", &row(Feature::Line, "t1", "a"))
                .unwrap()
        );
        store.delete_account(ME).unwrap();
        assert!(store.ai_results(ME, None).unwrap().is_empty());
        // A job that finishes after sign-out writes nothing.
        assert!(
            !store
                .put_ai_result(ME, "t1", &row(Feature::Line, "t1", "late"))
                .unwrap()
        );
        assert!(store.ai_results(ME, None).unwrap().is_empty());
    }

    #[test]
    fn put_bodies_after_delete_account_writes_nothing() {
        let store = store_with_thread();
        store.delete_account(ME).unwrap();
        let body = ParsedBody {
            html: "<p>late</p>".into(),
            ..Default::default()
        };
        store
            .put_bodies(ME, "t1", vec![("m1".into(), body)])
            .unwrap();
        assert!(
            store
                .load_threads(ME, &["t1".to_string()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn startup_cleanup_drops_rows_of_gone_conversations() {
        let store = store_with_thread();
        store
            .put_ai_result(ME, "t1", &row(Feature::Line, "t1", "a"))
            .unwrap();
        // Written while it was there; the conversation went since.
        store
            .conn()
            .execute("DELETE FROM threads WHERE id = 't1'", [])
            .unwrap();
        assert_eq!(store.prune_ai_results().unwrap(), 1);
        assert!(store.ai_results(ME, None).unwrap().is_empty());
    }
}

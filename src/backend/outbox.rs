//! The write queue (2.7). Every op is a row in the store's `outbox` table
//! until it commits, so it survives a quit (PER-2). Ops the UI can undo
//! wait out the undo window first; Undo inside it deletes the row, so
//! nothing was ever sent. Consecutive label changes to the same threads
//! merge, and a newer save of a draft replaces an older one.
//!
//! The commit itself (requests, retries, rollback, the pending-op overlay)
//! is W5's; demo mode commits by applying the op to the store.

use super::{MailOp, Op, OpId, store::Store};

pub struct Outbox {
    store: Store,
    account: String,
}

impl Outbox {
    pub fn new(store: Store, account: String) -> Self {
        Self { store, account }
    }

    /// Queue `op` to go at `now + delay_ms` (Unix ms).
    pub fn push(&self, op: Op, delay_ms: i64, now: i64) -> anyhow::Result<OpId> {
        let not_before = now + delay_ms;
        if let Some(id) = self.coalesce(&op, not_before, now)? {
            return Ok(id);
        }
        self.store.outbox_insert(
            &self.account,
            op.kind(),
            &op.resource(),
            &serde_json::to_string(&op)?,
            not_before,
            now,
        )
    }

    /// Fold `op` into a waiting row it supersedes, if there is one.
    fn coalesce(&self, op: &Op, not_before: i64, now: i64) -> anyhow::Result<Option<OpId>> {
        let waiting = self.store.outbox_pending(&self.account)?;
        let Some(last) = waiting
            .iter()
            .rev()
            .find(|r| r.resource == op.resource() && r.state == "pending")
        else {
            return Ok(None);
        };
        let Ok(queued) = serde_json::from_str::<Op>(&last.payload) else {
            return Ok(None);
        };
        let merged = match (queued, op) {
            // Read, star and label changes, which go at once.
            (
                Op::Mail(MailOp::Modify {
                    threads,
                    add,
                    remove,
                }),
                Op::Mail(MailOp::Modify {
                    threads: new_threads,
                    add: new_add,
                    remove: new_remove,
                }),
            ) if threads == *new_threads && last.not_before <= now && not_before <= now => {
                let mut add: Vec<String> = add
                    .into_iter()
                    .filter(|l| !new_remove.contains(l))
                    .collect();
                let mut remove: Vec<String> = remove
                    .into_iter()
                    .filter(|l| !new_add.contains(l))
                    .collect();
                for l in new_add {
                    if !add.contains(l) {
                        add.push(l.clone());
                    }
                }
                for l in new_remove {
                    if !remove.contains(l) {
                        remove.push(l.clone());
                    }
                }
                Op::Mail(MailOp::Modify {
                    threads,
                    add,
                    remove,
                })
            }
            (Op::Mail(MailOp::SaveDraft { .. }), Op::Mail(MailOp::SaveDraft { .. })) => op.clone(),
            _ => return Ok(None),
        };
        self.store
            .outbox_update_payload(last.id, &serde_json::to_string(&merged)?, not_before)?;
        Ok(Some(last.id))
    }

    /// Take back an op that hasn't started to go. False when it has (or
    /// is gone): the caller undoes it with the inverse op instead.
    pub fn cancel(&self, id: OpId) -> bool {
        match self.store.outbox_row(id) {
            Ok(Some(row)) if row.state == "pending" => {
                self.store.outbox_delete(id).unwrap_or(false)
            }
            _ => false,
        }
    }

    /// The ops due at `now`, marked in flight, oldest first; one at a time
    /// per thread, event or draft.
    pub fn take_due(&self, now: i64) -> anyhow::Result<Vec<(OpId, Op)>> {
        let rows = self.store.outbox_pending(&self.account)?;
        let mut busy: Vec<&str> = Vec::new();
        let mut due = Vec::new();
        for row in &rows {
            let blocked = busy.contains(&row.resource.as_str());
            busy.push(&row.resource);
            if blocked || row.state != "pending" || row.not_before > now {
                continue;
            }
            match serde_json::from_str::<Op>(&row.payload) {
                Ok(op) => {
                    self.store.outbox_set_state(row.id, "inflight", None)?;
                    due.push((row.id, op));
                }
                Err(e) => self
                    .store
                    .outbox_set_state(row.id, "failed", Some(&e.to_string()))?,
            }
        }
        Ok(due)
    }

    /// The op went: its row goes.
    pub fn finish(&self, id: OpId) -> anyhow::Result<()> {
        self.store.outbox_delete(id)?;
        Ok(())
    }

    pub fn fail(&self, id: OpId, error: &str) -> anyhow::Result<()> {
        self.store.outbox_set_state(id, "failed", Some(error))
    }

    /// When the next waiting op is due.
    pub fn next_due(&self) -> Option<i64> {
        self.store
            .outbox_pending(&self.account)
            .ok()?
            .iter()
            .filter(|r| r.state == "pending")
            .map(|r| r.not_before)
            .min()
    }

    /// How many ops are still to go ("3 changes waiting").
    pub fn waiting(&self) -> usize {
        self.store
            .outbox_pending(&self.account)
            .map_or(0, |rows| rows.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{CalOp, Outgoing, SendUpdates};

    fn outbox() -> Outbox {
        Outbox::new(Store::open_in_memory().unwrap(), "me@example.com".into())
    }

    fn modify(threads: &[&str], add: &[&str], remove: &[&str]) -> Op {
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect();
        Op::Mail(MailOp::Modify {
            threads: v(threads),
            add: v(add),
            remove: v(remove),
        })
    }

    #[test]
    fn delayed_ops_wait_and_cancel() {
        let outbox = outbox();
        let id = outbox
            .push(
                Op::Mail(MailOp::Trash {
                    threads: vec!["t1".into()],
                }),
                5_000,
                1_000,
            )
            .unwrap();
        assert!(outbox.take_due(1_000).unwrap().is_empty());
        assert_eq!(outbox.next_due(), Some(6_000));
        assert!(outbox.cancel(id));
        assert!(!outbox.cancel(id));
        assert!(outbox.take_due(10_000).unwrap().is_empty());
        assert_eq!(outbox.waiting(), 0);
    }

    #[test]
    fn due_ops_go_once_and_cant_be_cancelled() {
        let outbox = outbox();
        let id = outbox
            .push(
                Op::Calendar(CalOp::Delete {
                    id: "e1".into(),
                    occurrence: None,
                    scope: crate::calendar::SeriesScope::All,
                    send_updates: SendUpdates::None,
                }),
                5_000,
                0,
            )
            .unwrap();
        let due = outbox.take_due(5_000).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, id);
        assert!(!outbox.cancel(id), "in flight");
        assert!(outbox.take_due(6_000).unwrap().is_empty());
        outbox.finish(id).unwrap();
        assert_eq!(outbox.waiting(), 0);
    }

    #[test]
    fn label_changes_and_draft_saves_coalesce() {
        let outbox = outbox();
        let a = outbox
            .push(modify(&["t1"], &["STARRED"], &[]), 0, 0)
            .unwrap();
        let b = outbox
            .push(modify(&["t1"], &["work"], &["STARRED", "UNREAD"]), 0, 0)
            .unwrap();
        assert_eq!(a, b);
        // Another thread, and a delayed move, stay apart.
        outbox
            .push(modify(&["t2"], &["STARRED"], &[]), 0, 0)
            .unwrap();
        outbox
            .push(modify(&["t1"], &[], &["INBOX"]), 5_000, 0)
            .unwrap();
        assert_eq!(outbox.waiting(), 3);
        let due = outbox.take_due(0).unwrap();
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].1, modify(&["t1"], &["work"], &["STARRED", "UNREAD"]));

        let save = |subject: &str| {
            Op::Mail(MailOp::SaveDraft {
                local_id: "draft-local-1".into(),
                draft: Outgoing {
                    subject: subject.into(),
                    ..Outgoing::default()
                },
            })
        };
        let first = outbox.push(save("One"), 0, 0).unwrap();
        let second = outbox.push(save("Two"), 0, 0).unwrap();
        assert_eq!(first, second);
        assert_eq!(outbox.take_due(0).unwrap()[0].1, save("Two"));
    }

    #[test]
    fn one_op_at_a_time_per_resource() {
        let outbox = outbox();
        outbox
            .push(modify(&["t1"], &[], &["INBOX"]), 5_000, 0)
            .unwrap();
        outbox
            .push(modify(&["t1"], &["STARRED"], &[]), 0, 0)
            .unwrap();
        // The star waits behind the archive of the same thread.
        assert!(outbox.take_due(0).unwrap().is_empty());
        assert_eq!(outbox.take_due(5_000).unwrap().len(), 1);
    }
}

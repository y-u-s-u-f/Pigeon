//! The write queue (2.7). Every op is a row in the store's `outbox` table
//! until it commits, so it survives a quit (PER-2). Ops the UI can undo
//! wait out the undo window first; Undo inside it deletes the row, so
//! nothing was ever sent. Consecutive label changes to the same threads
//! merge, and a newer save of a draft replaces an older one.
//!
//! - The store keeps the server's state; ops still queued are laid over it
//!   whenever threads are read (`overlay`), so a poll inside the undo
//!   window doesn't bring an archived thread back.
//! - Thread ops commit with `threads.modify` / `trash` / `untrash`, several
//!   threads as one HTTP batch. What went is applied to the store; threads
//!   Gmail refused are read again from Gmail (the rollback, STA-2).
//! - No network or no sign-in: the op stays queued and goes later (STA-3).
//!
//! Demo mode commits by applying the op to the store.

use std::{collections::HashSet, sync::Arc};

use super::{
    MailOp, Op, OpId,
    gmail::{self, Gmail, GmailError},
    http::{GoogleTransport, Request, Response, Transport, TransportError},
    store::{Store, modify_thread},
};
use crate::data::{Account, Thread};

/// How long an op that couldn't reach Google waits before it goes again.
pub const RETRY_LATER_MS: i64 = 15_000;

pub struct Outbox {
    store: Store,
    account: String,
}

/// The threads, events or drafts a row's resource names: ops sharing one go
/// in order.
fn keys(resource: &str) -> impl Iterator<Item = &str> {
    resource.split(',')
}

impl Outbox {
    /// Rows left in flight by a quit go again (PER-2).
    pub fn new(store: Store, account: String) -> Self {
        let this = Self { store, account };
        if let Ok(rows) = this.store.outbox_pending(&this.account) {
            for row in rows.iter().filter(|r| r.state == "inflight") {
                this.store.outbox_set_state(row.id, "pending", None).ok();
            }
        }
        this
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

    /// W3 (DRF-5): the saves of draft `local_id` still waiting go at
    /// `now`, ahead of its send or discard.
    pub fn hurry(&self, local_id: &str, now: i64) -> anyhow::Result<()> {
        for row in self.store.outbox_pending(&self.account)? {
            if row.state == "pending"
                && row.kind == "mail.save_draft"
                && row.resource == local_id
                && row.not_before > now
            {
                self.store
                    .outbox_update_payload(row.id, &row.payload, now)?;
            }
        }
        Ok(())
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
        let mut busy: HashSet<&str> = HashSet::new();
        let mut due = Vec::new();
        for row in &rows {
            let blocked = keys(&row.resource).any(|k| busy.contains(k));
            busy.extend(keys(&row.resource));
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

    /// Couldn't reach Google: the op waits again, until `at`.
    pub fn retry_later(&self, id: OpId, at: i64, error: &str) -> anyhow::Result<()> {
        if let Some(row) = self.store.outbox_row(id)? {
            self.store.outbox_update_payload(id, &row.payload, at)?;
        }
        self.store.outbox_set_state(id, "pending", Some(error))
    }

    /// When the next op can go: the earliest of the ops not waiting behind
    /// another of the same thread, event or draft.
    pub fn next_due(&self) -> Option<i64> {
        let rows = self.store.outbox_pending(&self.account).ok()?;
        let mut busy: HashSet<&str> = HashSet::new();
        let mut next: Option<i64> = None;
        for row in &rows {
            let blocked = keys(&row.resource).any(|k| busy.contains(k));
            busy.extend(keys(&row.resource));
            if !blocked && row.state == "pending" {
                next = Some(next.map_or(row.not_before, |n| n.min(row.not_before)));
            }
        }
        next
    }

    /// Lay the thread ops still to go over `threads` read from the store.
    pub fn overlay(&self, threads: &mut [Thread]) {
        let Ok(rows) = self.store.outbox_pending(&self.account) else {
            return;
        };
        let me = self.store.me(&self.account);
        for row in rows {
            if let Ok(Op::Mail(op)) = serde_json::from_str::<Op>(&row.payload) {
                apply(&op, threads, &me);
            }
        }
    }

    /// W11, Quit (2.7): sends still waiting out their undo window are due
    /// at `now`.
    pub fn send_now(&self, now: i64) -> anyhow::Result<()> {
        for row in self.store.outbox_pending(&self.account)? {
            if row.state == "pending" && row.kind == "mail.send" && row.not_before > now {
                self.store
                    .outbox_update_payload(row.id, &row.payload, now)?;
            }
        }
        Ok(())
    }

    /// How many ops are still to go ("3 changes waiting").
    pub fn waiting(&self) -> usize {
        self.store
            .outbox_pending(&self.account)
            .map_or(0, |rows| rows.len())
    }
}

// Committing to Google.

/// Google's transport at the user's priority: ahead of sync, into the
/// quota's reserve.
pub struct UserPriority(pub Arc<GoogleTransport>);

impl Transport for UserPriority {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.0.send_user(request)
    }
}

/// How an op's commit ended.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Committed,
    /// For good. `changed` and `removed` are the threads put back to the
    /// server's state, for the app to reload (the rollback); `message` says
    /// why, in plain words.
    Failed {
        message: String,
        changed: Vec<String>,
        removed: Vec<String>,
    },
    /// No network, or the sign-in stopped working: it stays queued.
    Later(TransportError),
}

impl Outcome {
    pub fn failed(message: impl Into<String>) -> Self {
        Outcome::Failed {
            message: message.into(),
            changed: Vec::new(),
            removed: Vec::new(),
        }
    }
}

/// The labels a thread op adds and removes, and on which threads.
fn label_change(op: &MailOp) -> Option<(&[String], Vec<String>, Vec<String>)> {
    match op {
        MailOp::Modify {
            threads,
            add,
            remove,
        } => Some((threads, add.clone(), remove.clone())),
        MailOp::Trash { threads } => Some((threads, vec![gmail::TRASH.into()], Vec::new())),
        MailOp::Untrash { threads } => Some((threads, Vec::new(), vec![gmail::TRASH.into()])),
        MailOp::SaveDraft { .. } | MailOp::DeleteDraft { .. } | MailOp::Send { .. } => None,
    }
}

/// Apply a thread op to threads in memory, as Gmail will.
fn apply(op: &MailOp, threads: &mut [Thread], me: &Account) {
    let Some((ids, add, remove)) = label_change(op) else {
        return;
    };
    for thread in threads.iter_mut().filter(|t| ids.contains(&t.id)) {
        modify_thread(thread, &add, &remove, me);
    }
}

/// The same op on only some of its threads.
fn on_threads(op: &MailOp, ids: Vec<String>) -> MailOp {
    match op {
        MailOp::Modify { add, remove, .. } => MailOp::Modify {
            threads: ids,
            add: add.clone(),
            remove: remove.clone(),
        },
        MailOp::Trash { .. } => MailOp::Trash { threads: ids },
        MailOp::Untrash { .. } => MailOp::Untrash { threads: ids },
        other => other.clone(),
    }
}

/// The Gmail calls a thread op makes: one per thread.
pub fn requests(op: &MailOp) -> Vec<(String, Request)> {
    let each = |threads: &[String], request: &dyn Fn(&str) -> Request| {
        threads
            .iter()
            .map(|id| (id.clone(), request(id)))
            .collect::<Vec<_>>()
    };
    match op {
        MailOp::Modify {
            threads,
            add,
            remove,
        } => each(threads, &|id| gmail::thread_modify_request(id, add, remove)),
        MailOp::Trash { threads } => each(threads, &gmail::thread_trash_request),
        MailOp::Untrash { threads } => each(threads, &gmail::thread_untrash_request),
        MailOp::SaveDraft { .. } | MailOp::DeleteDraft { .. } | MailOp::Send { .. } => Vec::new(),
    }
}

/// Commit a thread op (`Modify`, `Trash`, `Untrash`): one call, or one
/// HTTP batch for several threads (ACT-6). What went is applied to the
/// store; the threads Gmail refused are read again from it, so only they
/// roll back.
pub fn commit_threads(gmail: &Gmail, store: &Store, account: &str, op: &MailOp) -> Outcome {
    let calls = requests(op);
    let results: Vec<Result<(), GmailError>> = match &calls[..] {
        [] => return Outcome::Committed,
        [(_, request)] => vec![gmail.call::<serde_json::Value>(request).map(|_| ())],
        _ => {
            let requests: Vec<Request> = calls.iter().map(|(_, r)| r.clone()).collect();
            gmail
                .batch::<serde_json::Value>(&requests)
                .into_iter()
                .map(|r| r.map(|_| ()))
                .collect()
        }
    };
    // Unreachable, or signed out: all of it goes again later.
    if let Some(e) = results
        .iter()
        .find_map(|r| r.as_ref().err().and_then(GmailError::later))
    {
        return Outcome::Later(e);
    }
    let mut done = Vec::new();
    let mut failed = Vec::new();
    let mut error = None;
    for ((id, _), result) in calls.into_iter().zip(results) {
        match result {
            Ok(()) => done.push(id),
            Err(e) => {
                failed.push(id);
                error.get_or_insert(e);
            }
        }
    }
    if !done.is_empty()
        && let Err(e) = store.apply_mail_op(account, &on_threads(op, done))
    {
        eprintln!("Pigeon: couldn't store a committed change: {e:#}");
    }
    let Some(error) = error else {
        return Outcome::Committed;
    };
    let message = failure_message(op, failed.len(), &error);
    let (changed, removed) = refetch(gmail, store, account, &failed);
    Outcome::Failed {
        message,
        changed,
        removed,
    }
}

/// Read threads again from Gmail and store them: (still there, gone). When
/// Gmail can't be read, the store's copy is still the server's last state.
fn refetch(
    gmail: &Gmail,
    store: &Store,
    account: &str,
    ids: &[String],
) -> (Vec<String>, Vec<String>) {
    let requests: Vec<Request> = ids.iter().map(|id| gmail::thread_get_request(id)).collect();
    let me = store.me(account);
    let mut fresh = Vec::new();
    let mut removed = Vec::new();
    for (id, result) in ids.iter().zip(gmail.batch::<gmail::GThread>(&requests)) {
        match result {
            Ok(t) => match gmail::thread_from_gmail(&t, &me) {
                Some(thread) => fresh.push(thread),
                None => removed.push(id.clone()),
            },
            Err(e) if e.is_not_found() => removed.push(id.clone()),
            Err(_) => {}
        }
    }
    if let Err(e) = store
        .put_synced_threads(account, fresh)
        .and_then(|()| store.delete_threads(account, &removed))
    {
        eprintln!("Pigeon: couldn't store threads read again: {e:#}");
    }
    let changed = ids
        .iter()
        .filter(|id| !removed.contains(id))
        .cloned()
        .collect();
    (changed, removed)
}

/// "Couldn't archive 2 conversations: Invalid label".
fn failure_message(op: &MailOp, count: usize, error: &GmailError) -> String {
    let has = |labels: &[String], label: &str| labels.iter().any(|l| l == label);
    let verb = match op {
        MailOp::Trash { .. } => "delete",
        MailOp::Untrash { .. } => "restore",
        MailOp::Modify { add, remove, .. } => {
            if has(add, gmail::INBOX) {
                "move to Inbox"
            } else if has(add, gmail::SPAM) {
                "move to Spam"
            } else if has(remove, gmail::INBOX) {
                "archive"
            } else if has(add, gmail::UNREAD) {
                "mark as unread"
            } else if has(remove, gmail::UNREAD) {
                "mark as read"
            } else if has(add, gmail::STARRED) {
                "star"
            } else if has(remove, gmail::STARRED) {
                "unstar"
            } else {
                "label"
            }
        }
        _ => "change",
    };
    let what = match count {
        1 => "the conversation".to_string(),
        n => format!("{n} conversations"),
    };
    let reason = match error {
        GmailError::Status { message, .. } if !message.is_empty() => {
            message.trim_end_matches('.').to_string()
        }
        GmailError::Status { status, .. } => format!("Gmail answered {status}"),
        GmailError::Network(_) => "Gmail couldn't be reached".into(),
        GmailError::Parse(_) => "Gmail's answer didn't make sense".into(),
    };
    format!("Couldn't {verb} {what}: {reason}")
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
                revision: 0,
            })
        };
        let first = outbox.push(save("One"), 0, 0).unwrap();
        let second = outbox.push(save("Two"), 0, 0).unwrap();
        assert_eq!(first, second);
        assert_eq!(outbox.take_due(0).unwrap()[0].1, save("Two"));
    }

    /// W3 (DRF-5): a send waits for the saves of its draft, and a save
    /// waiting out its 30 s goes at once when the draft is sent.
    #[test]
    fn a_send_goes_after_its_drafts_saves() {
        let outbox = outbox();
        let save = Op::Mail(MailOp::SaveDraft {
            local_id: "draft-local-1".into(),
            draft: Outgoing::default(),
            revision: 7,
        });
        let save = outbox.push(save, 30_000, 0).unwrap();
        let send = Op::Mail(MailOp::Send {
            draft: Outgoing::default(),
            local_id: Some("draft-local-1".into()),
            thread_id: "sent-local-1".into(),
        });
        let send = outbox.push(send, 5_000, 0).unwrap();
        // The send waits behind the save, even once its own time comes.
        assert!(outbox.take_due(5_000).unwrap().is_empty());
        outbox.hurry("draft-local-1", 6_000).unwrap();
        let due = outbox.take_due(6_000).unwrap();
        assert_eq!(due.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [save]);
        // In flight, the save still holds the send back.
        assert!(outbox.take_due(6_000).unwrap().is_empty());
        outbox.finish(save).unwrap();
        let due = outbox.take_due(6_000).unwrap();
        assert_eq!(due.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [send]);
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

    #[test]
    fn quit_sends_now_and_leaves_the_rest() {
        let outbox = outbox();
        let send = Op::Mail(MailOp::Send {
            draft: Outgoing::default(),
            local_id: None,
            thread_id: "sent-local-1".into(),
        });
        let send = outbox.push(send, 5_000, 1_000).unwrap();
        let trash = Op::Mail(MailOp::Trash {
            threads: vec!["t1".into()],
        });
        outbox.push(trash, 5_000, 1_000).unwrap();
        outbox.send_now(2_000).unwrap();
        let due = outbox.take_due(2_000).unwrap();
        assert_eq!(due.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [send]);
        assert_eq!(outbox.next_due(), Some(6_000));
    }

    // Commits (W5).

    use crate::backend::http::{FakeClock, FakeTransport};
    use crate::data::{Folder, Message};

    const ACCOUNT: &str = "me@example.com";
    const BATCH: &str = "/batch/gmail/v1";

    fn s(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|l| l.to_string()).collect()
    }

    /// A store holding Inbox threads with these ids.
    fn inbox(ids: &[&str]) -> Store {
        let store = Store::open_in_memory().unwrap();
        let threads: Vec<Thread> = ids
            .iter()
            .map(|id| {
                let mut thread = Thread::local(
                    id.to_string(),
                    format!("Subject {id}"),
                    Folder::Inbox,
                    vec![Message {
                        id: format!("m-{id}"),
                        label_ids: s(&[gmail::INBOX]),
                        ..Message::new(Default::default())
                    }],
                    None,
                );
                thread.label_ids = s(&[gmail::INBOX]);
                thread
            })
            .collect();
        store.put_threads(ACCOUNT, &threads).unwrap();
        store
    }

    fn folder(store: &Store, id: &str) -> Folder {
        store.load_threads(ACCOUNT, &[id.to_string()]).unwrap()[0].folder
    }

    /// Answers batches with boundary `b`.
    struct Batches(Arc<FakeTransport>);
    impl Transport for Batches {
        fn send(&self, r: &Request) -> Result<Response, TransportError> {
            let mut response = self.0.send(r)?;
            if r.path() == BATCH {
                response.headers =
                    vec![("Content-Type".into(), "multipart/mixed; boundary=b".into())];
            }
            Ok(response)
        }
    }

    fn batch_body(parts: &[(u16, &str)]) -> String {
        let mut out = String::new();
        for (ix, (status, body)) in parts.iter().enumerate() {
            out.push_str(&format!(
                "--b\r\nContent-ID: <response-item-{}>\r\n\r\nHTTP/1.1 {status} X\r\n\r\n{body}\r\n",
                ix + 1
            ));
        }
        out + "--b--\r\n"
    }

    fn gmail(fake: FakeTransport) -> (Gmail, Arc<FakeTransport>, Arc<FakeClock>) {
        let fake = Arc::new(fake);
        let clock = Arc::new(FakeClock::new());
        (
            Gmail::with_clock(Arc::new(Batches(fake.clone())), clock.clone()),
            fake,
            clock,
        )
    }

    /// (method, path, JSON body) of each call, batches opened up.
    fn calls(fake: &FakeTransport) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for request in fake.requests() {
            if request.path() != BATCH {
                out.push((
                    request.method.to_string(),
                    request.path().to_string(),
                    String::from_utf8_lossy(&request.body).to_string(),
                ));
                continue;
            }
            let body = String::from_utf8_lossy(&request.body).to_string();
            for part in body.split("--batch_pigeon").skip(1) {
                let Some(line) = part
                    .lines()
                    .find(|l| l.starts_with("POST ") || l.starts_with("GET "))
                else {
                    continue;
                };
                let mut words = line.split(' ');
                let method = words.next().unwrap().to_string();
                let path = words.next().unwrap().split('?').next().unwrap().to_string();
                let json = part
                    .lines()
                    .find(|l| l.starts_with('{'))
                    .unwrap_or_default()
                    .to_string();
                out.push((method, path, json));
            }
        }
        out
    }

    fn path(id: &str, verb: &str) -> String {
        format!("/gmail/v1/users/me/threads/{id}/{verb}")
    }

    #[test]
    fn each_action_maps_to_its_gmail_call() {
        let t = || vec!["t".to_string()];
        let one = |op: &MailOp| -> Vec<(String, String)> {
            requests(op)
                .into_iter()
                .map(|(_, r)| {
                    (
                        r.path().to_string(),
                        String::from_utf8(r.body).unwrap_or_default(),
                    )
                })
                .collect()
        };
        let modify = |add: &str, remove: &str| {
            (
                path("t", "modify"),
                format!(r#"{{"addLabelIds":[{add}],"removeLabelIds":[{remove}]}}"#),
            )
        };
        let moves = |from, to| -> Vec<(String, String)> {
            MailOp::moves(t(), from, to).iter().flat_map(one).collect()
        };
        // Archive, Move to Inbox (from Trash: untrash first), Spam, Not spam.
        assert_eq!(
            moves(Folder::Inbox, Folder::Archive),
            [modify("", r#""INBOX","SPAM""#)]
        );
        assert_eq!(
            moves(Folder::Trash, Folder::Inbox),
            [
                (path("t", "untrash"), "{}".into()),
                modify(r#""INBOX""#, r#""SPAM""#)
            ]
        );
        assert_eq!(
            moves(Folder::Inbox, Folder::Spam),
            [modify(r#""SPAM""#, r#""INBOX""#)]
        );
        assert_eq!(
            moves(Folder::Spam, Folder::Inbox),
            [modify(r#""INBOX""#, r#""SPAM""#)]
        );
        // Delete.
        assert_eq!(
            moves(Folder::Inbox, Folder::Trash),
            [(path("t", "trash"), "{}".into())]
        );
        // Read, unread, star and labels: the whole thread.
        let flag = |add: &[&str], remove: &[&str]| MailOp::Modify {
            threads: t(),
            add: s(add),
            remove: s(remove),
        };
        assert_eq!(one(&flag(&[], &["UNREAD"])), [modify("", r#""UNREAD""#)]);
        assert_eq!(one(&flag(&["UNREAD"], &[])), [modify(r#""UNREAD""#, "")]);
        assert_eq!(one(&flag(&["STARRED"], &[])), [modify(r#""STARRED""#, "")]);
        assert_eq!(
            one(&flag(&[], &["Label_73"])),
            [modify("", r#""Label_73""#)]
        );
        // One call per thread, each costing 10.
        let bulk = requests(&MailOp::Trash {
            threads: s(&["a", "b"]),
        });
        assert_eq!(bulk.len(), 2);
        assert!(bulk.iter().all(|(_, r)| r.cost == 10 && r.method == "POST"));
    }

    #[test]
    fn one_thread_commits_with_one_call_and_is_stored() {
        let store = inbox(&["t"]);
        let (gmail, fake, _) = gmail(FakeTransport::new().on(
            "POST",
            &path("t", "modify"),
            &[],
            &[(200, r#"{"id":"t"}"#)],
        ));
        let op = MailOp::Modify {
            threads: s(&["t"]),
            add: vec![],
            remove: s(&[gmail::INBOX]),
        };
        assert_eq!(
            commit_threads(&gmail, &store, ACCOUNT, &op),
            Outcome::Committed
        );
        assert_eq!(fake.requests().len(), 1);
        assert_eq!(folder(&store, "t"), Folder::Archive);
    }

    #[test]
    fn batch_partial_failure_rolls_back_only_the_failed_threads() {
        let store = inbox(&["a", "b", "c"]);
        let refused = r#"{"error":{"code":400,"message":"Invalid label."}}"#;
        let fresh = r#"{"id":"b","historyId":"9","messages":[{"id":"m-b","threadId":"b",
            "labelIds":["INBOX"],"internalDate":"1793000000000","payload":{"headers":[]}}]}"#;
        let (gmail, fake, _) = gmail(FakeTransport::new().on(
            "POST",
            BATCH,
            &[],
            &[
                (
                    200,
                    &batch_body(&[(200, r#"{"id":"a"}"#), (400, refused), (200, "{}")]),
                ),
                (200, &batch_body(&[(200, fresh)])),
            ],
        ));
        let op = MailOp::Modify {
            threads: s(&["a", "b", "c"]),
            add: vec![],
            remove: s(&[gmail::INBOX]),
        };
        assert_eq!(
            commit_threads(&gmail, &store, ACCOUNT, &op),
            Outcome::Failed {
                message: "Couldn't archive the conversation: Invalid label".into(),
                changed: s(&["b"]),
                removed: vec![],
            }
        );
        assert_eq!(folder(&store, "a"), Folder::Archive);
        assert_eq!(folder(&store, "b"), Folder::Inbox);
        assert_eq!(folder(&store, "c"), Folder::Archive);
        // The modify batch, then a read of only the refused thread.
        let sent = calls(&fake);
        assert_eq!(sent.len(), 4);
        assert_eq!(sent[3].0, "GET");
        assert_eq!(sent[3].1, "/gmail/v1/users/me/threads/b");
    }

    #[test]
    fn overlay_keeps_an_archived_thread_out_of_the_inbox_during_a_poll() {
        let store = inbox(&["t", "u"]);
        let outbox = Outbox::new(store.clone(), ACCOUNT.into());
        let ops = MailOp::moves(s(&["t"]), Folder::Inbox, Folder::Archive);
        let id = outbox.push(Op::Mail(ops[0].clone()), 5_000, 0).unwrap();
        // A poll inside the undo window stores the server's copy again.
        let server = store.load_all_threads(ACCOUNT).unwrap();
        store.put_synced_threads(ACCOUNT, server).unwrap();
        let mut threads = store.load_all_threads(ACCOUNT).unwrap();
        outbox.overlay(&mut threads);
        let of = |threads: &[Thread], id: &str| threads.iter().find(|t| t.id == id).unwrap().folder;
        assert_eq!(of(&threads, "t"), Folder::Archive);
        assert_eq!(of(&threads, "u"), Folder::Inbox);
        assert!(!threads[0].label_ids.contains(&gmail::INBOX.to_string()));
        // Undone: the server's state shows again.
        assert!(outbox.cancel(id));
        let mut threads = store.load_all_threads(ACCOUNT).unwrap();
        outbox.overlay(&mut threads);
        assert_eq!(of(&threads, "t"), Folder::Inbox);
    }

    #[test]
    fn archived_self_mail_lands_in_archive_and_sent() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_sync_state(
                ACCOUNT,
                crate::backend::store::ALIASES_KEY,
                r#"["alias@example.com"]"#,
            )
            .unwrap();
        let me = |email: &str| crate::data::Contact {
            name: String::new(),
            email: email.into(),
        };
        // To themselves, to an alias, and only to someone else.
        let threads: Vec<Thread> = [
            ("self", ACCOUNT),
            ("alias", "alias@example.com"),
            ("other", "ann@x.com"),
        ]
        .into_iter()
        .map(|(id, to)| {
            let mut thread = Thread::local(
                id.into(),
                id.into(),
                Folder::Inbox,
                vec![Message {
                    id: format!("m-{id}"),
                    label_ids: s(&[gmail::SENT, gmail::INBOX]),
                    to: vec![me(to)],
                    ..Message::new(me(ACCOUNT))
                }],
                None,
            );
            thread.label_ids = s(&[gmail::SENT, gmail::INBOX]);
            thread
        })
        .collect();
        store.put_threads(ACCOUNT, &threads).unwrap();
        let ids = s(&["self", "alias", "other"]);

        // Waiting out the undo window: the overlay already has it archived.
        let outbox = Outbox::new(store.clone(), ACCOUNT.into());
        let ops = MailOp::moves(ids.clone(), Folder::Inbox, Folder::Archive);
        outbox.push(Op::Mail(ops[0].clone()), 5_000, 0).unwrap();
        let mut overlaid = store.load_all_threads(ACCOUNT).unwrap();
        outbox.overlay(&mut overlaid);
        let of = |threads: &[Thread], id: &str| threads.iter().find(|t| t.id == id).unwrap().folder;
        assert_eq!(of(&overlaid, "self"), Folder::Archive);
        assert_eq!(of(&overlaid, "alias"), Folder::Archive);
        assert_eq!(of(&overlaid, "other"), Folder::Sent);

        // Committed to the store.
        store.apply_mail_op(ACCOUNT, &ops[0]).unwrap();
        assert_eq!(folder(&store, "self"), Folder::Archive);
        assert_eq!(folder(&store, "alias"), Folder::Archive);
        assert_eq!(folder(&store, "other"), Folder::Sent);
        let threads = store.load_all_threads(ACCOUNT).unwrap();
        assert!(threads.iter().all(Thread::in_sent));

        // Back to the Inbox (Undo past the window, or Move to Inbox).
        for op in MailOp::moves(ids, Folder::Archive, Folder::Inbox) {
            store.apply_mail_op(ACCOUNT, &op).unwrap();
        }
        assert_eq!(folder(&store, "self"), Folder::Inbox);
        assert_eq!(folder(&store, "alias"), Folder::Inbox);
    }

    #[test]
    fn undo_before_commit_sends_nothing() {
        let store = inbox(&["t"]);
        let outbox = Outbox::new(store.clone(), ACCOUNT.into());
        let (gmail, fake, _) = gmail(FakeTransport::new());
        let id = outbox
            .push(Op::Mail(MailOp::Trash { threads: s(&["t"]) }), 5_000, 0)
            .unwrap();
        assert!(outbox.cancel(id));
        for (_, op) in outbox.take_due(60_000).unwrap() {
            let Op::Mail(op) = op else { unreachable!() };
            commit_threads(&gmail, &store, ACCOUNT, &op);
        }
        assert!(fake.requests().is_empty());
        assert_eq!(outbox.next_due(), None);
    }

    #[test]
    fn undo_after_commit_sends_the_inverse() {
        let store = inbox(&["t"]);
        let outbox = Outbox::new(store.clone(), ACCOUNT.into());
        let (gmail, fake, _) = gmail(
            FakeTransport::new()
                .on("POST", &path("t", "trash"), &[], &[(200, "{}")])
                .on("POST", &path("t", "untrash"), &[], &[(200, "{}")])
                .on("POST", &path("t", "modify"), &[], &[(200, "{}")]),
        );
        let commit_due = |now| {
            for (id, op) in outbox.take_due(now).unwrap() {
                let Op::Mail(op) = op else { unreachable!() };
                assert_eq!(
                    commit_threads(&gmail, &store, ACCOUNT, &op),
                    Outcome::Committed
                );
                outbox.finish(id).unwrap();
            }
        };
        let id = outbox
            .push(Op::Mail(MailOp::Trash { threads: s(&["t"]) }), 5_000, 0)
            .unwrap();
        commit_due(5_000);
        assert_eq!(folder(&store, "t"), Folder::Trash);
        // Too late to take back: the inverse goes at once, in order.
        assert!(!outbox.cancel(id));
        for op in MailOp::moves(s(&["t"]), Folder::Trash, Folder::Inbox) {
            outbox.push(Op::Mail(op), 0, 6_000).unwrap();
        }
        commit_due(6_000);
        commit_due(6_000);
        let sent: Vec<String> = calls(&fake).into_iter().map(|(_, p, _)| p).collect();
        assert_eq!(
            sent,
            [
                path("t", "trash"),
                path("t", "untrash"),
                path("t", "modify")
            ]
        );
        assert_eq!(folder(&store, "t"), Folder::Inbox);
    }

    #[test]
    fn backoff_then_failure_rolls_back() {
        let store = inbox(&["a", "b"]);
        let busy = batch_body(&[(503, "{}"), (503, "{}")]);
        let (gmail, fake, clock) =
            gmail(FakeTransport::new().on("POST", BATCH, &[], &[(200, &busy)]));
        let op = MailOp::Trash {
            threads: s(&["a", "b"]),
        };
        let Outcome::Failed {
            message, changed, ..
        } = commit_threads(&gmail, &store, ACCOUNT, &op)
        else {
            panic!("should fail");
        };
        assert_eq!(
            message,
            "Couldn't delete 2 conversations: Gmail answered 503"
        );
        // Gmail couldn't be read either: the store's copy is the rollback.
        assert_eq!(changed, s(&["a", "b"]));
        assert_eq!(folder(&store, "a"), Folder::Inbox);
        // Backed off between tries, then gave up.
        assert!(clock.sleeps().len() >= (crate::backend::http::MAX_ATTEMPTS - 1) as usize);
        assert!(fake.requests().len() >= crate::backend::http::MAX_ATTEMPTS as usize);
    }

    #[test]
    fn offline_keeps_the_op_queued() {
        let store = inbox(&["t"]);
        let outbox = Outbox::new(store.clone(), ACCOUNT.into());
        let (gmail, _, _) =
            gmail(FakeTransport::new().on("POST", &path("t", "modify"), &[], &[(0, "")]));
        let op = MailOp::Modify {
            threads: s(&["t"]),
            add: s(&[gmail::STARRED]),
            remove: vec![],
        };
        let id = outbox.push(Op::Mail(op.clone()), 0, 0).unwrap();
        assert_eq!(outbox.take_due(0).unwrap().len(), 1);
        let outcome = commit_threads(&gmail, &store, ACCOUNT, &op);
        assert!(matches!(
            outcome,
            Outcome::Later(TransportError::Network(_))
        ));
        outbox.retry_later(id, RETRY_LATER_MS, "offline").unwrap();
        assert_eq!(outbox.waiting(), 1);
        assert_eq!(outbox.next_due(), Some(RETRY_LATER_MS));
        // Still shown starred while it waits.
        let mut threads = store.load_all_threads(ACCOUNT).unwrap();
        outbox.overlay(&mut threads);
        assert!(threads[0].starred);
        assert!(!store.load_all_threads(ACCOUNT).unwrap()[0].starred);
    }

    #[test]
    fn rows_in_flight_at_quit_go_again_on_launch() {
        let store = Store::open_in_memory().unwrap();
        let outbox = Outbox::new(store.clone(), ACCOUNT.into());
        outbox.push(modify(&["t1"], &[], &["INBOX"]), 0, 0).unwrap();
        assert_eq!(outbox.take_due(0).unwrap().len(), 1);
        assert_eq!(outbox.next_due(), None);
        let relaunched = Outbox::new(store, ACCOUNT.into());
        assert_eq!(relaunched.take_due(0).unwrap().len(), 1);
    }

    #[test]
    fn a_bulk_op_holds_back_later_ops_on_any_of_its_threads() {
        let outbox = outbox();
        outbox
            .push(modify(&["a", "b"], &[], &["INBOX"]), 5_000, 0)
            .unwrap();
        outbox
            .push(modify(&["b"], &["STARRED"], &[]), 0, 0)
            .unwrap();
        outbox
            .push(modify(&["c"], &["STARRED"], &[]), 0, 0)
            .unwrap();
        // Only c can go; the earliest time anything else can is the archive's.
        assert_eq!(outbox.take_due(0).unwrap().len(), 1);
        assert_eq!(outbox.next_due(), Some(5_000));
    }
}

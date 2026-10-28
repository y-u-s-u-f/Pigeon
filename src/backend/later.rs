//! Snooze (ACT-11) and Send Later (CMP-17). Gmail has neither, so both are
//! Pigeon's own:
//!
//! - Snoozing takes the conversation out of the Inbox and puts Pigeon's
//!   "Pigeon/Snoozed" label on it (made in Gmail, hidden from its label
//!   list, the first time). The wake time lives in the store (`snoozes`).
//!   At that time, or at the next launch or wake from sleep after it, the
//!   label goes and the conversation comes back to the Inbox, unread, and
//!   sorts as if it arrived then (here only: Gmail and other clients show
//!   it at its own date). New mail from someone else in it wakes it early.
//! - A scheduled send is an outbox row (`Op::Later`) due at its time, so
//!   it survives a quit and goes at the next launch when Pigeon wasn't
//!   running. Its draft stays saved (and in Gmail's Drafts) until it goes;
//!   the app shows it in Scheduled rather than Drafts. A draft deleted in
//!   another client meanwhile isn't sent.
//!
//! Timers wait at most `RECHECK`, so a clock changed by hand is seen.

use std::{collections::HashMap, time::Duration};

use gpui::Context;

use super::{
    Backend, BackendEvent, MailOp, Op, OpId, Source,
    gmail::{self, Gmail, GmailError, LabelList},
    now_ms,
    store::{Snooze, Store},
};
use crate::data::{Account, Folder, Thread};

/// The label a snoozed conversation carries: its name in Gmail, and its id
/// in ops until they commit (`resolve_snoozed`).
pub const SNOOZED: &str = "Pigeon/Snoozed";
/// The outbox kind of a scheduled send.
pub const SEND_LATER_KIND: &str = "mail.send_later";
/// A wake or scheduled send this far past its time was caught up: Pigeon
/// wasn't running, the Mac slept or was offline.
const LATE_MS: i64 = 60_000;
/// The longest a timer waits before it looks at the wall clock again.
pub(super) const RECHECK: Duration = Duration::from_secs(15 * 60);
/// A snooze this new isn't checked against what a sync stored: the sync
/// may have read Gmail before the snooze went.
const SETTLE_MS: i64 = 2 * 60_000;
/// Why a scheduled send whose draft went meanwhile wasn't sent.
pub const DRAFT_DELETED: &str = "Not sent: the draft was deleted";
/// How long a woken conversation keeps its place by its wake time.
const KEEP_WOKEN_MS: i64 = 30 * 24 * 60 * 60 * 1000;

// Snooze: the labels.

/// The Snoozed label's Gmail id: from the synced labels, else Gmail's list,
/// else made now.
fn snoozed_label_id(gmail: &Gmail, store: &Store, account: &str) -> Result<String, GmailError> {
    if let Some(label) = store
        .load_labels(account)
        .unwrap_or_default()
        .into_iter()
        .find(|l| l.name == SNOOZED)
    {
        return Ok(label.id);
    }
    let list: LabelList = gmail.call(&gmail::labels_list_request())?;
    let label = match list.labels.into_iter().find(|l| l.name == SNOOZED) {
        Some(label) => label,
        None => gmail.call(&gmail::hidden_label_create_request(SNOOZED))?,
    };
    // Known from now on, without asking Gmail again.
    if let Err(e) = store.put_gmail_label(account, &label) {
        eprintln!("Pigeon: couldn't store the Snoozed label: {e:#}");
    }
    Ok(label.id)
}

/// The ids the Snoozed label goes by in the store: its Gmail id once
/// known, and the name demo mode uses.
fn snoozed_ids(store: &Store, account: &str) -> Vec<String> {
    let mut ids = vec![SNOOZED.to_string()];
    ids.extend(
        store
            .load_labels(account)
            .unwrap_or_default()
            .into_iter()
            .filter(|l| l.name == SNOOZED)
            .map(|l| l.id),
    );
    ids
}

fn mentions_snoozed(op: &MailOp) -> bool {
    match op {
        MailOp::Modify { add, remove, .. } => add.iter().chain(remove).any(|l| l == SNOOZED),
        _ => false,
    }
}

/// `op` with the Snoozed label by its Gmail id, looked up (or made) only
/// when the op has it.
pub fn resolve_snoozed(
    gmail: &Gmail,
    store: &Store,
    account: &str,
    op: &MailOp,
) -> Result<MailOp, GmailError> {
    if !mentions_snoozed(op) {
        return Ok(op.clone());
    }
    let id = snoozed_label_id(gmail, store, account)?;
    let swap = |labels: &[String]| -> Vec<String> {
        labels
            .iter()
            .map(|l| if l == SNOOZED { id.clone() } else { l.clone() })
            .collect()
    };
    Ok(match op {
        MailOp::Modify {
            threads,
            add,
            remove,
        } => MailOp::Modify {
            threads: threads.clone(),
            add: swap(add),
            remove: swap(remove),
        },
        other => other.clone(),
    })
}

/// A conversation's snooze set to `to` (`None`: awake). Woken with `inbox`,
/// it goes back to the Inbox; else only the label goes (an undo of a
/// snooze made outside the Inbox).
#[derive(Clone, Debug, PartialEq)]
pub struct SnoozeChange {
    pub thread: String,
    pub to: Option<i64>,
    pub inbox: bool,
}

fn labels(l: &[&str]) -> Vec<String> {
    l.iter().map(|s| s.to_string()).collect()
}

/// The ops that put threads to sleep and wake them: out of the Inbox with
/// the label, or back with it gone.
fn snooze_ops(sleep: Vec<String>, wake_inbox: Vec<String>, wake_quiet: Vec<String>) -> Vec<MailOp> {
    [
        (sleep, labels(&[SNOOZED]), labels(&[gmail::INBOX])),
        (wake_inbox, labels(&[gmail::INBOX]), labels(&[SNOOZED])),
        (wake_quiet, Vec::new(), labels(&[SNOOZED])),
    ]
    .into_iter()
    .filter(|(threads, ..)| !threads.is_empty())
    .map(|(threads, add, remove)| MailOp::Modify {
        threads,
        add,
        remove,
    })
    .collect()
}

/// The snoozes due at `now`, oldest first, each with whether it's late.
fn due(snoozes: &[Snooze], now: i64) -> Vec<(String, bool)> {
    let mut due: Vec<(&Snooze, i64)> = snoozes
        .iter()
        .filter_map(|s| Some((s, s.wake_at.filter(|at| *at <= now)?)))
        .collect();
    due.sort_by_key(|(s, at)| (*at, s.thread_id.clone()));
    due.into_iter()
        .map(|(s, at)| (s.thread_id.clone(), now - at > LATE_MS))
        .collect()
}

fn next_wake(snoozes: &[Snooze]) -> Option<i64> {
    snoozes.iter().filter_map(|s| s.wake_at).min()
}

/// Snoozed conversations with mail from someone else since they were
/// snoozed: they wake now, like Gmail's.
fn woken_by_reply(snoozes: &[Snooze], threads: &[Thread], me: &Account) -> Vec<String> {
    snoozes
        .iter()
        .filter(|s| s.wake_at.is_some())
        .filter(|s| {
            threads.iter().any(|t| {
                t.id == s.thread_id
                    && t.messages
                        .iter()
                        .any(|m| m.internal_date > s.snoozed_at && !me.is_me(&m.from.email))
            })
        })
        .map(|s| s.thread_id.clone())
        .collect()
}

/// What became of snoozes Pigeon didn't change: (rows to drop, threads
/// still carrying the label to take it off). Moved back to the Inbox or
/// unlabeled in another client, or by a snooze that failed, they're
/// awake. `label` holds the label's ids, `None` when its Gmail id isn't
/// known yet (only the Inbox tells then); `queued` are threads with ops
/// still to go.
fn stale(
    snoozes: &[Snooze],
    threads: &[Thread],
    label: Option<&[String]>,
    queued: &[String],
    now: i64,
) -> (Vec<String>, Vec<String>) {
    let (mut drop, mut unlabel) = (Vec::new(), Vec::new());
    for s in snoozes.iter().filter(|s| s.wake_at.is_some()) {
        if now - s.snoozed_at < SETTLE_MS || queued.contains(&s.thread_id) {
            continue;
        }
        let Some(t) = threads.iter().find(|t| t.id == s.thread_id) else {
            continue;
        };
        let labeled = label.is_none_or(|ids| t.label_ids.iter().any(|l| ids.contains(l)));
        if t.folder == Folder::Inbox || !labeled {
            drop.push(s.thread_id.clone());
            if labeled {
                unlabel.push(s.thread_id.clone());
            }
        }
    }
    (drop, unlabel)
}

// Send Later.

/// Whether `op`'s draft was deleted since it was scheduled: saved (as
/// `schedule_send` does first), then gone from the store, as the drafts
/// sync leaves one deleted in another client.
pub(super) fn draft_deleted(store: &Store, account: &str, op: &MailOp) -> bool {
    match op {
        MailOp::Send {
            local_id: Some(id), ..
        } => matches!(store.stored_draft(account, id), Ok(None)),
        _ => false,
    }
}

/// A scheduled send that went more than a minute after its time.
#[derive(Clone, Debug, PartialEq)]
pub struct LateSend {
    pub subject: String,
    /// When it was due and when it went, Unix ms.
    pub due: i64,
    pub sent: i64,
}

/// A send waiting for its time.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledSend {
    pub op: OpId,
    /// The draft it's sent from.
    pub local_id: String,
    pub at: i64,
}

impl Backend {
    // Snooze.

    /// Every snooze by thread, asleep or woken.
    pub fn snoozes(&self) -> HashMap<String, Snooze> {
        self.store
            .snoozes(self.email())
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.thread_id.clone(), s))
            .collect()
    }

    /// Put the stored snoozes on `threads`.
    pub(super) fn overlay_snoozes(&self, threads: &mut [Thread]) {
        let snoozes = self.snoozes();
        if snoozes.is_empty() {
            return;
        }
        for thread in threads {
            if let Some(s) = snoozes.get(&thread.id) {
                thread.later.snoozed_until = s.wake_at;
                thread.later.woke_at = s.woke_at;
            }
        }
    }

    /// Snooze, re-snooze or unsnooze. Taking threads out of the Inbox or
    /// back waits out the undo window unless `at_once`.
    pub fn set_snoozes(
        &mut self,
        changes: Vec<SnoozeChange>,
        at_once: bool,
        cx: &mut Context<Self>,
    ) -> Vec<OpId> {
        let now = now_ms();
        let email = self.email().to_string();
        let rows = self.snoozes();
        let (mut sleep, mut wake_inbox, mut wake_quiet) = (Vec::new(), Vec::new(), Vec::new());
        for change in changes {
            let row = rows.get(&change.thread).filter(|r| r.wake_at.is_some());
            let written = match change.to {
                Some(at) => {
                    if row.is_none() {
                        sleep.push(change.thread.clone());
                    }
                    self.store.put_snooze(
                        &email,
                        &Snooze {
                            thread_id: change.thread,
                            wake_at: Some(at),
                            snoozed_at: row.map_or(now, |r| r.snoozed_at),
                            woke_at: None,
                        },
                    )
                }
                None => {
                    if row.is_some() {
                        if change.inbox {
                            wake_inbox.push(change.thread.clone());
                        } else {
                            wake_quiet.push(change.thread.clone());
                        }
                    }
                    self.store.delete_snooze(&email, &change.thread)
                }
            };
            if let Err(e) = written {
                eprintln!("Pigeon: couldn't store a snooze: {e:#}");
            }
        }
        let ops = snooze_ops(sleep, wake_inbox, wake_quiet)
            .into_iter()
            .map(|op| {
                if at_once {
                    self.mail_now(op, cx)
                } else {
                    self.mail(op, cx)
                }
            })
            .collect();
        self.schedule_wakes(cx);
        ops
    }

    /// Undo inside the undo window: the ops never go, and the rows are put
    /// back as `rows` had them. False once one has gone; nothing changes
    /// then, and the caller sets the snoozes back with ops instead.
    pub fn cancel_snoozes(
        &mut self,
        ops: &[OpId],
        rows: &[(String, Option<Snooze>)],
        cx: &mut Context<Self>,
    ) -> bool {
        let cancelled: Vec<bool> = ops.iter().map(|&op| self.cancel(op)).collect();
        if !cancelled.into_iter().all(|c| c) {
            return false;
        }
        let email = self.email().to_string();
        for (thread, row) in rows {
            let written = match row {
                Some(row) => self.store.put_snooze(&email, row),
                None => self.store.delete_snooze(&email, thread),
            };
            if let Err(e) = written {
                eprintln!("Pigeon: couldn't store a snooze: {e:#}");
            }
        }
        self.schedule_wakes(cx);
        true
    }

    /// At launch: woken rows long past go, and what came due while Pigeon
    /// wasn't running wakes (just after, so the app hears of it).
    pub(super) fn start_snoozes(&mut self, cx: &mut Context<Self>) {
        let before = now_ms() - KEEP_WOKEN_MS;
        if let Err(e) = self.store.prune_woken(self.email(), before) {
            eprintln!("Pigeon: couldn't prune snoozes: {e:#}");
        }
        self.schedule_wakes(cx);
    }

    /// Wake when the next snooze is due.
    pub(super) fn schedule_wakes(&mut self, cx: &mut Context<Self>) {
        let Some(at) = next_wake(&self.store.snoozes(self.email()).unwrap_or_default()) else {
            self._wake = None;
            return;
        };
        let wait = Duration::from_millis((at - now_ms()).max(0) as u64).min(RECHECK);
        self._wake = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| this.wake_due(cx)).ok();
        }));
    }

    fn wake_due(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let due = due(&self.store.snoozes(self.email()).unwrap_or_default(), now);
        if !due.is_empty() {
            let late = due
                .iter()
                .filter(|(_, late)| *late)
                .map(|(id, _)| id.clone())
                .collect();
            let threads: Vec<String> = due.into_iter().map(|(id, _)| id).collect();
            crate::logging::info(&format!("snooze: {} woke", threads.len()));
            self.wake_threads(threads, true, now, late, cx);
        }
        self.schedule_wakes(cx);
    }

    /// Back to the Inbox at once, unread unless new mail woke them. Gone,
    /// spam or deleted ones only stop being snoozed.
    fn wake_threads(
        &mut self,
        ids: Vec<String>,
        unread: bool,
        now: i64,
        late: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let email = self.email().to_string();
        let rows = self.snoozes();
        let back: Vec<String> = self
            .load_threads(&ids)
            .into_iter()
            .filter(|t| !matches!(t.folder, Folder::Spam | Folder::Trash))
            .map(|t| t.id)
            .collect();
        for id in &ids {
            let row = Snooze {
                thread_id: id.clone(),
                wake_at: None,
                snoozed_at: rows.get(id).map_or(now, |r| r.snoozed_at),
                woke_at: back.contains(id).then_some(now),
            };
            let written = if back.contains(id) {
                self.store.put_snooze(&email, &row)
            } else {
                self.store.delete_snooze(&email, id)
            };
            if let Err(e) = written {
                eprintln!("Pigeon: couldn't store a snooze: {e:#}");
            }
        }
        if !back.is_empty() {
            let add = if unread {
                labels(&[gmail::INBOX, gmail::UNREAD])
            } else {
                labels(&[gmail::INBOX])
            };
            self.mail_now(
                MailOp::Modify {
                    threads: back.clone(),
                    add,
                    remove: labels(&[SNOOZED]),
                },
                cx,
            );
        }
        cx.emit(BackendEvent::ThreadsChanged(ids));
        if !back.is_empty() {
            let late = late.into_iter().filter(|id| back.contains(id)).collect();
            cx.emit(BackendEvent::Woke {
                threads: back,
                late,
            });
        }
    }

    /// A sync stored `changed`: snoozed ones with new mail from someone
    /// else wake now, and ones another client woke stop being snoozed.
    pub(super) fn wake_on_reply(&mut self, changed: &[String], cx: &mut Context<Self>) {
        let snoozes: Vec<Snooze> = self
            .store
            .snoozes(self.email())
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.wake_at.is_some() && changed.contains(&s.thread_id))
            .collect();
        if snoozes.is_empty() {
            return;
        }
        let ids: Vec<String> = snoozes.iter().map(|s| s.thread_id.clone()).collect();
        let threads = self
            .store
            .load_threads(self.email(), &ids)
            .unwrap_or_default();
        let woken = woken_by_reply(&snoozes, &threads, &self.store.me(self.email()));
        if !woken.is_empty() {
            crate::logging::info(&format!("snooze: {} woke on new mail", woken.len()));
            self.wake_threads(woken.clone(), false, now_ms(), Vec::new(), cx);
        }
        let rest: Vec<Snooze> = snoozes
            .into_iter()
            .filter(|s| !woken.contains(&s.thread_id))
            .collect();
        let mut threads = threads;
        self.outbox.overlay(&mut threads);
        let label = snoozed_ids(&self.store, self.email());
        let known = self.source == Source::Demo || label.len() > 1;
        let (drop, unlabel) = stale(
            &rest,
            &threads,
            known.then_some(label.as_slice()),
            &self.outbox.queued_threads(),
            now_ms(),
        );
        if !drop.is_empty() {
            crate::logging::info(&format!("snooze: {} woken elsewhere", drop.len()));
            self.forget_snoozes(&drop, cx);
        }
        if !unlabel.is_empty() {
            self.mail_now(
                MailOp::Modify {
                    threads: unlabel,
                    add: Vec::new(),
                    remove: labels(&[SNOOZED]),
                },
                cx,
            );
        }
        if !woken.is_empty() || !drop.is_empty() {
            self.schedule_wakes(cx);
        }
    }

    /// These stop being snoozed, with no op: Gmail has them awake already,
    /// or their snooze failed.
    pub(super) fn forget_snoozes(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let email = self.email().to_string();
        for id in ids {
            if let Err(e) = self.store.delete_snooze(&email, id) {
                eprintln!("Pigeon: couldn't store a snooze: {e:#}");
            }
        }
        cx.emit(BackendEvent::ThreadsChanged(ids.to_vec()));
    }

    // Send Later.

    /// Send `op` (a `MailOp::Send` from a saved draft) at `at`, Unix ms.
    pub fn send_later(&mut self, op: MailOp, at: i64, cx: &mut Context<Self>) -> OpId {
        if let MailOp::Send {
            draft,
            local_id: Some(id),
            ..
        } = &op
        {
            self.note_recipients(draft, cx);
            if let Err(e) = self.outbox.hurry(id, now_ms()) {
                eprintln!("Pigeon: outbox: {e:#}");
            }
        }
        let delay = (at - now_ms()).max(0);
        self.push(Op::Later { at, op }, delay, cx)
    }

    /// The sends waiting for their time, soonest first.
    pub fn scheduled_sends(&self) -> Vec<ScheduledSend> {
        let mut sends: Vec<ScheduledSend> = self
            .store
            .outbox_pending(self.email())
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.kind == SEND_LATER_KIND && r.state == "pending")
            .filter_map(|r| match serde_json::from_str::<Op>(&r.payload) {
                Ok(Op::Later {
                    at,
                    op:
                        MailOp::Send {
                            local_id: Some(local_id),
                            ..
                        },
                }) => Some(ScheduledSend {
                    op: r.id,
                    local_id,
                    at,
                }),
                _ => None,
            })
            .collect();
        sends.sort_by_key(|s| s.at);
        sends
    }

    /// A scheduled send went: its draft is gone, the message is in its
    /// conversation, and one that went more than a minute late is noted
    /// for the notice.
    pub(super) fn scheduled_sent(
        &mut self,
        at: i64,
        op: &MailOp,
        late: &mut Vec<LateSend>,
        cx: &mut Context<Self>,
    ) {
        let MailOp::Send {
            draft,
            local_id,
            thread_id,
        } = op
        else {
            return;
        };
        if let Some(local_id) = local_id {
            cx.emit(BackendEvent::ThreadsRemoved(vec![local_id.clone()]));
        }
        cx.emit(BackendEvent::ThreadsChanged(vec![thread_id.clone()]));
        let now = now_ms();
        if now - at > LATE_MS {
            late.push(LateSend {
                subject: draft.subject.clone(),
                due: at,
                sent: now,
            });
        }
    }

    /// A scheduled send failed for good: the app is told, with whether its
    /// draft is back in Drafts.
    pub(super) fn scheduled_failed(
        &mut self,
        id: OpId,
        op: &MailOp,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        let MailOp::Send {
            draft, local_id, ..
        } = op
        else {
            return;
        };
        let in_drafts = local_id.as_ref().is_some_and(|l| {
            !self
                .store
                .load_threads(self.email(), std::slice::from_ref(l))
                .unwrap_or_default()
                .is_empty()
        });
        cx.emit(BackendEvent::ScheduledFailed {
            op: id,
            subject: draft.subject.clone(),
            message: message.to_string(),
            in_drafts,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        Outgoing,
        http::{FakeClock, FakeTransport, Transport},
        outbox::{Outbox, Outcome, commit_threads},
    };
    use crate::data::{Contact, Message};
    use std::sync::Arc;

    const ACCOUNT: &str = "me@example.com";

    fn snooze(id: &str, wake_at: Option<i64>, snoozed_at: i64) -> Snooze {
        Snooze {
            thread_id: id.into(),
            wake_at,
            snoozed_at,
            woke_at: None,
        }
    }

    #[test]
    fn due_snoozes_wake_in_order_and_late_ones_are_caught_up() {
        let rows = [
            snooze("later", Some(10 * LATE_MS), 0),
            snooze("now", Some(5 * LATE_MS), 0),
            snooze("missed", Some(LATE_MS), 0),
            snooze("woken", None, 0),
        ];
        assert_eq!(next_wake(&rows), Some(LATE_MS));
        assert!(due(&rows, LATE_MS - 1).is_empty());
        // On time (the timer), then at a launch long after.
        assert_eq!(due(&rows, LATE_MS), [("missed".to_string(), false)]);
        assert_eq!(
            due(&rows, 5 * LATE_MS),
            [("missed".to_string(), true), ("now".to_string(), false)]
        );
        assert_eq!(due(&rows, 100 * LATE_MS).len(), 3);
        assert_eq!(next_wake(&rows[3..]), None);
    }

    #[test]
    fn new_mail_from_someone_else_wakes_early() {
        let me = Account {
            email: ACCOUNT.into(),
            ..Default::default()
        };
        let thread = |id: &str, from: &str, at: i64| {
            let mut thread = Thread::local(
                id.into(),
                id.into(),
                Folder::Archive,
                vec![Message {
                    internal_date: at,
                    ..Message::new(Contact {
                        name: String::new(),
                        email: from.into(),
                    })
                }],
                None,
            );
            thread.label_ids = labels(&[SNOOZED]);
            thread
        };
        let rows = [
            snooze("reply", Some(1_000), 100),
            snooze("mine", Some(1_000), 100),
            snooze("old", Some(1_000), 100),
            snooze("woken", None, 100),
        ];
        let threads = [
            thread("reply", "ann@x.com", 200),
            thread("mine", ACCOUNT, 200),
            thread("old", "ann@x.com", 50),
            thread("woken", "ann@x.com", 200),
        ];
        assert_eq!(woken_by_reply(&rows, &threads, &me), ["reply"]);
    }

    #[test]
    fn snooze_ops_take_threads_out_and_bring_them_back() {
        let s = |v: &[&str]| labels(v);
        assert_eq!(
            snooze_ops(s(&["a"]), s(&["b"]), s(&["c"])),
            [
                MailOp::Modify {
                    threads: s(&["a"]),
                    add: s(&[SNOOZED]),
                    remove: s(&[gmail::INBOX]),
                },
                MailOp::Modify {
                    threads: s(&["b"]),
                    add: s(&[gmail::INBOX]),
                    remove: s(&[SNOOZED]),
                },
                MailOp::Modify {
                    threads: s(&["c"]),
                    add: vec![],
                    remove: s(&[SNOOZED]),
                },
            ]
        );
        assert!(snooze_ops(vec![], vec![], vec![]).is_empty());
        // Out of the Inbox: it waits out the undo window.
        assert!(snooze_ops(s(&["a"]), vec![], vec![])[0].undoable());
    }

    #[test]
    fn snoozes_woken_in_another_client_stop() {
        let now = 10 * SETTLE_MS;
        let thread = |id: &str, folder: Folder, labels_on: &[&str]| {
            let mut thread = Thread::local(id.into(), id.into(), folder, Vec::new(), None);
            thread.label_ids = labels(labels_on);
            thread
        };
        let rows = [
            snooze("moved", Some(now * 2), 0),
            snooze("unlabeled", Some(now * 2), 0),
            snooze("asleep", Some(now * 2), 0),
            snooze("queued", Some(now * 2), 0),
            snooze("new", Some(now * 2), now - 1_000),
            snooze("woken", None, 0),
        ];
        let threads = [
            thread("moved", Folder::Inbox, &[gmail::INBOX, "Label_9"]),
            thread("unlabeled", Folder::Archive, &[]),
            thread("asleep", Folder::Archive, &["Label_9"]),
            thread("queued", Folder::Inbox, &[gmail::INBOX, "Label_9"]),
            thread("new", Folder::Inbox, &[gmail::INBOX]),
            thread("woken", Folder::Inbox, &[gmail::INBOX]),
        ];
        let label = labels(&[SNOOZED, "Label_9"]);
        let queued = labels(&["queued"]);
        // Moved to the Inbox still labeled: the label comes off too.
        assert_eq!(
            stale(&rows, &threads, Some(&label), &queued, now),
            (labels(&["moved", "unlabeled"]), labels(&["moved"]))
        );
        // The label's Gmail id not known yet: only the Inbox tells.
        assert_eq!(
            stale(&rows, &threads, None, &queued, now),
            (labels(&["moved"]), labels(&["moved"]))
        );
    }

    fn inbox_store(id: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        let mut thread = Thread::local(
            id.into(),
            id.into(),
            Folder::Inbox,
            vec![Message {
                id: format!("m-{id}"),
                label_ids: labels(&[gmail::INBOX]),
                ..Message::new(Default::default())
            }],
            None,
        );
        thread.label_ids = labels(&[gmail::INBOX]);
        store.put_threads(ACCOUNT, &[thread]).unwrap();
        store
    }

    fn gmail_over(fake: FakeTransport) -> (Gmail, Arc<FakeTransport>) {
        let fake = Arc::new(fake);
        let transport: Arc<dyn Transport> = fake.clone();
        (
            Gmail::with_clock(transport, Arc::new(FakeClock::new())),
            fake,
        )
    }

    const LABELS: &str = "/gmail/v1/users/me/labels";
    const MODIFY: &str = "/gmail/v1/users/me/threads/t/modify";

    #[test]
    fn the_first_snooze_makes_the_hidden_label_then_uses_it() {
        let store = inbox_store("t");
        let (gmail, fake) = gmail_over(
            FakeTransport::new()
                .on("GET", LABELS, &[], &[(200, r#"{"labels":[]}"#)])
                .on(
                    "POST",
                    LABELS,
                    &[],
                    &[(200, r#"{"id":"Label_9","name":"Pigeon/Snoozed"}"#)],
                )
                .on("POST", MODIFY, &[], &[(200, "{}")]),
        );
        let op = snooze_ops(labels(&["t"]), vec![], vec![]).remove(0);
        let resolved = resolve_snoozed(&gmail, &store, ACCOUNT, &op).unwrap();
        assert_eq!(
            resolved,
            MailOp::Modify {
                threads: labels(&["t"]),
                add: labels(&["Label_9"]),
                remove: labels(&[gmail::INBOX]),
            }
        );
        assert_eq!(
            commit_threads(&gmail, &store, ACCOUNT, &resolved),
            Outcome::Committed
        );
        let sent: Vec<(String, String, String)> = fake
            .requests()
            .iter()
            .map(|r| {
                (
                    r.method.to_string(),
                    r.path().to_string(),
                    String::from_utf8_lossy(&r.body).to_string(),
                )
            })
            .collect();
        assert_eq!(sent[0].0, "GET");
        assert_eq!((sent[1].0.as_str(), sent[1].1.as_str()), ("POST", LABELS));
        assert!(sent[1].2.contains(r#""labelListVisibility":"labelHide""#));
        assert_eq!(
            sent[2].2,
            r#"{"addLabelIds":["Label_9"],"removeLabelIds":["INBOX"]}"#
        );
        let thread = &store.load_threads(ACCOUNT, &labels(&["t"])).unwrap()[0];
        assert_eq!(thread.folder, Folder::Archive);
        assert!(thread.label_ids.contains(&"Label_9".to_string()));
    }

    #[test]
    fn a_label_gmail_has_already_is_found_not_made() {
        let store = inbox_store("t");
        let (gmail, fake) = gmail_over(FakeTransport::new().on(
            "GET",
            LABELS,
            &[],
            &[(
                200,
                r#"{"labels":[{"id":"INBOX","name":"INBOX"},{"id":"Label_3","name":"Pigeon/Snoozed"}]}"#,
            )],
        ));
        let wake = snooze_ops(vec![], labels(&["t"]), vec![]).remove(0);
        let MailOp::Modify { remove, .. } =
            resolve_snoozed(&gmail, &store, ACCOUNT, &wake).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(remove, ["Label_3"]);
        assert_eq!(fake.requests().len(), 1);
        // Other ops never ask.
        let star = MailOp::Modify {
            threads: labels(&["t"]),
            add: labels(&[gmail::STARRED]),
            remove: vec![],
        };
        assert_eq!(
            resolve_snoozed(&gmail, &store, ACCOUNT, &star).unwrap(),
            star
        );
        assert_eq!(fake.requests().len(), 1);
    }

    #[test]
    fn offline_the_snooze_waits() {
        let store = inbox_store("t");
        let (gmail, _) = gmail_over(FakeTransport::new().on("GET", LABELS, &[], &[(0, "")]));
        let op = snooze_ops(labels(&["t"]), vec![], vec![]).remove(0);
        let error = resolve_snoozed(&gmail, &store, ACCOUNT, &op).unwrap_err();
        assert!(error.later().is_some());
    }

    #[test]
    fn demo_snoozes_apply_to_the_store_with_the_placeholder_label() {
        let store = inbox_store("t");
        let op = snooze_ops(labels(&["t"]), vec![], vec![]).remove(0);
        store.apply_mail_op(ACCOUNT, &op).unwrap();
        let thread = &store.load_threads(ACCOUNT, &labels(&["t"])).unwrap()[0];
        assert_eq!(thread.folder, Folder::Archive);
        let wake = MailOp::Modify {
            threads: labels(&["t"]),
            add: labels(&[gmail::INBOX, gmail::UNREAD]),
            remove: labels(&[SNOOZED]),
        };
        store.apply_mail_op(ACCOUNT, &wake).unwrap();
        let thread = &store.load_threads(ACCOUNT, &labels(&["t"])).unwrap()[0];
        assert_eq!(thread.folder, Folder::Inbox);
        assert!(thread.unread);
        assert!(!thread.label_ids.contains(&SNOOZED.to_string()));
    }

    // Send Later through the outbox.

    fn send(local_id: &str) -> MailOp {
        MailOp::Send {
            draft: Outgoing {
                subject: "Plan".into(),
                ..Outgoing::default()
            },
            local_id: Some(local_id.into()),
            thread_id: "sent-local-1".into(),
        }
    }

    #[test]
    fn a_scheduled_send_waits_for_its_time_without_holding_others_back() {
        let store = Store::open_in_memory().unwrap();
        let outbox = Outbox::new(store.clone(), ACCOUNT.into());
        let at = 3_600_000;
        let later = outbox
            .push(
                Op::Later {
                    at,
                    op: send("draft-local-1"),
                },
                at - 1_000,
                1_000,
            )
            .unwrap();
        // A send right after goes at its own time.
        outbox
            .push(Op::Mail(send("draft-local-2")), 5_000, 1_000)
            .unwrap();
        let due = outbox.take_due(6_000).unwrap();
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0].1, Op::Mail(MailOp::Send { .. })));
        // Quit doesn't hurry it.
        outbox.send_now(7_000).unwrap();
        assert!(outbox.take_due(7_000).unwrap().is_empty());
        assert_eq!(outbox.next_due(), Some(at));
        let due = outbox.take_due(at).unwrap();
        assert_eq!(due.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [later]);
    }

    #[test]
    fn scheduled_sends_go_at_their_own_times() {
        let outbox = Outbox::new(Store::open_in_memory().unwrap(), ACCOUNT.into());
        let (friday, today) = (9_000_000, 5_000_000);
        let later = |at: i64, draft: &str| Op::Later {
            at,
            op: send(draft),
        };
        // Scheduled first, due last.
        let first = outbox
            .push(later(friday, "draft-local-1"), friday - 1_000, 1_000)
            .unwrap();
        let second = outbox
            .push(later(today, "draft-local-2"), today - 1_000, 1_000)
            .unwrap();
        assert_eq!(outbox.next_due(), Some(today));
        let due = outbox.take_due(today).unwrap();
        assert_eq!(due.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [second]);
        // Going, it doesn't hold the other back either.
        assert_eq!(outbox.next_due(), Some(friday));
        outbox.finish(second).unwrap();
        assert!(outbox.take_due(friday - 1).unwrap().is_empty());
        let due = outbox.take_due(friday).unwrap();
        assert_eq!(due.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [first]);
    }

    #[test]
    fn a_scheduled_draft_deleted_elsewhere_isnt_sent() {
        let store = Store::open_in_memory().unwrap();
        let draft = Outgoing {
            subject: "Plan".into(),
            ..Outgoing::default()
        };
        store
            .put_draft(ACCOUNT, &draft.to_draft_thread("draft-local-1".into()), 1)
            .unwrap();
        store.set_draft_id(ACCOUNT, "draft-local-1", "r-1").unwrap();
        let op = Op::Later {
            at: 0,
            op: send("draft-local-1"),
        };
        let Op::Later { op: send_op, .. } = &op else {
            unreachable!()
        };
        assert!(!draft_deleted(&store, ACCOUNT, send_op));
        // Deleted in Gmail web: the drafts sync drops it.
        store
            .delete_threads(ACCOUNT, &labels(&["draft-local-1"]))
            .unwrap();
        assert!(draft_deleted(&store, ACCOUNT, send_op));
        let (gmail, fake) = gmail_over(FakeTransport::new());
        let transport: Arc<dyn Transport> = fake.clone();
        let (outcome, events) =
            super::super::commit_google(&transport, &gmail, &store, ACCOUNT, &op);
        assert_eq!(outcome, Outcome::failed(DRAFT_DELETED));
        assert!(events.is_empty());
        assert!(fake.requests().is_empty(), "nothing went to Gmail");
    }

    #[test]
    fn a_scheduled_draft_stays_marked_until_cancelled() {
        let store = Store::open_in_memory().unwrap();
        let outbox = Outbox::new(store.clone(), ACCOUNT.into());
        let draft = Outgoing {
            subject: "Plan".into(),
            ..Outgoing::default()
        };
        store
            .put_draft(ACCOUNT, &draft.to_draft_thread("draft-local-1".into()), 1)
            .unwrap();
        let id = outbox
            .push(
                Op::Later {
                    at: 50_000,
                    op: send("draft-local-1"),
                },
                49_000,
                1_000,
            )
            .unwrap();
        let mut threads = store.load_all_threads(ACCOUNT).unwrap();
        outbox.overlay(&mut threads);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].later.send_at, Some(50_000));
        assert!(outbox.queued_sends().is_empty());
        // Cancelled (Edit, Cancel Send or Undo): a plain draft again.
        assert!(outbox.cancel(id));
        let mut threads = store.load_all_threads(ACCOUNT).unwrap();
        outbox.overlay(&mut threads);
        assert_eq!(threads[0].later.send_at, None);
    }

    #[test]
    fn a_scheduled_send_commits_like_a_send() {
        let store = Store::open_in_memory().unwrap();
        let draft = Outgoing {
            subject: "Plan".into(),
            ..Outgoing::default()
        };
        store
            .put_draft(ACCOUNT, &draft.to_draft_thread("draft-local-1".into()), 1)
            .unwrap();
        let (outcome, _) = super::super::commit_demo(
            &store,
            ACCOUNT,
            &Op::Later {
                at: 0,
                op: send("draft-local-1"),
            },
        );
        assert_eq!(outcome, Outcome::Committed);
        let threads = store.load_all_threads(ACCOUNT).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].folder, Folder::Sent);
        assert_eq!(threads[0].id, "sent-local-1");
    }
}

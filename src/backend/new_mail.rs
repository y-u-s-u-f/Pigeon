//! New mail to notify about (NTF-1, sprint 2 W1). History's `messagesAdded`
//! ids ride along the partial sync's plan to its last step; by then the
//! threads are fetched and stored, so each message is judged on its labels
//! and `internalDate` as Gmail has them now. The ones notified are recorded
//! with the new history id in one transaction: a retry, a crash or a
//! relaunch neither repeats nor drops one. A full sync or a history reset
//! never gets here.

use std::collections::HashSet;

use super::{
    gmail::{self, DRAFT, INBOX, SPAM, TRASH, UNREAD},
    store::Store,
};
use crate::data::{Account, Message};

/// Mail older than this when the poll sees it isn't news any more.
pub const FRESH_MS: i64 = 15 * 60 * 1000;

/// One message to notify about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMail {
    pub message_id: String,
    pub thread_id: String,
    pub from: String,
    pub subject: String,
    pub snippet: String,
}

/// The messages a page of history added, as (message id, thread id), once
/// each, drafts left out.
pub fn added_messages(history: &[gmail::History]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    history
        .iter()
        .flat_map(|h| &h.messages_added)
        .map(|added| &added.message)
        .filter(|m| !m.label_ids.iter().any(|l| l == DRAFT))
        .filter(|m| seen.insert(m.id.clone()))
        .map(|m| (m.id.clone(), m.thread_id.clone()))
        .collect()
}

/// Whether a stored message is news: unread in the Inbox, not spam, trash
/// or a draft, received (someone else sent it, or the user sent it to
/// themselves or an alias), and dated within `FRESH_MS` of `now`.
pub fn eligible(message: &Message, me: &Account, now: i64) -> bool {
    let has = |label: &str| message.label_ids.iter().any(|l| l == label);
    has(INBOX)
        && has(UNREAD)
        && ![SPAM, TRASH, DRAFT].into_iter().any(has)
        && gmail::received(message, me)
        && (now - message.internal_date).abs() <= FRESH_MS
}

/// Judge `added` against the store, record the new ones as notified and
/// set `history_key` to `history`: what to notify about.
pub fn checkpoint(
    store: &Store,
    account: &str,
    added: &[(String, String)],
    history_key: &str,
    history: u64,
    now: i64,
) -> anyhow::Result<Vec<NewMail>> {
    let mut fresh = Vec::new();
    if !added.is_empty() {
        let me = store.me(account);
        let ids: Vec<String> = added.iter().map(|(m, _)| m.clone()).collect();
        let already = store.notified(account, &ids)?;
        let mut thread_ids: Vec<String> = added.iter().map(|(_, t)| t.clone()).collect();
        thread_ids.dedup();
        let threads = store.load_threads(account, &thread_ids)?;
        for (message_id, thread_id) in added {
            if already.contains(message_id) {
                continue;
            }
            let Some(thread) = threads.iter().find(|t| &t.id == thread_id) else {
                continue;
            };
            let Some(message) = thread.messages.iter().find(|m| &m.id == message_id) else {
                continue;
            };
            if eligible(message, &me, now) {
                fresh.push(NewMail {
                    message_id: message_id.clone(),
                    thread_id: thread_id.clone(),
                    from: message.from.display_name().to_string(),
                    subject: thread.subject.clone(),
                    snippet: message.snippet.clone(),
                });
            }
        }
    }
    let ids: Vec<String> = fresh.iter().map(|m| m.message_id.clone()).collect();
    store.checkpoint_notified(account, &ids, history_key, &history.to_string(), now)?;
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Contact, Folder, Thread};

    const ME: &str = "me@x.com";
    const NOW: i64 = 1_800_000_000_000;

    fn me() -> Account {
        Account {
            email: ME.into(),
            aliases: vec!["alias@x.com".into()],
            send_as: Vec::new(),
            ..Default::default()
        }
    }

    fn contact(email: &str) -> Contact {
        Contact {
            name: String::new(),
            email: email.into(),
        }
    }

    fn message(id: &str, labels: &[&str], from: &str, to: &str, at: i64) -> Message {
        Message {
            id: id.into(),
            label_ids: labels.iter().map(|l| l.to_string()).collect(),
            to: vec![contact(to)],
            internal_date: at,
            snippet: format!("snippet {id}"),
            ..Message::new(contact(from))
        }
    }

    fn store_with(messages: Vec<(&str, Message)>) -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .set_sync_state(ME, crate::backend::store::ALIASES_KEY, r#"["alias@x.com"]"#)
            .unwrap();
        let mut threads: Vec<Thread> = Vec::new();
        for (thread_id, m) in messages {
            match threads.iter_mut().find(|t| t.id == thread_id) {
                Some(t) => t.messages.push(m),
                None => threads.push(Thread::local(
                    thread_id.into(),
                    format!("Subject {thread_id}"),
                    Folder::Inbox,
                    vec![m],
                    None,
                )),
            }
        }
        store.put_threads(ME, &threads).unwrap();
        store
    }

    fn added(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(m, t)| (m.to_string(), t.to_string()))
            .collect()
    }

    #[test]
    fn only_fresh_unread_inbox_mail_received_counts() {
        let ok = |labels: &[&str], from: &str, to: &str, at: i64| {
            eligible(&message("m", labels, from, to, at), &me(), NOW)
        };
        assert!(ok(&["INBOX", "UNREAD"], "a@y.com", ME, NOW));
        assert!(ok(
            &["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"],
            "a@y.com",
            ME,
            NOW
        ));
        assert!(!ok(&["INBOX"], "a@y.com", ME, NOW));
        assert!(!ok(&["UNREAD"], "a@y.com", ME, NOW));
        assert!(!ok(&["INBOX", "UNREAD", "SPAM"], "a@y.com", ME, NOW));
        assert!(!ok(&["INBOX", "UNREAD", "TRASH"], "a@y.com", ME, NOW));
        // Too old for news, after an offline gap.
        assert!(!ok(&["INBOX", "UNREAD"], "a@y.com", ME, NOW - FRESH_MS - 1));
        assert!(ok(&["INBOX", "UNREAD"], "a@y.com", ME, NOW - FRESH_MS));
        // Self-mail, to the main address or an alias, counts; mail the
        // user sent to someone else doesn't.
        assert!(ok(&["INBOX", "UNREAD", "SENT"], ME, ME, NOW));
        assert!(ok(&["INBOX", "UNREAD", "SENT"], ME, "alias@x.com", NOW));
        assert!(!ok(&["INBOX", "UNREAD", "SENT"], ME, "b@y.com", NOW));
    }

    #[test]
    fn added_messages_skip_drafts_and_repeats() {
        let history: Vec<gmail::History> = serde_json::from_str(
            r#"[{"messagesAdded":[{"message":{"id":"m1","threadId":"t1","labelIds":["INBOX"]}},
                                  {"message":{"id":"d1","threadId":"t2","labelIds":["DRAFT"]}}]},
                {"messagesAdded":[{"message":{"id":"m1","threadId":"t1"}},
                                  {"message":{"id":"m2","threadId":"t1"}}],
                 "labelsAdded":[{"message":{"id":"m9","threadId":"t9"}}]}]"#,
        )
        .unwrap();
        assert_eq!(
            added_messages(&history),
            added(&[("m1", "t1"), ("m2", "t1")])
        );
    }

    #[test]
    fn retry_and_relaunch_notify_once() {
        let store = store_with(vec![(
            "t1",
            message("m1", &["INBOX", "UNREAD"], "a@y.com", ME, NOW),
        )]);
        let pairs = added(&[("m1", "t1")]);
        let first = checkpoint(&store, ME, &pairs, "h", 10, NOW).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].thread_id, "t1");
        assert_eq!(first[0].subject, "Subject t1");
        assert_eq!(store.sync_state(ME, "h").unwrap().as_deref(), Some("10"));
        // The same history again (a retried step, or a relaunch before the
        // cursor moved on elsewhere): nothing new.
        assert!(
            checkpoint(&store, ME, &pairs, "h", 10, NOW + 1000)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn self_mail_and_alias_notify_but_sent_mail_does_not() {
        let store = store_with(vec![
            (
                "t1",
                message("m1", &["INBOX", "UNREAD", "SENT"], ME, ME, NOW),
            ),
            (
                "t2",
                message("m2", &["INBOX", "UNREAD", "SENT"], ME, "alias@x.com", NOW),
            ),
            ("t3", message("m3", &["SENT"], ME, "b@y.com", NOW)),
            // Gone from the store by the time it's judged: skipped.
        ]);
        let fresh = checkpoint(
            &store,
            ME,
            &added(&[("m1", "t1"), ("m2", "t2"), ("m3", "t3"), ("gone", "t4")]),
            "h",
            11,
            NOW,
        )
        .unwrap();
        let ids: Vec<&str> = fresh.iter().map(|m| m.message_id.as_str()).collect();
        assert_eq!(ids, ["m1", "m2"]);
    }

    #[test]
    fn nothing_added_still_moves_the_cursor() {
        let store = Store::open_in_memory().unwrap();
        assert!(
            checkpoint(&store, ME, &[], "h", 12, NOW)
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.sync_state(ME, "h").unwrap().as_deref(), Some("12"));
    }
}

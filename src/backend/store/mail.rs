//! Mail in the store: labels, threads, messages, bodies and drafts, read
//! back into the UI's model (`data::Thread`).

use std::collections::HashMap;

use chrono::DateTime;
use rusqlite::{Transaction, params};

use super::{Store, from_json, json};
use crate::{
    backend::{MailOp, gmail},
    data::{Body, DraftOrigin, Folder, Invite, InviteMethod, Label, Loaded, Message, Thread},
};

impl Store {
    pub fn put_labels(&self, account: &str, labels: &[Label]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for label in labels {
            let kind = if gmail::is_system_label(&label.id) {
                "system"
            } else {
                "user"
            };
            tx.execute(
                "INSERT INTO labels (account, id, name, type, bg) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (account, id) DO UPDATE SET
                    name = excluded.name, type = excluded.type, bg = excluded.bg",
                params![
                    account,
                    label.id,
                    label.name,
                    kind,
                    crate::data::to_hex(label.color)
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The user's labels, by name.
    pub fn load_labels(&self, account: &str) -> anyhow::Result<Vec<Label>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, bg FROM labels WHERE account = ?1 AND type = 'user' ORDER BY rowid",
        )?;
        let rows = stmt.query_map([account], |r| {
            Ok(Label {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r
                    .get::<_, Option<String>>(2)?
                    .and_then(|hex| crate::data::from_hex(&hex))
                    .unwrap_or(gpui::hsla(0.6, 0.1, 0.5, 1.)),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Store threads whole (their messages, bodies and draft), replacing
    /// what was there.
    pub fn put_threads(&self, account: &str, threads: &[Thread]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for thread in threads {
            write_thread(&tx, account, thread)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_threads(&self, account: &str, ids: &[String]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for id in ids {
            delete_thread(&tx, account, id)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every stored thread, in the order stored (callers sort).
    pub fn load_all_threads(&self, account: &str) -> anyhow::Result<Vec<Thread>> {
        self.load_threads_where(account, None)
    }

    /// The threads among `ids` that are stored, in the order stored.
    pub fn load_threads(&self, account: &str, ids: &[String]) -> anyhow::Result<Vec<Thread>> {
        self.load_threads_where(account, Some(ids))
    }

    fn load_threads_where(
        &self,
        account: &str,
        ids: Option<&[String]>,
    ) -> anyhow::Result<Vec<Thread>> {
        let conn = self.conn();
        let wanted = |id: &str| ids.is_none_or(|ids| ids.iter().any(|i| i == id));

        let mut messages: HashMap<String, Vec<Message>> = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT m.thread_id, m.id, m.label_ids, m.\"from\", m.\"to\", m.cc, m.bcc, m.date,
                    m.internal_date, m.snippet, m.attachments, m.message_id, m.in_reply_to,
                    m.\"references\", m.reply_to, m.size_estimate,
                    b.html, b.invite_method,
                    COALESCE(NULLIF(b.invite_event_id, ''),
                        (SELECT e.id FROM events e
                         WHERE e.account = m.account AND e.ical_uid = b.invite_uid
                         ORDER BY e.recurring_event_id IS NOT NULL
                         LIMIT 1)),
                    b.invite_uid, b.invite_details
             FROM messages m
             LEFT JOIN bodies b ON b.account = m.account AND b.message_id = m.id
             WHERE m.account = ?1
             ORDER BY m.internal_date, m.rowid",
        )?;
        let mut rows = stmt.query([account])?;
        while let Some(r) = rows.next()? {
            let thread_id: String = r.get(0)?;
            if !wanted(&thread_id) {
                continue;
            }
            let date: String = r.get(7)?;
            let html: Option<String> = r.get(16)?;
            let method: Option<String> = r.get(17)?;
            let invite = method.map(|method| Invite {
                event_id: r
                    .get::<_, Option<String>>(18)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                method: if method == "cancel" {
                    InviteMethod::Cancel
                } else {
                    InviteMethod::Request
                },
                ical_uid: r
                    .get::<_, Option<String>>(19)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                details: r
                    .get::<_, Option<String>>(20)
                    .ok()
                    .flatten()
                    .and_then(|j| serde_json::from_str(&j).ok()),
            });
            let message = Message {
                id: r.get(1)?,
                label_ids: from_json(&r.get::<_, String>(2)?),
                from: from_json(&r.get::<_, String>(3)?),
                to: from_json(&r.get::<_, String>(4)?),
                cc: from_json(&r.get::<_, String>(5)?),
                bcc: from_json(&r.get::<_, String>(6)?),
                date: DateTime::parse_from_rfc3339(&date)?,
                internal_date: r.get(8)?,
                snippet: r.get(9)?,
                body: match html {
                    Some(html) => Body::Loaded { html },
                    None => Body::NotLoaded,
                },
                attachments: from_json(&r.get::<_, String>(10)?),
                invite,
                message_id: r.get(11)?,
                in_reply_to: r.get(12)?,
                references: from_json(&r.get::<_, String>(13)?),
                reply_to: from_json(&r.get::<_, String>(14)?),
                size_estimate: r.get::<_, i64>(15)? as u64,
            };
            messages.entry(thread_id).or_default().push(message);
        }

        let mut origins: HashMap<String, Option<DraftOrigin>> = HashMap::new();
        let mut stmt = conn.prepare("SELECT local_id, origin FROM drafts WHERE account = ?1")?;
        let mut rows = stmt.query([account])?;
        while let Some(r) = rows.next()? {
            let origin: Option<String> = r.get(1)?;
            origins.insert(
                r.get(0)?,
                origin.and_then(|o| serde_json::from_str(&o).ok()),
            );
        }

        let mut threads = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT id, history_id, subject, folder, unread, starred, label_ids, search_text, loaded
             FROM threads WHERE account = ?1 ORDER BY rowid",
        )?;
        let mut rows = stmt.query([account])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            if !wanted(&id) {
                continue;
            }
            let Some(messages) = messages.remove(&id) else {
                continue;
            };
            let label_ids: Vec<String> = from_json(&r.get::<_, String>(6)?);
            let mut thread = Thread {
                history_id: r.get::<_, Option<i64>>(1)?.map(|h| h as u64),
                subject: r.get(2)?,
                folder: Folder::from_key(&r.get::<_, String>(3)?).unwrap_or(Folder::Archive),
                unread: r.get(4)?,
                starred: r.get(5)?,
                labels: label_ids
                    .iter()
                    .filter(|l| !gmail::is_system_label(l))
                    .cloned()
                    .collect(),
                label_ids,
                loaded: if r.get::<_, String>(8)? == "full" {
                    Loaded::Full
                } else {
                    Loaded::Metadata
                },
                messages,
                search: Default::default(),
                origin: origins.remove(&id).flatten(),
                id,
            };
            thread.set_search(r.get(7)?);
            threads.push(thread);
        }
        Ok(threads)
    }

    /// Apply a mail op the way Gmail will (or, in demo mode, instead of it).
    pub fn apply_mail_op(&self, account: &str, op: &MailOp) -> anyhow::Result<()> {
        match op {
            MailOp::Modify {
                threads,
                add,
                remove,
            } => self.modify(account, threads, add, remove),
            MailOp::Trash { threads } => self.modify(account, threads, &[gmail::TRASH.into()], &[]),
            MailOp::Untrash { threads } => {
                self.modify(account, threads, &[], &[gmail::TRASH.into()])
            }
            MailOp::SaveDraft { local_id, draft } => {
                self.put_threads(account, &[draft.to_draft_thread(local_id.clone())])
            }
            MailOp::DeleteDraft { local_id } => {
                self.delete_threads(account, std::slice::from_ref(local_id))
            }
            MailOp::Send {
                draft,
                local_id,
                thread_id,
            } => {
                if let Some(local_id) = local_id {
                    self.delete_threads(account, std::slice::from_ref(local_id))?;
                }
                let message = draft.to_message(vec![gmail::SENT.into()]);
                let thread = match self
                    .load_threads(account, std::slice::from_ref(thread_id))?
                    .pop()
                {
                    Some(mut thread) => {
                        thread.messages.push(message);
                        thread.reindex();
                        thread
                    }
                    None => Thread::local(
                        thread_id.clone(),
                        draft.subject.clone(),
                        Folder::Sent,
                        vec![message],
                        None,
                    ),
                };
                self.put_threads(account, &[thread])
            }
        }
    }

    /// Add and remove labels on every message of the threads, like
    /// `threads.modify`, and derive the threads' flags and folder again.
    fn modify(
        &self,
        account: &str,
        ids: &[String],
        add: &[String],
        remove: &[String],
    ) -> anyhow::Result<()> {
        let mut threads = self.load_threads(account, ids)?;
        for thread in &mut threads {
            modify_thread(thread, add, remove);
        }
        self.put_threads(account, &threads)
    }

    /// A thread's id changed (a local id the server replaced).
    pub fn replace_thread_id(&self, account: &str, old: &str, new: &str) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE threads SET id = ?3 WHERE account = ?1 AND id = ?2",
            params![account, old, new],
        )?;
        tx.execute(
            "UPDATE messages SET thread_id = ?3 WHERE account = ?1 AND thread_id = ?2",
            params![account, old, new],
        )?;
        tx.execute(
            "UPDATE drafts SET thread_id = ?3 WHERE account = ?1 AND thread_id = ?2",
            params![account, old, new],
        )?;
        tx.commit()?;
        Ok(())
    }

    // Gmail sync (W3).

    /// Every Gmail label as listed, with its counts: the user's visible
    /// ones become the sidebar's labels, the rest are kept for their
    /// counts. Labels gone from Gmail go.
    pub fn replace_gmail_labels(
        &self,
        account: &str,
        labels: &[gmail::GLabel],
    ) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let ids: Vec<&str> = labels.iter().map(|l| l.id.as_str()).collect();
        let stored: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM labels WHERE account = ?1")?;
            stmt.query_map([account], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        for id in stored.iter().filter(|id| !ids.contains(&id.as_str())) {
            tx.execute(
                "DELETE FROM labels WHERE account = ?1 AND id = ?2",
                params![account, id],
            )?;
        }
        for label in labels {
            let kind = if gmail::shows_label(label) {
                "user"
            } else if label.kind == "user" {
                "hidden"
            } else {
                "system"
            };
            let model = gmail::label_from_gmail(label);
            tx.execute(
                "INSERT INTO labels (account, id, name, type, bg, threads_unread, threads_total)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (account, id) DO UPDATE SET
                    name = excluded.name, type = excluded.type, bg = excluded.bg,
                    threads_unread = COALESCE(excluded.threads_unread, threads_unread),
                    threads_total = COALESCE(excluded.threads_total, threads_total)",
                params![
                    account,
                    label.id,
                    label.name,
                    kind,
                    crate::data::to_hex(model.color),
                    label.threads_unread.map(|n| n as i64),
                    label.threads_total.map(|n| n as i64),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Gmail's thread counts by label id: (unread, total). Empty in demo
    /// mode, which has none.
    pub fn label_counts(&self, account: &str) -> anyhow::Result<HashMap<String, (u64, u64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, threads_unread, threads_total FROM labels
             WHERE account = ?1 AND threads_unread IS NOT NULL",
        )?;
        let rows = stmt.query_map([account], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
                ),
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Each stored thread's history id, to skip the unchanged ones.
    pub fn thread_history_ids(&self, account: &str) -> anyhow::Result<HashMap<String, u64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, history_id FROM threads WHERE account = ?1 AND history_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([account], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Store threads fetched from Gmail without bodies, keeping the bodies,
    /// attachments and invitations already stored for their messages.
    pub fn put_synced_threads(&self, account: &str, threads: Vec<Thread>) -> anyhow::Result<()> {
        let ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
        let mut stored: HashMap<String, Message> = self
            .load_threads(account, &ids)?
            .into_iter()
            .flat_map(|t| t.messages)
            .map(|m| (m.id.clone(), m))
            .collect();
        let mut threads = threads;
        for thread in &mut threads {
            for message in &mut thread.messages {
                if let Some(old) = stored.remove(&message.id)
                    && matches!(old.body, Body::Loaded { .. })
                {
                    message.body = old.body;
                    message.attachments = old.attachments;
                    message.invite = old.invite;
                }
            }
            thread.loaded = loaded(thread);
            thread.reindex();
        }
        self.put_threads(account, &threads)
    }

    /// Store message bodies parsed from Gmail's raw messages (RDR-6).
    pub fn put_bodies(
        &self,
        account: &str,
        thread_id: &str,
        bodies: Vec<(String, crate::backend::mime::ParsedBody)>,
    ) -> anyhow::Result<()> {
        let Some(mut thread) = self.load_threads(account, &[thread_id.to_string()])?.pop() else {
            return Ok(());
        };
        for (id, parsed) in bodies {
            let Some(message) = thread.messages.iter_mut().find(|m| m.id == id) else {
                continue;
            };
            message.body = Body::Loaded { html: parsed.html };
            message.attachments = parsed.attachments;
            let details = parsed.invite_details;
            message.invite = parsed.invite.map(|(ical_uid, method)| Invite {
                event_id: String::new(),
                method,
                ical_uid,
                details,
            });
        }
        thread.loaded = loaded(&thread);
        thread.reindex();
        self.put_threads(account, &[thread])
    }

    /// The Drafts threads that are Gmail drafts: (thread id, Gmail draft
    /// id if known, message id).
    pub fn stored_drafts(
        &self,
        account: &str,
    ) -> anyhow::Result<Vec<(String, Option<String>, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT local_id, draft_id, COALESCE(message_id, '') FROM drafts WHERE account = ?1",
        )?;
        let rows = stmt.query_map([account], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_draft_id(
        &self,
        account: &str,
        local_id: &str,
        draft_id: &str,
    ) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE drafts SET draft_id = ?3 WHERE account = ?1 AND local_id = ?2",
            params![account, local_id, draft_id],
        )?;
        Ok(())
    }
}

/// Add and remove labels on every message of the thread, like
/// `threads.modify`, and derive its flags, labels and folder again.
pub fn modify_thread(thread: &mut Thread, add: &[String], remove: &[String]) {
    let moves = add
        .iter()
        .chain(remove)
        .any(|l| gmail::FOLDER_LABELS.contains(&l.as_str()));
    for message in &mut thread.messages {
        message.label_ids.retain(|l| !remove.contains(l));
        for label in add {
            if !message.label_ids.contains(label) {
                message.label_ids.push(label.clone());
            }
        }
    }
    let labels: Vec<Vec<String>> = thread
        .messages
        .iter()
        .map(|m| m.label_ids.clone())
        .collect();
    thread.label_ids = gmail::union_labels(&labels);
    thread.labels = thread
        .label_ids
        .iter()
        .filter(|l| !gmail::is_system_label(l))
        .cloned()
        .collect();
    let has = |label: &str| thread.label_ids.iter().any(|l| l == label);
    thread.unread = has(gmail::UNREAD);
    thread.starred = has(gmail::STARRED);
    if moves && thread.folder != Folder::Drafts {
        thread.folder = gmail::derive_folder(&labels);
    }
}

/// Full once every message has its body.
fn loaded(thread: &Thread) -> Loaded {
    if thread
        .messages
        .iter()
        .all(|m| matches!(m.body, Body::Loaded { .. }))
    {
        Loaded::Full
    } else {
        Loaded::Metadata
    }
}

fn write_thread(tx: &Transaction, account: &str, thread: &Thread) -> anyhow::Result<()> {
    let Some(last) = thread.messages.last() else {
        return delete_thread(tx, account, &thread.id);
    };
    let mut label_ids: Vec<String> = Vec::new();
    for label in thread.messages.iter().flat_map(|m| &m.label_ids) {
        if !label_ids.contains(label) {
            label_ids.push(label.clone());
        }
    }
    tx.execute(
        "INSERT INTO threads (account, id, history_id, subject, snippet, last_date, folder,
            unread, starred, has_attachments, label_ids, search_text, loaded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT (account, id) DO UPDATE SET
            history_id = excluded.history_id, subject = excluded.subject,
            snippet = excluded.snippet, last_date = excluded.last_date,
            folder = excluded.folder, unread = excluded.unread, starred = excluded.starred,
            has_attachments = excluded.has_attachments, label_ids = excluded.label_ids,
            search_text = excluded.search_text, loaded = excluded.loaded",
        params![
            account,
            thread.id,
            thread.history_id.map(|h| h as i64),
            thread.subject,
            last.snippet,
            last.internal_date,
            thread.folder.key(),
            thread.unread,
            thread.starred,
            thread.has_attachments(),
            json(&label_ids),
            thread.search.all,
            match thread.loaded {
                Loaded::Full => "full",
                Loaded::Metadata => "metadata",
            },
        ],
    )?;

    // Bodies of messages that left the thread go with them; the others
    // stay, even when this copy of the thread hasn't loaded them.
    let old: Vec<String> = {
        let mut stmt =
            tx.prepare("SELECT id FROM messages WHERE account = ?1 AND thread_id = ?2")?;
        stmt.query_map(params![account, thread.id], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    for id in old
        .iter()
        .filter(|id| !thread.messages.iter().any(|m| &m.id == *id))
    {
        tx.execute(
            "DELETE FROM bodies WHERE account = ?1 AND message_id = ?2",
            params![account, id],
        )?;
    }
    tx.execute(
        "DELETE FROM messages WHERE account = ?1 AND thread_id = ?2",
        params![account, thread.id],
    )?;
    for message in &thread.messages {
        tx.execute(
            "INSERT INTO messages (account, id, thread_id, internal_date, date, label_ids,
                \"from\", \"to\", cc, bcc, reply_to, subject, snippet, message_id, in_reply_to,
                \"references\", size_estimate, attachments)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18)
             ON CONFLICT (account, id) DO UPDATE SET thread_id = excluded.thread_id",
            params![
                account,
                message.id,
                thread.id,
                message.internal_date,
                message.date.to_rfc3339(),
                json(&message.label_ids),
                json(&message.from),
                json(&message.to),
                json(&message.cc),
                json(&message.bcc),
                json(&message.reply_to),
                thread.subject,
                message.snippet,
                message.message_id,
                message.in_reply_to,
                json(&message.references),
                message.size_estimate as i64,
                json(&message.attachments),
            ],
        )?;
        if let Body::Loaded { html } = &message.body {
            let invite = message.invite.as_ref();
            tx.execute(
                "INSERT INTO bodies (account, message_id, html, invite_uid, invite_method,
                    invite_event_id, bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (account, message_id) DO UPDATE SET
                    html = excluded.html, invite_uid = excluded.invite_uid,
                    invite_method = excluded.invite_method,
                    invite_event_id = excluded.invite_event_id, bytes = excluded.bytes",
                params![
                    account,
                    message.id,
                    html,
                    invite.map(|i| i.ical_uid.as_str()),
                    invite.map(|i| match i.method {
                        InviteMethod::Request => "request",
                        InviteMethod::Cancel => "cancel",
                    }),
                    invite.map(|i| i.event_id.as_str()),
                    html.len() as i64,
                ],
            )?;
            // W10 (CAL-20).
            tx.execute(
                "UPDATE bodies SET invite_details = ?3 WHERE account = ?1 AND message_id = ?2",
                params![
                    account,
                    message.id,
                    invite.and_then(|i| i.details.as_ref()).map(json),
                ],
            )?;
        }
    }

    if thread.folder == Folder::Drafts {
        tx.execute(
            "INSERT INTO drafts (account, local_id, message_id, thread_id, markdown, origin,
                updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (account, local_id) DO UPDATE SET
                message_id = excluded.message_id, thread_id = excluded.thread_id,
                markdown = excluded.markdown, origin = excluded.origin,
                updated_at = excluded.updated_at",
            params![
                account,
                thread.id,
                last.id,
                thread.origin.as_ref().and_then(|o| o.reply_to.clone()),
                thread.origin.as_ref().map(|o| o.markdown.as_str()),
                thread.origin.as_ref().map(json),
                chrono::Utc::now().timestamp_millis(),
            ],
        )?;
    }
    Ok(())
}

fn delete_thread(tx: &Transaction, account: &str, id: &str) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM bodies WHERE account = ?1 AND message_id IN
            (SELECT id FROM messages WHERE account = ?1 AND thread_id = ?2)",
        params![account, id],
    )?;
    for table in [
        "messages WHERE thread_id",
        "threads WHERE id",
        "drafts WHERE local_id",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} = ?2 AND account = ?1"),
            params![account, id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::backend::{MailOp, demo, gmail, store::Store};
    use crate::data::Folder;

    fn seeded() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let account = demo::seed(&store).unwrap();
        (store, account.email)
    }

    /// Thread → rows → thread keeps everything the UI shows and searches.
    #[test]
    fn threads_round_trip() {
        let (store, account) = seeded();
        let original = demo::mail_data();
        let mut stored = store.load_all_threads(&account).unwrap();
        assert_eq!(stored.len(), original.threads.len());
        stored.sort_by_key(|t| std::cmp::Reverse(t.last_message().internal_date));
        for (a, b) in original.threads.iter().zip(&stored) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.subject, b.subject);
            assert_eq!(a.folder, b.folder, "{}", a.id);
            assert_eq!((a.unread, a.starred), (b.unread, b.starred), "{}", a.id);
            assert_eq!(a.labels, b.labels, "{}", a.id);
            assert_eq!(a.label_ids, b.label_ids, "{}", a.id);
            assert_eq!(a.search.all, b.search.all, "{}", a.id);
            assert_eq!(a.search.from, b.search.from);
            assert_eq!(a.messages.len(), b.messages.len());
            for (m, n) in a.messages.iter().zip(&b.messages) {
                assert_eq!(m.id, n.id);
                assert_eq!(m.from, n.from);
                assert_eq!((&m.to, &m.cc, &m.bcc), (&n.to, &n.cc, &n.bcc));
                assert_eq!(m.date, n.date);
                assert_eq!(m.snippet, n.snippet);
                assert_eq!(m.body, n.body);
                assert_eq!(m.attachments, n.attachments);
                assert_eq!(
                    m.invite.as_ref().map(|i| (&i.event_id, i.method)),
                    n.invite.as_ref().map(|i| (&i.event_id, i.method))
                );
            }
        }
        let labels = store.load_labels(&account).unwrap();
        assert_eq!(
            labels.iter().map(|l| &l.id).collect::<Vec<_>>(),
            original.labels.iter().map(|l| &l.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn modify_derives_flags_and_folder() {
        let (store, account) = seeded();
        let inbox = store
            .load_all_threads(&account)
            .unwrap()
            .into_iter()
            .find(|t| t.folder == Folder::Inbox && !t.unread)
            .unwrap();
        let ids = vec![inbox.id.clone()];
        let op = |add: &[&str], remove: &[&str]| MailOp::Modify {
            threads: ids.clone(),
            add: add.iter().map(|s| s.to_string()).collect(),
            remove: remove.iter().map(|s| s.to_string()).collect(),
        };
        store
            .apply_mail_op(
                &account,
                &op(&[gmail::UNREAD, gmail::STARRED], &[gmail::INBOX]),
            )
            .unwrap();
        let t = &store.load_threads(&account, &ids).unwrap()[0];
        assert!(t.unread && t.starred);
        assert_ne!(t.folder, Folder::Inbox);
        store
            .apply_mail_op(
                &account,
                &MailOp::Trash {
                    threads: ids.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            store.load_threads(&account, &ids).unwrap()[0].folder,
            Folder::Trash
        );
        store
            .apply_mail_op(&account, &op(&[gmail::INBOX], &[]))
            .unwrap();
        store
            .apply_mail_op(
                &account,
                &MailOp::Untrash {
                    threads: ids.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            store.load_threads(&account, &ids).unwrap()[0].folder,
            Folder::Inbox
        );
    }
}

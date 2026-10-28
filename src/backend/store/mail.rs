//! Mail in the store: labels, threads, messages, bodies and drafts, read
//! back into the UI's model (`data::Thread`).

use std::collections::HashMap;

use chrono::DateTime;
use rusqlite::{Connection, OptionalExtension as _, Transaction, params, params_from_iter};

use super::{Store, from_json, json};
use crate::{
    backend::{MailOp, gmail},
    data::{Account, Body, DraftOrigin, Folder, Label, Loaded, Message, Thread},
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
            tx.prepare_cached(
                "INSERT INTO labels (account, id, name, type, bg, list_visibility, message_visibility)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (account, id) DO UPDATE SET
                    name = excluded.name, type = excluded.type, bg = excluded.bg,
                    list_visibility = excluded.list_visibility,
                    message_visibility = excluded.message_visibility",
            )?
            .execute(params![
                    account,
                    label.id,
                    label.name,
                    kind,
                    crate::data::to_hex(label.color),
                    label.list_visibility.gmail(),
                    message_visibility(label.in_message_list),
            ])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The user's labels, hidden ones too (FOL-10), in the order stored.
    pub fn load_labels(&self, account: &str) -> anyhow::Result<Vec<Label>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, bg, list_visibility, message_visibility FROM labels
             WHERE account = ?1 AND type IN ('user', 'hidden') ORDER BY rowid",
        )?;
        let rows = stmt.query_map([account], |r| {
            let color = r
                .get::<_, Option<String>>(2)?
                .and_then(|hex| crate::data::from_hex(&hex))
                .unwrap_or(gpui::hsla(0.6, 0.1, 0.5, 1.));
            Ok(Label {
                list_visibility: crate::data::ListVisibility::from_gmail(
                    r.get::<_, Option<String>>(3)?.as_deref(),
                ),
                in_message_list: r.get::<_, Option<String>>(4)?.as_deref() != Some("hide"),
                ..Label::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?, color)
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// W4b: one label as Gmail answered a create or a change.
    pub fn put_gmail_label(&self, account: &str, label: &gmail::GLabel) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        upsert_gmail_label(&tx, account, label)?;
        tx.commit()?;
        Ok(())
    }

    /// W4b: a label gone, and gone from the threads that had it, as Gmail
    /// does when it deletes one: those threads, by id. The mail stays.
    pub fn delete_label(&self, account: &str, id: &str) -> anyhow::Result<Vec<String>> {
        let me = self.me(account);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.prepare_cached("DELETE FROM labels WHERE account = ?1 AND id = ?2")?
            .execute(params![account, id])?;
        let ids: Vec<String> = tx
            .prepare_cached(
                "SELECT id FROM threads WHERE account = ?1 AND instr(label_ids, ?2) > 0",
            )?
            .query_map(params![account, json(&id)], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        if !ids.is_empty() {
            modify_in(&tx, account, &ids, &[], &[id.to_string()], &me)?;
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Store threads whole (their messages, bodies and draft), replacing
    /// what was there.
    pub fn put_threads(&self, account: &str, threads: &[Thread]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for thread in threads {
            write_thread(&tx, account, thread, |_| true)?;
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
        load_threads_in(&self.conn(), account, Which::All, true)
    }

    /// The threads among `ids` that are stored, in the order stored.
    pub fn load_threads(&self, account: &str, ids: &[String]) -> anyhow::Result<Vec<Thread>> {
        load_threads_in(&self.conn(), account, Which::Ids(ids), true)
    }

    /// Every stored thread without its bodies (`Body::NotLoaded`), for the
    /// first frame (MA-072): `load_threads` reads them on selection.
    pub fn load_all_thread_heads(&self, account: &str) -> anyhow::Result<Vec<Thread>> {
        load_threads_in(&self.conn(), account, Which::All, false)
    }

    /// The Inbox threads, and `also` (threads a queued change may bring
    /// back to it), without their bodies: the menu bar's recent mail.
    pub fn load_inbox_heads(&self, account: &str, also: &[String]) -> anyhow::Result<Vec<Thread>> {
        load_threads_in(&self.conn(), account, Which::Inbox(also), false)
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
            MailOp::SaveDraft {
                local_id,
                draft,
                revision,
            } => self
                .put_draft(account, &draft.to_draft_thread(local_id.clone()), *revision)
                .map(|_| ()),
            MailOp::DeleteDraft { local_id } => self.bury_draft(account, local_id),
            MailOp::Send {
                draft,
                local_id,
                thread_id,
            } => {
                if let Some(local_id) = local_id {
                    self.bury_draft(account, local_id)?;
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
        let me = self.me(account);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        modify_in(&tx, account, ids, add, remove, &me)?;
        tx.commit()?;
        Ok(())
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
            tx.prepare_cached("DELETE FROM labels WHERE account = ?1 AND id = ?2")?
                .execute(params![account, id])?;
        }
        for label in labels {
            upsert_gmail_label(&tx, account, label)?;
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
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut stored: HashMap<String, Message> =
            load_threads_in(&tx, account, Which::Ids(&ids), true)?
                .into_iter()
                .flat_map(|t| t.messages)
                .map(|m| (m.id.clone(), m))
                .collect();
        let mut threads = threads;
        let mut kept: std::collections::HashSet<String> = Default::default();
        for thread in &mut threads {
            for message in &mut thread.messages {
                if let Some(old) = stored.remove(&message.id)
                    && matches!(old.body, Body::Loaded { .. })
                {
                    kept.insert(message.id.clone());
                    message.body = old.body;
                    message.attachments = old.attachments;
                }
            }
            thread.loaded = loaded(thread);
            thread.reindex();
        }
        // The bodies kept are there already (MC-003).
        for thread in &threads {
            write_thread(&tx, account, thread, |m| !kept.contains(&m.id))?;
        }
        tx.commit()?;
        Ok(())
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
        let fetched: Vec<String> = bodies.iter().map(|(id, _)| id.clone()).collect();
        let mut images = Vec::new();
        for (id, parsed) in bodies {
            let Some(message) = thread.messages.iter_mut().find(|m| m.id == id) else {
                continue;
            };
            // W2 (ATT-2): inline images, with the body.
            images.extend(parsed.inline_images);
            message.body = Body::Loaded { html: parsed.html };
            message.attachments = parsed.attachments;
        }
        if !images.is_empty() {
            self.put_images(
                images
                    .iter()
                    .map(|i| (i.url.as_str(), i.mime, i.bytes.as_slice())),
                chrono::Utc::now().timestamp_millis(),
                super::IMAGE_CACHE_CAP,
            )
            .ok();
        }
        thread.loaded = loaded(&thread);
        thread.reindex();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        write_thread(&tx, account, &thread, |m| fetched.contains(&m.id))?;
        tx.commit()?;
        Ok(())
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

    /// One Drafts thread's Gmail draft id, if known, and message id.
    pub fn stored_draft(
        &self,
        account: &str,
        local_id: &str,
    ) -> anyhow::Result<Option<(Option<String>, String)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT draft_id, COALESCE(message_id, '') FROM drafts
                 WHERE account = ?1 AND local_id = ?2",
                params![account, local_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
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

    // W3 (DRF-5): revisions.

    /// The newest revision written of the draft, or left by it when it was
    /// sent or discarded; 0 for one never saved with a revision.
    pub fn draft_revision(&self, account: &str, local_id: &str) -> anyhow::Result<u64> {
        draft_revision(&self.conn(), account, local_id)
    }

    /// Whether the draft was sent or discarded at `revision` or later.
    pub fn draft_buried(&self, account: &str, local_id: &str, revision: u64) -> bool {
        draft_buried(&self.conn(), account, local_id, revision)
    }

    /// Write the draft at `revision`, unless a newer revision is stored or
    /// it was sent or discarded since. False when it wasn't written. One
    /// transaction, so an older save can't slip in between (MC-006).
    pub fn put_draft(&self, account: &str, thread: &Thread, revision: u64) -> anyhow::Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if revision > 0 && draft_revision(&tx, account, &thread.id)? > revision {
            return Ok(false);
        }
        if draft_buried(&tx, account, &thread.id, revision) {
            return Ok(false);
        }
        write_thread(&tx, account, thread, |_| true)?;
        tx.execute(
            "UPDATE drafts SET revision = ?3 WHERE account = ?1 AND local_id = ?2",
            params![account, thread.id, revision as i64],
        )?;
        // Saved again after it went (Undo after the window): it's back.
        tx.execute(
            "DELETE FROM draft_tombstones WHERE account = ?1 AND local_id = ?2",
            params![account, thread.id],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// The draft was sent or discarded: it goes, leaving its revision.
    pub fn bury_draft(&self, account: &str, local_id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let revision = draft_revision(&tx, account, local_id)?;
        tx.execute(
            "INSERT INTO draft_tombstones (account, local_id, revision) VALUES (?1, ?2, ?3)
             ON CONFLICT (account, local_id) DO UPDATE SET revision = excluded.revision",
            params![account, local_id, revision as i64],
        )?;
        delete_thread(&tx, account, local_id)?;
        tx.commit()?;
        Ok(())
    }
}

/// Gmail's `messageListVisibility`.
fn message_visibility(shown: bool) -> &'static str {
    if shown { "show" } else { "hide" }
}

/// A label as Gmail lists it, with its counts when it has them: user
/// labels (hidden ones too, FOL-10) for the sidebar, the rest for their
/// counts.
fn upsert_gmail_label(
    tx: &Transaction,
    account: &str,
    label: &gmail::GLabel,
) -> anyhow::Result<()> {
    let kind = if label.kind == "user" {
        "user"
    } else {
        "system"
    };
    let model = gmail::label_from_gmail(label);
    tx.prepare_cached(
        "INSERT INTO labels (account, id, name, type, bg, threads_unread, threads_total,
            list_visibility, message_visibility)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (account, id) DO UPDATE SET
            name = excluded.name, type = excluded.type, bg = excluded.bg,
            threads_unread = COALESCE(excluded.threads_unread, threads_unread),
            threads_total = COALESCE(excluded.threads_total, threads_total),
            list_visibility = excluded.list_visibility,
            message_visibility = excluded.message_visibility",
    )?
    .execute(params![
        account,
        label.id,
        label.name,
        kind,
        crate::data::to_hex(model.color),
        label.threads_unread.map(|n| n as i64),
        label.threads_total.map(|n| n as i64),
        model.list_visibility.gmail(),
        message_visibility(model.in_message_list),
    ])?;
    Ok(())
}

/// Add and remove labels on every message of the thread, like
/// `threads.modify`, and derive its flags, labels and folder again.
pub fn modify_thread(thread: &mut Thread, add: &[String], remove: &[String], me: &Account) {
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
    thread.derive_from_labels();
    if moves && thread.folder != Folder::Drafts {
        thread.folder = gmail::derive_folder(&thread.messages, me);
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

/// Which threads a load reads.
#[derive(Clone, Copy)]
enum Which<'a> {
    All,
    Ids(&'a [String]),
    /// The Inbox's, and these.
    Inbox(&'a [String]),
}

impl Which<'_> {
    /// The condition on `column` (a thread id), and its parameters after
    /// the account.
    fn filter(self, column: &str) -> (String, Vec<String>) {
        match self {
            Which::All => (String::new(), Vec::new()),
            Which::Ids(ids) => (
                format!(" AND {column} IN (SELECT value FROM json_each(?2))"),
                vec![json(&ids)],
            ),
            Which::Inbox(also) => (
                format!(
                    " AND {column} IN (SELECT id FROM threads WHERE account = ?1
                        AND (folder = 'inbox' OR id IN (SELECT value FROM json_each(?2))))"
                ),
                vec![json(&also)],
            ),
        }
    }
}

/// The threads `which` names, in the order stored, with their messages'
/// bodies or without (`Body::NotLoaded`).
fn load_threads_in(
    conn: &Connection,
    account: &str,
    which: Which,
    bodies: bool,
) -> anyhow::Result<Vec<Thread>> {
    let args = |extra: Vec<String>| {
        std::iter::once(account.to_string())
            .chain(extra)
            .collect::<Vec<_>>()
    };

    let mut messages: HashMap<String, Vec<Message>> = HashMap::new();
    let (filter, extra) = which.filter("m.thread_id");
    let (html, join) = if bodies {
        (
            "b.html",
            "LEFT JOIN bodies b ON b.account = m.account AND b.message_id = m.id",
        )
    } else {
        ("NULL", "")
    };
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT m.thread_id, m.id, m.label_ids, m.\"from\", m.\"to\", m.cc, m.bcc, m.date,
                m.internal_date, m.snippet, m.attachments, m.message_id, m.in_reply_to,
                m.\"references\", m.reply_to, m.size_estimate,
                {html}
         FROM messages m
         {join}
         WHERE m.account = ?1{filter}
         ORDER BY m.internal_date, m.rowid"
    ))?;
    let mut rows = stmt.query(params_from_iter(args(extra)))?;
    while let Some(r) = rows.next()? {
        let thread_id: String = r.get(0)?;
        let date: String = r.get(7)?;
        let html: Option<String> = r.get(16)?;
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
            message_id: r.get(11)?,
            in_reply_to: r.get(12)?,
            references: from_json(&r.get::<_, String>(13)?),
            reply_to: from_json(&r.get::<_, String>(14)?),
            size_estimate: r.get::<_, i64>(15)? as u64,
        };
        messages.entry(thread_id).or_default().push(message);
    }

    let mut origins: HashMap<String, Option<DraftOrigin>> = HashMap::new();
    let (filter, extra) = which.filter("local_id");
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT local_id, origin FROM drafts WHERE account = ?1{filter}"
    ))?;
    let mut rows = stmt.query(params_from_iter(args(extra)))?;
    while let Some(r) = rows.next()? {
        let origin: Option<String> = r.get(1)?;
        origins.insert(
            r.get(0)?,
            origin.and_then(|o| serde_json::from_str(&o).ok()),
        );
    }

    let mut threads = Vec::new();
    let (filter, extra) = which.filter("id");
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT id, history_id, subject, folder, unread, starred, label_ids, search_text, loaded
         FROM threads WHERE account = ?1{filter} ORDER BY rowid"
    ))?;
    let mut rows = stmt.query(params_from_iter(args(extra)))?;
    while let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
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
            important: label_ids.iter().any(|l| l == gmail::IMPORTANT),
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

/// `Store::modify` inside a transaction: only the labels and flags are
/// written (MC-003), never the messages' bodies.
fn modify_in(
    tx: &Transaction,
    account: &str,
    ids: &[String],
    add: &[String],
    remove: &[String],
    me: &Account,
) -> anyhow::Result<()> {
    for mut thread in load_threads_in(tx, account, Which::Ids(ids), false)? {
        modify_thread(&mut thread, add, remove, me);
        write_labels(tx, account, &thread)?;
    }
    Ok(())
}

/// What `modify_thread` changes, and nothing else.
fn write_labels(tx: &Transaction, account: &str, thread: &Thread) -> anyhow::Result<()> {
    tx.prepare_cached(
        "UPDATE threads SET folder = ?3, unread = ?4, starred = ?5, label_ids = ?6
         WHERE account = ?1 AND id = ?2",
    )?
    .execute(params![
        account,
        thread.id,
        thread.folder.key(),
        thread.unread,
        thread.starred,
        json(&gmail::union_labels(
            thread.messages.iter().map(|m| &m.label_ids)
        )),
    ])?;
    let mut stmt =
        tx.prepare_cached("UPDATE messages SET label_ids = ?3 WHERE account = ?1 AND id = ?2")?;
    for message in &thread.messages {
        stmt.execute(params![account, message.id, json(&message.label_ids)])?;
    }
    Ok(())
}

fn draft_revision(conn: &Connection, account: &str, local_id: &str) -> anyhow::Result<u64> {
    let stored: Option<i64> = conn
        .prepare_cached(
            "SELECT MAX(revision) FROM (
                SELECT revision FROM drafts WHERE account = ?1 AND local_id = ?2
                UNION ALL
                SELECT revision FROM draft_tombstones WHERE account = ?1 AND local_id = ?2)",
        )?
        .query_row(params![account, local_id], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(stored.unwrap_or(0) as u64)
}

fn draft_buried(conn: &Connection, account: &str, local_id: &str, revision: u64) -> bool {
    conn.prepare_cached(
        "SELECT revision FROM draft_tombstones WHERE account = ?1 AND local_id = ?2",
    )
    .and_then(|mut stmt| stmt.query_row(params![account, local_id], |r| r.get::<_, i64>(0)))
    .is_ok_and(|r| r as u64 >= revision)
}

/// Writes the thread, its messages and its draft; of the bodies loaded,
/// only those of the messages `write_body` picks.
fn write_thread(
    tx: &Transaction,
    account: &str,
    thread: &Thread,
    write_body: impl Fn(&Message) -> bool,
) -> anyhow::Result<()> {
    let Some(last) = thread.messages.last() else {
        return delete_thread(tx, account, &thread.id);
    };
    let label_ids = gmail::union_labels(thread.messages.iter().map(|m| &m.label_ids));
    tx.prepare_cached(
        "INSERT INTO threads (account, id, history_id, subject, snippet, last_date, folder,
            unread, starred, has_attachments, label_ids, search_text, loaded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT (account, id) DO UPDATE SET
            history_id = excluded.history_id, subject = excluded.subject,
            snippet = excluded.snippet, last_date = excluded.last_date,
            folder = excluded.folder, unread = excluded.unread, starred = excluded.starred,
            has_attachments = excluded.has_attachments, label_ids = excluded.label_ids,
            search_text = excluded.search_text, loaded = excluded.loaded",
    )?
    .execute(params![
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
    ])?;

    // Bodies of messages that left the thread go with them; the others
    // stay, even when this copy of the thread hasn't loaded them.
    let old: Vec<String> = tx
        .prepare_cached("SELECT id FROM messages WHERE account = ?1 AND thread_id = ?2")?
        .query_map(params![account, thread.id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut delete_body =
        tx.prepare_cached("DELETE FROM bodies WHERE account = ?1 AND message_id = ?2")?;
    for id in old
        .iter()
        .filter(|id| !thread.messages.iter().any(|m| &m.id == *id))
    {
        delete_body.execute(params![account, id])?;
    }
    tx.prepare_cached("DELETE FROM messages WHERE account = ?1 AND thread_id = ?2")?
        .execute(params![account, thread.id])?;
    let mut insert_message = tx.prepare_cached(
        "INSERT INTO messages (account, id, thread_id, internal_date, date, label_ids,
            \"from\", \"to\", cc, bcc, reply_to, subject, snippet, message_id, in_reply_to,
            \"references\", size_estimate, attachments)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
            ?17, ?18)
         ON CONFLICT (account, id) DO UPDATE SET thread_id = excluded.thread_id",
    )?;
    let mut upsert_body = tx.prepare_cached(
        "INSERT INTO bodies (account, message_id, html, bytes)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (account, message_id) DO UPDATE SET
            html = excluded.html, bytes = excluded.bytes",
    )?;
    for message in &thread.messages {
        insert_message.execute(params![
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
        ])?;
        if let Body::Loaded { html } = &message.body
            && write_body(message)
        {
            upsert_body.execute(params![account, message.id, html, html.len() as i64])?;
        }
    }

    if thread.folder == Folder::Drafts {
        tx.prepare_cached(
            "INSERT INTO drafts (account, local_id, message_id, thread_id, markdown, origin,
                updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (account, local_id) DO UPDATE SET
                message_id = excluded.message_id, thread_id = excluded.thread_id,
                markdown = excluded.markdown, origin = excluded.origin,
                updated_at = excluded.updated_at",
        )?
        .execute(params![
            account,
            thread.id,
            last.id,
            thread.origin.as_ref().and_then(|o| o.reply_to.clone()),
            thread.origin.as_ref().map(|o| o.markdown.as_str()),
            thread.origin.as_ref().map(json),
            chrono::Utc::now().timestamp_millis(),
        ])?;
    }
    Ok(())
}

fn delete_thread(tx: &Transaction, account: &str, id: &str) -> anyhow::Result<()> {
    for sql in [
        "DELETE FROM bodies WHERE account = ?1 AND message_id IN
            (SELECT id FROM messages WHERE account = ?1 AND thread_id = ?2)",
        "DELETE FROM messages WHERE thread_id = ?2 AND account = ?1",
        "DELETE FROM threads WHERE id = ?2 AND account = ?1",
        "DELETE FROM drafts WHERE local_id = ?2 AND account = ?1",
    ] {
        tx.prepare_cached(sql)?.execute(params![account, id])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::backend::{MailOp, demo, gmail, store::Store};
    use crate::data::{Body, Folder, Thread};

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
            }
        }
        let labels = store.load_labels(&account).unwrap();
        assert_eq!(
            labels.iter().map(|l| &l.id).collect::<Vec<_>>(),
            original.labels.iter().map(|l| &l.id).collect::<Vec<_>>()
        );
    }

    /// MC-001, MC-003, MC-009: heads come without bodies, label changes
    /// leave the bodies alone, and the Inbox heads are the Inbox's.
    #[test]
    fn heads_and_label_changes_keep_bodies() {
        let (store, account) = seeded();
        let all = store.load_all_threads(&account).unwrap();
        let heads = store.load_all_thread_heads(&account).unwrap();
        assert_eq!(heads.len(), all.len());
        for (a, h) in all.iter().zip(&heads) {
            assert_eq!(a.id, h.id);
            assert_eq!(a.search.all, h.search.all);
            assert_eq!(a.messages.len(), h.messages.len());
            assert!(h.messages.iter().all(|m| m.body == Body::NotLoaded));
        }

        let inbox: Vec<&Thread> = all.iter().filter(|t| t.folder == Folder::Inbox).collect();
        let other = all.iter().find(|t| t.folder != Folder::Inbox).unwrap();
        let heads = store
            .load_inbox_heads(&account, std::slice::from_ref(&other.id))
            .unwrap();
        assert_eq!(heads.len(), inbox.len() + 1);
        assert!(heads.iter().any(|t| t.id == other.id));

        let thread = inbox[0];
        let ids = vec![thread.id.clone()];
        store
            .apply_mail_op(
                &account,
                &MailOp::Modify {
                    threads: ids.clone(),
                    add: vec![gmail::STARRED.into()],
                    remove: vec![gmail::INBOX.into()],
                },
            )
            .unwrap();
        let after = &store.load_threads(&account, &ids).unwrap()[0];
        assert!(after.starred && after.folder != Folder::Inbox);
        assert_eq!(after.search.all, thread.search.all);
        for (m, n) in thread.messages.iter().zip(&after.messages) {
            assert_eq!(m.body, n.body);
            assert!(n.label_ids.iter().any(|l| l == gmail::STARRED));
        }
        assert!(store.load_threads(&account, &[]).unwrap().is_empty());
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

//! Drafts and sending on Gmail (W6): what `SaveDraft`, `DeleteDraft` and
//! `Send` do once the outbox lets them go, and opening a Gmail draft.
//!
//! - SaveDraft: the draft is stored here first, then `drafts.create` or
//!   `drafts.update` (DRF-1). A failure keeps the local copy.
//! - DeleteDraft: `drafts.delete`, then the local copy goes (UND-2).
//! - Send: `drafts.send` for a draft saved in Gmail, else `messages.send`
//!   (CMP-8). The sent copy is stored under Gmail's ids, so the history
//!   poll finds it already there (CMP-13). A failure puts the draft back in
//!   Drafts with "Not sent: <reason>" (CMP-12).
//! - No network or no sign-in: the error is the `TransportError`, and the
//!   op stays queued with nothing rolled back (STA-3).
//! - Opening a Gmail draft: `drafts.get format=raw`, the markdown recovered
//!   from a draft written here (DRF-3, DRF-6).

use std::sync::Arc;

use anyhow::anyhow;
use gpui::Context;

use super::{
    Backend, BackendEvent, MailOp, Outgoing, Source,
    gmail::{self, DraftRef, GMessage, Gmail, GmailError},
    http::TransportError,
    mime,
    outbox::UserPriority,
    store::Store,
};
use crate::data::{Account, Body, DraftOrigin, Folder, Thread};

/// Whether `commit_compose` takes the op.
pub fn handles(op: &MailOp) -> bool {
    matches!(
        op,
        MailOp::SaveDraft { .. } | MailOp::DeleteDraft { .. } | MailOp::Send { .. }
    )
}

/// Commit a draft or send op to Gmail: whether it went, and what changed
/// for the app to reload, failed or not.
pub fn commit_compose(
    gmail: &Gmail,
    store: &Store,
    account: &str,
    op: &MailOp,
) -> (anyhow::Result<()>, Vec<BackendEvent>) {
    let mut events = Vec::new();
    let result = match op {
        MailOp::SaveDraft {
            local_id,
            draft,
            revision,
        } => save_draft(gmail, store, account, local_id, draft, *revision),
        MailOp::DeleteDraft { local_id } => delete_draft(gmail, store, account, local_id),
        MailOp::Send {
            draft,
            local_id,
            thread_id,
        } => send(
            gmail,
            store,
            account,
            draft,
            local_id.as_deref(),
            thread_id,
            &mut events,
        ),
        _ => Err(anyhow!("not a draft or a send")),
    };
    (result, events)
}

/// The Gmail draft id stored for a Drafts thread, and its message's id.
fn stored_draft(store: &Store, account: &str, local_id: &str) -> Option<(String, String)> {
    store
        .stored_drafts(account)
        .ok()?
        .into_iter()
        .find(|(local, _, _)| local == local_id)
        .and_then(|(_, draft_id, message)| Some((draft_id?, message)))
}

/// A thread id Gmail knows: not one made up here.
fn gmail_thread(id: Option<&str>) -> Option<&str> {
    id.filter(|id| !id.starts_with("sent-local-") && !id.starts_with("draft-"))
}

/// The headers that thread a reply: from the last message of the thread it
/// answers, drafts left out.
fn threading(store: &Store, account: &str, draft: &Outgoing) -> Option<mime::Threading> {
    let id = draft.reply_to.clone()?;
    let thread = store.load_threads(account, &[id]).ok()?.pop()?;
    let parent = thread
        .messages
        .iter()
        .rev()
        .find(|m| !m.label_ids.iter().any(|l| l == gmail::DRAFT) && !m.message_id.is_empty())?;
    mime::reply_threading(parent)
}

/// The files, read: from disk, or for a draft saved in Gmail, from it.
fn read_files(
    gmail: &Gmail,
    draft: &Outgoing,
    draft_id: Option<&str>,
) -> anyhow::Result<Vec<(crate::data::Attachment, Vec<u8>)>> {
    let total: u64 = draft.attachments.iter().map(|a| a.size_bytes).sum();
    if total > crate::ui::composer::ATTACHMENT_LIMIT {
        return Err(anyhow!(crate::ui::composer::ATTACHMENT_LIMIT_ERROR));
    }
    let mut saved: Option<Vec<u8>> = None;
    // W2 (ATT-7): the originals a forward's files come from, by id.
    let mut originals: Vec<(String, Vec<u8>)> = Vec::new();
    let mut files = Vec::new();
    for attachment in &draft.attachments {
        let on_disk = (!attachment.path.is_empty())
            .then(|| std::fs::read(&attachment.path).ok())
            .flatten();
        let original = &attachment.message_id;
        let bytes = if on_disk.is_some() {
            on_disk
        } else if !original.is_empty() && !attachment.part_id.is_empty() {
            if !originals.iter().any(|(id, _)| id == original) {
                match super::attachments::fetch_raw(gmail, original) {
                    Ok(raw) => originals.push((original.clone(), raw)),
                    Err(e) => match e.later() {
                        Some(e) => return Err(e.into()),
                        None => originals.push((original.clone(), Vec::new())),
                    },
                }
            }
            originals
                .iter()
                .find(|(id, _)| id == original)
                .and_then(|(_, raw)| mime::part_bytes(raw, &attachment.part_id))
        } else if let Some(id) = draft_id {
            if saved.is_none() {
                saved = match gmail.call::<DraftRef>(&gmail::draft_get_raw_request(id)) {
                    Ok(d) => d.message.raw.and_then(|raw| mime::decode_raw(&raw)),
                    Err(e) => match e.later() {
                        Some(e) => return Err(e.into()),
                        None => None,
                    },
                };
            }
            saved
                .as_deref()
                .and_then(|raw| mime::attachment_bytes(raw, &attachment.name))
        } else {
            None
        };
        let bytes = bytes.ok_or_else(|| anyhow!("couldn't read “{}”", attachment.name))?;
        files.push((attachment.clone(), bytes));
    }
    Ok(files)
}

fn build(
    store: &Store,
    account: &str,
    draft: &Outgoing,
    is_draft: bool,
    files: Vec<(crate::data::Attachment, Vec<u8>)>,
) -> (Vec<u8>, String) {
    let message_id = mime::new_message_id(&draft.from.email);
    let raw = mime::build_message(
        draft,
        &mime::Build {
            message_id: message_id.clone(),
            date: chrono::Utc::now().timestamp(),
            threading: threading(store, account, draft),
            draft: is_draft,
            files,
        },
    );
    (raw, message_id)
}

/// The draft as a Drafts thread, under the message id Gmail has for it, so
/// the drafts sync sees it unchanged.
fn draft_thread(draft: &Outgoing, local_id: &str, message_id: Option<String>) -> Thread {
    let mut thread = draft.to_draft_thread(local_id.to_string());
    if let (Some(id), Some(message)) = (message_id, thread.messages.last_mut())
        && !id.is_empty()
    {
        message.id = id;
    }
    thread
}

/// Gmail's reason in plain words.
fn reason(e: &GmailError) -> String {
    match e {
        GmailError::Status { message, .. } if !message.is_empty() => message.clone(),
        e => e.to_string(),
    }
}

/// `what` failed for `e`; no network or no sign-in comes back as the
/// `TransportError`, which keeps the op queued.
fn refused(what: &str, e: GmailError) -> anyhow::Error {
    match e.later() {
        Some(e) => e.into(),
        None if what.is_empty() => anyhow!("{}", reason(&e)),
        None => anyhow!("{what}: {}", reason(&e)),
    }
}

/// Whether `e` is no network or no sign-in: the op goes again later.
pub fn is_later(e: &anyhow::Error) -> bool {
    e.downcast_ref::<TransportError>().is_some()
}

fn save_draft(
    gmail: &Gmail,
    store: &Store,
    account: &str,
    local_id: &str,
    draft: &Outgoing,
    revision: u64,
) -> anyhow::Result<()> {
    // W3 (DRF-5): sent or discarded since, it stays gone.
    if store.draft_buried(account, local_id, revision) {
        return Ok(());
    }
    let stored = stored_draft(store, account, local_id);
    // Here first: the text is never lost, whatever Gmail says. A newer
    // local save stays as it is.
    store.put_draft(
        account,
        &draft_thread(draft, local_id, stored.as_ref().map(|s| s.1.clone())),
        revision,
    )?;
    let files = read_files(gmail, draft, stored.as_ref().map(|s| s.0.as_str()))?;
    let (raw, _) = build(store, account, draft, true, files);
    let thread_id = gmail_thread(draft.reply_to.as_deref());
    let request = match &stored {
        Some((id, _)) => gmail::draft_update_request(id, &raw, thread_id),
        None => gmail::draft_create_request(&raw, thread_id),
    };
    let saved: DraftRef = gmail
        .call(&request)
        .map_err(|e| refused("Couldn't save the draft to Gmail", e))?;
    // The answer names Gmail's message; the text kept is the newest one
    // stored, which may be newer than this save's.
    let Some(mut thread) = store
        .load_threads(account, &[local_id.to_string()])?
        .pop()
        .filter(|t| t.folder == Folder::Drafts)
    else {
        return Ok(());
    };
    if let Some(message) = thread.messages.last_mut() {
        message.id = saved.message.id;
    }
    store.put_threads(account, &[thread])?;
    store.set_draft_id(account, local_id, &saved.id)?;
    Ok(())
}

/// W3 (DRF-5): a save while typing, here only; Gmail gets it with the
/// `SaveDraft` op that follows. False when a newer save, a send or a
/// discard came first.
pub fn save_locally(
    store: &Store,
    account: &str,
    local_id: &str,
    draft: &Outgoing,
    revision: u64,
) -> anyhow::Result<bool> {
    let message = stored_draft(store, account, local_id).map(|s| s.1);
    let message = message.or_else(|| {
        // Not in Gmail yet: keep the message id it was stored under.
        store
            .load_threads(account, &[local_id.to_string()])
            .ok()?
            .pop()
            .map(|t| t.last_message().id.clone())
    });
    store.put_draft(account, &draft_thread(draft, local_id, message), revision)
}

fn delete_draft(gmail: &Gmail, store: &Store, account: &str, local_id: &str) -> anyhow::Result<()> {
    let stored = stored_draft(store, account, local_id);
    let result = match stored {
        Some((id, _)) => gmail
            .call_empty(&gmail::draft_delete_request(&id))
            .map_err(|e| refused("Couldn't delete the draft in Gmail", e)),
        None => Ok(()),
    };
    // Kept while it waits to go again, so the retry knows the Gmail draft.
    if !result.as_ref().is_err_and(is_later) {
        store.bury_draft(account, local_id)?;
    }
    result
}

fn send(
    gmail: &Gmail,
    store: &Store,
    account: &str,
    draft: &Outgoing,
    local_id: Option<&str>,
    thread_id: &str,
    events: &mut Vec<BackendEvent>,
) -> anyhow::Result<()> {
    let stored = local_id.and_then(|id| stored_draft(store, account, id));
    let sent = read_files(gmail, draft, stored.as_ref().map(|s| s.0.as_str())).and_then(|files| {
        let (raw, message_id) = build(store, account, draft, false, files);
        let joins = gmail_thread(draft.reply_to.as_deref());
        let request = match &stored {
            Some((id, _)) => gmail::draft_send_request(id, &raw, joins),
            None => gmail::send_request(&raw, joins),
        };
        gmail
            .call::<GMessage>(&request)
            .map(|m| (m, message_id))
            .map_err(|e| refused("", e))
    });
    let (sent, message_id) = match sent {
        Ok(sent) => sent,
        // No network or no sign-in: it stays queued, still shown as sent.
        Err(e) if is_later(&e) => return Err(e),
        Err(why) => {
            // Back to Drafts, and out of the conversation it was shown in.
            let id = local_id.map_or_else(
                || format!("draft-local-{}", uuid::Uuid::new_v4().simple()),
                str::to_string,
            );
            store.put_threads(account, &[draft_thread(draft, &id, stored.map(|s| s.1))])?;
            events.push(BackendEvent::ThreadsChanged(vec![
                id,
                thread_id.to_string(),
            ]));
            return Err(anyhow!("Not sent: {why}"));
        }
    };

    if let Some(local) = local_id {
        store.bury_draft(account, local)?;
    }
    // W2: a forward's held copies aren't needed any more.
    super::attachments::Files::in_data_dir().release(&draft.attachments);
    let labels = if sent.label_ids.is_empty() {
        vec![gmail::SENT.to_string()]
    } else {
        sent.label_ids.clone()
    };
    let mut message = draft.to_message(labels);
    message.id = sent.id.clone();
    message.message_id = message_id;
    if let Some(t) = threading(store, account, draft) {
        message.in_reply_to = t.in_reply_to;
        message.references = t.references;
    }
    let new_id = if sent.thread_id.is_empty() {
        thread_id.to_string()
    } else {
        sent.thread_id.clone()
    };
    let thread = match store
        .load_threads(account, std::slice::from_ref(&new_id))?
        .pop()
    {
        Some(mut thread) => {
            thread.messages.retain(|m| m.id != message.id);
            thread.messages.push(message);
            rederive(&mut thread, &store.me(account));
            thread
        }
        None => {
            let mut thread = Thread::local(
                new_id.clone(),
                draft.subject.clone(),
                Folder::Sent,
                vec![message],
                None,
            );
            rederive(&mut thread, &store.me(account));
            thread
        }
    };
    store.put_threads(account, &[thread])?;
    if new_id != thread_id {
        events.push(BackendEvent::IdReplaced {
            old: thread_id.to_string(),
            new: new_id.clone(),
        });
    }
    events.push(BackendEvent::ThreadsChanged(vec![new_id]));
    Ok(())
}

/// Folder, flags and labels from the messages' labels, as sync derives them.
fn rederive(thread: &mut Thread, me: &Account) {
    thread.folder = gmail::derive_folder(&thread.messages, me);
    thread.label_ids = gmail::union_labels(thread.messages.iter().map(|m| &m.label_ids));
    thread.unread = thread.label_ids.iter().any(|l| l == gmail::UNREAD);
    thread.starred = thread.label_ids.iter().any(|l| l == gmail::STARRED);
    thread.labels = thread
        .label_ids
        .iter()
        .filter(|l| !gmail::is_system_label(l))
        .cloned()
        .collect();
    thread.reindex();
}

/// A Gmail draft's text and files, from `drafts.get format=raw`, stored on
/// its Drafts thread with the markdown recovered.
fn load_draft(
    gmail: &Gmail,
    store: &Store,
    account: &str,
    thread_id: &str,
    draft_id: &str,
) -> anyhow::Result<()> {
    let fetched: DraftRef = gmail
        .call(&gmail::draft_get_raw_request(draft_id))
        .map_err(|e| anyhow!("{e}"))?;
    let parsed = fetched
        .message
        .raw
        .as_deref()
        .and_then(mime::decode_raw)
        .and_then(|raw| mime::parse_draft(&raw))
        .ok_or_else(|| anyhow!("the draft couldn't be read"))?;
    let Some(mut thread) = store.load_threads(account, &[thread_id.to_string()])?.pop() else {
        return Ok(());
    };
    let Some(message) = thread.messages.last_mut() else {
        return Ok(());
    };
    message.body = Body::Loaded { html: parsed.html };
    message.attachments = parsed.attachments;
    let mut origin = thread.origin.take().unwrap_or(DraftOrigin {
        kind: crate::app::ComposeKind::New,
        markdown: String::new(),
        reply_to: None,
        quoted: None,
        signature: false,
    });
    origin.markdown = parsed.markdown;
    // W3 (MST-2): its signature was taken out of the text.
    origin.signature = parsed.signature;
    thread.origin = Some(origin);
    thread.loaded = crate::data::Loaded::Full;
    thread.reindex();
    store.put_threads(account, &[thread])
}

impl Backend {
    /// Opening a Gmail draft whose text isn't here yet: fetch it whole
    /// (DRF-3). True when it's a draft, so the message bodies aren't
    /// fetched the other way too.
    pub(super) fn fetch_draft(&mut self, thread_id: &str, cx: &mut Context<Self>) -> bool {
        if self.source != Source::Google {
            return false;
        }
        let Some(thread) = self.load_threads(&[thread_id.to_string()]).pop() else {
            return false;
        };
        if thread.folder != Folder::Drafts {
            return false;
        }
        if matches!(thread.last_message().body, Body::Loaded { .. }) {
            return true;
        }
        let account = self.email().to_string();
        let (Some((draft_id, _)), Some(transport)) = (
            stored_draft(&self.store, &account, thread_id),
            self.transport(),
        ) else {
            return true;
        };
        let store = self.store.clone();
        let id = thread_id.to_string();
        cx.spawn(async move |this, cx| {
            let tid = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let gmail = Gmail::new(Arc::new(UserPriority(transport)));
                    load_draft(&gmail, &store, &account, &tid, &draft_id)
                })
                .await;
            match result {
                Ok(()) => {
                    this.update(cx, |_, cx| cx.emit(BackendEvent::ThreadsChanged(vec![id])))
                        .ok();
                }
                Err(e) => eprintln!("Pigeon: couldn't open the draft: {e:#}"),
            }
        })
        .detach();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::ComposeKind,
        backend::http::{FakeTransport, Request},
        data::{Contact, Message},
    };

    const ME: &str = "me@example.com";

    fn me() -> Contact {
        Contact {
            name: "Me".into(),
            email: ME.into(),
        }
    }

    fn outgoing(subject: &str) -> Outgoing {
        Outgoing {
            kind: ComposeKind::New,
            from: me(),
            to: vec![Contact {
                name: "Ana".into(),
                email: "ana@example.com".into(),
            }],
            subject: subject.into(),
            markdown: "Hi **Ana**".into(),
            html: "<p>Hi <strong>Ana</strong></p>".into(),
            ..Outgoing::default()
        }
    }

    fn gmail(fake: FakeTransport) -> (Gmail, Arc<FakeTransport>) {
        let fake = Arc::new(fake);
        (Gmail::new(fake.clone()), fake)
    }

    fn raw_of(request: &Request) -> Vec<u8> {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let raw = body["raw"]
            .as_str()
            .or(body["message"]["raw"].as_str())
            .unwrap();
        mime::decode_raw(raw).unwrap()
    }

    /// A thread from Gmail with one message from Ana.
    fn inbox_thread(store: &Store) {
        let mut m = Message::new(Contact {
            name: "Ana".into(),
            email: "ana@example.com".into(),
        });
        m.id = "m1".into();
        m.label_ids = vec![gmail::INBOX.into()];
        m.message_id = "m1@example.com".into();
        m.references = vec!["m0@example.com".into()];
        let mut t = Thread::local("t1".into(), "Lunch".into(), Folder::Inbox, vec![m], None);
        t.label_ids = vec![gmail::INBOX.into()];
        store.put_threads(ME, &[t]).unwrap();
    }

    #[test]
    fn a_new_message_goes_through_messages_send_and_takes_gmails_ids() {
        let store = Store::open_in_memory().unwrap();
        let (gmail, fake) = gmail(FakeTransport::new().on(
            "POST",
            "/gmail/v1/users/me/messages/send",
            &[],
            &[(200, r#"{"id":"s1","threadId":"T1","labelIds":["SENT"]}"#)],
        ));
        let op = MailOp::Send {
            draft: outgoing("[Pigeon test] new"),
            local_id: None,
            thread_id: "sent-local-1".into(),
        };
        let (result, events) = commit_compose(&gmail, &store, ME, &op);
        result.unwrap();
        assert_eq!(
            events,
            [
                BackendEvent::IdReplaced {
                    old: "sent-local-1".into(),
                    new: "T1".into()
                },
                BackendEvent::ThreadsChanged(vec!["T1".into()])
            ]
        );
        let request = &fake.requests()[0];
        assert_eq!(request.cost, 100);
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert!(body.get("threadId").is_none());
        let raw = String::from_utf8(raw_of(request)).unwrap();
        assert!(!raw.contains(mime::MARKDOWN_HEADER));
        // The sent copy is stored under Gmail's ids, with its Message-ID.
        let t = &store.load_threads(ME, &["T1".into()]).unwrap()[0];
        assert_eq!(t.folder, Folder::Sent);
        let m = t.last_message();
        assert_eq!(m.id, "s1");
        assert!(raw.contains(&format!("Message-ID: <{}>", m.message_id)));
    }

    #[test]
    fn a_reply_threads_and_joins_the_conversation() {
        let store = Store::open_in_memory().unwrap();
        inbox_thread(&store);
        let (gmail, fake) = gmail(FakeTransport::new().on(
            "POST",
            "/gmail/v1/users/me/messages/send",
            &[],
            &[(200, r#"{"id":"s2","threadId":"t1","labelIds":["SENT"]}"#)],
        ));
        let draft = Outgoing {
            kind: ComposeKind::Reply,
            reply_to: Some("t1".into()),
            ..outgoing("Re: Lunch")
        };
        let op = MailOp::Send {
            draft,
            local_id: None,
            thread_id: "t1".into(),
        };
        let (result, events) = commit_compose(&gmail, &store, ME, &op);
        result.unwrap();
        assert_eq!(events, [BackendEvent::ThreadsChanged(vec!["t1".into()])]);
        let request = &fake.requests()[0];
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["threadId"], "t1");
        let raw = String::from_utf8(raw_of(request)).unwrap();
        assert!(raw.contains("In-Reply-To: <m1@example.com>"), "{raw}");
        assert!(raw.contains("References: <m0@example.com> <m1@example.com>"));
        let t = &store.load_threads(ME, &["t1".into()]).unwrap()[0];
        assert_eq!(t.messages.len(), 2);
        assert_eq!(t.folder, Folder::Inbox);
        assert_eq!(t.last_message().in_reply_to, "m1@example.com");
    }

    #[test]
    fn a_failed_send_goes_back_to_drafts() {
        let store = Store::open_in_memory().unwrap();
        let (gmail, _) = gmail(FakeTransport::new().on(
            "POST",
            "/gmail/v1/users/me/messages/send",
            &[],
            &[(
                400,
                r#"{"error":{"code":400,"message":"Invalid To header"}}"#,
            )],
        ));
        let op = MailOp::Send {
            draft: outgoing("Plan"),
            local_id: Some("draft-local-9".into()),
            thread_id: "sent-local-2".into(),
        };
        let (result, events) = commit_compose(&gmail, &store, ME, &op);
        assert_eq!(
            result.unwrap_err().to_string(),
            "Not sent: Invalid To header"
        );
        assert_eq!(
            events,
            [BackendEvent::ThreadsChanged(vec![
                "draft-local-9".into(),
                "sent-local-2".into()
            ])]
        );
        let t = &store.load_threads(ME, &["draft-local-9".into()]).unwrap()[0];
        assert_eq!(t.folder, Folder::Drafts);
        assert_eq!(t.origin.as_ref().unwrap().markdown, "Hi **Ana**");
        assert!(
            store
                .load_threads(ME, &["sent-local-2".into()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn drilled_send_failures_go_back_to_drafts_with_the_reason() {
        use crate::backend::http::{CONFLICT_BODY, REFUSAL_BODY};
        for (status, body, toast) in [
            (400, REFUSAL_BODY, "Not sent: Simulated refusal (drill)"),
            (412, CONFLICT_BODY, "Not sent: Precondition Failed"),
        ] {
            let store = Store::open_in_memory().unwrap();
            let (gmail, _) = gmail(FakeTransport::new().on(
                "POST",
                "/gmail/v1/users/me/messages/send",
                &[],
                &[(status, body)],
            ));
            let op = MailOp::Send {
                draft: outgoing("Plan"),
                local_id: None,
                thread_id: "sent-local-3".into(),
            };
            let (result, events) = commit_compose(&gmail, &store, ME, &op);
            assert_eq!(result.unwrap_err().to_string(), toast);
            let BackendEvent::ThreadsChanged(ids) = &events[0] else {
                panic!("{events:?}");
            };
            let t = &store.load_threads(ME, &ids[..1]).unwrap()[0];
            assert_eq!(t.folder, Folder::Drafts);
        }
    }

    #[test]
    fn an_offline_send_stays_queued_and_rolls_nothing_back() {
        let store = Store::open_in_memory().unwrap();
        let (gmail, _) = gmail(FakeTransport::new().on(
            "POST",
            "/gmail/v1/users/me/messages/send",
            &[],
            &[(0, "")],
        ));
        let op = MailOp::Send {
            draft: outgoing("Plan"),
            local_id: Some("draft-local-9".into()),
            thread_id: "sent-local-2".into(),
        };
        let (result, events) = commit_compose(&gmail, &store, ME, &op);
        assert!(is_later(&result.unwrap_err()));
        assert!(events.is_empty());
        assert!(
            store
                .load_threads(ME, &["draft-local-9".into()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn drafts_are_created_then_updated_then_sent_as_drafts() {
        let store = Store::open_in_memory().unwrap();
        let (gmail, fake) = gmail(
            FakeTransport::new()
                .on(
                    "POST",
                    "/gmail/v1/users/me/drafts",
                    &[],
                    &[(
                        200,
                        r#"{"id":"r1","message":{"id":"dm1","threadId":"dm1"}}"#,
                    )],
                )
                .on(
                    "PUT",
                    "/gmail/v1/users/me/drafts/r1",
                    &[],
                    &[(
                        200,
                        r#"{"id":"r1","message":{"id":"dm2","threadId":"dm1"}}"#,
                    )],
                )
                .on(
                    "POST",
                    "/gmail/v1/users/me/drafts/send",
                    &[],
                    &[(200, r#"{"id":"s3","threadId":"dm1","labelIds":["SENT"]}"#)],
                ),
        );
        let save = |subject: &str| MailOp::SaveDraft {
            local_id: "draft-local-1".into(),
            draft: outgoing(subject),
            revision: 0,
        };
        commit_compose(&gmail, &store, ME, &save("One")).0.unwrap();
        assert_eq!(
            store.stored_drafts(ME).unwrap(),
            [("draft-local-1".into(), Some("r1".into()), "dm1".into())]
        );
        let raw = String::from_utf8(raw_of(&fake.requests()[0])).unwrap();
        assert!(raw.contains("X-Pigeon-Markdown: 1"));
        commit_compose(&gmail, &store, ME, &save("Two")).0.unwrap();
        assert_eq!(fake.requests()[1].method, "PUT");
        assert_eq!(store.stored_drafts(ME).unwrap()[0].2, "dm2");

        let send = MailOp::Send {
            draft: outgoing("Two"),
            local_id: Some("draft-local-1".into()),
            thread_id: "sent-local-3".into(),
        };
        let (result, events) = commit_compose(&gmail, &store, ME, &send);
        result.unwrap();
        let request = &fake.requests()[2];
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["id"], "r1");
        assert!(events.contains(&BackendEvent::IdReplaced {
            old: "sent-local-3".into(),
            new: "dm1".into()
        }));
        assert!(store.stored_drafts(ME).unwrap().is_empty());
    }

    #[test]
    fn a_failed_save_keeps_the_text() {
        let store = Store::open_in_memory().unwrap();
        let (gmail, _) =
            gmail(FakeTransport::new().on("POST", "/gmail/v1/users/me/drafts", &[], &[(0, "")]));
        let op = MailOp::SaveDraft {
            local_id: "draft-local-2".into(),
            draft: outgoing("Kept"),
            revision: 0,
        };
        assert!(commit_compose(&gmail, &store, ME, &op).0.is_err());
        let t = &store.load_threads(ME, &["draft-local-2".into()]).unwrap()[0];
        assert_eq!(t.origin.as_ref().unwrap().markdown, "Hi **Ana**");
    }

    #[test]
    fn discarding_deletes_the_gmail_draft() {
        let store = Store::open_in_memory().unwrap();
        let (gmail, fake) = gmail(
            FakeTransport::new()
                .on(
                    "POST",
                    "/gmail/v1/users/me/drafts",
                    &[],
                    &[(200, r#"{"id":"r9","message":{"id":"dm9"}}"#)],
                )
                .on("DELETE", "/gmail/v1/users/me/drafts/r9", &[], &[(204, "")]),
        );
        let save = MailOp::SaveDraft {
            local_id: "draft-local-5".into(),
            draft: outgoing("Bye"),
            revision: 0,
        };
        commit_compose(&gmail, &store, ME, &save).0.unwrap();
        let delete = MailOp::DeleteDraft {
            local_id: "draft-local-5".into(),
        };
        commit_compose(&gmail, &store, ME, &delete).0.unwrap();
        assert_eq!(fake.requests()[1].method, "DELETE");
        assert!(store.load_all_threads(ME).unwrap().is_empty());
    }

    #[test]
    fn opening_a_gmail_draft_recovers_its_markdown() {
        use base64::Engine as _;
        let store = Store::open_in_memory().unwrap();
        // As the drafts sync stores it: headers only.
        let listed = gmail::draft_thread(
            "r4",
            &GMessage {
                id: "dm4".into(),
                thread_id: "dm4".into(),
                label_ids: vec![gmail::DRAFT.into()],
                ..Default::default()
            },
        );
        store.put_threads(ME, &[listed]).unwrap();
        store.set_draft_id(ME, "draft-r4", "r4").unwrap();
        let draft = Outgoing {
            markdown: "- one\n- **two**".into(),
            html: String::new(),
            ..outgoing("Plan")
        };
        let raw = mime::build_message(
            &draft,
            &mime::Build {
                message_id: "x@example.com".into(),
                draft: true,
                ..Default::default()
            },
        );
        let encoded = base64::engine::general_purpose::URL_SAFE.encode(&raw);
        let answer = format!(r#"{{"id":"r4","message":{{"id":"dm4","raw":"{encoded}"}}}}"#);
        let (gmail, fake) = gmail(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/drafts/r4",
            &[("format", "raw")],
            &[(200, &answer)],
        ));
        load_draft(&gmail, &store, ME, "draft-r4", "r4").unwrap();
        assert_eq!(fake.requests()[0].cost, 5);
        let t = &store.load_threads(ME, &["draft-r4".into()]).unwrap()[0];
        assert_eq!(t.origin.as_ref().unwrap().markdown, "- one\n- **two**");
        assert!(t.last_message().html().contains("<strong>two</strong>"));
        // The drafts sync still sees it unchanged.
        assert_eq!(store.stored_drafts(ME).unwrap()[0].2, "dm4");
    }

    // W3 (MST-1, MST-2, DRF-5).

    fn save_op(local_id: &str, subject: &str, revision: u64) -> MailOp {
        MailOp::SaveDraft {
            local_id: local_id.into(),
            draft: outgoing(subject),
            revision,
        }
    }

    fn create_answer(fake: FakeTransport, draft: &str, message: &str) -> FakeTransport {
        fake.on(
            "POST",
            "/gmail/v1/users/me/drafts",
            &[],
            &[(
                200,
                &format!(r#"{{"id":"{draft}","message":{{"id":"{message}"}}}}"#),
            )],
        )
    }

    #[test]
    fn typing_during_a_save_keeps_the_newer_text() {
        let store = Store::open_in_memory().unwrap();
        let (gmail, fake) = gmail(create_answer(FakeTransport::new(), "r1", "dm1"));
        // Revision 2 is saved here while revision 1 is on its way to Gmail.
        assert!(save_locally(&store, ME, "draft-local-1", &outgoing("Newer"), 2).unwrap());
        commit_compose(&gmail, &store, ME, &save_op("draft-local-1", "Older", 1))
            .0
            .unwrap();
        assert_eq!(fake.requests().len(), 1);
        let t = &store.load_threads(ME, &["draft-local-1".into()]).unwrap()[0];
        assert_eq!(t.subject, "Newer", "the stale answer kept the newer text");
        // Gmail's ids are kept, so the drafts sync sees it unchanged.
        assert_eq!(
            store.stored_drafts(ME).unwrap(),
            [("draft-local-1".into(), Some("r1".into()), "dm1".into())]
        );
        assert_eq!(store.draft_revision(ME, "draft-local-1").unwrap(), 2);
        // An older local save loses too.
        assert!(!save_locally(&store, ME, "draft-local-1", &outgoing("Oldest"), 1).unwrap());
    }

    #[test]
    fn a_save_after_the_send_never_brings_the_draft_back() {
        let store = Store::open_in_memory().unwrap();
        let fake = create_answer(FakeTransport::new(), "r1", "dm1").on(
            "POST",
            "/gmail/v1/users/me/drafts/send",
            &[],
            &[(200, r#"{"id":"s1","threadId":"T1","labelIds":["SENT"]}"#)],
        );
        let (gmail, fake) = gmail(fake);
        commit_compose(&gmail, &store, ME, &save_op("draft-local-1", "Hi", 3))
            .0
            .unwrap();
        let send = MailOp::Send {
            draft: outgoing("Hi"),
            local_id: Some("draft-local-1".into()),
            thread_id: "sent-local-1".into(),
        };
        commit_compose(&gmail, &store, ME, &send).0.unwrap();
        assert!(fake.requests()[1].url.contains("drafts/send"));
        // A late save of the same or an older revision does nothing.
        commit_compose(&gmail, &store, ME, &save_op("draft-local-1", "Hi", 3))
            .0
            .unwrap();
        assert!(!save_locally(&store, ME, "draft-local-1", &outgoing("Hi"), 2).unwrap());
        assert_eq!(fake.requests().len(), 2);
        assert!(
            store
                .load_threads(ME, &["draft-local-1".into()])
                .unwrap()
                .is_empty()
        );
        assert!(store.stored_drafts(ME).unwrap().is_empty());
    }

    #[test]
    fn a_save_after_the_discard_never_brings_the_draft_back() {
        let store = Store::open_in_memory().unwrap();
        let fake = create_answer(FakeTransport::new(), "r9", "dm9").on(
            "DELETE",
            "/gmail/v1/users/me/drafts/r9",
            &[],
            &[(204, "")],
        );
        let (gmail, fake) = gmail(fake);
        commit_compose(&gmail, &store, ME, &save_op("draft-local-5", "Bye", 5))
            .0
            .unwrap();
        let delete = MailOp::DeleteDraft {
            local_id: "draft-local-5".into(),
        };
        commit_compose(&gmail, &store, ME, &delete).0.unwrap();
        commit_compose(&gmail, &store, ME, &save_op("draft-local-5", "Bye", 4))
            .0
            .unwrap();
        assert_eq!(fake.requests().len(), 2);
        assert!(store.load_all_threads(ME).unwrap().is_empty());
        // Undo after the window saves it again, newer: it's back.
        assert!(save_locally(&store, ME, "draft-local-5", &outgoing("Bye"), 6).unwrap());
        assert_eq!(store.load_all_threads(ME).unwrap().len(), 1);
    }

    #[test]
    fn a_draft_saved_while_typing_survives_a_crash() {
        let path =
            std::env::temp_dir().join(format!("pigeon-drf5-{}.sqlite", uuid::Uuid::new_v4()));
        {
            let store = Store::open(&path).unwrap();
            let draft = Outgoing {
                from: Contact {
                    name: "Work Me".into(),
                    email: "alias@example.com".into(),
                },
                signature: "<b>Me</b>".into(),
                ..outgoing("Unsent")
            };
            assert!(save_locally(&store, ME, "draft-local-7", &draft, 10).unwrap());
            // Quit without closing: nothing else runs.
        }
        let store = Store::open(&path).unwrap();
        let t = &store.load_threads(ME, &["draft-local-7".into()]).unwrap()[0];
        assert_eq!(t.folder, Folder::Drafts);
        let origin = t.origin.as_ref().unwrap();
        assert_eq!(origin.markdown, "Hi **Ana**");
        // Reopening keeps From and the signature.
        assert!(origin.signature);
        assert_eq!(t.last_message().from.email, "alias@example.com");
        assert_eq!(store.draft_revision(ME, "draft-local-7").unwrap(), 10);
        for ext in ["", "-wal", "-shm"] {
            std::fs::remove_file(format!("{}{ext}", path.display())).ok();
        }
    }

    #[test]
    fn sent_mail_carries_the_aliass_name_reply_to_and_signature() {
        let store = Store::open_in_memory().unwrap();
        let (gmail, fake) = gmail(FakeTransport::new().on(
            "POST",
            "/gmail/v1/users/me/messages/send",
            &[],
            &[(200, r#"{"id":"s1","threadId":"T1"}"#)],
        ));
        let signature = "<b>Work Me</b><br>Example Ltd";
        let draft = Outgoing {
            from: Contact {
                name: "Work Me".into(),
                email: "alias@example.com".into(),
            },
            reply_to_address: "team@example.com".into(),
            signature: signature.into(),
            html: format!("<p>Hi</p>{}", mime::signature_block(signature)),
            ..outgoing("[Pigeon test] alias")
        };
        let op = MailOp::Send {
            draft,
            local_id: None,
            thread_id: "sent-local-1".into(),
        };
        commit_compose(&gmail, &store, ME, &op).0.unwrap();
        let raw = raw_of(&fake.requests()[0]);
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.contains("From: \"Work Me\" <alias@example.com>"),
            "{text}"
        );
        assert!(text.contains("Reply-To: <team@example.com>"), "{text}");
        assert!(
            !text.contains(mime::SIGNATURE_HEADER),
            "only drafts mark it"
        );
        let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
        let plain = parsed.body_text(0).unwrap();
        assert!(
            plain.ends_with("-- \r\nWork Me\r\nExample Ltd")
                || plain.ends_with("-- \nWork Me\nExample Ltd"),
            "{plain:?}"
        );
        let html = parsed.body_html(0).unwrap();
        assert_eq!(
            html.matches("class=\"gmail_signature\"").count(),
            1,
            "{html}"
        );
        // The sent copy keeps the Reply-To.
        let t = &store.load_threads(ME, &["T1".into()]).unwrap()[0];
        assert_eq!(t.last_message().reply_to[0].email, "team@example.com");
    }

    // W2 (ATT-7).

    fn forward_of_fixture() -> (Outgoing, Vec<u8>) {
        let eml = include_bytes!("../../tests/fixtures/google/gmail/attachments.eml").to_vec();
        let attachments = mime::parse_message(&eml)
            .unwrap()
            .attachments
            .into_iter()
            .map(|a| crate::data::Attachment {
                message_id: "orig".into(),
                ..a
            })
            .collect();
        let draft = Outgoing {
            kind: ComposeKind::Forward,
            attachments,
            ..outgoing("Fwd: Q3")
        };
        (draft, eml)
    }

    #[test]
    fn a_forward_carries_the_originals_files() {
        use base64::Engine as _;
        let store = Store::open_in_memory().unwrap();
        let (draft, eml) = forward_of_fixture();
        let encoded = base64::engine::general_purpose::URL_SAFE.encode(&eml);
        let (gmail, fake) = gmail(
            FakeTransport::new()
                .on(
                    "GET",
                    "/gmail/v1/users/me/messages/orig",
                    &[("format", "raw")],
                    &[(200, &format!(r#"{{"id":"orig","raw":"{encoded}"}}"#))],
                )
                .on(
                    "POST",
                    "/gmail/v1/users/me/messages/send",
                    &[],
                    &[(200, r#"{"id":"s9","threadId":"T9","labelIds":["SENT"]}"#)],
                ),
        );
        let op = MailOp::Send {
            draft: draft.clone(),
            local_id: None,
            thread_id: "sent-local-9".into(),
        };
        commit_compose(&gmail, &store, ME, &op).0.unwrap();
        // One fetch of the original for both files.
        assert_eq!(fake.requests().len(), 2);
        let sent = raw_of(&fake.requests()[1]);
        let parsed = mime::parse_message(&sent).unwrap();
        let names: Vec<_> = parsed.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Q3 report.pdf", "résumé.txt"]);
        for (a, original) in parsed.attachments.iter().zip(&draft.attachments) {
            assert_eq!(
                mime::part_bytes(&sent, &a.part_id),
                mime::part_bytes(&eml, &original.part_id)
            );
        }
    }

    #[test]
    fn a_forward_queued_offline_stays_queued() {
        let store = Store::open_in_memory().unwrap();
        let (draft, _) = forward_of_fixture();
        let (gmail, _) = gmail(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/messages/orig",
            &[("format", "raw")],
            &[(0, "")],
        ));
        let op = MailOp::Send {
            draft,
            local_id: None,
            thread_id: "sent-local-9".into(),
        };
        assert!(is_later(
            &commit_compose(&gmail, &store, ME, &op).0.unwrap_err()
        ));
    }

    #[test]
    fn a_held_copy_sends_after_the_cache_is_evicted() {
        let store = Store::open_in_memory().unwrap();
        let root = std::env::temp_dir().join(format!("pigeon-fwd-{}", uuid::Uuid::new_v4()));
        let files = super::super::attachments::Files::new(root.clone());
        let cached = files.put("k", "plan.pdf", b"%PDF-plan", 1 << 20).unwrap();
        let held = files.hold(&cached).unwrap();
        files.evict(0, None);
        let draft = Outgoing {
            kind: ComposeKind::Forward,
            attachments: vec![crate::data::Attachment {
                path: held.to_string_lossy().into_owned(),
                message_id: "orig".into(),
                part_id: "3".into(),
                ..crate::data::Attachment::from_file("plan.pdf".into(), 9)
            }],
            ..outgoing("Fwd: plan")
        };
        let (gmail, fake) = gmail(FakeTransport::new().on(
            "POST",
            "/gmail/v1/users/me/messages/send",
            &[],
            &[(200, r#"{"id":"s8","threadId":"T8"}"#)],
        ));
        let op = MailOp::Send {
            draft,
            local_id: None,
            thread_id: "sent-local-8".into(),
        };
        commit_compose(&gmail, &store, ME, &op).0.unwrap();
        // No fetch of the original: the held copy went.
        assert_eq!(fake.requests().len(), 1);
        let sent = raw_of(&fake.requests()[0]);
        assert_eq!(
            mime::attachment_bytes(&sent, "plan.pdf").unwrap(),
            b"%PDF-plan"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn over_25_mb_is_refused_before_anything_goes() {
        let store = Store::open_in_memory().unwrap();
        let (mut draft, _) = forward_of_fixture();
        draft.attachments[0].size_bytes = crate::ui::composer::ATTACHMENT_LIMIT;
        let (gmail, fake) = gmail(FakeTransport::new());
        let op = MailOp::Send {
            draft,
            local_id: None,
            thread_id: "sent-local-7".into(),
        };
        let error = commit_compose(&gmail, &store, ME, &op).0.unwrap_err();
        assert!(
            error
                .to_string()
                .contains(crate::ui::composer::ATTACHMENT_LIMIT_ERROR)
        );
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn big_messages_use_the_upload_endpoint() {
        let raw = vec![b'a'; gmail::SIMPLE_UPLOAD_LIMIT + 1];
        let request = gmail::draft_send_request("r1", &raw, Some("t1"));
        assert_eq!(
            request.url,
            "https://gmail.googleapis.com/upload/gmail/v1/users/me/drafts/send?uploadType=multipart"
        );
        let body = String::from_utf8_lossy(&request.body);
        assert!(
            body.contains(r#"{"message":{"threadId":"t1"},"id":"r1"}"#),
            "{}",
            &body[..200]
        );
        assert!(body.contains("Content-Type: message/rfc822"));
        let small = gmail::send_request(b"hi", None);
        assert_eq!(
            small.url,
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/send"
        );
    }
}

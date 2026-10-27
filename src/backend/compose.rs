//! Drafts and sending on Gmail (W6): what `SaveDraft`, `DeleteDraft` and
//! `Send` do once the outbox lets them go, and opening a Gmail draft.
//!
//! - SaveDraft: the draft is stored here first, then `drafts.create` or
//!   `drafts.update` (DRF-1). A failure keeps the local copy.
//! - DeleteDraft: the local copy goes, then `drafts.delete` (UND-2).
//! - Send: `drafts.send` for a draft saved in Gmail, else `messages.send`
//!   (CMP-8). The sent copy is stored under Gmail's ids, so the history
//!   poll finds it already there (CMP-13). A failure puts the draft back in
//!   Drafts with "Not sent: <reason>" (CMP-12).
//! - Opening a Gmail draft: `drafts.get format=raw`, the markdown recovered
//!   from a draft written here (DRF-3, DRF-6).

use std::sync::Arc;

use anyhow::anyhow;
use gpui::Context;

use super::{
    Backend, BackendEvent, MailOp, Outgoing, Source,
    gmail::{self, DraftRef, GMessage, Gmail, GmailError},
    http::{GoogleTransport, Request, Response, Transport, TransportError},
    mime,
    store::Store,
};
use crate::data::{Body, DraftOrigin, Folder, Thread};

/// Google's transport at the user's priority: ahead of sync, into the
/// quota's reserve.
pub struct UserCalls(pub Arc<GoogleTransport>);

impl Transport for UserCalls {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.0.send_user(request)
    }
}

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
        MailOp::SaveDraft { local_id, draft } => save_draft(gmail, store, account, local_id, draft),
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
    let mut files = Vec::new();
    for attachment in &draft.attachments {
        let bytes = if !attachment.path.is_empty() {
            std::fs::read(&attachment.path).ok()
        } else if let Some(id) = draft_id {
            if saved.is_none() {
                saved = gmail
                    .call::<DraftRef>(&gmail::draft_get_raw_request(id))
                    .ok()
                    .and_then(|d| d.message.raw)
                    .and_then(|raw| mime::decode_raw(&raw));
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

fn save_draft(
    gmail: &Gmail,
    store: &Store,
    account: &str,
    local_id: &str,
    draft: &Outgoing,
) -> anyhow::Result<()> {
    let stored = stored_draft(store, account, local_id);
    // Here first: the text is never lost, whatever Gmail says.
    store.put_threads(
        account,
        &[draft_thread(
            draft,
            local_id,
            stored.as_ref().map(|s| s.1.clone()),
        )],
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
        .map_err(|e| anyhow!("Couldn't save the draft to Gmail: {}", reason(&e)))?;
    store.put_threads(
        account,
        &[draft_thread(draft, local_id, Some(saved.message.id))],
    )?;
    store.set_draft_id(account, local_id, &saved.id)?;
    Ok(())
}

fn delete_draft(gmail: &Gmail, store: &Store, account: &str, local_id: &str) -> anyhow::Result<()> {
    let stored = stored_draft(store, account, local_id);
    store.delete_threads(account, &[local_id.to_string()])?;
    if let Some((id, _)) = stored {
        gmail
            .call_empty(&gmail::draft_delete_request(&id))
            .map_err(|e| anyhow!("Couldn't delete the draft in Gmail: {}", reason(&e)))?;
    }
    Ok(())
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
    let sent = read_files(gmail, draft, stored.as_ref().map(|s| s.0.as_str()))
        .map_err(|e| e.to_string())
        .and_then(|files| {
            let (raw, message_id) = build(store, account, draft, false, files);
            let joins = gmail_thread(draft.reply_to.as_deref());
            let request = match &stored {
                Some((id, _)) => gmail::draft_send_request(id, &raw, joins),
                None => gmail::send_request(&raw, joins),
            };
            gmail
                .call::<GMessage>(&request)
                .map(|m| (m, message_id))
                .map_err(|e| reason(&e))
        });
    let (sent, message_id) = match sent {
        Ok(sent) => sent,
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
        store.delete_threads(account, &[local.to_string()])?;
    }
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
            rederive(&mut thread);
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
            rederive(&mut thread);
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
fn rederive(thread: &mut Thread) {
    let labels: Vec<Vec<String>> = thread
        .messages
        .iter()
        .map(|m| m.label_ids.clone())
        .collect();
    thread.folder = gmail::derive_folder(&labels);
    thread.label_ids = gmail::union_labels(&labels);
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
    });
    origin.markdown = parsed.markdown;
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
                    let gmail = Gmail::new(Arc::new(UserCalls(transport)));
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
        backend::http::FakeTransport,
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

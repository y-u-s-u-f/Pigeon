//! Writing mail: composing, replying and forwarding, drafts, sending, and
//! Undo (of a send, a discard, a move or a deleted event). Moved out of
//! `app.rs`; the backend gets a `MailOp` for each save, send and discard.

use gpui::{AppContext as _, Context, Entity, Window, point, px};

use super::{ComposeKind, Mode, PigeonApp, UndoAction};
use crate::{
    actions::*,
    backend::{MailOp, OpId, Outgoing, gmail, mime},
    data::{Body, Contact, Folder, Message, Quote, SendAs, Thread, full_date},
    html,
    ui::composer::{
        ATTACHMENT_LIMIT, ATTACHMENT_LIMIT_ERROR, Composer, ComposerEvent, DraftContent, Quoted,
        REMOTE_SAVE_EVERY_MS, attachments_size, parse_address,
    },
};

/// What a Drafts thread holds, for the composer.
fn draft_content(thread: &Thread) -> DraftContent {
    let message = thread.last_message();
    let markdown = thread
        .origin
        .as_ref()
        .map(|o| o.markdown.clone())
        .filter(|m| !m.is_empty());
    // Otherwise its text, without a Gmail signature, which comes back on
    // its own (MST-2).
    let stripped = markdown
        .is_none()
        .then(|| mime::strip_signature(message.html()))
        .flatten();
    let body = markdown.unwrap_or_else(|| {
        let html = stripped.as_deref().unwrap_or(message.html());
        html::plain_text(&html::parse(html))
    });
    DraftContent {
        to: message.to.clone(),
        cc: message.cc.clone(),
        bcc: message.bcc.clone(),
        subject: thread.subject.clone(),
        // As typed, when it was written here or recovered; otherwise its text.
        body,
        attachments: message.attachments.clone(),
        // W3: as it was saved, here or in Gmail.
        from: message.from.email.clone(),
        signature: stripped.is_some() || thread.origin.as_ref().is_some_and(|o| o.signature),
    }
}

/// W3 (MST-1): the address a reply or forward of `last` goes from: the one
/// I sent it from, else the first of mine it was sent to; a new message,
/// or mail to none of them, the default.
fn pick_from(kind: ComposeKind, last: Option<&Message>, identities: &[SendAs]) -> String {
    let mine = |email: &str| {
        identities
            .iter()
            .find(|s| s.email.eq_ignore_ascii_case(email))
            .map(|s| s.email.clone())
    };
    let answered = last.filter(|_| kind != ComposeKind::New).and_then(|m| {
        mine(&m.from.email).or_else(|| {
            m.to.iter()
                .chain(&m.cc)
                .chain(&m.bcc)
                .find_map(|c| mine(&c.email))
        })
    });
    answered
        .or_else(|| {
            identities
                .iter()
                .find(|s| s.is_default)
                .map(|s| s.email.clone())
        })
        .or_else(|| identities.first().map(|s| s.email.clone()))
        .unwrap_or_default()
}

/// Who a reply to `last` goes to, as (To, Cc), like Gmail: to the sender, or
/// where they asked (Reply-To); for Reply All also everyone else on it. To a
/// message I sent: its recipients. Never to me, unless I'm all there is.
fn reply_recipients(
    kind: ComposeKind,
    last: &Message,
    is_me: impl Fn(&Contact) -> bool,
) -> (Vec<Contact>, Vec<Contact>) {
    if matches!(kind, ComposeKind::Forward | ComposeKind::New) {
        return (Vec::new(), Vec::new());
    }
    let mine = is_me(&last.from);
    let sender = if mine {
        &[][..]
    } else if last.reply_to.is_empty() {
        std::slice::from_ref(&last.from)
    } else {
        last.reply_to.as_slice()
    };
    let others = |list: &mut Vec<Contact>, from: &[Contact], taken: &[Contact]| {
        for contact in from {
            let seen = taken
                .iter()
                .chain(list.iter())
                .any(|c| c.email.eq_ignore_ascii_case(&contact.email));
            if !is_me(contact) && !seen {
                list.push(contact.clone());
            }
        }
    };
    let mut to = Vec::new();
    let mut cc = Vec::new();
    match kind {
        ComposeKind::Reply if mine => others(&mut to, &last.to, &[]),
        ComposeKind::Reply => others(&mut to, sender, &[]),
        _ => {
            others(&mut to, sender, &[]);
            others(&mut to, &last.to, &[]);
            others(&mut cc, &last.cc, &to);
        }
    }
    if to.is_empty() {
        to = std::mem::take(&mut cc);
    }
    if to.is_empty() {
        to.push(last.from.clone());
    }
    (to, cc)
}

/// W2 (ATT-7): `original`'s files, pointing back at it.
fn forwarded_attachments(original: &Message) -> Vec<crate::data::Attachment> {
    original
        .attachments
        .iter()
        .cloned()
        .map(|mut a| {
            if a.message_id.is_empty() {
                a.message_id = original.id.clone();
            }
            a
        })
        .collect()
}

impl PigeonApp {
    /// Delete in Drafts discards the draft, like the discard button, then
    /// moves to the neighbor.
    pub(super) fn discard_selected_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let next = self
            .next_after_selected()
            .filter(|_| self.settings.auto_advance)
            .map(|next| self.data.threads[next].id.clone());
        self.select_thread(ix, window, cx);
        self.discard_draft(&DiscardDraft, window, cx);
        if let Some(ix) = next.and_then(|id| self.data.thread_ix(&id)) {
            self.select_thread(ix, window, cx);
        }
    }

    pub(super) fn compose_new(&mut self, _: &Compose, window: &mut Window, cx: &mut Context<Self>) {
        // From the menu while the calendar shows.
        self.set_mode(Mode::Mail, window, cx);
        self.compose(ComposeKind::New, window, cx);
    }

    pub(super) fn reply(&mut self, _: &Reply, window: &mut Window, cx: &mut Context<Self>) {
        self.compose(ComposeKind::Reply, window, cx);
    }

    pub(super) fn reply_all(&mut self, _: &ReplyAll, window: &mut Window, cx: &mut Context<Self>) {
        self.compose(ComposeKind::ReplyAll, window, cx);
    }

    pub(super) fn forward(&mut self, _: &Forward, window: &mut Window, cx: &mut Context<Self>) {
        self.compose(ComposeKind::Forward, window, cx);
    }

    /// Open a draft: alone in the reading pane for a new message, under the
    /// conversation for a reply or forward.
    pub(super) fn compose(
        &mut self,
        kind: ComposeKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stash_composer(cx);
        if kind != ComposeKind::New && !self.checked.is_empty() {
            return;
        }
        // One draft per conversation: answering again reopens it.
        if matches!(kind, ComposeKind::Reply | ComposeKind::ReplyAll)
            && let Some(ix) = self.selected
            && let Some(draft) = self.reply_draft_of(&self.data.threads[ix].id)
        {
            self.open_draft(draft, true, window, cx);
            if let Some((composer, _)) = &self.composer {
                composer.update(cx, |c, cx| c.focus_first(window, cx));
            }
            self.reader_focused = true;
            self.reader_scroll.scroll_to_bottom();
            cx.notify();
            return;
        }
        let thread = self.selected.map(|ix| &self.data.threads[ix]);
        let (mut content, reply_to, quoted) = match (kind, thread) {
            (ComposeKind::New, _) => (DraftContent::default(), None, None),
            (_, None) => return,
            (_, Some(thread)) if thread.folder == Folder::Drafts => return,
            (kind, Some(thread)) => {
                let (mut content, quoted) = self.reply_content(kind, thread);
                // W2 (ATT-7): cached files are held for the send.
                self.backend
                    .read(cx)
                    .hold_attachments(&thread.last_message().id, &mut content.attachments);
                // A forward starts a new thread, so it answers nothing.
                let reply_to = (kind != ComposeKind::Forward).then(|| thread.id.clone());
                (content, reply_to, quoted)
            }
        };
        // W3: from the alias it answers, with its signature (MST-1, MST-2).
        let identities = self.data.account.identities();
        let last = thread.map(|t| t.last_message());
        content.from = pick_from(kind, last, &identities);
        content.signature = crate::settings::current(cx).signatures;
        self.open_composer(kind, content, reply_to, quoted, window, cx);
    }

    /// A new composer for a draft not saved yet, focused where writing
    /// starts.
    fn open_composer(
        &mut self,
        kind: ComposeKind,
        content: DraftContent,
        reply_to: Option<String>,
        quoted: Option<Quoted>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let identities = self.data.account.identities();
        let composer = cx.new(|cx| {
            Composer::new(
                kind,
                content,
                reply_to.clone(),
                None,
                quoted,
                reply_to.is_some(),
                self.contact_book.clone(),
                window,
                cx,
            )
            .with_identities(identities)
        });
        self.set_composer(composer.clone(), cx);
        composer.update(cx, |c, cx| {
            c.focus_first(window, cx);
            // W2 (ATT-7): a forward over the limit says so at once.
            if attachments_size(&c.content(cx).attachments) > ATTACHMENT_LIMIT {
                c.set_attachment_error(ATTACHMENT_LIMIT_ERROR, cx);
            }
        });
        self.reader_focused = reply_to.is_some();
        if reply_to.is_some() {
            self.reader_scroll.scroll_to_bottom();
        } else {
            self.reader_scroll.set_offset(point(px(0.), px(0.)));
        }
        cx.notify();
    }

    pub(super) fn reply_content(
        &self,
        kind: ComposeKind,
        thread: &Thread,
    ) -> (DraftContent, Option<Quoted>) {
        let last = thread.last_message();
        let (to, cc) = reply_recipients(kind, last, |c| self.data.is_me(c));

        let subject = &thread.subject;
        let lower = subject.to_lowercase();
        let subject = match kind {
            ComposeKind::Forward if lower.starts_with("fwd:") || lower.starts_with("fw:") => {
                subject.clone()
            }
            ComposeKind::Forward => format!("Fwd: {subject}"),
            _ if lower.starts_with("re:") => subject.clone(),
            _ => format!("Re: {subject}"),
        };

        let quoted = (kind == ComposeKind::Forward).then(|| {
            let address = |c: &Contact| {
                if c.name.is_empty() {
                    c.email.clone()
                } else {
                    format!("{} <{}>", c.name, c.email)
                }
            };
            let to = last.to.iter().map(address).collect::<Vec<_>>().join(", ");
            Quoted::new(
                vec![
                    ("From".into(), address(&last.from)),
                    ("Date".into(), full_date(&last.date)),
                    ("Subject".into(), thread.subject.clone()),
                    ("To".into(), to),
                ],
                last.html().to_string(),
            )
        });

        // W2 (ATT-7): a forward carries the original's files, taken from it
        // when it goes.
        let attachments = match kind {
            ComposeKind::Forward => forwarded_attachments(last),
            _ => Vec::new(),
        };
        let content = DraftContent {
            to,
            cc,
            bcc: Vec::new(),
            subject,
            body: String::new(),
            attachments,
            ..Default::default()
        };
        (content, quoted)
    }

    /// The Drafts thread that answers the thread `id`, if there is one.
    pub(super) fn reply_draft_of(&self, id: &str) -> Option<usize> {
        self.data.threads.iter().position(|t| {
            t.folder == Folder::Drafts
                && t.origin
                    .as_ref()
                    .is_some_and(|o| o.reply_to.as_deref() == Some(id))
        })
    }

    /// Show a Drafts thread in the composer, without taking focus from the
    /// list: alone, or `inline` under the conversation it answers.
    pub(super) fn open_draft(
        &mut self,
        ix: usize,
        inline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let thread = &self.data.threads[ix];
        let content = draft_content(thread);
        // A Gmail draft's text comes when it's fetched (DRF-3).
        let awaiting_body = !matches!(thread.last_message().body, Body::Loaded { .. });
        let origin = thread.origin.as_ref();
        let kind = origin.map_or(ComposeKind::New, |o| o.kind);
        let reply_to = origin.and_then(|o| o.reply_to.clone());
        let quoted = origin
            .and_then(|o| o.quoted.as_ref())
            .map(|q| Quoted::new(q.header.clone(), q.html.clone()));
        let draft_id = Some(thread.id.clone());
        let book = self.contact_book.clone();
        let identities = self.data.account.identities();
        let composer = cx.new(|cx| {
            let mut composer = Composer::new(
                kind, content, reply_to, draft_id, quoted, inline, book, window, cx,
            )
            .with_identities(identities);
            composer.awaiting_body = awaiting_body;
            composer
        });
        self.set_composer(composer, cx);
    }

    /// The open draft's text arrived from Gmail: show it, unless the user
    /// has started writing (DRF-6: never over their edits).
    pub(super) fn refresh_open_draft(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let Some((composer, _)) = &self.composer else {
            return;
        };
        let c = composer.read(cx);
        let Some(id) = c
            .draft_id
            .clone()
            .filter(|id| c.awaiting_body && ids.contains(id))
        else {
            return;
        };
        if !c.is_pristine(cx) {
            return;
        }
        let Some(thread) = self.data.thread_ix(&id).map(|ix| &self.data.threads[ix]) else {
            return;
        };
        if !matches!(thread.last_message().body, Body::Loaded { .. }) {
            return;
        }
        let content = draft_content(thread);
        composer.clone().update(cx, |c, cx| c.load(content, cx));
    }

    /// "Sending…" went: "Message sent", unless another toast took its place.
    pub(super) fn on_send_committed(&mut self, op: OpId, cx: &mut Context<Self>) {
        let Some(ix) = self.sends.iter().position(|s| *s == op) else {
            return;
        };
        self.sends.remove(ix);
        let replaces = match &self.toast {
            None => true,
            Some(toast) => match &toast.undo {
                None => true,
                Some(UndoAction::Send { op: sent, .. }) => *sent == op,
                Some(_) => false,
            },
        };
        if replaces {
            self.show_toast("Message sent", None, cx);
        }
    }

    pub(super) fn set_composer(&mut self, composer: Entity<Composer>, cx: &mut Context<Self>) {
        let reader_scroll = self.reader_scroll.clone();
        composer.update(cx, |composer, _| {
            composer.outer_scroll = Some(reader_scroll)
        });
        let events = cx.subscribe(&composer, |this, composer, event, cx| match event {
            ComposerEvent::Edited { at_end } => {
                if *at_end && composer.read(cx).quoted.is_none() {
                    this.reader_scroll.scroll_to_bottom();
                }
            }
            // W3 (DRF-5).
            ComposerEvent::Autosave => this.save_draft(&composer, false, cx),
        });
        self.composer = Some((composer, events));
        cx.notify();
    }

    /// Whether the composer shows under the selected conversation, rather than
    /// alone in the reading pane.
    pub fn composer_inline(&self, cx: &gpui::App) -> bool {
        let Some((composer, _)) = &self.composer else {
            return false;
        };
        let composer = composer.read(cx);
        composer.inline
            && composer.reply_to.is_some()
            && self.selected.map(|ix| &self.data.threads[ix].id) == composer.reply_to.as_ref()
    }

    /// `draft-local-<uuid>` or `sent-local-<uuid>`, until the server gives
    /// the thread its id (DAT-5).
    fn new_id(&self, prefix: &str) -> String {
        format!("{prefix}-local-{}", uuid::Uuid::new_v4().simple())
    }

    /// What the composer holds, as a message to save or send carrying
    /// `html`.
    fn outgoing(&self, composer: &Composer, content: DraftContent, html: String) -> Outgoing {
        // W3 (MST-1, MST-2): the alias's name, Reply-To and signature.
        let identity = self.data.account.identity(&content.from);
        let signature = composer
            .signature_html()
            .map(str::to_string)
            .unwrap_or_default();
        Outgoing {
            kind: composer.kind,
            from: self.data.account.sender(&identity),
            reply_to_address: identity.reply_to.clone(),
            signature,
            to: content.to,
            cc: content.cc,
            bcc: content.bcc,
            subject: content.subject,
            markdown: content.body,
            html,
            attachments: content.attachments,
            dropped_files: composer.dropped_files.clone(),
            reply_to: composer.reply_to.clone(),
            quoted: composer.quoted.as_ref().map(|q| Quote {
                header: q.header.clone(),
                html: q.html.clone(),
            }),
        }
    }

    /// Save the draft to Drafts, replacing the thread it was opened from:
    /// here at once, and on Gmail now when it closes (`remote_now`), else at
    /// most every 30 s (DRF-5). Each save carries a newer revision of the
    /// draft's one local id.
    pub(super) fn save_draft(
        &mut self,
        composer: &Entity<Composer>,
        remote_now: bool,
        cx: &mut Context<Self>,
    ) {
        let c = composer.read(cx);
        let content = c.content(cx);
        // The quote is kept on the origin, so the body holds only the note
        // and the signature.
        let html = c.draft_html(cx);
        let draft = self.outgoing(c, content.clone(), html);
        let id = c.draft_id.clone().unwrap_or_else(|| self.new_id("draft"));
        let now = chrono::Utc::now().timestamp_millis();
        let due = match (remote_now, c.remote_due) {
            (false, Some(due)) => due.max(now),
            _ => now,
        };
        let revision = composer.update(cx, |c, _| {
            c.draft_id = Some(id.clone());
            // The next save while typing waits 30 s after this one goes.
            c.remote_due = Some(due + REMOTE_SAVE_EVERY_MS);
            c.next_revision()
        });
        let thread = draft.to_draft_thread(id.clone());
        let saved = self.backend.update(cx, |b, cx| {
            b.save_draft(id.clone(), draft, revision, due - now, cx)
        });
        if !saved {
            return;
        }
        self.update_threads(|data| match data.thread_ix(&id) {
            Some(ix) => data.threads[ix] = thread,
            None => data.threads.push(thread),
        });
        composer.update(cx, |c, cx| c.mark_saved(content, cx));
    }

    /// Close the composer without losing anything: a changed draft goes to
    /// Drafts.
    pub(super) fn stash_composer(&mut self, cx: &mut Context<Self>) {
        let Some((composer, _)) = self.composer.take() else {
            return;
        };
        composer.update(cx, |c, _| c.set_closed(true));
        let c = composer.read(cx);
        if !c.is_pristine(cx) {
            self.save_draft(&composer, true, cx);
            self.show_toast("Saved to Drafts", None, cx);
        } else if c.draft_id.is_none() {
            // Never saved: nothing needs its held copies (ATT-7).
            let files = c.file_paths();
            self.backend.read(cx).release_files(&files);
        }
        cx.notify();
    }

    pub(super) fn close_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(super) fn send_message(
        &mut self,
        _: &SendMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((composer, _)) = &self.composer else {
            return;
        };
        let composer = composer.clone();
        if !composer.update(cx, |c, cx| c.validate(window, cx)) {
            return;
        }
        let c = composer.read(cx);
        let content = c.content(cx);
        if !content.has_recipients() {
            composer.update(cx, |c, cx| {
                c.set_error("Add at least one recipient", cx);
                c.focus_to(window, cx);
            });
            return;
        }
        if attachments_size(&content.attachments) > ATTACHMENT_LIMIT {
            composer.update(cx, |c, cx| {
                c.set_attachment_error(ATTACHMENT_LIMIT_ERROR, cx)
            });
            return;
        }
        let html = c.html(cx);
        let draft = self.outgoing(c, content, html);
        let message = draft.to_message(vec![gmail::SENT.into()]);
        let reply_to = c
            .reply_to
            .clone()
            .filter(|id| self.data.thread_ix(id).is_some());
        let draft_id = c.draft_id.clone();
        let new_id = self.new_id("sent");
        let subject = draft.subject.clone();

        let (thread_id, appended, removed_draft) = self.update_threads(|data| {
            let removed_draft = draft_id
                .as_ref()
                .and_then(|id| data.thread_ix(id))
                .map(|ix| data.threads.remove(ix));
            let (thread_id, appended) = match reply_to {
                Some(id) => {
                    let ix = data.thread_ix(&id).expect("checked above");
                    let thread = &mut data.threads[ix];
                    thread.messages.push(message);
                    thread.reindex();
                    (id, true)
                }
                None => {
                    data.threads.push(Thread::local(
                        new_id.clone(),
                        subject,
                        Folder::Sent,
                        vec![message],
                        None,
                    ));
                    (new_id, false)
                }
            };
            (thread_id, appended, removed_draft)
        });
        let op = self.backend.update(cx, |b, cx| {
            b.mail(
                MailOp::Send {
                    draft,
                    local_id: draft_id,
                    thread_id: thread_id.clone(),
                },
                cx,
            )
        });

        // A reply stays in view, ending with the message just sent.
        if appended {
            if let Some(ix) = self.data.thread_ix(&thread_id) {
                if self.selected != Some(ix) {
                    self.reveal_thread(ix, window, cx);
                }
                self.reader_scroll.scroll_to_bottom();
            }
        } else if removed_draft.is_some() {
            self.selected = None;
        }
        composer.update(cx, |c, cx| {
            c.remember_focus(window, cx);
            c.set_closed(true);
        });
        self.close_composer(window, cx);
        self.sends.push(op);
        self.show_toast(
            "Sending…",
            Some(UndoAction::Send {
                thread_id,
                appended,
                removed_draft,
                composer,
                op,
            }),
            cx,
        );
    }

    /// Escape: close the draft, keeping it in Drafts if it says anything.
    pub(super) fn close_draft(
        &mut self,
        _: &CloseDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer.is_none() {
            return cx.propagate();
        }
        let was_inline = self.composer_inline(cx);
        self.stash_composer(cx);
        self.reader_focused = was_inline;
        window.focus(&self.focus_handle, cx);
    }

    pub(super) fn discard_draft(
        &mut self,
        _: &DiscardDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((composer, _)) = &self.composer else {
            return;
        };
        let composer = composer.clone();
        composer.update(cx, |c, _| c.set_closed(true));
        let c = composer.read(cx);
        let pristine = c.is_pristine(cx) && c.draft_id.is_none();
        let draft_id = c.draft_id.clone();
        let removed_draft = self.update_threads(|data| {
            draft_id
                .as_ref()
                .and_then(|id| data.thread_ix(id))
                .map(|ix| data.threads.remove(ix))
        });
        let op = removed_draft.as_ref().map(|draft| {
            let local_id = draft.id.clone();
            self.backend
                .update(cx, |b, cx| b.mail(MailOp::DeleteDraft { local_id }, cx))
        });
        if removed_draft.is_some() {
            self.selected = None;
        }
        composer.update(cx, |c, cx| c.remember_focus(window, cx));
        self.close_composer(window, cx);
        if pristine {
            let files = composer.read(cx).file_paths();
            self.backend.read(cx).release_files(&files);
        } else {
            self.show_toast(
                "Draft discarded",
                Some(UndoAction::Discard {
                    removed_draft,
                    composer,
                    op,
                }),
                cx,
            );
        }
    }

    /// W3 (LNK-3): a new message from a `mailto:` link.
    pub(super) fn compose_mailto(
        &mut self,
        mailto: crate::app::shell::Mailto,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_mode(Mode::Mail, window, cx);
        self.stash_composer(cx);
        let contacts = |list: &[String]| -> Vec<Contact> {
            list.iter()
                .map(|a| {
                    parse_address(a).unwrap_or_else(|| Contact {
                        name: String::new(),
                        email: a.clone(),
                    })
                })
                .collect()
        };
        let identities = self.data.account.identities();
        let content = DraftContent {
            to: contacts(&mailto.to),
            cc: contacts(&mailto.cc),
            bcc: contacts(&mailto.bcc),
            subject: mailto.subject,
            body: mailto.body,
            from: pick_from(ComposeKind::New, None, &identities),
            signature: crate::settings::current(cx).signatures,
            ..Default::default()
        };
        self.open_composer(ComposeKind::New, content, None, None, window, cx);
    }

    /// W3 (MST-1): ⌘K "Change From…" opens the draft's From menu.
    pub(super) fn change_from(
        &mut self,
        _: &ChangeFrom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((composer, _)) = &self.composer {
            composer.update(cx, |c, cx| c.open_from_menu(window, cx));
        }
    }

    /// W3 (MST-2): add the signature to the open draft, or take it off.
    pub(super) fn toggle_signature(
        &mut self,
        _: &ToggleSignature,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((composer, _)) = &self.composer {
            composer.update(cx, |c, cx| c.toggle_signature(cx));
        }
    }

    pub(super) fn show_cc_bcc(
        &mut self,
        _: &ShowCcBcc,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((composer, _)) = &self.composer {
            composer.update(cx, |c, cx| c.show_cc_bcc(window, cx));
        }
    }

    pub(super) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        let Some(undo) = self.toast.take().and_then(|t| t.undo) else {
            return;
        };
        let (composer, removed_draft) = match undo {
            UndoAction::DeleteEvent(removal) => {
                self.calendar
                    .update(cx, |c, cx| c.restore(*removal, window, cx));
                // Undone from an invite card: focus stays with the mail.
                if self.mode == Mode::Mail {
                    window.focus(&self.focus_handle, cx);
                }
                cx.notify();
                return;
            }
            UndoAction::Send {
                thread_id,
                appended,
                removed_draft,
                composer,
                op,
            } => {
                if !self.backend.update(cx, |b, _| b.cancel(op)) {
                    self.show_toast("Already sent", None, cx);
                    return;
                }
                self.sends.retain(|s| *s != op);
                self.update_threads(|data| {
                    if let Some(ix) = data.thread_ix(&thread_id) {
                        if appended {
                            // The message sent here, even if a sync has
                            // reloaded the thread since.
                            let thread = &mut data.threads[ix];
                            if let Some(m) = thread
                                .messages
                                .iter()
                                .rposition(|m| m.id.starts_with("local-"))
                            {
                                thread.messages.remove(m);
                            }
                            thread.reindex();
                        } else {
                            data.threads.remove(ix);
                        }
                    }
                });
                (composer, removed_draft)
            }
            UndoAction::Discard {
                removed_draft,
                composer,
                op,
            } => {
                // Too late to take back: save it again.
                if let Some(op) = op
                    && !self.backend.update(cx, |b, _| b.cancel(op))
                {
                    self.save_draft(&composer, true, cx);
                }
                (composer, removed_draft)
            }
            UndoAction::Move {
                moves,
                selected,
                ops,
            } => {
                return self.undo_move(moves, selected, ops, window, cx);
            }
        };
        let draft_id = removed_draft.as_ref().map(|t| t.id.clone());
        if let Some(thread) = removed_draft {
            self.update_threads(|data| data.threads.push(thread));
        }
        // Back to where the draft was, then the draft itself.
        let home = draft_id.or_else(|| composer.read(cx).reply_to.clone());
        if let Some(ix) = home.and_then(|id| self.data.thread_ix(&id)) {
            self.reveal_thread(ix, window, cx);
        }
        self.composer = None;
        composer.update(cx, |c, _| c.set_closed(false));
        self.set_composer(composer.clone(), cx);
        self.reader_focused = self.composer_inline(cx);
        composer.update(cx, |c, cx| c.restore_focus(window, cx));
        // Reopening the thread can take focus back once the frame settles,
        // and the draft's shortcuts need it.
        cx.on_next_frame(window, move |this, window, cx| {
            let current = this.composer.as_ref().is_some_and(|(c, _)| *c == composer);
            if current && !composer.read(cx).has_focus(window, cx) {
                composer.update(cx, |c, cx| c.focus_first(window, cx));
            }
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(email: &str) -> Contact {
        Contact {
            name: String::new(),
            email: email.into(),
        }
    }

    fn message(from: &str, to: &[&str], cc: &[&str], reply_to: &[&str]) -> Message {
        Message {
            to: to.iter().map(|e| person(e)).collect(),
            cc: cc.iter().map(|e| person(e)).collect(),
            reply_to: reply_to.iter().map(|e| person(e)).collect(),
            ..Message::new(person(from))
        }
    }

    /// Me, and my alias.
    fn is_me(c: &Contact) -> bool {
        ["me@x.com", "alias@x.com"]
            .iter()
            .any(|e| e.eq_ignore_ascii_case(&c.email))
    }

    /// "To | Cc", for comparing.
    fn recipients(kind: ComposeKind, m: &Message) -> String {
        let (to, cc) = reply_recipients(kind, m, is_me);
        let join = |l: Vec<Contact>| {
            l.into_iter()
                .map(|c| c.email)
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("{} | {}", join(to), join(cc))
    }

    #[test]
    fn a_forward_points_its_files_at_the_original() {
        let mut m = message("ana@x.com", &["me@x.com"], &[], &[]);
        m.id = "orig".into();
        m.attachments = vec![
            crate::data::Attachment {
                part_id: "3".into(),
                ..crate::data::Attachment::from_file("a.pdf".into(), 5)
            },
            crate::data::Attachment {
                message_id: "older".into(),
                ..crate::data::Attachment::from_file("b.pdf".into(), 5)
            },
        ];
        let files = forwarded_attachments(&m);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].message_id, "orig");
        assert_eq!(files[0].part_id, "3");
        assert_eq!(files[1].message_id, "older", "already a forward's copy");
    }

    #[test]
    fn reply_to_someone_else_goes_to_the_sender() {
        let m = message("ann@x.com", &["me@x.com", "bob@x.com"], &["cat@x.com"], &[]);
        assert_eq!(recipients(ComposeKind::Reply, &m), "ann@x.com | ");
        assert_eq!(
            recipients(ComposeKind::ReplyAll, &m),
            "ann@x.com, bob@x.com | cat@x.com"
        );
    }

    #[test]
    fn reply_to_my_sent_mail_goes_to_its_recipients() {
        let m = message("me@x.com", &["bob@x.com"], &["cat@x.com", "me@x.com"], &[]);
        assert_eq!(recipients(ComposeKind::Reply, &m), "bob@x.com | ");
        assert_eq!(
            recipients(ComposeKind::ReplyAll, &m),
            "bob@x.com | cat@x.com"
        );
    }

    #[test]
    fn reply_to_my_mail_to_myself_goes_to_me() {
        let m = message("me@x.com", &["me@x.com"], &[], &[]);
        assert_eq!(recipients(ComposeKind::Reply, &m), "me@x.com | ");
        assert_eq!(recipients(ComposeKind::ReplyAll, &m), "me@x.com | ");
    }

    #[test]
    fn reply_all_to_my_mail_only_cced_moves_cc_to_to() {
        let m = message("me@x.com", &["me@x.com"], &["cat@x.com"], &[]);
        assert_eq!(recipients(ComposeKind::ReplyAll, &m), "cat@x.com | ");
    }

    #[test]
    fn reply_all_honours_reply_to() {
        let m = message(
            "ann@x.com",
            &["me@x.com", "bob@x.com"],
            &["list@x.com", "bob@x.com"],
            &["list@x.com"],
        );
        assert_eq!(recipients(ComposeKind::Reply, &m), "list@x.com | ");
        assert_eq!(
            recipients(ComposeKind::ReplyAll, &m),
            "list@x.com, bob@x.com | "
        );
    }

    #[test]
    fn aliases_count_as_me() {
        let m = message("ALIAS@x.com", &["bob@x.com", "me@x.com"], &[], &[]);
        assert_eq!(recipients(ComposeKind::Reply, &m), "bob@x.com | ");
        let m = message("alias@x.com", &["me@x.com"], &[], &[]);
        assert_eq!(recipients(ComposeKind::Reply, &m), "alias@x.com | ");
        let m = message("bob@x.com", &["alias@x.com", "cat@x.com"], &[], &[]);
        assert_eq!(
            recipients(ComposeKind::ReplyAll, &m),
            "bob@x.com, cat@x.com | "
        );
    }

    fn ids() -> Vec<SendAs> {
        ["me@x.com", "alias@x.com", "work@x.com"]
            .iter()
            .enumerate()
            .map(|(i, e)| SendAs {
                email: e.to_string(),
                is_default: i == 2,
                ..Default::default()
            })
            .collect()
    }

    /// W3 (MST-1).
    #[test]
    fn replies_go_from_the_alias_the_mail_was_sent_to() {
        let ids = ids();
        // New: the default address, whatever is selected.
        let m = message("ann@x.com", &["ALIAS@x.com"], &[], &[]);
        assert_eq!(pick_from(ComposeKind::New, Some(&m), &ids), "work@x.com");
        assert_eq!(pick_from(ComposeKind::New, None, &ids), "work@x.com");
        // Reply, reply all and forward: the alias it was sent to.
        for kind in [
            ComposeKind::Reply,
            ComposeKind::ReplyAll,
            ComposeKind::Forward,
        ] {
            assert_eq!(pick_from(kind, Some(&m), &ids), "alias@x.com");
        }
        // Cc'd to me.
        let m = message("ann@x.com", &["bob@x.com"], &["me@x.com"], &[]);
        assert_eq!(pick_from(ComposeKind::Reply, Some(&m), &ids), "me@x.com");
        // My own sent mail: the address I sent it from.
        let m = message("alias@x.com", &["me@x.com"], &[], &[]);
        assert_eq!(pick_from(ComposeKind::Reply, Some(&m), &ids), "alias@x.com");
        // To none of mine (a list): the default.
        let m = message("ann@x.com", &["list@x.com"], &[], &[]);
        assert_eq!(pick_from(ComposeKind::Reply, Some(&m), &ids), "work@x.com");
    }

    #[test]
    fn forwards_start_empty() {
        let m = message("ann@x.com", &["me@x.com"], &[], &[]);
        assert_eq!(recipients(ComposeKind::Forward, &m), " | ");
    }
}

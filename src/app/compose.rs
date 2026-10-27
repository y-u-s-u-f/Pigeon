//! Writing mail: composing, replying and forwarding, drafts, sending, and
//! Undo (of a send, a discard, a move or a deleted event). Moved out of
//! `app.rs`; the backend gets a `MailOp` for each save, send and discard.

use gpui::{AppContext as _, Context, Entity, Window, point, px};

use super::{ComposeKind, Mode, PigeonApp, UndoAction};
use crate::{
    actions::*,
    backend::{MailOp, Outgoing, gmail},
    data::{Contact, Folder, Quote, Thread, full_date},
    html,
    ui::composer::{Composer, ComposerEvent, DraftContent, Quoted},
};

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
        let (content, reply_to, quoted) = match (kind, thread) {
            (ComposeKind::New, _) => (DraftContent::default(), None, None),
            (_, None) => return,
            (_, Some(thread)) if thread.folder == Folder::Drafts => return,
            (kind, Some(thread)) => {
                let (content, quoted) = self.reply_content(kind, thread);
                // A forward starts a new thread, so it answers nothing.
                let reply_to = (kind != ComposeKind::Forward).then(|| thread.id.clone());
                (content, reply_to, quoted)
            }
        };
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
        });
        self.set_composer(composer.clone(), cx);
        composer.update(cx, |c, cx| c.focus_first(window, cx));
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
        let mine = self.data.is_me(&last.from);
        let mut to: Vec<Contact> = Vec::new();
        let mut cc: Vec<Contact> = Vec::new();
        let add = |list: &mut Vec<Contact>, contact: &Contact, all: &[Contact]| {
            let taken = all
                .iter()
                .chain(list.iter())
                .any(|c| c.email.eq_ignore_ascii_case(&contact.email));
            if !self.data.is_me(contact) && !taken {
                list.push(contact.clone());
            }
        };
        match kind {
            ComposeKind::Reply if mine => last.to.iter().for_each(|c| add(&mut to, c, &[])),
            ComposeKind::Reply => add(&mut to, &last.from, &[]),
            ComposeKind::ReplyAll => {
                for c in [&last.from].into_iter().chain(&last.to) {
                    add(&mut to, c, &[]);
                }
                for c in &last.cc {
                    add(&mut cc, c, &to);
                }
            }
            ComposeKind::Forward | ComposeKind::New => {}
        }

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

        let content = DraftContent {
            to,
            cc,
            bcc: Vec::new(),
            subject,
            body: String::new(),
            attachments: Vec::new(),
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
        let message = thread.last_message();
        let content = DraftContent {
            to: message.to.clone(),
            cc: message.cc.clone(),
            bcc: message.bcc.clone(),
            subject: thread.subject.clone(),
            // As typed, when it was written here; otherwise its text.
            body: thread
                .origin
                .as_ref()
                .map(|o| o.markdown.clone())
                .unwrap_or_else(|| html::plain_text(&html::parse(message.html()))),
            attachments: message.attachments.clone(),
        };
        let origin = thread.origin.as_ref();
        let kind = origin.map_or(ComposeKind::New, |o| o.kind);
        let reply_to = origin.and_then(|o| o.reply_to.clone());
        let quoted = origin
            .and_then(|o| o.quoted.as_ref())
            .map(|q| Quoted::new(q.header.clone(), q.html.clone()));
        let draft_id = Some(thread.id.clone());
        let book = self.contact_book.clone();
        let composer = cx.new(|cx| {
            Composer::new(
                kind, content, reply_to, draft_id, quoted, inline, book, window, cx,
            )
        });
        self.set_composer(composer, cx);
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
        Outgoing {
            kind: composer.kind,
            from: self.data.me(),
            to: content.to,
            cc: content.cc,
            bcc: content.bcc,
            subject: content.subject,
            markdown: content.body,
            html,
            attachments: content.attachments,
            reply_to: composer.reply_to.clone(),
            quoted: composer.quoted.as_ref().map(|q| Quote {
                header: q.header.clone(),
                html: q.html.clone(),
            }),
        }
    }

    /// Save the draft to Drafts, replacing the thread it was opened from.
    pub(super) fn save_draft(&mut self, composer: &Entity<Composer>, cx: &mut Context<Self>) {
        let c = composer.read(cx);
        let content = c.content(cx);
        // The quote is kept on the origin, so the body holds only the note.
        let html = content.body_html();
        let draft = self.outgoing(c, content, html);
        let id = c.draft_id.clone().unwrap_or_else(|| self.new_id("draft"));
        let thread = draft.to_draft_thread(id.clone());
        self.update_threads(|data| match data.thread_ix(&id) {
            Some(ix) => data.threads[ix] = thread,
            None => data.threads.push(thread),
        });
        self.backend.update(cx, |b, cx| {
            b.mail(
                MailOp::SaveDraft {
                    local_id: id,
                    draft,
                },
                cx,
            )
        });
    }

    /// Close the composer without losing anything: a changed draft goes to
    /// Drafts.
    pub(super) fn stash_composer(&mut self, cx: &mut Context<Self>) {
        let Some((composer, _)) = self.composer.take() else {
            return;
        };
        if !composer.read(cx).is_pristine(cx) {
            self.save_draft(&composer, cx);
            self.show_toast("Saved to Drafts", None, cx);
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
        self.close_composer(window, cx);
        self.show_toast(
            "Message sent",
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
        self.close_composer(window, cx);
        if !pristine {
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
                self.update_threads(|data| {
                    if let Some(ix) = data.thread_ix(&thread_id) {
                        if appended {
                            let thread = &mut data.threads[ix];
                            thread.messages.pop();
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
                    self.save_draft(&composer, cx);
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
        self.set_composer(composer.clone(), cx);
        self.reader_focused = self.composer_inline(cx);
        composer.update(cx, |c, cx| c.focus_first(window, cx));
        cx.notify();
    }
}

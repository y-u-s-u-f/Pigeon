//! Snooze (ACT-11) and Send Later (CMP-17): the shared time picker, the
//! change it makes with its undo, and Edit and Cancel Send on a scheduled
//! draft. `backend::later` keeps the times and wakes and sends.

use gpui::{AppContext as _, Context, Div, Entity, Stateful, Window, prelude::*};

use super::{Mode, Nav, PigeonApp, Row, SnoozeUndo, ToastAction, ToastKind, UndoAction};
use crate::{
    actions::{
        CancelScheduledSend, EditScheduledSend, GoToScheduled, GoToSnoozed, SendLater, Snooze,
        Unsnooze,
    },
    backend::{MailOp, OpId, later::SnoozeChange, store::Snooze as SnoozeRow},
    data::Folder,
    later,
    ui::{
        composer::Composer,
        later_picker::{LaterEvent, LaterPicker},
    },
};

/// A snooze change for the undo stack: what puts it back, the stored rows
/// before, and its ops.
type SnoozeSet = (Vec<SnoozeUndo>, Vec<(String, Option<SnoozeRow>)>, Vec<OpId>);

/// What the open picker's time is for.
pub(super) enum LaterTarget {
    Snooze(Vec<String>),
    Send(Entity<Composer>),
}

/// "tomorrow, 8:00 AM", from now.
fn when(at: i64) -> String {
    later::in_sentence(later::local_time(at), later::now_local())
}

impl PigeonApp {
    /// Registers the handlers of this module's actions on the root.
    pub(super) fn on_later_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(
            cx.listener(|this, _: &Snooze, window, cx| this.open_snooze_picker(window, cx)),
        )
        .on_action(cx.listener(|this, _: &Unsnooze, window, cx| this.unsnooze(window, cx)))
        .on_action(
            cx.listener(|this, _: &SendLater, window, cx| this.open_send_later_picker(window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &GoToSnoozed, window, cx| this.go_to(Nav::Snoozed, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &GoToScheduled, window, cx| {
                this.go_to(Nav::Scheduled, window, cx)
            }),
        )
        .on_action(cx.listener(|this, _: &EditScheduledSend, window, cx| {
            this.stop_scheduled(true, window, cx)
        }))
        .on_action(cx.listener(|this, _: &CancelScheduledSend, window, cx| {
            this.stop_scheduled(false, window, cx)
        }))
    }

    fn open_later_picker(
        &mut self,
        title: &str,
        subtitle: String,
        target: LaterTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.palette = None;
        self.help_open = false;
        self.contact_card = None;
        let picker = cx.new(|cx| LaterPicker::new(title.to_string(), subtitle, window, cx));
        let events = cx.subscribe_in(&picker, window, Self::on_later_event);
        picker.update(cx, |p, cx| p.focus(window, cx));
        self.later_picker = Some((picker, target, events));
        cx.notify();
    }

    /// `b`: when the targets come back. Drafts, spam and deleted mail
    /// can't be snoozed.
    fn open_snooze_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Mail {
            return;
        }
        let ids = self.ids_where(&self.targets(), |t| {
            !matches!(t.folder, Folder::Drafts | Folder::Spam | Folder::Trash)
        });
        let subtitle = match ids.as_slice() {
            [] => {
                if !self.targets().is_empty() {
                    self.show_toast(
                        ToastKind::Info,
                        "Drafts, spam and deleted conversations can't be snoozed",
                        None,
                        cx,
                    );
                }
                return;
            }
            [id] => self
                .data
                .thread_ix(id)
                .map(|ix| self.data.threads[ix].subject_or_placeholder().to_string())
                .unwrap_or_default(),
            ids => format!("{} conversations", ids.len()),
        };
        self.open_later_picker(
            "Snooze Until…",
            subtitle,
            LaterTarget::Snooze(ids),
            window,
            cx,
        );
    }

    /// ⇧⌘↩: when the open draft goes. It's checked like a send first.
    fn open_send_later_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((composer, _)) = &self.composer else {
            return;
        };
        let composer = composer.clone();
        if !super::compose::ready_to_send(&composer, window, cx) {
            return;
        }
        composer.update(cx, |c, cx| c.remember_focus(window, cx));
        let subject = composer.read(cx).content(cx).subject;
        let subtitle = if subject.trim().is_empty() {
            "(no subject)".to_string()
        } else {
            subject
        };
        self.open_later_picker(
            "Send Later…",
            subtitle,
            LaterTarget::Send(composer),
            window,
            cx,
        );
    }

    fn on_later_event(
        &mut self,
        _: &Entity<LaterPicker>,
        event: &LaterEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .later_picker
            .as_mut()
            .map(|(_, target, _)| std::mem::replace(target, LaterTarget::Snooze(Vec::new())));
        self.close_overlays(window, cx);
        match (event, target) {
            (LaterEvent::Picked(at), Some(LaterTarget::Snooze(ids))) => {
                self.snooze_threads(ids, *at, window, cx)
            }
            (LaterEvent::Picked(at), Some(LaterTarget::Send(composer))) => {
                self.schedule_send(composer, *at, window, cx)
            }
            // Back to the draft as it was.
            (LaterEvent::Dismiss, Some(LaterTarget::Send(composer))) => {
                composer.update(cx, |c, cx| c.restore_focus(window, cx))
            }
            _ => {}
        }
    }

    // Snooze.

    /// The stored snoozes of `ids`, for an undo inside the window.
    fn snooze_rows(&self, ids: &[String], cx: &gpui::App) -> Vec<(String, Option<SnoozeRow>)> {
        let rows = self.backend.read(cx).snoozes();
        ids.iter()
            .map(|id| (id.clone(), rows.get(id).cloned()))
            .collect()
    }

    /// Put each thread's snooze at `to` here, and hand the change to the
    /// backend; what puts it back, for the undo stack.
    fn set_snoozes(
        &mut self,
        targets: Vec<(String, Option<i64>, bool)>,
        at_once: bool,
        cx: &mut Context<Self>,
    ) -> SnoozeSet {
        let ids: Vec<String> = targets.iter().map(|(id, ..)| id.clone()).collect();
        let rows = self.snooze_rows(&ids, cx);
        let mut undo = Vec::new();
        let mut changes = Vec::new();
        for (id, to, inbox) in targets {
            let Some(ix) = self.data.thread_ix(&id) else {
                continue;
            };
            let thread = &mut self.data.threads[ix];
            let from = thread.later.snoozed_until;
            if from == to {
                continue;
            }
            undo.push(SnoozeUndo {
                thread: id.clone(),
                from,
                to,
                folder: thread.folder,
            });
            thread.later.snoozed_until = to;
            match to {
                Some(_) => {
                    thread.later.woke_at = None;
                    if thread.folder == Folder::Inbox {
                        thread.folder = Folder::Archive;
                    }
                }
                None if inbox && thread.folder == Folder::Archive => thread.folder = Folder::Inbox,
                None => {}
            }
            changes.push(SnoozeChange {
                thread: id,
                to,
                inbox,
            });
        }
        let ops = if changes.is_empty() {
            Vec::new()
        } else {
            self.backend
                .update(cx, |b, cx| b.set_snoozes(changes, at_once, cx))
        };
        (undo, rows, ops)
    }

    /// Snooze `ids` until `at`, with the selection moving on like an
    /// archive, and Undo on the toast.
    pub(crate) fn snooze_threads(
        &mut self,
        ids: Vec<String>,
        at: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stash_composer(cx);
        let selected = self.selected.map(|ix| self.data.threads[ix].id.clone());
        let next = self.next_staying(&ids);
        let count = ids.len();
        let targets = ids.into_iter().map(|id| (id, Some(at), true)).collect();
        let (changes, rows, ops) = self.set_snoozes(targets, false, cx);
        if changes.is_empty() {
            return;
        }
        self.clear_checked();
        self.refresh_rows(true);
        if self.selected_row().is_none() && selected.is_some() {
            match next
                .filter(|_| self.settings.auto_advance)
                .and_then(|id| self.data.thread_ix(&id))
                .filter(|ix| self.rows.contains(&Row::Thread(*ix)))
            {
                Some(ix) => self.select_thread(ix, window, cx),
                None => self.selected = None,
            }
        }
        let what = match count {
            1 => String::new(),
            n => format!(" {n} conversations"),
        };
        self.show_undoable(
            format!("Snoozed{what} until {}", when(at)),
            "Snooze",
            UndoAction::Snooze { changes, rows, ops },
            cx,
        );
        cx.notify();
    }

    /// `⇧B`: the snoozed targets back in the Inbox now.
    fn unsnooze(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Mail {
            return;
        }
        let ids = self.ids_where(&self.targets(), |t| t.later.snoozed_until.is_some());
        if ids.is_empty() {
            return;
        }
        let count = ids.len();
        let targets = ids.into_iter().map(|id| (id, None, true)).collect();
        let (changes, rows, ops) = self.set_snoozes(targets, false, cx);
        self.clear_checked();
        self.refresh_rows(true);
        if self.selected_row().is_none() {
            self.selected = None;
        }
        let message = match count {
            1 => "Unsnoozed: back in Inbox".to_string(),
            n => format!("Unsnoozed {n} conversations: back in Inbox"),
        };
        self.show_undoable(
            message,
            "Unsnooze",
            UndoAction::Snooze { changes, rows, ops },
            cx,
        );
        cx.notify();
    }

    /// Put snoozes back as `changes` had them before. Inside the undo
    /// window the ops are cancelled; past it, the reverse goes (at once
    /// for an undo, after the window again for a redo).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn undo_snooze(
        &mut self,
        changes: Vec<SnoozeUndo>,
        rows: Vec<(String, Option<SnoozeRow>)>,
        ops: Vec<OpId>,
        fresh: bool,
        undoing: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> UndoAction {
        let ids: Vec<String> = changes.iter().map(|c| c.thread.clone()).collect();
        let cancelled = fresh
            && !ops.is_empty()
            && self
                .backend
                .update(cx, |b, cx| b.cancel_snoozes(&ops, &rows, cx));
        let (inverse, rows, ops) = if cancelled {
            let now_rows = self.snooze_rows(&ids, cx);
            let mut inverse = Vec::new();
            for c in &changes {
                if let Some(ix) = self.data.thread_ix(&c.thread) {
                    let thread = &mut self.data.threads[ix];
                    inverse.push(SnoozeUndo {
                        thread: c.thread.clone(),
                        from: c.to,
                        to: c.from,
                        folder: thread.folder,
                    });
                    thread.later.snoozed_until = c.from;
                    thread.folder = c.folder;
                }
            }
            (inverse, now_rows, Vec::new())
        } else {
            let targets = changes
                .iter()
                .map(|c| (c.thread.clone(), c.from, c.folder == Folder::Inbox))
                .collect();
            self.set_snoozes(targets, undoing, cx)
        };
        self.refresh_rows(true);
        if self.selected_row().is_none() {
            self.selected = None;
        }
        cx.notify();
        UndoAction::Snooze {
            changes: inverse,
            rows,
            ops,
        }
    }

    // Send Later.

    /// Send the draft at `at`: saved first, so it waits in Scheduled (and
    /// in Gmail's Drafts) until then. Undo takes it back and reopens it.
    fn schedule_send(
        &mut self,
        composer: Entity<Composer>,
        at: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_draft(&composer, true, cx);
        let c = composer.read(cx);
        let Some(draft_id) = c.draft_id.clone() else {
            return;
        };
        let content = c.content(cx);
        let html = c.html(&content);
        let draft = self.outgoing(c, content, html);
        let thread_id = c
            .reply_to
            .clone()
            .filter(|id| self.data.thread_ix(id).is_some())
            .unwrap_or_else(|| self.new_id("sent"));
        let op = self.backend.update(cx, |b, cx| {
            b.send_later(
                MailOp::Send {
                    draft,
                    local_id: Some(draft_id.clone()),
                    thread_id,
                },
                at,
                cx,
            )
        });
        if let Some(ix) = self.data.thread_ix(&draft_id) {
            self.data.threads[ix].later.send_at = Some(at);
        }
        composer.update(cx, |c, _| c.set_closed(true));
        self.forget_draft_window(&composer);
        let was_selected = self
            .selected
            .is_some_and(|ix| self.data.threads[ix].id == draft_id);
        self.close_composer(window, cx);
        self.refresh_rows(true);
        if was_selected || self.selected_row().is_none() {
            self.selected = None;
        }
        let undo = self.push_undo(
            "Send Later",
            UndoAction::SendLater {
                draft_id,
                composer,
                op,
            },
            cx,
        );
        self.show_toast(
            ToastKind::Done,
            format!("Scheduled for {}", when(at)),
            Some(ToastAction::Undo(undo)),
            cx,
        );
        cx.notify();
    }

    /// The selected scheduled draft's send stops: back to Drafts, or open
    /// to edit (`edit`). Too late once it has started to go.
    fn stop_scheduled(&mut self, edit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else {
            return;
        };
        if self.data.threads[ix].later.send_at.is_none() {
            return;
        }
        let id = self.data.threads[ix].id.clone();
        if !self
            .cancel_schedules(std::slice::from_ref(&id), cx)
            .is_empty()
        {
            self.show_toast(ToastKind::Info, "Already sending", None, cx);
            return;
        }
        if edit {
            self.set_nav(Nav::Folder(Folder::Drafts), cx);
            if let Some(ix) = self.data.thread_ix(&id) {
                self.selected = None;
                self.select_thread(ix, window, cx);
            }
        } else {
            self.refresh_rows(true);
            if self.selected_row().is_none() {
                self.selected = None;
            }
            let open = ToastAction::run("Open Drafts", |this, window, cx| {
                this.go_to(Nav::Folder(Folder::Drafts), window, cx)
            });
            self.show_toast(
                ToastKind::Done,
                "Send canceled. It’s in Drafts",
                Some(open),
                cx,
            );
        }
        cx.notify();
    }

    /// Take back the scheduled sends of these scheduled drafts, before a
    /// discard or an edit: those that have started to go already.
    pub(super) fn cancel_schedules(
        &mut self,
        draft_ids: &[String],
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let sends: Vec<_> = self
            .backend
            .read(cx)
            .scheduled_sends()
            .into_iter()
            .filter(|s| draft_ids.contains(&s.local_id))
            .collect();
        // One already going isn't pending any more.
        let mut going: Vec<String> = draft_ids
            .iter()
            .filter(|id| !sends.iter().any(|s| &s.local_id == *id))
            .cloned()
            .collect();
        for send in sends {
            if !self.backend.update(cx, |b, _| b.cancel(send.op)) {
                going.push(send.local_id);
                continue;
            }
            self.forget_send(send.op, cx);
            if let Some(ix) = self.data.thread_ix(&send.local_id) {
                self.data.threads[ix].later.send_at = None;
            }
        }
        going
    }
}

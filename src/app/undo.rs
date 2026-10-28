//! The session's undo and redo (MA-071): the last `UNDO_DEPTH` changes,
//! apart from the toast that offers the newest. Inside the backend's undo
//! window a change's op is cancelled; after it, the inverse op goes.

use std::time::Instant;

use chrono::NaiveDateTime;
use gpui::{Context, Entity, SharedString, Window};

use super::{Mode, PigeonApp, ToastAction, ToastKind};
use crate::{
    actions::Undo,
    backend::{OpId, UNDO_WINDOW, categories::Category, gmail},
    data::{Folder, Thread},
    ui::{
        calendar::{Discarded, Removal},
        composer::Composer,
    },
};

/// How many changes Undo can walk back.
pub const UNDO_DEPTH: usize = 20;

pub enum UndoAction {
    /// Take back a sent message and reopen its draft. Never redone.
    Send {
        thread_id: String,
        /// Appended to an existing thread, rather than a new one in Sent.
        appended: bool,
        /// The Drafts thread sending removed.
        removed_draft: Option<Thread>,
        composer: Entity<Composer>,
        op: OpId,
    },
    Discard {
        removed_draft: Option<Thread>,
        composer: Entity<Composer>,
        /// Deleting the saved draft, if there was one.
        op: Option<OpId>,
    },
    /// Put deleted calendar events (or occurrences) back, or a split series
    /// together again.
    DeleteEvent(Box<Removal>),
    /// Bring back a discarded new calendar event, open and still unsaved.
    DiscardEvent(Box<Discarded>),
    /// Put moved threads back in their folders: (thread id, old folder).
    Move {
        moves: Vec<(String, Folder)>,
        selected: Option<String>,
        ops: Vec<OpId>,
    },
    /// Label, star, read or Important changes: undone by taking `add` off
    /// `threads` and putting `remove` back. Only threads the change changed.
    Labels {
        threads: Vec<String>,
        add: Vec<String>,
        remove: Vec<String>,
    },
    /// Threads moved to a Gmail category: (thread id, category before).
    Category { moves: Vec<(String, Category)> },
    /// A calendar event moved or resized.
    Retime(Retime),
}

/// An event's times before and after a change. Undo retimes it back,
/// finding the occurrence by its start now (`after.0`).
#[derive(Clone, Debug, PartialEq)]
pub struct Retime {
    pub event_id: String,
    pub before: (NaiveDateTime, NaiveDateTime),
    pub after: (NaiveDateTime, NaiveDateTime),
}

pub struct UndoEntry {
    /// What the change was, for "Undo Archive" ("Archive").
    pub title: SharedString,
    pub action: UndoAction,
    at: Instant,
    id: u64,
}

#[derive(Default)]
pub struct UndoStack {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    next_id: u64,
}

impl UndoStack {
    fn entry(&mut self, title: SharedString, action: UndoAction) -> UndoEntry {
        self.next_id += 1;
        UndoEntry {
            title,
            action,
            at: Instant::now(),
            id: self.next_id,
        }
    }

    /// A new change: redo history goes. Returns its id and the entries that
    /// fell off.
    fn push(&mut self, title: SharedString, action: UndoAction) -> (u64, Vec<UndoEntry>) {
        let mut dropped = std::mem::take(&mut self.redo);
        let (id, more) = self.push_undo(title, action);
        dropped.extend(more);
        (id, dropped)
    }

    /// Onto the undo side, keeping redo history (a redo).
    fn push_undo(&mut self, title: SharedString, action: UndoAction) -> (u64, Vec<UndoEntry>) {
        let entry = self.entry(title, action);
        let id = entry.id;
        self.undo.push(entry);
        let over = self.undo.len().saturating_sub(UNDO_DEPTH);
        (id, self.undo.drain(..over).collect())
    }

    fn push_redo(&mut self, title: SharedString, action: UndoAction) {
        let entry = self.entry(title, action);
        self.redo.push(entry);
        let over = self.redo.len().saturating_sub(UNDO_DEPTH);
        self.redo.drain(..over);
    }

    pub fn newest_id(&self) -> Option<u64> {
        self.undo.last().map(|e| e.id)
    }

    /// "Archive", for "Undo Archive".
    pub fn undo_title(&self) -> Option<&SharedString> {
        self.undo.last().map(|e| &e.title)
    }

    pub fn redo_title(&self) -> Option<&SharedString> {
        self.redo.last().map(|e| &e.title)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Take out the entries `keep` says no to (a send that went).
    fn retain(&mut self, keep: impl Fn(&UndoAction) -> bool) {
        self.undo.retain(|e| keep(&e.action));
        self.redo.retain(|e| keep(&e.action));
    }

    /// Everything goes (another account's data).
    fn clear(&mut self) -> Vec<UndoEntry> {
        self.undo.drain(..).chain(self.redo.drain(..)).collect()
    }
}

/// Put a label change into a thread's fields.
pub(super) fn apply_labels(thread: &mut Thread, add: &[String], remove: &[String]) {
    for (ids, on) in [(add, true), (remove, false)] {
        for id in ids {
            match id.as_str() {
                gmail::UNREAD => thread.unread = on,
                gmail::STARRED => thread.starred = on,
                gmail::IMPORTANT => thread.important = on,
                _ if on => {
                    if !thread.labels.contains(id) {
                        thread.labels.push(id.clone());
                    }
                }
                _ => thread.labels.retain(|l| l != id),
            }
        }
    }
}

impl PigeonApp {
    /// Put a change on the undo stack without a toast: for a change the
    /// user sees happen (one star, one read). Returns its id for a toast.
    pub(crate) fn push_undo(
        &mut self,
        title: impl Into<SharedString>,
        action: UndoAction,
        cx: &mut Context<Self>,
    ) -> u64 {
        let (id, dropped) = self.undo_stack.push(title.into(), action);
        self.release(dropped, cx);
        self.undo_changed(cx);
        id
    }

    /// Put a change on the undo stack and say so, with Undo on the toast.
    pub(crate) fn show_undoable(
        &mut self,
        message: impl Into<SharedString>,
        title: impl Into<SharedString>,
        action: UndoAction,
        cx: &mut Context<Self>,
    ) {
        let id = self.push_undo(title, action, cx);
        self.show_toast(ToastKind::Done, message, Some(ToastAction::Undo(id)), cx);
    }

    /// A label change on `threads` (only those it changed): on the stack,
    /// with a toast when `message` is given.
    pub(super) fn push_label_change(
        &mut self,
        title: &str,
        threads: Vec<String>,
        add: Vec<String>,
        remove: Vec<String>,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if threads.is_empty() {
            return;
        }
        let action = UndoAction::Labels {
            threads,
            add,
            remove,
        };
        match message {
            Some(message) => self.show_undoable(message, title.to_string(), action, cx),
            None => {
                self.push_undo(title.to_string(), action, cx);
            }
        }
    }

    /// Entries that left the stack for good: a discarded draft's held
    /// copies go with it (ATT-7).
    fn release(&mut self, entries: Vec<UndoEntry>, cx: &mut Context<Self>) {
        for entry in entries {
            if let UndoAction::Discard { composer, .. } = entry.action {
                let files = composer.read(cx).file_paths();
                self.backend.read(cx).release_files(&files);
            }
        }
    }

    pub(super) fn clear_undo(&mut self, cx: &mut Context<Self>) {
        let entries = self.undo_stack.clear();
        self.release(entries, cx);
        self.undo_changed(cx);
    }

    /// A send went: it can no longer be taken back.
    pub(super) fn forget_send(&mut self, op: OpId, cx: &mut Context<Self>) {
        self.undo_stack
            .retain(|a| !matches!(a, UndoAction::Send { op: sent, .. } if *sent == op));
        self.undo_changed(cx);
    }

    /// The Edit menu's titles follow the stack (MB-006), from `render`.
    fn undo_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub(super) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.undo_last(window, cx);
    }

    pub(super) fn undo_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.undo_stack.undo.pop() else {
            self.show_toast(ToastKind::Info, "Nothing to undo", None, cx);
            return;
        };
        if self
            .toast
            .as_ref()
            .is_some_and(|t| matches!(t.action, Some(ToastAction::Undo(id)) if id == entry.id))
        {
            self.toast = None;
        }
        let fresh = entry.at.elapsed() < UNDO_WINDOW;
        let hidden = self.undoes_hidden_mail(&entry.action);
        if hidden {
            self.show_toast(ToastKind::Info, format!("Undid {}", entry.title), None, cx);
        }
        if let Some(inverse) = self.apply_undo(entry.action, fresh, true, window, cx) {
            self.undo_stack.push_redo(entry.title, inverse);
        }
        self.undo_changed(cx);
        cx.notify();
    }

    pub(super) fn redo_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.undo_stack.redo.pop() else {
            self.show_toast(ToastKind::Info, "Nothing to redo", None, cx);
            return;
        };
        let fresh = entry.at.elapsed() < UNDO_WINDOW;
        if self.undoes_hidden_mail(&entry.action) {
            self.show_toast(ToastKind::Info, format!("Redid {}", entry.title), None, cx);
        }
        if let Some(inverse) = self.apply_undo(entry.action, fresh, false, window, cx) {
            let (_, dropped) = self.undo_stack.push_undo(entry.title, inverse);
            self.release(dropped, cx);
        }
        self.undo_changed(cx);
        cx.notify();
    }

    /// A mail change undone or redone out of Mail's sight, which says so in
    /// a toast instead (MA-071).
    fn undoes_hidden_mail(&self, action: &UndoAction) -> bool {
        self.mode != Mode::Mail
            && matches!(
                action,
                UndoAction::Move { .. } | UndoAction::Labels { .. } | UndoAction::Category { .. }
            )
    }

    /// Put `action` into effect, and return what puts it back, for the
    /// other side of the stack. `fresh`: inside the undo window, so a held
    /// op can still be cancelled. `undoing`: an undo, not a redo.
    fn apply_undo(
        &mut self,
        action: UndoAction,
        fresh: bool,
        undoing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UndoAction> {
        match action {
            UndoAction::DeleteEvent(removal) => {
                self.calendar
                    .update(cx, |c, cx| c.restore(*removal, window, cx));
                // Undone in Mail: focus stays with the mail.
                if self.mode == Mode::Mail {
                    window.focus(&self.focus_handle, cx);
                }
                None
            }
            UndoAction::DiscardEvent(discarded) => {
                // The event comes back open in the calendar.
                if self.mode != Mode::Calendar {
                    self.set_mode(Mode::Calendar, window, cx);
                }
                self.calendar
                    .update(cx, |c, cx| c.restore_discarded(*discarded, window, cx));
                None
            }
            action @ (UndoAction::Send { .. } | UndoAction::Discard { .. }) => {
                self.undo_compose(action, fresh, window, cx);
                None
            }
            UndoAction::Move {
                moves,
                selected,
                ops,
            } => Some(self.undo_move(moves, selected, ops, fresh, undoing, window, cx)),
            UndoAction::Labels {
                threads,
                add,
                remove,
            } => Some(self.undo_labels(threads, add, remove, cx)),
            UndoAction::Category { moves } => Some(self.undo_category(moves, cx)),
            UndoAction::Retime(retime) => {
                let Retime {
                    event_id,
                    before,
                    after,
                } = retime;
                self.calendar.update(cx, |c, cx| {
                    c.retime_occurrence(&event_id, after.0, before.0, before.1, window, cx);
                    c.flush_edit(cx);
                });
                Some(UndoAction::Retime(Retime {
                    event_id,
                    before: after,
                    after: before,
                }))
            }
        }
    }

    /// Take `add` off the threads and put `remove` back, at once.
    fn undo_labels(
        &mut self,
        threads: Vec<String>,
        add: Vec<String>,
        remove: Vec<String>,
        cx: &mut Context<Self>,
    ) -> UndoAction {
        let targets: Vec<usize> = threads
            .iter()
            .filter_map(|id| self.data.thread_ix(id))
            .collect();
        for &ix in &targets {
            apply_labels(&mut self.data.threads[ix], &remove, &add);
        }
        self.queue_labels(&targets, remove.clone(), add.clone(), cx);
        self.refresh_rows(true);
        UndoAction::Labels {
            threads,
            add: remove,
            remove: add,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(n: usize) -> UndoAction {
        UndoAction::Labels {
            threads: vec![n.to_string()],
            add: vec![gmail::STARRED.into()],
            remove: Vec::new(),
        }
    }

    #[test]
    fn stack_keeps_the_newest_and_a_new_change_clears_redo() {
        let mut stack = UndoStack::default();
        for n in 0..UNDO_DEPTH + 3 {
            let (_, dropped) = stack.push(format!("Star {n}").into(), labels(n));
            assert_eq!(dropped.len(), usize::from(n >= UNDO_DEPTH));
        }
        assert_eq!(stack.undo.len(), UNDO_DEPTH);
        assert_eq!(
            stack.undo_title().map(|t| t.to_string()),
            Some(format!("Star {}", UNDO_DEPTH + 2))
        );
        let entry = stack.undo.pop().unwrap();
        stack.push_redo(entry.title, entry.action);
        assert!(stack.can_redo());
        stack.push("Archive".into(), labels(99));
        assert!(!stack.can_redo());
        assert_eq!(stack.undo_title().map(|t| t.as_ref()), Some("Archive"));
    }

    #[test]
    fn apply_labels_sets_flags_and_labels() {
        let mut thread = Thread::local("t".into(), "s".into(), Folder::Inbox, Vec::new(), None);
        let add = [gmail::STARRED.to_string(), "Label_1".to_string()];
        apply_labels(&mut thread, &add, &[gmail::UNREAD.to_string()]);
        assert!(thread.starred && !thread.unread);
        assert_eq!(thread.labels, vec!["Label_1".to_string()]);
        apply_labels(&mut thread, &[gmail::UNREAD.to_string()], &add);
        assert!(!thread.starred && thread.unread && thread.labels.is_empty());
    }
}

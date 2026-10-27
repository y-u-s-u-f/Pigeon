//! Mail beyond one thread at a time: picking several threads and acting on
//! them together, moving and labelling, the list's density, search
//! suggestions, and which commands can run now.

use gpui::{
    Action, App, Context, Div, Focusable as _, KeyBinding, ListOffset, Modifiers, Pixels, Point,
    Stateful, Window, component::native_menu::NativeMenu, prelude::*,
};

use std::time::Duration;

use super::{Mode, Nav, PigeonApp, Row, UndoAction};
use crate::{
    actions::*,
    assets::Icon,
    backend::{MailOp, Older, OpId, ViewKey, categories::Category, gmail},
    data::{Folder, InviteMethod, Thread},
    search::{GmailQuery, NextSuggestion, PrevSuggestion, ServerState},
    ui::command_palette::{Leading, PaletteCommand, PaletteItem},
};

/// How much of each thread the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density {
    /// Sender, subject and snippet on three lines.
    Comfortable,
    /// One line per thread.
    Compact,
}

/// A row of the dropdown under the search field.
#[derive(Clone, Debug, PartialEq)]
pub enum SearchSuggestion {
    /// An operator to start a term with, and what it does.
    Operator(&'static str, &'static str),
    Recent(String),
    /// A contact for the `from:` or `to:` being typed: the operator and
    /// the address.
    Contact {
        operator: &'static str,
        name: String,
        email: String,
    },
}

const OPERATORS: [(&str, &str); 13] = [
    ("from:", "Sender"),
    ("to:", "Recipient"),
    ("subject:", "Words in the subject"),
    ("label:", "Has a label"),
    ("is:unread", "Unread"),
    ("is:starred", "Starred"),
    ("has:attachment", "Has attachments"),
    ("in:all", "Every folder but Spam and Trash"),
    // W4b (SRC-9): Gmail answers these.
    ("after:", "Sent after a date, like 2026/01/31"),
    ("older_than:", "Older than 2d, 3m or 1y"),
    ("larger:", "Bigger than a size, like 5M"),
    ("is:important", "Marked important"),
    ("category:", "An Inbox category, like promotions"),
];

const RECENT_SEARCHES: usize = 5;

impl PigeonApp {
    /// Registers the handlers of this module's actions on the root.
    pub(super) fn on_mail_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(Self::toggle_selected))
            .on_action(cx.listener(|this, _: &ExtendSelectionDown, window, cx| {
                this.extend_selection(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExtendSelectionUp, window, cx| {
                this.extend_selection(false, window, cx)
            }))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(
                cx.listener(|this, _: &MoveTo, window, cx| this.open_move_palette(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ApplyLabel, window, cx| this.open_label_palette(window, cx)),
            )
            .on_action(cx.listener(|this, action: &MoveToFolder, window, cx| {
                this.move_to(action.0, window, cx)
            }))
            .on_action(
                cx.listener(|this, action: &ToggleLabel, _, cx| this.toggle_label(action.0, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleDensity, _, cx| {
                this.toggle_density();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextSuggestion, window, cx| {
                this.step_search_suggestion(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PrevSuggestion, window, cx| {
                this.step_search_suggestion(false, window, cx)
            }))
            .map(|root| {
                // From the palette, which leaves focus outside the draft.
                // With focus in the draft, the composer already passed it on.
                let app = cx.entity().downgrade();
                crate::ui::composer::on_format_actions(root, move |format, window, cx| {
                    let composer = app
                        .upgrade()
                        .and_then(|app| Some(app.read(cx).composer.as_ref()?.0.clone()));
                    match composer {
                        Some(composer) if !composer.read(cx).is_focused(window, cx) => {
                            composer.update(cx, |c, cx| c.format(format, window, cx))
                        }
                        _ => cx.propagate(),
                    }
                })
            })
    }

    pub(crate) fn toggle_density(&mut self) {
        self.density = match self.density {
            Density::Comfortable => Density::Compact,
            Density::Compact => Density::Comfortable,
        };
        self.list_state.remeasure();
        self.reveal_selected();
    }

    /// Unread threads in the Inbox, for the rail: Primary's, the number
    /// the sidebar and the Dock show (FOL-11).
    pub fn inbox_unread(&self) -> usize {
        self.view_unread(Nav::Folder(Folder::Inbox))
    }

    /// Scroll the selected thread's row into view. A row the list hasn't
    /// laid out (after a reset or a splice, or far from where it was) has no
    /// height yet, so `scroll_to_reveal_item` can't place it: that waits for
    /// the next frame's layout (see `finish_reveal`).
    pub(super) fn reveal_selected(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let list = &self.list_state;
        if row <= list.logical_scroll_top().item_ix || list.bounds_for_item(row).is_some() {
            list.scroll_to_reveal_item(row);
        } else {
            self.reveal_pending = true;
        }
    }

    /// From `render`: finish a reveal once this frame has laid the list out.
    pub(super) fn schedule_reveal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.reveal_pending) {
            return;
        }
        cx.on_next_frame(window, |this, _, cx| {
            this.finish_reveal();
            cx.notify();
        });
    }

    /// Layout has measured the rows around the viewport: a row among them
    /// is revealed exactly; one further away goes near the top.
    pub(super) fn finish_reveal(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let list = &self.list_state;
        if row <= list.logical_scroll_top().item_ix || list.bounds_for_item(row).is_some() {
            list.scroll_to_reveal_item(row);
        } else {
            list.scroll_to(ListOffset {
                item_ix: row.saturating_sub(1),
                offset_in_item: Pixels::ZERO,
            });
        }
    }

    // Picking several threads.

    pub fn is_checked(&self, ix: usize) -> bool {
        !self.checked.is_empty() && self.checked.contains(&self.data.threads[ix].id)
    }

    pub(super) fn clear_checked(&mut self) {
        self.checked.clear();
        self.select_anchor = None;
    }

    /// What an action applies to: the picked threads in list order, else
    /// the selected one.
    pub(crate) fn targets(&self) -> Vec<usize> {
        if self.checked.is_empty() {
            return self.selected.into_iter().collect();
        }
        let mut targets: Vec<usize> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Thread(ix) if self.is_checked(*ix) => Some(*ix),
                _ => None,
            })
            .collect();
        // Picked threads the list no longer shows (a search narrowed it)
        // still count.
        for (ix, thread) in self.data.threads.iter().enumerate() {
            if self.checked.contains(&thread.id) && !targets.contains(&ix) {
                targets.push(ix);
            }
        }
        targets
    }

    fn toggle_checked(&mut self, ix: usize) {
        let id = self.data.threads[ix].id.clone();
        if !self.checked.remove(&id) {
            self.checked.insert(id);
        }
    }

    /// `x`: pick or unpick the selected thread.
    fn toggle_selected(&mut self, _: &ToggleSelected, _: &mut Window, cx: &mut Context<Self>) {
        self.stash_composer(cx);
        let Some(ix) = self.selected else { return };
        self.toggle_checked(ix);
        self.select_anchor = Some(self.data.threads[ix].id.clone());
        self.reader_focused = false;
        cx.notify();
    }

    /// Pick every thread row between the anchor and `ix`.
    fn check_range(&mut self, ix: usize) {
        let anchor = self
            .select_anchor
            .as_ref()
            .and_then(|id| self.data.thread_ix(id))
            .unwrap_or(ix);
        let row_of = |ix: usize| self.rows.iter().position(|r| *r == Row::Thread(ix));
        let (Some(a), Some(b)) = (row_of(anchor), row_of(ix)) else {
            return;
        };
        self.checked = self.rows[a.min(b)..=a.max(b)]
            .iter()
            .filter_map(|row| match row {
                Row::Thread(ix) => Some(self.data.threads[*ix].id.clone()),
                _ => None,
            })
            .collect();
        if self.select_anchor.is_none() {
            self.select_anchor = Some(self.data.threads[anchor].id.clone());
        }
    }

    /// Shift-j / shift-k: grow or shrink the picked range from the anchor.
    fn extend_selection(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(start) = self.selected else {
            return self.select_adjacent(forward, window, cx);
        };
        self.stash_composer(cx);
        if self.select_anchor.is_none() || self.checked.is_empty() {
            self.select_anchor = Some(self.data.threads[start].id.clone());
        }
        let Some(row) = self.selected_row() else {
            return;
        };
        let Some(next) = self.neighbor_thread(row, forward) else {
            return;
        };
        self.selected = Some(next);
        self.check_range(next);
        self.reader_focused = false;
        self.reveal_selected();
        cx.notify();
    }

    /// Escape: let go of the picked threads, back to the selected one.
    fn clear_selection(&mut self, _: &ClearSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.checked.is_empty() {
            return cx.propagate();
        }
        self.clear_checked();
        if let Some(ix) = self.selected {
            self.select_thread(ix, window, cx);
        }
        cx.notify();
    }

    /// A click on a row: open it; with shift, pick the range to it; with
    /// ⌘, pick it too.
    pub(crate) fn click_row(
        &mut self,
        ix: usize,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modifiers.platform || modifiers.shift {
            self.stash_composer(cx);
            if modifiers.shift {
                if self.select_anchor.is_none() {
                    self.select_anchor = self.selected.map(|s| self.data.threads[s].id.clone());
                }
                self.check_range(ix);
            } else {
                if self.checked.is_empty()
                    && let Some(current) = self.selected.filter(|&s| s != ix)
                {
                    self.toggle_checked(current);
                }
                self.toggle_checked(ix);
                self.select_anchor = Some(self.data.threads[ix].id.clone());
            }
            self.selected = Some(ix);
            self.reader_focused = false;
            cx.notify();
            return;
        }
        self.clear_checked();
        self.select_thread(ix, window, cx);
    }

    /// The row's checkbox: pick or unpick it alone.
    pub(crate) fn toggle_row_checked(&mut self, ix: usize, cx: &mut Context<Self>) {
        let id = self.data.threads[ix].id.clone();
        self.stash_composer(cx);
        let Some(ix) = self.data.thread_ix(&id) else {
            return;
        };
        self.toggle_checked(ix);
        self.select_anchor = Some(self.data.threads[ix].id.clone());
        cx.notify();
    }

    // Acting on threads. Each takes the threads to act on, so the row hover
    // buttons can act on their own row and the rest on `targets()`.

    pub(crate) fn archive_threads(
        &mut self,
        targets: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_threads(targets, |t| t.folder.archive_target(), window, cx);
    }

    pub(crate) fn delete_threads(
        &mut self,
        targets: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drafts: Vec<usize> = targets
            .iter()
            .copied()
            .filter(|&ix| self.data.threads[ix].folder == Folder::Drafts)
            .collect();
        match drafts[..] {
            [] => {}
            // One draft: discarded like the discard button, with its undo.
            [ix] if targets.len() == 1 => {
                self.selected = Some(ix);
                return self.discard_selected_draft(window, cx);
            }
            _ => {
                let ids: Vec<String> = drafts
                    .iter()
                    .map(|&ix| self.data.threads[ix].id.clone())
                    .collect();
                if self.composer.as_ref().is_some_and(|(c, _)| {
                    c.read(cx)
                        .draft_id
                        .as_ref()
                        .is_some_and(|id| ids.contains(id))
                }) {
                    self.composer = None;
                }
                let selected = self.selected.map(|ix| self.data.threads[ix].id.clone());
                self.data.threads.retain(|t| !ids.contains(&t.id));
                self.backend.update(cx, |b, cx| {
                    for local_id in ids.iter().cloned() {
                        b.mail(MailOp::DeleteDraft { local_id }, cx);
                    }
                });
                self.selected = selected.and_then(|id| self.data.thread_ix(&id));
                self.checked.retain(|id| !ids.contains(id));
                self.refresh_rows(true);
                self.show_toast(format!("Discarded {} drafts", ids.len()), None, cx);
            }
        }
        let rest = self
            .targets_without_drafts(&targets)
            .into_iter()
            .collect::<Vec<_>>();
        self.move_threads(
            rest,
            |t| (t.folder != Folder::Trash).then_some(Folder::Trash),
            window,
            cx,
        );
    }

    fn targets_without_drafts(&self, targets: &[usize]) -> Vec<usize> {
        targets
            .iter()
            .copied()
            .filter(|&ix| {
                self.data
                    .threads
                    .get(ix)
                    .is_some_and(|t| t.folder != Folder::Drafts)
            })
            .collect()
    }

    pub(crate) fn move_to(&mut self, folder: Folder, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.targets_without_drafts(&self.targets());
        self.move_threads(targets, |_| Some(folder), window, cx);
    }

    /// Move threads to the folder `to` says (none: leave it), with an undo
    /// toast. When the selected thread goes, the selection moves on to the
    /// next thread that stays, like Gmail's auto-advance.
    fn move_threads(
        &mut self,
        targets: Vec<usize>,
        to: impl Fn(&Thread) -> Option<Folder>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            return;
        }
        // Saving an open draft can reorder the threads: hold on by id.
        let ids: Vec<String> = targets
            .iter()
            .map(|&ix| self.data.threads[ix].id.clone())
            .collect();
        self.stash_composer(cx);
        let selected = self.selected.map(|ix| self.data.threads[ix].id.clone());
        let selected_goes = selected.as_ref().is_some_and(|id| ids.contains(id));

        let thread_at = |row: &Row| match row {
            Row::Thread(ix) => Some(*ix),
            _ => None,
        };
        let moved_rows: Vec<usize> = ids
            .iter()
            .filter_map(|id| self.data.thread_ix(id))
            .filter_map(|ix| self.rows.iter().position(|r| *r == Row::Thread(ix)))
            .collect();
        let stays = |ix: &usize| !ids.contains(&self.data.threads[*ix].id);
        let next = moved_rows.iter().max().and_then(|&last| {
            self.rows[last + 1..]
                .iter()
                .filter_map(thread_at)
                .find(stays)
                .or_else(|| {
                    let first = *moved_rows.iter().min().expect("non-empty");
                    self.rows[..first]
                        .iter()
                        .rev()
                        .filter_map(thread_at)
                        .find(stays)
                })
                .map(|ix| self.data.threads[ix].id.clone())
        });

        let mut moves = Vec::new();
        for id in &ids {
            let Some(ix) = self.data.thread_ix(id) else {
                continue;
            };
            let thread = &mut self.data.threads[ix];
            if let Some(folder) = to(thread).filter(|f| *f != thread.folder) {
                moves.push((id.clone(), thread.folder));
                thread.folder = folder;
            }
        }
        if moves.is_empty() {
            return;
        }
        let destination = self
            .data
            .thread_ix(&moves[0].0)
            .map(|ix| self.data.threads[ix].folder)
            .expect("just moved");
        let ops = self.queue_moves(
            moves
                .iter()
                .map(|(id, from)| (id.clone(), *from, destination)),
            false,
            cx,
        );
        self.clear_checked();
        self.refresh_rows(true);
        if selected_goes {
            match next
                .filter(|_| self.settings.auto_advance)
                .and_then(|id| self.data.thread_ix(&id))
            {
                Some(ix) => self.select_thread(ix, window, cx),
                None => self.selected = None,
            }
        }

        let count = moves.len();
        let what = match count {
            1 => String::new(),
            n => format!(" {n} conversations"),
        };
        let message = match destination {
            Folder::Archive => format!("Archived{what}"),
            Folder::Trash => format!("Moved{what} to Trash"),
            folder => format!("Moved{what} to {}", folder.title()),
        };
        self.show_toast(
            message,
            Some(UndoAction::Move {
                moves,
                selected,
                ops,
            }),
            cx,
        );
        cx.notify();
    }

    /// Hand the backend the ops for threads moving (id, from, to), one
    /// batch per pair of folders. They wait out the undo window unless
    /// `at_once`.
    fn queue_moves(
        &mut self,
        moves: impl IntoIterator<Item = (String, Folder, Folder)>,
        at_once: bool,
        cx: &mut Context<Self>,
    ) -> Vec<OpId> {
        let mut groups: Vec<(Folder, Folder, Vec<String>)> = Vec::new();
        for (id, from, to) in moves {
            match groups.iter_mut().find(|(f, t, _)| *f == from && *t == to) {
                Some((_, _, ids)) => ids.push(id),
                None => groups.push((from, to, vec![id])),
            }
        }
        self.backend.update(cx, |b, cx| {
            groups
                .into_iter()
                .flat_map(|(from, to, ids)| MailOp::moves(ids, from, to))
                .map(|op| {
                    if at_once {
                        b.mail_now(op, cx)
                    } else {
                        b.mail(op, cx)
                    }
                })
                .collect()
        })
    }

    /// Add and remove labels on threads (by index), at once.
    pub(super) fn queue_labels(
        &mut self,
        targets: &[usize],
        add: Vec<String>,
        remove: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            return;
        }
        let threads = targets
            .iter()
            .map(|&ix| self.data.threads[ix].id.clone())
            .collect();
        self.backend.update(cx, |b, cx| {
            b.mail(
                MailOp::Modify {
                    threads,
                    add,
                    remove,
                },
                cx,
            )
        });
    }

    /// Put moved threads back, and the selection where it was.
    pub(super) fn undo_move(
        &mut self,
        moves: Vec<(String, Folder)>,
        selected: Option<String>,
        ops: Vec<OpId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Inside the undo window nothing went; past it, move them back at
        // once (for a delete: untrash, then back to the folder).
        let all_cancelled = self.backend.update(cx, |b, _| {
            // Every one, not just up to the first that already went.
            let cancelled: Vec<bool> = ops.iter().map(|&op| b.cancel(op)).collect();
            cancelled.into_iter().all(|c| c)
        });
        let mut back = Vec::new();
        for (id, folder) in moves {
            if let Some(ix) = self.data.thread_ix(&id) {
                back.push((id, self.data.threads[ix].folder, folder));
                self.data.threads[ix].folder = folder;
            }
        }
        if !all_cancelled {
            self.queue_moves(back, true, cx);
        }
        self.refresh_rows(true);
        // Back where it was, without marking it read.
        if let Some(ix) = selected.and_then(|id| self.data.thread_ix(&id)) {
            if self.rows.contains(&Row::Thread(ix)) {
                self.selected = Some(ix);
                self.reveal_selected();
            } else {
                self.reveal_thread(ix, window, cx);
            }
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(crate) fn star_threads(&mut self, targets: Vec<usize>, cx: &mut Context<Self>) {
        let star = targets.iter().any(|&ix| !self.data.threads[ix].starred);
        for &ix in &targets {
            self.data.threads[ix].starred = star;
        }
        let label = vec![gmail::STARRED.to_string()];
        if star {
            self.queue_labels(&targets, label, Vec::new(), cx);
        } else {
            self.queue_labels(&targets, Vec::new(), label, cx);
        }
        if self.nav == Nav::Starred {
            self.refresh_rows(true);
        }
        cx.notify();
    }

    pub(crate) fn set_unread(&mut self, targets: Vec<usize>, unread: bool, cx: &mut Context<Self>) {
        for &ix in &targets {
            self.data.threads[ix].unread = unread;
        }
        let label = vec![gmail::UNREAD.to_string()];
        if unread {
            self.queue_labels(&targets, label, Vec::new(), cx);
        } else {
            self.queue_labels(&targets, Vec::new(), label, cx);
        }
        cx.notify();
    }

    /// Add the label to the targets, or take it off when they all have it.
    pub(crate) fn toggle_label(&mut self, label: usize, cx: &mut Context<Self>) {
        let Some(id) = self.data.labels.get(label).map(|l| l.id.clone()) else {
            return;
        };
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let remove = targets
            .iter()
            .all(|&ix| self.data.threads[ix].labels.contains(&id));
        for &ix in &targets {
            let labels = &mut self.data.threads[ix].labels;
            if remove {
                labels.retain(|l| *l != id);
            } else if !labels.contains(&id) {
                labels.push(id.clone());
            }
        }
        if remove {
            self.queue_labels(&targets, Vec::new(), vec![id], cx);
        } else {
            self.queue_labels(&targets, vec![id], Vec::new(), cx);
        }
        if matches!(self.nav, Nav::Label(_)) {
            self.refresh_rows(true);
        }
        let name = &self.data.labels[label].name;
        let message = if remove {
            format!("Removed “{name}”")
        } else {
            format!("Labelled “{name}”")
        };
        self.show_toast(message, None, cx);
        cx.notify();
    }

    /// `v`: the palette, listing the folders to move to.
    fn open_move_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.targets_without_drafts(&self.targets());
        if targets.is_empty() {
            return;
        }
        let here = targets
            .iter()
            .map(|&ix| self.data.threads[ix].folder)
            .reduce(|a, b| if a == b { a } else { Folder::Drafts });
        let items = Folder::ALL
            .into_iter()
            .filter(|&f| f != Folder::Drafts && f != Folder::Sent && Some(f) != here)
            .map(|folder| PaletteItem {
                command: PaletteCommand::MoveTo(folder),
                title: folder.title().into(),
                subtitle: None,
                leading: Leading::Icon(folder_icon(folder)),
                binding: None,
                search_only: false,
            })
            .collect();
        self.show_palette(
            items,
            "",
            "Move to…".into(),
            "No such folder".into(),
            window,
            cx,
        );
    }

    /// `l`: the palette, listing the labels to add or take off.
    fn open_label_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        // W4b: as a tree (FOL-9).
        let items = self.label_picker_items(&targets);
        self.show_palette(
            items,
            "",
            "Label as…".into(),
            "No such label".into(),
            window,
            cx,
        );
    }

    /// The thread actions in a native menu at `position`: the toolbar's "…"
    /// (`with_replies` false) or a row's context menu.
    pub(crate) fn show_thread_menu(
        &mut self,
        position: Point<Pixels>,
        with_replies: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.targets();
        let Some(&first) = targets.first() else {
            return;
        };
        let thread = &self.data.threads[first];
        let several = targets.len() > 1;
        let unread = targets.iter().all(|&ix| self.data.threads[ix].unread);
        let starred = targets.iter().all(|&ix| self.data.threads[ix].starred);
        let draft = thread.folder == Folder::Drafts;

        let mut menu = NativeMenu::new();
        if with_replies && !several && !draft {
            menu = menu
                .menu("Reply", Box::new(Reply))
                .menu("Reply All", Box::new(ReplyAll))
                .menu("Forward", Box::new(Forward))
                .separator();
        }
        if with_replies {
            if let Some(target) = thread.folder.archive_target() {
                let label = if target == Folder::Inbox {
                    "Move to Inbox"
                } else {
                    "Archive"
                };
                menu = menu.menu(label, Box::new(Archive));
            }
            if thread.folder != Folder::Trash {
                menu = menu.menu(
                    if draft { "Discard Draft" } else { "Delete" },
                    Box::new(Delete),
                );
            }
            menu = menu
                .menu(
                    if starred { "Unstar" } else { "Star" },
                    Box::new(ToggleStar),
                )
                .separator();
        }
        menu = if unread {
            menu.menu("Mark as Read", Box::new(MarkRead))
        } else {
            menu.menu("Mark as Unread", Box::new(MarkUnread))
        };
        // W4b (FOL-12).
        let important = targets.iter().all(|&ix| self.data.threads[ix].important);
        menu = menu.menu(
            if important {
                "Mark as Not Important"
            } else {
                "Mark as Important"
            },
            Box::new(ToggleImportant),
        );
        if !draft {
            let mut folders = NativeMenu::new();
            for folder in Folder::ALL {
                if folder != Folder::Drafts && folder != Folder::Sent {
                    folders = folders.menu_with_disabled(
                        folder.title(),
                        !several && folder == thread.folder,
                        Box::new(MoveToFolder(folder)),
                    );
                }
            }
            menu = menu.submenu("Move to", folders);
        }
        let mut labels = NativeMenu::new();
        for (ix, label) in self.data.labels.iter().enumerate() {
            let on = targets
                .iter()
                .all(|&t| self.data.threads[t].labels.contains(&label.id));
            labels = labels.menu_with_check(label.name.clone(), on, Box::new(ToggleLabel(ix)));
        }
        menu = menu.submenu("Label", labels);
        menu.show(position, window, cx);
    }

    /// Right-click on a row: act on it (or on the picked threads it's one
    /// of) from a menu.
    pub(crate) fn row_context_menu(
        &mut self,
        ix: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_checked(ix) {
            self.clear_checked();
            self.select_thread(ix, window, cx);
        }
        self.reader_focused = false;
        window.focus(&self.focus_handle, cx);
        self.show_thread_menu(position, true, window, cx);
    }

    // The palette.

    /// Whether a command can do anything now; the palette offers only
    /// those.
    pub(crate) fn command_available(&self, when: When, cx: &App) -> bool {
        let thread = self.selected.map(|ix| &self.data.threads[ix]);
        let mail = self.mode == Mode::Mail;
        match when {
            When::Always => true,
            When::Thread => mail && (thread.is_some() || !self.checked.is_empty()),
            When::Replyable => {
                mail && self.checked.is_empty()
                    && thread.is_some_and(|t| t.folder != Folder::Drafts)
            }
            When::Messages => mail && thread.is_some_and(|t| t.messages.len() > 1),
            When::Draft => mail && self.composer.is_some(),
            When::Undo => self.toast.as_ref().is_some_and(|t| t.undo.is_some()),
            When::Invite | When::Cancelled => {
                let method = if when == When::Invite {
                    InviteMethod::Request
                } else {
                    InviteMethod::Cancel
                };
                mail && self.selected_invite().is_some_and(|invite| {
                    invite.method == method
                        && self
                            .calendar
                            .read(cx)
                            .data
                            .event_ix(&invite.event_id)
                            .is_some()
                })
            }
            When::Searching => mail && self.search_open,
            When::Selection => mail && !self.checked.is_empty(),
            When::Inbox => mail && self.nav == Nav::Folder(Folder::Inbox),
            When::Labels => mail && !self.data.labels.is_empty(),
        }
    }

    /// The shortcut the mail panes run `action` with, for showing.
    pub(crate) fn binding_for(&self, action: &dyn Action, window: &Window) -> Option<KeyBinding> {
        lookup_contexts(Scope::Mail, false)
            .into_iter()
            .find_map(|context| {
                window.highest_precedence_binding_for_action_in_context(action, context)
            })
    }

    // Search suggestions.

    /// What the dropdown under the search field offers now: operators and
    /// recent searches while it's empty, contacts while a `from:` or `to:`
    /// is being typed.
    pub fn search_suggestions(&self, window: &Window, cx: &App) -> Vec<SearchSuggestion> {
        if !self.search_open || !self.search_input.focus_handle(cx).is_focused(window) {
            return Vec::new();
        }
        let value = self.search_input.read(cx).value();
        if value.trim().is_empty() {
            return OPERATORS
                .iter()
                .map(|&(op, what)| SearchSuggestion::Operator(op, what))
                .chain(
                    self.recent_searches
                        .iter()
                        .map(|q| SearchSuggestion::Recent(q.clone())),
                )
                .collect();
        }
        if value.ends_with(char::is_whitespace) {
            return Vec::new();
        }
        let last = value.split_whitespace().last().unwrap_or_default();
        let Some((operator, query)) = ["from:", "to:"]
            .into_iter()
            .find_map(|op| Some((op, last.strip_prefix(op)?)))
        else {
            return Vec::new();
        };
        if query.is_empty() || query.contains('@') {
            return Vec::new();
        }
        let book = self.contact_book.borrow();
        book.suggest(query, |_| false)
            .into_iter()
            .take(5)
            .map(|ix| {
                let contact = book.contact(ix);
                SearchSuggestion::Contact {
                    operator,
                    name: contact.display_name().to_string(),
                    email: contact.email.clone(),
                }
            })
            .collect()
    }

    fn step_search_suggestion(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.search_suggestions(window, cx).len();
        if count == 0 {
            return cx.propagate();
        }
        self.search_highlight = Some(match (self.search_highlight, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(ix), true) => (ix + 1) % count,
            (Some(ix), false) => (ix + count - 1) % count,
        });
        cx.notify();
    }

    /// Enter in the search field: take the suggestion moved to, if any.
    pub(super) fn accept_search_suggestion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(ix) = self.search_highlight else {
            return false;
        };
        match self.search_suggestions(window, cx).into_iter().nth(ix) {
            Some(suggestion) => {
                self.apply_search_suggestion(suggestion, window, cx);
                true
            }
            None => false,
        }
    }

    pub(crate) fn apply_search_suggestion(
        &mut self,
        suggestion: SearchSuggestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.search_input.read(cx).value().to_string();
        let text = match suggestion {
            // Operators that need a value wait for it; the rest are whole.
            SearchSuggestion::Operator(op, _) if op.ends_with(':') => op.to_string(),
            SearchSuggestion::Operator(op, _) => format!("{op} "),
            SearchSuggestion::Recent(query) => query,
            SearchSuggestion::Contact {
                operator, email, ..
            } => {
                let head = value
                    .rsplit_once(char::is_whitespace)
                    .map_or("", |(head, _)| head);
                let sep = if head.is_empty() { "" } else { " " };
                format!("{head}{sep}{operator}{email} ")
            }
        };
        self.set_search_text(&text, window, cx);
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Replace the search text, and search for it.
    fn set_search_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
        self.search_query = crate::search::Query::parse(text);
        self.search_highlight = None;
        self.refresh_rows(false);
        self.search_gmail(cx);
        cx.notify();
    }

    /// Keep the query for the suggestions, most recent first.
    pub(super) fn remember_search(&mut self, cx: &App) {
        let query = self.search_input.read(cx).value().trim().to_string();
        if query.is_empty() {
            return;
        }
        self.recent_searches.retain(|q| *q != query);
        self.recent_searches.insert(0, query);
        self.recent_searches.truncate(RECENT_SEARCHES);
    }

    /// The Search scope setting: where a new search looks. The open
    /// search takes it too.
    pub(crate) fn set_default_search_scope(&mut self, all: bool, cx: &mut Context<Self>) {
        self.settings.search_all_mail = all;
        self.search_all = all;
        if self.is_searching() {
            self.refresh_rows(false);
            self.search_gmail(cx);
        }
        cx.notify();
    }

    /// The scope chip: search this view, or all mail.
    pub(crate) fn toggle_search_scope(&mut self, cx: &mut Context<Self>) {
        self.search_all = !self.search_all;
        self.refresh_rows(false);
        self.search_gmail(cx);
        cx.notify();
    }
}

// Search and paging past the cache (W9).

impl PigeonApp {
    /// The view the list shows, as the backend names it.
    pub(crate) fn view_key(&self) -> ViewKey {
        match self.nav {
            // W4b: the Inbox shows Primary (FOL-11).
            Nav::Folder(Folder::Inbox) => ViewKey::Category(Category::Primary),
            Nav::Category(category) => ViewKey::Category(category),
            Nav::Folder(folder) => ViewKey::Folder(folder),
            Nav::Starred => ViewKey::Starred,
            Nav::Label(ix) => ViewKey::Label(self.data.labels[ix].id.clone()),
        }
    }

    /// Ask Gmail about the open search too (SRC-7), once typing has
    /// settled for 300 ms (SRC-8). Local results show meanwhile; a newer
    /// query drops the answer to this one.
    pub(crate) fn search_gmail(&mut self, cx: &mut Context<Self>) {
        // The Inbox's search covers all of it, every category and the
        // archived threads (`refresh_rows`).
        let view = (!self.search_all).then(|| match self.nav {
            Nav::Folder(Folder::Inbox) => ViewKey::Folder(Folder::Inbox),
            _ => self.view_key(),
        });
        let query = (self.is_searching() && self.backend.read(cx).can_search())
            .then(|| {
                self.search_query
                    .to_gmail_q(&self.data.labels, view.as_ref())
            })
            .flatten();
        let Some(serial) = self.server_search.ask(query.clone()) else {
            return;
        };
        let query = query.expect("asked");
        self.request_search(serial, query, None, crate::search::SEARCH_DEBOUNCE, cx);
    }

    /// The next page of Gmail's results, at the end of the search's rows.
    fn more_search_results(&mut self, cx: &mut Context<Self>) {
        if let Some((serial, query, page)) = self.server_search.more() {
            self.request_search(serial, query, Some(page), Duration::ZERO, cx);
            cx.notify();
        }
    }

    fn request_search(
        &mut self,
        serial: u64,
        query: GmailQuery,
        page: Option<String>,
        wait: Duration,
        cx: &mut Context<Self>,
    ) {
        self.server_search.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let Ok(request) = this.update(cx, |this, cx| {
                this.backend.update(cx, |b, cx| b.search(&query, page, cx))
            }) else {
                return;
            };
            let result = request.await;
            this.update(cx, |this, cx| {
                if this.server_search.answer(serial, result) {
                    this.refresh_rows(true);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Whether the list ends in "Load older" (LST-7): views Gmail pages,
    /// signed in; or more of Gmail's results for a search.
    pub(crate) fn shows_load_older(&self) -> bool {
        if !self.server_search.available {
            return false;
        }
        if self.is_searching() {
            return self.server_search.has_more();
        }
        self.nav != Nav::Folder(Folder::Drafts)
    }

    /// "Load older": Enter or a click on the row, `j` onto it, or the
    /// command.
    pub(crate) fn activate_load_older(&mut self, cx: &mut Context<Self>) {
        if self.is_searching() {
            return self.more_search_results(cx);
        }
        // The view's oldest cached thread: older ones come from Gmail.
        let oldest = self
            .data
            .threads
            .iter()
            .filter(|t| self.nav_contains(self.nav, t))
            .map(|t| t.last_message().internal_date)
            .min();
        let view = self.view_key();
        self.backend
            .update(cx, |b, cx| b.load_older(view, oldest, cx));
        cx.notify();
    }

    /// What the "Load older" row says, and whether it does anything now.
    pub(crate) fn load_older_label(&self, cx: &App) -> (&'static str, bool) {
        if self.is_searching() {
            return match self.server_search.state {
                ServerState::Waiting => ("Searching Gmail…", false),
                _ => ("More results from Gmail", true),
            };
        }
        match self.backend.read(cx).older(&self.view_key()) {
            Older::More => ("Load older conversations", true),
            Older::Loading => ("Loading older conversations…", false),
            Older::Done => ("No older conversations", false),
        }
    }
}

pub fn folder_icon(folder: Folder) -> Icon {
    match folder {
        Folder::Inbox => Icon::Inbox,
        Folder::Archive => Icon::Archive,
        Folder::Sent => Icon::Send,
        Folder::Drafts => Icon::FilePen,
        Folder::Spam => Icon::OctagonAlert,
        Folder::Trash => Icon::Trash,
    }
}

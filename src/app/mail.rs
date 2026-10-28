//! Mail beyond one thread at a time: picking several threads and acting on
//! them together, moving and labelling, the list's density, search
//! suggestions, and which commands can run now.

use gpui::{
    Action, App, Context, Div, Focusable as _, KeyBinding, ListOffset, Modifiers, Pixels, Point,
    Stateful, Window, component::native_menu::NativeMenu, prelude::*,
};

use std::time::Duration;

use super::{Mode, Nav, PigeonApp, Row, ToastKind, UndoAction};
use crate::backend::Source;
use crate::{
    actions::*,
    assets::Icon,
    backend::{MailOp, Older, OpId, ViewKey, categories::Category, gmail},
    data::{Folder, Thread},
    search::{GmailQuery, NextSuggestion, PrevSuggestion, ServerState},
    ui::{
        command_palette::{Leading, PaletteCommand, PaletteItem},
        thread_list::DraggedThreads,
    },
};

/// MB-062: move the selected threads to a Gmail category; from the thread
/// menu and the `Move to Category…` palette.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct MoveToCategoryOf(pub Category);

type ActionFn = fn() -> Box<dyn Action>;

/// The palette's rows run actions by `fn`, one per category.
const CATEGORY_ACTIONS: [(Category, ActionFn); 5] = [
    (Category::Primary, || {
        Box::new(MoveToCategoryOf(Category::Primary))
    }),
    (Category::Social, || {
        Box::new(MoveToCategoryOf(Category::Social))
    }),
    (Category::Promotions, || {
        Box::new(MoveToCategoryOf(Category::Promotions))
    }),
    (Category::Updates, || {
        Box::new(MoveToCategoryOf(Category::Updates))
    }),
    (Category::Forums, || {
        Box::new(MoveToCategoryOf(Category::Forums))
    }),
];

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
    /// "Clear Recent Searches", under the recent ones (ME-015).
    ClearRecent,
}

/// The first `COMMON_OPERATORS` show in an empty field; the rest once a
/// prefix of them is typed (ME-015).
const OPERATORS: [(&str, &str); 13] = [
    ("from:", "Sender"),
    ("to:", "Recipient"),
    ("subject:", "Words in the subject"),
    ("is:unread", "Unread"),
    ("has:attachment", "Has attachments"),
    ("in:all", "Every folder but Spam and Trash"),
    ("label:", "Has a label"),
    ("is:starred", "Starred"),
    // W4b (SRC-9): Gmail answers these.
    ("after:", "Sent after a date, like 2026/01/31"),
    ("older_than:", "Older than 2d, 3m or 1y"),
    ("larger:", "Bigger than a size, like 5M"),
    ("is:important", "Marked important"),
    ("category:", "An Inbox category, like promotions"),
];

const COMMON_OPERATORS: usize = 6;

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
            .on_action(cx.listener(|this, _: &SelectAllThreads, _, cx| this.select_all_threads(cx)))
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
            .on_action(
                cx.listener(|this, _: &ClearRecentSearches, _, cx| this.clear_recent_searches(cx)),
            )
            .on_action(cx.listener(|this, _: &MoveToCategory, window, cx| {
                this.open_category_palette(window, cx)
            }))
            .on_action(cx.listener(|this, action: &MoveToCategoryOf, _, cx| {
                let targets = this.targets();
                this.move_to_category(targets, action.0, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenInGmail, _, cx| this.open_in_gmail(cx)))
            // The reader pages itself (W2-SHELL); these are the list's.
            .on_action(
                cx.listener(|this, _: &PageDown, window, cx| this.page_list(true, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PageUp, window, cx| this.page_list(false, window, cx)),
            )
            .map(|root| {
                // From the palette, which leaves focus outside the draft.
                // With focus in the draft, the composer already passed it on.
                let app = cx.entity().downgrade();
                crate::ui::composer::on_format_actions(root, move |format, window, cx| {
                    let composer = app
                        .upgrade()
                        .and_then(|app| Some(app.read(cx).composer.as_ref()?.0.clone()));
                    match composer {
                        Some(composer) if !composer.read(cx).has_focus(window, cx) => {
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

    pub(crate) fn clear_checked(&mut self) {
        self.checked.clear();
        self.select_anchor = None;
    }

    /// Whether a row's context menu is open on it.
    pub fn is_context_target(&self, ix: usize) -> bool {
        self.context_target
            .as_ref()
            .is_some_and(|id| *id == self.data.threads[ix].id)
    }

    /// What an action applies to: the row whose context menu is open, else
    /// the picked threads in list order, else the selected one.
    pub(crate) fn targets(&self) -> Vec<usize> {
        let context = self
            .context_target
            .as_ref()
            .and_then(|id| self.data.thread_ix(id));
        action_targets(context, self.selected, || self.picked())
    }

    fn picked(&self) -> Vec<usize> {
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

    /// Make the context target the selected thread, alone: for the actions
    /// that open a conversation (reply, forward).
    pub(super) fn adopt_context_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self
            .context_target
            .take()
            .and_then(|id| self.data.thread_ix(&id))
        else {
            return;
        };
        self.clear_checked();
        if self.selected != Some(ix) {
            self.select_thread(ix, window, cx);
        }
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

    /// ⌘A in the list (MA-078): pick every conversation loaded, keeping
    /// the anchor.
    pub(crate) fn select_all_threads(&mut self, cx: &mut Context<Self>) {
        let rows: Vec<usize> = self.rows.iter().filter_map(Row::thread).collect();
        let Some(&first) = rows.first() else {
            return;
        };
        let ids: Vec<String> = rows
            .iter()
            .map(|&ix| self.data.threads[ix].id.clone())
            .collect();
        self.stash_composer(cx);
        if self.select_anchor.is_none() {
            let anchor = self.selected.unwrap_or(first);
            self.select_anchor = self.data.threads.get(anchor).map(|t| t.id.clone());
        }
        // Saving a draft can reorder the threads: back by id.
        self.checked = ids.into_iter().collect();
        if self.selected_row().is_none() {
            self.selected = self.rows.iter().find_map(Row::thread);
        }
        self.reader_focused = false;
        cx.notify();
    }

    /// PageDown / PageUp in the list (MB-045): the selection moves by a
    /// screenful of rows. With the reader focused they're the reader's.
    fn page_list(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.reader_focused {
            return cx.propagate();
        }
        let threads: Vec<(usize, usize)> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(row, r)| match r {
                Row::Thread(ix) => Some((row, *ix)),
                _ => None,
            })
            .collect();
        if threads.is_empty() {
            return;
        }
        let list = &self.list_state;
        let bottom = list.viewport_bounds().bottom();
        let top = list.logical_scroll_top().item_ix;
        let shown = (top..self.rows.len())
            .take_while(|&row| list.bounds_for_item(row).is_some_and(|b| b.top() < bottom))
            .count();
        let step = shown.saturating_sub(1).max(1);
        let at = self
            .selected_row()
            .and_then(|row| threads.iter().position(|&(r, _)| r == row));
        let to = match (at, down) {
            (None, _) => 0,
            (Some(at), true) => (at + step).min(threads.len() - 1),
            (Some(at), false) => at.saturating_sub(step),
        };
        self.move_selection_to(threads[to].1, window, cx);
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
        // CMP-17: a scheduled draft stops waiting to send first; one
        // already going stays.
        let scheduled = self.ids_where(&targets, |t| t.later.send_at.is_some());
        let going = if scheduled.is_empty() {
            Vec::new()
        } else {
            self.cancel_schedules(&scheduled, cx)
        };
        let mut targets = targets;
        if !going.is_empty() {
            targets.retain(|&ix| !going.contains(&self.data.threads[ix].id));
            self.show_toast(ToastKind::Info, "Already sending", None, cx);
            if targets.is_empty() {
                return;
            }
        }
        let drafts: Vec<usize> = targets
            .iter()
            .copied()
            .filter(|&ix| self.data.threads[ix].folder == Folder::Drafts)
            .collect();
        match drafts[..] {
            [] => {}
            // One draft: discarded like the discard button, with its undo.
            [ix] if targets.len() == 1 && self.selected == Some(ix) => {
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
                let message = match ids.len() {
                    1 => "Discarded draft".to_string(),
                    n => format!("Discarded {n} drafts"),
                };
                self.show_toast(ToastKind::Done, message, None, cx);
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
        let next = self.next_staying(&ids);

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
        let (message, title) = match destination {
            Folder::Archive => (format!("Archived{what}"), "Archive".to_string()),
            Folder::Trash => (format!("Moved{what} to Trash"), "Move to Trash".into()),
            folder => (
                format!("Moved{what} to {}", folder.title()),
                format!("Move to {}", folder.title()),
            ),
        };
        self.show_undoable(
            message,
            title,
            UndoAction::Move {
                moves,
                selected,
                ops,
            },
            cx,
        );
        cx.notify();
    }

    /// The thread to select once `ids` leave the list: the next row that
    /// stays, else the one before them.
    pub(super) fn next_staying(&self, ids: &[String]) -> Option<String> {
        let moved_rows: Vec<usize> = ids
            .iter()
            .filter_map(|id| self.data.thread_ix(id))
            .filter_map(|ix| self.rows.iter().position(|r| *r == Row::Thread(ix)))
            .collect();
        let stays = |ix: &usize| !ids.contains(&self.data.threads[*ix].id);
        moved_rows.iter().max().and_then(|&last| {
            self.rows[last + 1..]
                .iter()
                .filter_map(Row::thread)
                .find(stays)
                .or_else(|| {
                    let first = *moved_rows.iter().min().expect("non-empty");
                    self.rows[..first]
                        .iter()
                        .rev()
                        .filter_map(Row::thread)
                        .find(stays)
                })
                .map(|ix| self.data.threads[ix].id.clone())
        })
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

    /// Put one label on the targets (`on`) or take it off; the add and
    /// remove lists, for the undo stack.
    pub(super) fn queue_label(
        &mut self,
        targets: &[usize],
        label: String,
        on: bool,
        cx: &mut Context<Self>,
    ) -> (Vec<String>, Vec<String>) {
        let (add, remove) = if on {
            (vec![label], Vec::new())
        } else {
            (Vec::new(), vec![label])
        };
        self.queue_labels(targets, add.clone(), remove.clone(), cx);
        (add, remove)
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

    /// Put moved threads in the folders `moves` names, and the selection
    /// where it was; returns what moves them back again. Undo puts them
    /// back at once; redo holds the moves back for the undo window again.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn undo_move(
        &mut self,
        moves: Vec<(String, Folder)>,
        selected: Option<String>,
        ops: Vec<OpId>,
        fresh: bool,
        undoing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> UndoAction {
        // Inside the undo window nothing went; past it, move them back
        // with the inverse ops (for a delete: untrash, then back to the
        // folder).
        let all_cancelled = fresh
            && !ops.is_empty()
            && self.backend.update(cx, |b, _| {
                // Every one, not just up to the first that already went.
                let cancelled: Vec<bool> = ops.iter().map(|&op| b.cancel(op)).collect();
                cancelled.into_iter().all(|c| c)
            });
        let selected_before = self.selected.map(|ix| self.data.threads[ix].id.clone());
        let mut back = Vec::new();
        for (id, folder) in moves {
            if let Some(ix) = self.data.thread_ix(&id) {
                back.push((id, self.data.threads[ix].folder, folder));
                self.data.threads[ix].folder = folder;
            }
        }
        let inverse: Vec<(String, Folder)> = back
            .iter()
            .map(|(id, from, _)| (id.clone(), *from))
            .collect();
        let ops = if all_cancelled {
            Vec::new()
        } else {
            self.queue_moves(back, undoing, cx)
        };
        self.refresh_rows(true);
        // Back where it was, without marking it read. Undone from the
        // calendar: the list changes, but focus and the view stay there.
        let in_mail = self.mode == Mode::Mail;
        if let Some(ix) = selected.and_then(|id| self.data.thread_ix(&id)) {
            if self.rows.contains(&Row::Thread(ix)) {
                self.selected = Some(ix);
                if in_mail {
                    self.reveal_selected();
                }
            } else if in_mail {
                self.reveal_thread(ix, window, cx);
            }
        } else if self.selected_row().is_none() {
            // A redo took the selected thread out of view.
            self.selected = None;
        }
        if in_mail {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
        UndoAction::Move {
            moves: inverse,
            selected: if undoing { None } else { selected_before },
            ops,
        }
    }

    /// Whether the conversation's star was set a moment ago, so it pops
    /// as it shows (MD-027).
    pub(crate) fn star_pops(&self, id: &str) -> bool {
        const POP: std::time::Duration = std::time::Duration::from_millis(400);
        let (ids, at) = &self.just_starred;
        at.is_some_and(|at| at.elapsed() < POP) && ids.iter().any(|i| i == id)
    }

    pub(crate) fn star_threads(&mut self, targets: Vec<usize>, cx: &mut Context<Self>) {
        let star = targets.iter().any(|&ix| !self.data.threads[ix].starred);
        let changed = self.ids_where(&targets, |t| t.starred != star);
        for &ix in &targets {
            self.data.threads[ix].starred = star;
        }
        if star {
            self.just_starred = (changed.clone(), Some(std::time::Instant::now()));
        }
        let (add, remove) = self.queue_label(&targets, gmail::STARRED.to_string(), star, cx);
        let (title, done) = if star {
            ("Star", "Starred")
        } else {
            ("Unstar", "Unstarred")
        };
        let message = several(changed.len()).map(|n| format!("{done} {n} conversations"));
        self.push_label_change(title, changed, add, remove, message, cx);
        if self.nav == Nav::Starred {
            self.refresh_rows(true);
        }
        cx.notify();
    }

    pub(crate) fn set_unread(&mut self, targets: Vec<usize>, unread: bool, cx: &mut Context<Self>) {
        let changed = self.ids_where(&targets, |t| t.unread != unread);
        for &ix in &targets {
            self.data.threads[ix].unread = unread;
        }
        let (add, remove) = self.queue_label(&targets, gmail::UNREAD.to_string(), unread, cx);
        let (title, how) = if unread {
            ("Mark as Unread", "unread")
        } else {
            ("Mark as Read", "read")
        };
        let message = several(changed.len()).map(|n| format!("Marked {n} conversations as {how}"));
        self.push_label_change(title, changed, add, remove, message, cx);
        cx.notify();
    }

    /// The ids of the threads at `targets` that `pick` picks.
    pub(super) fn ids_where(
        &self,
        targets: &[usize],
        pick: impl Fn(&Thread) -> bool,
    ) -> Vec<String> {
        targets
            .iter()
            .map(|&ix| &self.data.threads[ix])
            .filter(|t| pick(t))
            .map(|t| t.id.clone())
            .collect()
    }

    /// Add the label to the targets, or take it off when they all have it.
    pub(crate) fn toggle_label(&mut self, label: usize, cx: &mut Context<Self>) {
        let Some(id) = self.data.labels.get(label).map(|l| l.id.clone()) else {
            return;
        };
        let targets = self.targets();
        let remove = targets
            .iter()
            .all(|&ix| self.data.threads[ix].labels.contains(&id));
        self.set_label(label, targets, remove, cx);
    }

    /// Add the label to `targets`, or take it off them.
    fn set_label(
        &mut self,
        label: usize,
        targets: Vec<usize>,
        remove: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.data.labels.get(label).map(|l| l.id.clone()) else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        let changed = self.ids_where(&targets, |t| t.labels.contains(&id) == remove);
        for &ix in &targets {
            let labels = &mut self.data.threads[ix].labels;
            if remove {
                labels.retain(|l| *l != id);
            } else if !labels.contains(&id) {
                labels.push(id.clone());
            }
        }
        let (add, take) = self.queue_label(&targets, id, !remove, cx);
        if matches!(self.nav, Nav::Label(_)) {
            self.refresh_rows(true);
        }
        let name = &self.data.labels[label].name;
        let (title, message) = if remove {
            ("Remove Label", format!("Removed “{name}”"))
        } else {
            ("Label", format!("Labeled “{name}”"))
        };
        self.push_label_change(title, changed, add, take, Some(message), cx);
        cx.notify();
    }

    /// MB-062: the Inbox threads among `targets` go to `category`, at once.
    /// Gmail's own categories, so no undo.
    pub(crate) fn move_to_category(
        &mut self,
        targets: Vec<usize>,
        category: Category,
        cx: &mut Context<Self>,
    ) {
        let targets: Vec<usize> = targets
            .into_iter()
            .filter(|&ix| leaves_for(&self.data.threads[ix], category))
            .collect();
        if targets.is_empty() {
            return;
        }
        let before = targets
            .iter()
            .map(|&ix| {
                let thread = &self.data.threads[ix];
                (thread.id.clone(), current_category(thread))
            })
            .collect();
        let mut change = (Vec::new(), Vec::new());
        for &ix in &targets {
            change = self.data.threads[ix].set_category(category);
        }
        let (add, remove) = change;
        self.queue_labels(&targets, add, remove, cx);
        self.refresh_rows(true);
        self.refresh_server_counts(cx);
        let message = match targets.len() {
            1 => format!("Moved to {}", category.name()),
            n => format!("Moved {n} conversations to {}", category.name()),
        };
        self.show_undoable(
            message,
            "Move to Category",
            UndoAction::Category { moves: before },
            cx,
        );
        cx.notify();
    }

    /// Put each thread (by id) back in its category; returns where they
    /// were, for the other side of the undo stack.
    pub(super) fn undo_category(
        &mut self,
        moves: Vec<(String, Category)>,
        cx: &mut Context<Self>,
    ) -> UndoAction {
        let mut before = Vec::new();
        for (id, category) in moves {
            let Some(ix) = self.data.thread_ix(&id) else {
                continue;
            };
            before.push((id, current_category(&self.data.threads[ix])));
            let (add, remove) = self.data.threads[ix].set_category(category);
            self.queue_labels(&[ix], add, remove, cx);
        }
        self.refresh_rows(true);
        self.refresh_server_counts(cx);
        UndoAction::Category { moves: before }
    }

    /// `Move to Category…`: the palette, listing the categories.
    fn open_category_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.targets();
        if !targets
            .iter()
            .any(|&ix| self.data.threads[ix].folder == Folder::Inbox)
        {
            self.show_toast(
                ToastKind::Info,
                "Only conversations in the Inbox have a category",
                None,
                cx,
            );
            return;
        }
        let items = CATEGORY_ACTIONS
            .iter()
            .map(|&(category, action)| PaletteItem {
                command: PaletteCommand::Action(action),
                title: category.name().into(),
                subtitle: None,
                leading: Leading::Icon(category_icon(category)),
                binding: None,
                search_only: false,
            })
            .collect();
        self.show_palette(
            items,
            "",
            "Move to category…".into(),
            "No such category".into(),
            window,
            cx,
        );
    }

    /// MB-061: the conversation in Gmail on the web, in this account.
    fn open_in_gmail(&mut self, cx: &mut Context<Self>) {
        let Some(&ix) = self.targets().first() else {
            return;
        };
        if self.backend.read(cx).is_demo() {
            return self.show_toast(ToastKind::Info, "Demo data isn't in Gmail", None, cx);
        }
        match self.data.threads[ix].gmail_link(&self.data.account.email) {
            Some(url) => cx.open_url(&url),
            None => self.show_toast(
                ToastKind::Info,
                "This conversation isn't in Gmail yet",
                None,
                cx,
            ),
        }
    }

    // Dragging conversations onto the sidebar (MA-087).

    /// Whether dropping `dragged` on `nav` changes anything, so only those
    /// rows light up.
    pub(crate) fn accepts_drop(&self, nav: Nav, dragged: &DraggedThreads) -> bool {
        let threads = dragged
            .ids
            .iter()
            .filter_map(|id| self.data.thread_ix(id))
            .map(|ix| &self.data.threads[ix]);
        let not_draft = |t: &&Thread| t.folder != Folder::Drafts;
        match nav {
            Nav::Folder(
                folder @ (Folder::Inbox | Folder::Archive | Folder::Spam | Folder::Trash),
            ) => threads.filter(not_draft).any(|t| t.folder != folder),
            Nav::Folder(_) | Nav::Snoozed | Nav::Scheduled => false,
            Nav::Starred => threads.filter(not_draft).any(|t| !t.starred),
            Nav::Label(ix) => self.data.labels.get(ix).is_some_and(|label| {
                threads
                    .filter(not_draft)
                    .any(|t| !t.labels.contains(&label.id))
            }),
            Nav::Category(category) => threads.into_iter().any(|t| leaves_for(t, category)),
        }
    }

    /// Drop `dragged` on a sidebar row: move, star, label or recategorize.
    pub(crate) fn drop_threads(
        &mut self,
        nav: Nav,
        dragged: &DraggedThreads,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_drop(nav, dragged) {
            return;
        }
        let targets: Vec<usize> = dragged
            .ids
            .iter()
            .filter_map(|id| self.data.thread_ix(id))
            .collect();
        let movable = self.targets_without_drafts(&targets);
        match nav {
            Nav::Folder(folder) => self.move_threads(movable, |_| Some(folder), window, cx),
            Nav::Starred => {
                let unstarred = movable
                    .into_iter()
                    .filter(|&ix| !self.data.threads[ix].starred)
                    .collect();
                self.star_threads(unstarred, cx)
            }
            Nav::Label(ix) => self.set_label(ix, movable, false, cx),
            Nav::Category(category) => self.move_to_category(targets, category, cx),
            Nav::Snoozed | Nav::Scheduled => {}
        }
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
        // MB-018: a menu for several says so first.
        if several {
            menu = menu
                .menu_with_disabled(
                    format!("{} Conversations", targets.len()),
                    true,
                    Box::new(ClearSelection),
                )
                .separator();
        }
        if with_replies && !several && !draft {
            menu = menu
                .menu("Reply", Box::new(Reply))
                .menu("Reply All", Box::new(ReplyAll))
                .menu("Forward", Box::new(Forward))
                .separator();
        }
        if with_replies {
            if let Some(title) = thread.folder.archive_title() {
                menu = menu.menu(title, Box::new(Archive));
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
            // The toolbar's "…" leaves out what sits beside it (MB-020).
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
        }
        if !draft {
            // ACT-11.
            let snoozed = targets
                .iter()
                .any(|&ix| self.data.threads[ix].later.snoozed_until.is_some());
            menu = menu.menu(
                if snoozed {
                    "Change Snooze…"
                } else {
                    "Snooze…"
                },
                Box::new(Snooze),
            );
            if snoozed {
                menu = menu.menu("Unsnooze", Box::new(Unsnooze));
            }
            // MB-018: not the folder they're all in already.
            let mut folders = NativeMenu::new();
            for folder in Folder::ALL {
                let here = targets
                    .iter()
                    .all(|&ix| self.data.threads[ix].folder == folder);
                if folder != Folder::Drafts && folder != Folder::Sent && !here {
                    folders = folders.menu(folder.title(), Box::new(MoveToFolder(folder)));
                }
            }
            menu = menu.submenu("Move to", folders);
            // MB-062: Gmail's categories, for Inbox conversations.
            let inbox: Vec<&Thread> = targets
                .iter()
                .map(|&ix| &self.data.threads[ix])
                .filter(|t| t.folder == Folder::Inbox)
                .collect();
            if !inbox.is_empty() {
                let mut categories = NativeMenu::new();
                for (category, action) in CATEGORY_ACTIONS {
                    let here = inbox.iter().all(|t| !leaves_for(t, category));
                    categories = categories.menu_with_check(category.name(), here, action());
                }
                menu = menu.submenu("Move to Category", categories);
            }
        }
        if self.data.labels.is_empty() {
            menu = menu.menu("New Label…", Box::new(NewLabel));
        } else {
            let mut labels = NativeMenu::new();
            for (ix, label) in self.data.labels.iter().enumerate() {
                let on = targets
                    .iter()
                    .all(|&t| self.data.threads[t].labels.contains(&label.id));
                labels = labels.menu_with_check(label.name.clone(), on, Box::new(ToggleLabel(ix)));
            }
            menu = menu.submenu("Label", labels);
        }
        // MB-061.
        if !several && !draft {
            menu = menu
                .separator()
                .menu("Open in Gmail", Box::new(OpenInGmail));
        }
        menu.show(position, window, cx);
    }

    /// Right-click on a row: act on it (or on the picked threads it's one
    /// of) from a menu, leaving the selection and the open thread alone. The
    /// row is the context target until the menu closes.
    pub(crate) fn row_context_menu(
        &mut self,
        ix: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_checked(ix) {
            self.context_target = Some(self.data.threads[ix].id.clone());
        }
        self.show_thread_menu(position, true, window, cx);
        // The menu runs its own loop and dispatches the chosen action when it
        // closes, ahead of this task: it clears the target whether the menu
        // chose something or was dismissed.
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| {
                this.context_target = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
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
            When::Undo => self.undo_stack.can_undo(),
            When::Redo => self.undo_stack.can_redo(),
            When::Searching => mail && self.search_open,
            When::Selection => mail && !self.checked.is_empty(),
            When::Inbox => mail && self.nav == Nav::Folder(Folder::Inbox),
            When::Labels => mail && !self.data.labels.is_empty(),
            When::LabelOpen => mail && matches!(self.nav, Nav::Label(_)),
            When::FromChoice => {
                mail && self.composer.is_some() && self.data.account.has_several_identities()
            }
            When::Event => !mail && self.calendar.read(cx).selected.is_some(),
            When::EventOpen => !mail && self.calendar.read(cx).panel.is_some(),
            When::CalendarInvite => {
                let calendar = self.calendar.read(cx);
                !mail
                    && calendar
                        .selected
                        .as_ref()
                        .and_then(|s| calendar.event(&s.event_id))
                        .is_some_and(|e| {
                            crate::calendar::policy::my_answer(&calendar.data, e).is_some()
                        })
            }
            When::SignedIn => self.backend.read(cx).source() == Source::Google,
            When::SignedOut => self.backend.read(cx).source() == Source::SignedOut,
            When::CanSignIn => {
                let b = self.backend.read(cx);
                b.source() == Source::SignedOut || b.demo_by_choice()
            }
            When::CanSignOut => {
                let b = self.backend.read(cx);
                b.source() == Source::Google || b.demo_by_choice()
            }
            When::Snoozed => {
                mail && self
                    .targets()
                    .iter()
                    .any(|&ix| self.data.threads[ix].later.snoozed_until.is_some())
            }
            When::Scheduled => mail && thread.is_some_and(|t| t.later.send_at.is_some()),
        }
    }

    /// Whether a command's toggle is on now, for its title (ME-055).
    pub(crate) fn toggle_on(&self, toggle: Toggle, window: &Window, cx: &App) -> bool {
        let targets = self.targets();
        let all = |f: fn(&Thread) -> bool| {
            !targets.is_empty() && targets.iter().all(|&ix| f(&self.data.threads[ix]))
        };
        match toggle {
            Toggle::Star => all(|t| t.starred),
            Toggle::Important => all(|t| t.important),
            Toggle::Selected => self.selected.is_some_and(|ix| self.is_checked(ix)),
            Toggle::ArchivedSection => self.archived_expanded,
            Toggle::CompactList => self.density == Density::Compact,
            Toggle::Sidebar => {
                self.pane_layout(window.viewport_size().width).sidebar > Pixels::ZERO
            }
            Toggle::WeekNumbers => self.calendar.read(cx).show_week_numbers,
            Toggle::FullScreen => window.is_fullscreen(),
            Toggle::Notifications => crate::settings::get(cx).notifications,
            Toggle::OpenAtLogin => super::shell::login_item() == crate::native::LoginItem::On,
            Toggle::EventReminders => crate::settings::get(cx).event_reminders,
            Toggle::HiddenLabels => self.labels_ui.show_hidden,
            Toggle::LabelFolded => match self.nav {
                Nav::Label(ix) => self
                    .data
                    .labels
                    .get(ix)
                    .is_some_and(|l| self.labels_ui.folded.contains(&l.id)),
                _ => false,
            },
            Toggle::Signature => self
                .composer
                .as_ref()
                .is_some_and(|(c, _)| c.read(cx).signature_html().is_some()),
            Toggle::Snoozed => all(|t| t.later.snoozed_until.is_some()),
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

    /// What the dropdown under the search field offers now: recent
    /// searches and common operators while it's empty, the operators a
    /// typed prefix starts, contacts while a `from:` or `to:` is being
    /// typed (ME-015).
    pub fn search_suggestions(&self, window: &Window, cx: &App) -> Vec<SearchSuggestion> {
        if !self.search_open || !self.search_input.focus_handle(cx).is_focused(window) {
            return Vec::new();
        }
        let operator =
            |&(op, what): &(&'static str, &'static str)| SearchSuggestion::Operator(op, what);
        let value = self.search_input.read(cx).value();
        if value.trim().is_empty() {
            let mut out: Vec<SearchSuggestion> = self
                .recent_searches
                .iter()
                .map(|q| SearchSuggestion::Recent(q.clone()))
                .collect();
            if !out.is_empty() {
                out.push(SearchSuggestion::ClearRecent);
            }
            out.extend(OPERATORS[..COMMON_OPERATORS].iter().map(operator));
            return out;
        }
        if value.ends_with(char::is_whitespace) {
            return Vec::new();
        }
        let last = value.split_whitespace().last().unwrap_or_default();
        let Some((operator_typed, query)) = ["from:", "to:"]
            .into_iter()
            .find_map(|op| Some((op, last.strip_prefix(op)?)))
        else {
            // A start of an operator: "has" offers "has:attachment".
            let typed = last.to_lowercase();
            if typed.chars().count() < 2 {
                return Vec::new();
            }
            return OPERATORS
                .iter()
                .filter(|(op, _)| op.starts_with(&typed) && *op != typed)
                .map(operator)
                .collect();
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
                    operator: operator_typed,
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
        // What's before the word being typed, which a suggestion completes.
        let head = value
            .rsplit_once(char::is_whitespace)
            .map_or("", |(head, _)| head);
        let sep = if head.is_empty() { "" } else { " " };
        let text = match suggestion {
            // Operators that need a value wait for it; the rest are whole.
            SearchSuggestion::Operator(op, _) if op.ends_with(':') => format!("{head}{sep}{op}"),
            SearchSuggestion::Operator(op, _) => format!("{head}{sep}{op} "),
            SearchSuggestion::Recent(query) => query,
            SearchSuggestion::Contact {
                operator, email, ..
            } => format!("{head}{sep}{operator}{email} "),
            SearchSuggestion::ClearRecent => {
                self.clear_recent_searches(cx);
                return;
            }
        };
        self.set_search_text(&text, window, cx);
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// The search field's X (MA-082): the text goes, the search stays open
    /// with the keyboard.
    pub(crate) fn clear_search_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_search_text("", window, cx);
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Replace the search text, and search for it.
    pub(super) fn set_search_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
        self.search_query = crate::search::Query::parse(text);
        self.search_highlight = None;
        self.refresh_rows(false);
        self.search_gmail(cx);
        cx.notify();
    }

    /// "Clear Recent Searches" (ME-015).
    fn clear_recent_searches(&mut self, cx: &mut Context<Self>) {
        self.recent_searches.clear();
        self.search_highlight = None;
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
            // Local views: Archive and Drafts hold their threads.
            Nav::Snoozed => ViewKey::Folder(Folder::Archive),
            Nav::Scheduled => ViewKey::Folder(Folder::Drafts),
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

    /// The search summary's Retry (MA-076): ask Gmail again.
    pub(crate) fn retry_search(&mut self, cx: &mut Context<Self>) {
        self.server_search.ask(None);
        self.search_gmail(cx);
        cx.notify();
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
                    // Gmail's extra results fade in (MD-024).
                    this.with_row_cause(super::RowCause::Sync, |this| this.refresh_rows(true));
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
        !matches!(
            self.nav,
            Nav::Folder(Folder::Drafts) | Nav::Snoozed | Nav::Scheduled
        )
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
        Folder::Spam => Icon::ShieldAlert,
        Folder::Trash => Icon::Trash,
    }
}

/// A category's glyph, as the sidebar shows it.
fn category_icon(category: Category) -> Icon {
    match category {
        Category::Primary => Icon::Inbox,
        Category::Social => Icon::Users,
        Category::Promotions => Icon::Megaphone,
        Category::Updates => Icon::Bell,
        Category::Forums => Icon::MessagesSquare,
    }
}

/// Whether moving `thread` to `category` changes it: it's in the Inbox
/// and not in that category alone.
/// The Inbox category a thread shows under; Primary when none.
fn current_category(thread: &Thread) -> Category {
    use crate::backend::categories::in_category;
    Category::ALL
        .into_iter()
        .find(|&c| in_category(thread, c))
        .unwrap_or(Category::Primary)
}

fn leaves_for(thread: &Thread, category: Category) -> bool {
    use crate::backend::categories::in_category;
    thread.folder == Folder::Inbox
        && Category::ALL
            .into_iter()
            .any(|c| in_category(thread, c) != (c == category))
}

/// The threads an action applies to: the context menu's row, else the picked
/// threads, else the selected one.
fn action_targets(
    context: Option<usize>,
    selected: Option<usize>,
    picked: impl FnOnce() -> Vec<usize>,
) -> Vec<usize> {
    if let Some(ix) = context {
        return vec![ix];
    }
    let picked = picked();
    if picked.is_empty() {
        selected.into_iter().collect()
    } else {
        picked
    }
}

/// `Some(n)` when `n` is more than one: a change to one thread shows
/// itself, to several it says so.
fn several(n: usize) -> Option<usize> {
    (n > 1).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::action_targets;

    #[test]
    fn selection_when_nothing_is_picked() {
        assert_eq!(action_targets(None, Some(3), Vec::new), vec![3]);
        assert!(action_targets(None, None, Vec::new).is_empty());
    }

    #[test]
    fn picked_threads_win_over_the_selection() {
        assert_eq!(action_targets(None, Some(3), || vec![1, 2]), vec![1, 2]);
    }

    #[test]
    fn context_row_wins_over_both() {
        assert_eq!(action_targets(Some(7), Some(3), || vec![1, 2]), vec![7]);
        assert_eq!(action_targets(Some(7), None, Vec::new), vec![7]);
    }
}

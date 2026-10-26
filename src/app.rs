//! The root view. It owns all app state; the panes in `ui/` render from it.

use std::{collections::HashSet, rc::Rc, time::Duration};

use chrono::Local;
use gpui::{
    AppContext as _, Context, DragMoveEvent, Entity, FocusHandle, Focusable as _,
    InteractiveElement, IntoElement, KeyContext, ListAlignment, ListState, MouseButton,
    ParentElement, Pixels, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled, Subscription, Task, Window,
    component::input::{InputEvent, InputState},
    div, point,
    prelude::FluentBuilder as _,
    px,
};

use crate::{
    actions::*,
    assets::Icon,
    data::{Contact, Folder, MailData, Message, Thread, full_date},
    html,
    search::{FocusResults, Query},
    theme::Theme,
    ui::{
        self, PaneResize,
        command_palette::{CommandPalette, Leading, PaletteCommand, PaletteEvent, PaletteItem},
        composer::{Composer, ComposerEvent, ContactBook, DraftContent, Quoted},
    },
};

/// How long the "Sent" toast, and its Undo, stays up.
const TOAST_DURATION: Duration = Duration::from_secs(5);

pub const RAIL_WIDTH: Pixels = px(52.);
const SIDEBAR_WIDTH: (Pixels, Pixels) = (px(180.), px(360.));
const LIST_WIDTH: (Pixels, Pixels) = (px(300.), px(640.));

/// What the sidebar has selected; the thread list shows its threads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Folder(Folder),
    Starred,
    /// Index into `MailData::labels`.
    Label(usize),
}

/// One row of the thread list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// Index into `MailData::threads`.
    Thread(usize),
    /// The collapsible "Archived" header under the Inbox.
    ArchivedHeader { count: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

/// A brief note at the bottom of the window, optionally undoable.
pub struct Toast {
    pub message: SharedString,
    pub undo: Option<UndoAction>,
    _timer: Task<()>,
}

pub enum UndoAction {
    /// Take back a sent message and reopen its draft.
    Send {
        thread_id: String,
        /// Appended to an existing thread, rather than a new one in Sent.
        appended: bool,
        /// The Drafts thread sending removed.
        removed_draft: Option<Thread>,
        composer: Entity<Composer>,
    },
    Discard {
        removed_draft: Option<Thread>,
        composer: Entity<Composer>,
    },
}

pub struct PigeonApp {
    pub data: MailData,
    pub nav: Nav,
    pub rows: Vec<Row>,
    /// Index into `MailData::threads`.
    pub selected: Option<usize>,
    /// Older messages of the selected thread the user expanded. The newest is
    /// always expanded.
    pub expanded_messages: HashSet<usize>,
    pub archived_expanded: bool,
    pub sidebar_open: bool,
    pub sidebar_width: Pixels,
    pub list_width: Pixels,
    pub search_open: bool,
    pub list_state: ListState,
    pub reader_scroll: ScrollHandle,
    pub html_cache: crate::html::HtmlCache,
    /// The reading pane has keyboard focus (after `o`): arrows scroll it and
    /// escape returns to the list.
    pub reader_focused: bool,
    /// Focus of the panes; see `actions.rs` for the key contexts.
    focus_handle: FocusHandle,
    palette: Option<(Entity<CommandPalette>, Subscription)>,
    pub help_open: bool,
    pub help_focus: FocusHandle,
    /// The open draft, with its event subscription.
    pub composer: Option<(Entity<Composer>, Subscription)>,
    contact_book: Rc<ContactBook>,
    pub search_input: Entity<InputState>,
    pub search_query: Query,
    /// The message `n` and `p` move between, in the selected thread.
    pub focused_message: Option<usize>,
    pub toast: Option<Toast>,
    next_id: usize,
    _subscriptions: Vec<Subscription>,
}

impl PigeonApp {
    pub fn new(data: MailData, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync(window, cx);
            cx.notify();
        });
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search mail"));
        let search_events = cx.subscribe_in(&search_input, window, Self::on_search_event);
        let contact_book = Rc::new(ContactBook::new(data.contacts()));

        let mut this = Self {
            data,
            nav: Nav::Folder(Folder::Inbox),
            rows: Vec::new(),
            selected: None,
            expanded_messages: HashSet::new(),
            archived_expanded: false,
            sidebar_open: true,
            sidebar_width: px(236.),
            list_width: px(400.),
            search_open: false,
            list_state: ListState::new(0, ListAlignment::Top, px(400.)),
            reader_scroll: ScrollHandle::new(),
            html_cache: Default::default(),
            reader_focused: false,
            focus_handle,
            palette: None,
            help_open: false,
            help_focus: cx.focus_handle(),
            composer: None,
            contact_book,
            search_input,
            search_query: Query::default(),
            focused_message: None,
            toast: None,
            next_id: 0,
            _subscriptions: vec![appearance, search_events],
        };
        this.refresh_rows(false);
        this
    }

    pub fn nav_contains(&self, nav: Nav, thread: &Thread) -> bool {
        match nav {
            Nav::Folder(folder) => thread.folder == folder,
            Nav::Starred => thread.starred && !thread.is_hidden(),
            Nav::Label(ix) => {
                !thread.is_hidden() && thread.labels.contains(&self.data.labels[ix].id)
            }
        }
    }

    /// Recompute the list rows for the current nav. With `keep_scroll`, only
    /// the rows that changed are spliced so the list stays where it was.
    fn refresh_rows(&mut self, keep_scroll: bool) {
        let threads = &self.data.threads;
        let searching = self.is_searching();
        let in_view = |thread: &Thread| {
            if !searching {
                return self.nav_contains(self.nav, thread);
            }
            let in_scope = match self.search_query.scope {
                Some(scope) => Query::in_scope(scope, thread),
                // The Inbox shows its archived threads too, so its search does.
                None if self.nav == Nav::Folder(Folder::Inbox) => {
                    matches!(thread.folder, Folder::Inbox | Folder::Archive)
                }
                None => self.nav_contains(self.nav, thread),
            };
            in_scope && self.search_query.matches(thread, &self.data)
        };
        let mut rows: Vec<Row> = (0..threads.len())
            .filter(|&ix| in_view(&threads[ix]))
            .map(Row::Thread)
            .collect();

        if self.nav == Nav::Folder(Folder::Inbox) && !searching {
            let archived = (0..threads.len()).filter(|&ix| threads[ix].folder == Folder::Archive);
            let count = archived.clone().count();
            if count > 0 {
                rows.push(Row::ArchivedHeader { count });
                if self.archived_expanded {
                    rows.extend(archived.map(Row::Thread));
                }
            }
        }

        if keep_scroll {
            let old = &self.rows;
            let prefix = old.iter().zip(&rows).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            self.list_state
                .splice(prefix..old.len() - suffix, rows.len() - prefix - suffix);
        } else {
            self.list_state.reset(rows.len());
        }
        self.rows = rows;
    }

    pub fn is_searching(&self) -> bool {
        self.search_open && !self.search_query.is_empty()
    }

    pub fn set_nav(&mut self, nav: Nav, cx: &mut Context<Self>) {
        if self.nav != nav {
            self.stash_composer(cx);
            self.nav = nav;
            self.selected = None;
            self.reader_focused = false;
            self.refresh_rows(false);
        }
        cx.notify();
    }

    pub fn select_thread(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Leaving a draft saves it, which can reorder the threads.
        let id = self.data.threads[ix].id.clone();
        let already_open = self
            .composer
            .as_ref()
            .is_some_and(|(c, _)| c.read(cx).draft_id.as_deref() == Some(&id));
        if !already_open {
            self.stash_composer(cx);
        }
        let Some(ix) = self.data.thread_ix(&id) else {
            return;
        };
        if self.selected != Some(ix) {
            self.expanded_messages.clear();
            self.focused_message = None;
            self.reader_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.selected = Some(ix);
        self.data.threads[ix].unread = false;
        if let Some(row) = self.rows.iter().position(|r| *r == Row::Thread(ix)) {
            self.list_state.scroll_to_reveal_item(row);
        }
        if self.data.threads[ix].folder == Folder::Drafts && !already_open {
            self.open_draft(ix, window, cx);
        }
        cx.notify();
    }

    pub fn toggle_message(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.expanded_messages.remove(&ix) {
            self.expanded_messages.insert(ix);
        }
        cx.notify();
    }

    fn selected_row(&self) -> Option<usize> {
        let selected = Row::Thread(self.selected?);
        self.rows.iter().position(|r| *r == selected)
    }

    /// The thread row after `row`, or before it when there is none after.
    fn neighbor_thread(&self, row: usize, forward: bool) -> Option<usize> {
        let thread_at = |r: &Row| match r {
            Row::Thread(ix) => Some(*ix),
            Row::ArchivedHeader { .. } => None,
        };
        if forward {
            self.rows[row + 1..].iter().find_map(thread_at)
        } else {
            self.rows[..row].iter().rev().find_map(thread_at)
        }
    }

    fn select_adjacent(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let next = match self.selected_row() {
            Some(row) => self.neighbor_thread(row, forward),
            None => self.rows.iter().find_map(|r| match r {
                Row::Thread(ix) => Some(*ix),
                Row::ArchivedHeader { .. } => None,
            }),
        };
        if let Some(ix) = next {
            self.select_thread(ix, window, cx);
        }
    }

    /// Move the selected thread to `folder` and advance to its neighbor, like
    /// Gmail's auto-advance.
    fn move_selected(
        &mut self,
        folder: impl Fn(Folder) -> Folder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stash_composer(cx);
        let Some(ix) = self.selected else { return };
        let next = self.selected_row().and_then(|row| {
            self.neighbor_thread(row, true)
                .or_else(|| self.neighbor_thread(row, false))
        });
        let thread = &mut self.data.threads[ix];
        thread.folder = folder(thread.folder);
        self.refresh_rows(true);
        match next {
            Some(next) => self.select_thread(next, window, cx),
            None => self.selected = None,
        }
        cx.notify();
    }

    // Action handlers.

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    /// Opens the search bar, or returns to it, and focuses it.
    fn toggle_search(&mut self, _: &ToggleSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close_search(&mut self, _: &CloseSearch, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_open {
            return cx.propagate();
        }
        self.search_open = false;
        self.search_query = Query::default();
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh_rows(false);
        if let Some(row) = self.selected_row() {
            self.list_state.scroll_to_reveal_item(row);
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn on_search_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.search_query = Query::parse(&input.read(cx).value());
                self.refresh_rows(false);
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.focus_results(&FocusResults, window, cx),
            _ => {}
        }
    }

    /// From the search bar into its results.
    fn focus_results(&mut self, _: &FocusResults, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        self.reader_focused = false;
        if self.selected_row().is_none() {
            self.selected = None;
            self.select_adjacent(true, window, cx);
        }
        cx.notify();
    }

    fn toggle_archived_section(
        &mut self,
        _: &ToggleArchivedSection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.archived_expanded = !self.archived_expanded;
        self.refresh_rows(true);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNextThread, window: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(true, window, cx);
    }

    fn select_prev(&mut self, _: &SelectPrevThread, window: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(false, window, cx);
    }

    fn archive(&mut self, _: &Archive, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selected(
            |f| match f {
                Folder::Archive => Folder::Inbox,
                _ => Folder::Archive,
            },
            window,
            cx,
        );
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selected(|_| Folder::Trash, window, cx);
    }

    fn toggle_star(&mut self, _: &ToggleStar, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let thread = &mut self.data.threads[ix];
        thread.starred = !thread.starred;
        if self.nav == Nav::Starred {
            self.refresh_rows(true);
        }
        cx.notify();
    }

    fn mark_unread(&mut self, _: &MarkUnread, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        self.data.threads[ix].unread = true;
        cx.notify();
    }

    fn mark_read(&mut self, _: &MarkRead, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        self.data.threads[ix].unread = false;
        cx.notify();
    }

    fn compose_new(&mut self, _: &Compose, window: &mut Window, cx: &mut Context<Self>) {
        self.compose(ComposeKind::New, window, cx);
    }

    fn reply(&mut self, _: &Reply, window: &mut Window, cx: &mut Context<Self>) {
        self.compose(ComposeKind::Reply, window, cx);
    }

    fn reply_all(&mut self, _: &ReplyAll, window: &mut Window, cx: &mut Context<Self>) {
        self.compose(ComposeKind::ReplyAll, window, cx);
    }

    fn forward(&mut self, _: &Forward, window: &mut Window, cx: &mut Context<Self>) {
        self.compose(ComposeKind::Forward, window, cx);
    }

    /// Open a draft: alone in the reading pane for a new message, under the
    /// conversation for a reply or forward.
    fn compose(&mut self, kind: ComposeKind, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_composer(cx);
        let thread = self.selected.map(|ix| &self.data.threads[ix]);
        let (content, reply_to, quoted) = match (kind, thread) {
            (ComposeKind::New, _) => (DraftContent::default(), None, None),
            (_, None) => return,
            (_, Some(thread)) if thread.folder == Folder::Drafts => return,
            (kind, Some(thread)) => {
                let (content, quoted) = self.reply_content(kind, thread);
                (content, Some(thread.id.clone()), quoted)
            }
        };
        let composer = cx.new(|cx| {
            Composer::new(
                kind,
                content,
                reply_to.clone(),
                None,
                quoted,
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

    fn reply_content(&self, kind: ComposeKind, thread: &Thread) -> (DraftContent, Option<Quoted>) {
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
                    ("From", address(&last.from)),
                    ("Date", full_date(&last.date)),
                    ("Subject", thread.subject.clone()),
                    ("To", to),
                ],
                last.html.clone(),
            )
        });

        let content = DraftContent {
            to,
            cc,
            bcc: Vec::new(),
            subject,
            body: String::new(),
        };
        (content, quoted)
    }

    /// Show a Drafts thread in the composer, without taking focus from the list.
    fn open_draft(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let thread = &self.data.threads[ix];
        let message = thread.last_message();
        let content = DraftContent {
            to: message.to.clone(),
            cc: message.cc.clone(),
            bcc: message.bcc.clone(),
            subject: thread.subject.clone(),
            body: html::plain_text(&html::parse(&message.html)),
        };
        let kind = if thread.reply_to.is_some() {
            ComposeKind::Reply
        } else {
            ComposeKind::New
        };
        let (reply_to, draft_id) = (thread.reply_to.clone(), Some(thread.id.clone()));
        let book = self.contact_book.clone();
        let composer =
            cx.new(|cx| Composer::new(kind, content, reply_to, draft_id, None, book, window, cx));
        self.set_composer(composer, cx);
    }

    fn set_composer(&mut self, composer: Entity<Composer>, cx: &mut Context<Self>) {
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
        composer.draft_id.is_none()
            && composer.reply_to.is_some()
            && self.selected.map(|ix| &self.data.threads[ix].id) == composer.reply_to.as_ref()
    }

    /// Put a change to the threads into effect: newest first again, with the
    /// selection kept on the same thread.
    fn update_threads<R>(&mut self, f: impl FnOnce(&mut MailData) -> R) -> R {
        let selected = self.selected.map(|ix| self.data.threads[ix].id.clone());
        let result = f(&mut self.data);
        self.data
            .threads
            .sort_by_key(|t| std::cmp::Reverse(t.last_message().date));
        self.selected = selected.and_then(|id| self.data.thread_ix(&id));
        self.refresh_rows(true);
        result
    }

    fn new_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-local-{}", self.next_id)
    }

    fn message_from(&self, content: &DraftContent, html: String) -> Message {
        let snippet: String = content
            .body
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(140)
            .collect();
        Message {
            from: self.data.me(),
            to: content.to.clone(),
            cc: content.cc.clone(),
            bcc: content.bcc.clone(),
            date: Local::now().fixed_offset(),
            snippet,
            html,
            attachments: Vec::new(),
        }
    }

    /// Save the draft to Drafts, replacing the thread it was opened from.
    fn save_draft(&mut self, composer: &Entity<Composer>, cx: &mut Context<Self>) {
        let c = composer.read(cx);
        let content = c.content(cx);
        let message = self.message_from(&content, c.html(cx));
        let (draft_id, reply_to) = (c.draft_id.clone(), c.reply_to.clone());
        let id = draft_id.unwrap_or_else(|| self.new_id("draft"));
        self.update_threads(|data| match data.thread_ix(&id) {
            Some(ix) => {
                let thread = &mut data.threads[ix];
                thread.subject = content.subject.clone();
                thread.messages = vec![message];
                thread.reindex();
            }
            None => {
                let mut thread = Thread {
                    id,
                    subject: content.subject.clone(),
                    folder: Folder::Drafts,
                    labels: Vec::new(),
                    starred: false,
                    unread: false,
                    messages: vec![message],
                    search: Default::default(),
                    reply_to,
                };
                thread.reindex();
                data.threads.push(thread);
            }
        });
    }

    /// Close the composer without losing anything: a changed draft goes to
    /// Drafts.
    fn stash_composer(&mut self, cx: &mut Context<Self>) {
        let Some((composer, _)) = self.composer.take() else {
            return;
        };
        if !composer.read(cx).is_pristine(cx) {
            self.save_draft(&composer, cx);
            self.show_toast("Saved to Drafts", None, cx);
        }
        cx.notify();
    }

    fn show_toast(
        &mut self,
        message: impl Into<SharedString>,
        undo: Option<UndoAction>,
        cx: &mut Context<Self>,
    ) {
        let timer = cx.spawn(async |this, cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            })
            .ok();
        });
        self.toast = Some(Toast {
            message: message.into(),
            undo,
            _timer: timer,
        });
        cx.notify();
    }

    fn close_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn send_message(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        let Some((composer, _)) = &self.composer else {
            return;
        };
        let composer = composer.clone();
        let c = composer.read(cx);
        let content = c.content(cx);
        if !content.has_recipients() {
            composer.update(cx, |c, cx| {
                c.set_error("Add at least one recipient", cx);
                c.focus_to(window, cx);
            });
            return;
        }
        let message = self.message_from(&content, c.html(cx));
        let reply_to = c
            .reply_to
            .clone()
            .filter(|id| self.data.thread_ix(id).is_some());
        let draft_id = c.draft_id.clone();
        let new_id = self.new_id("sent");

        let (thread_id, appended, removed_draft) = self.update_threads(|data| {
            let removed_draft = draft_id
                .and_then(|id| data.thread_ix(&id))
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
                    let mut thread = Thread {
                        id: new_id.clone(),
                        subject: content.subject.clone(),
                        folder: Folder::Sent,
                        labels: Vec::new(),
                        starred: false,
                        unread: false,
                        messages: vec![message],
                        search: Default::default(),
                        reply_to: None,
                    };
                    thread.reindex();
                    data.threads.push(thread);
                    (new_id, false)
                }
            };
            (thread_id, appended, removed_draft)
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
            }),
            cx,
        );
    }

    /// Escape: close the draft, keeping it in Drafts if it says anything.
    fn close_draft(&mut self, _: &CloseDraft, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_none() {
            return cx.propagate();
        }
        let was_inline = self.composer_inline(cx);
        self.stash_composer(cx);
        self.reader_focused = was_inline;
        window.focus(&self.focus_handle, cx);
    }

    fn discard_draft(&mut self, _: &DiscardDraft, window: &mut Window, cx: &mut Context<Self>) {
        let Some((composer, _)) = &self.composer else {
            return;
        };
        let composer = composer.clone();
        let c = composer.read(cx);
        let pristine = c.is_pristine(cx) && c.draft_id.is_none();
        let draft_id = c.draft_id.clone();
        let removed_draft = self.update_threads(|data| {
            draft_id
                .and_then(|id| data.thread_ix(&id))
                .map(|ix| data.threads.remove(ix))
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
                }),
                cx,
            );
        }
    }

    fn show_cc_bcc(&mut self, _: &ShowCcBcc, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((composer, _)) = &self.composer {
            composer.update(cx, |c, cx| c.show_cc_bcc(window, cx));
        }
    }

    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        let Some(undo) = self.toast.take().and_then(|t| t.undo) else {
            return;
        };
        let (composer, removed_draft) = match undo {
            UndoAction::Send {
                thread_id,
                appended,
                removed_draft,
                composer,
            } => {
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
            } => (composer, removed_draft),
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

    // Keyboard focus between the list and the reader.

    fn open_thread(&mut self, _: &OpenThread, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((composer, _)) = &self.composer {
            let composer = composer.clone();
            self.reader_focused = self.composer_inline(cx);
            composer.update(cx, |c, cx| c.focus_first(window, cx));
            return;
        }
        if self.selected.is_none() {
            self.select_adjacent(true, window, cx);
        }
        self.reader_focused = self.selected.is_some();
        cx.notify();
    }

    /// `n` / `p`: move through the messages of the open conversation,
    /// expanding the one moved to.
    fn step_message(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(thread) = self.selected.map(|ix| &self.data.threads[ix]) else {
            return;
        };
        let last = thread.messages.len() - 1;
        let target = match (self.focused_message, forward) {
            (None, true) => last,
            (None, false) => last.saturating_sub(1),
            (Some(ix), true) => (ix + 1).min(last),
            (Some(ix), false) => ix.saturating_sub(1),
        };
        self.focused_message = Some(target);
        if target != last {
            self.expanded_messages.insert(target);
        }
        self.reader_scroll
            .scroll_to_item(ui::reader::MESSAGES_START + target);
        cx.notify();
    }

    fn next_message(&mut self, _: &NextMessage, _: &mut Window, cx: &mut Context<Self>) {
        self.step_message(true, cx);
    }

    fn prev_message(&mut self, _: &PrevMessage, _: &mut Window, cx: &mut Context<Self>) {
        self.step_message(false, cx);
    }

    fn back_to_list(&mut self, _: &BackToList, _: &mut Window, cx: &mut Context<Self>) {
        self.reader_focused = false;
        cx.notify();
    }

    fn scroll_reader(&mut self, delta: Pixels, cx: &mut Context<Self>) {
        let offset = self.reader_scroll.offset();
        let max = self.reader_scroll.max_offset().y;
        let y = (offset.y - delta).clamp(-max, px(0.));
        self.reader_scroll.set_offset(point(offset.x, y));
        cx.notify();
    }

    fn scroll_reader_down(&mut self, _: &ScrollReaderDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_reader(px(64.), cx);
    }

    fn scroll_reader_up(&mut self, _: &ScrollReaderUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_reader(px(-64.), cx);
    }

    fn mailbox_key_context(&self) -> KeyContext {
        let mut context = KeyContext::default();
        context.add(MAILBOX);
        if self.reader_focused {
            context.add(READER);
        }
        context
    }

    // Go to.

    fn go_to_inbox(&mut self, _: &GoToInbox, _: &mut Window, cx: &mut Context<Self>) {
        self.set_nav(Nav::Folder(Folder::Inbox), cx);
    }

    fn go_to_starred(&mut self, _: &GoToStarred, _: &mut Window, cx: &mut Context<Self>) {
        self.set_nav(Nav::Starred, cx);
    }

    fn go_to_sent(&mut self, _: &GoToSent, _: &mut Window, cx: &mut Context<Self>) {
        self.set_nav(Nav::Folder(Folder::Sent), cx);
    }

    fn go_to_drafts(&mut self, _: &GoToDrafts, _: &mut Window, cx: &mut Context<Self>) {
        self.set_nav(Nav::Folder(Folder::Drafts), cx);
    }

    fn go_to_archive(&mut self, _: &GoToArchive, _: &mut Window, cx: &mut Context<Self>) {
        self.set_nav(Nav::Folder(Folder::Archive), cx);
    }

    fn go_to_spam(&mut self, _: &GoToSpam, _: &mut Window, cx: &mut Context<Self>) {
        self.set_nav(Nav::Folder(Folder::Spam), cx);
    }

    fn go_to_trash(&mut self, _: &GoToTrash, _: &mut Window, cx: &mut Context<Self>) {
        self.set_nav(Nav::Folder(Folder::Trash), cx);
    }

    /// Gmail's `g l`: the palette, narrowed to labels.
    fn go_to_label(&mut self, _: &GoToLabel, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette("label: ", window, cx);
    }

    /// Select a thread from anywhere, switching to its folder if the current
    /// view doesn't show it.
    fn reveal_thread(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.rows.contains(&Row::Thread(ix)) {
            self.set_nav(Nav::Folder(self.data.threads[ix].folder), cx);
        }
        self.select_thread(ix, window, cx);
    }

    // Overlays: the command palette and the shortcuts sheet. While one is
    // open it holds focus; closing it hands focus back to the panes.

    fn toggle_command_palette(
        &mut self,
        _: &ToggleCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() {
            self.close_overlays(window, cx);
        } else {
            self.open_palette("", window, cx);
        }
    }

    fn open_palette(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.help_open = false;
        let items = self.palette_items(window);
        let palette = cx.new(|cx| CommandPalette::new(items, query, window, cx));
        let events = cx.subscribe_in(&palette, window, Self::on_palette_event);
        window.focus(&palette.focus_handle(cx), cx);
        self.palette = Some((palette, events));
        cx.notify();
    }

    fn palette_items(&self, window: &Window) -> Vec<PaletteItem> {
        let binding = |action: &dyn gpui::Action| {
            lookup_contexts(self.reader_focused)
                .into_iter()
                .find_map(|context| {
                    window.highest_precedence_binding_for_action_in_context(action, context)
                })
        };
        let commands = COMMANDS
            .iter()
            .filter(|c| !c.help_only)
            .map(|c| PaletteItem {
                command: PaletteCommand::Action(c.action),
                title: c.name.into(),
                subtitle: None,
                leading: Leading::Icon(c.icon),
                binding: binding(&*(c.action)()),
                search_only: false,
            });
        let labels = self
            .data
            .labels
            .iter()
            .enumerate()
            .map(|(ix, label)| PaletteItem {
                command: PaletteCommand::Nav(Nav::Label(ix)),
                title: format!("Go to Label: {}", label.name).into(),
                subtitle: None,
                leading: Leading::Dot(label.color),
                binding: None,
                search_only: false,
            });
        let threads = self.data.threads.iter().enumerate().map(|(ix, thread)| {
            let from = thread
                .messages
                .iter()
                .rev()
                .map(|m| &m.from)
                .find(|c| !self.data.is_me(c))
                .unwrap_or(&thread.last_message().from);
            PaletteItem {
                command: PaletteCommand::Thread(ix),
                title: thread.subject.clone().into(),
                subtitle: Some(from.display_name().to_string().into()),
                leading: Leading::Icon(Icon::Mail),
                binding: None,
                search_only: true,
            }
        });
        commands.chain(labels).chain(threads).collect()
    }

    fn on_palette_event(
        &mut self,
        _: &Entity<CommandPalette>,
        event: &PaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlays(window, cx);
        let PaletteEvent::Run(command) = event else {
            return;
        };
        match *command {
            // Dispatched from the panes, which now have focus, exactly as if
            // its shortcut had been pressed there.
            PaletteCommand::Action(action) => window.dispatch_action(action(), cx),
            PaletteCommand::Nav(nav) => self.set_nav(nav, cx),
            PaletteCommand::Thread(ix) => self.reveal_thread(ix, window, cx),
        }
    }

    fn toggle_shortcuts_help(
        &mut self,
        _: &ToggleShortcutsHelp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.help_open {
            self.close_overlays(window, cx);
        } else {
            self.palette = None;
            self.help_open = true;
            window.focus(&self.help_focus, cx);
            cx.notify();
        }
    }

    fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.help_open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn render_toast(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let toast = self.toast.as_ref()?;
        let theme = Theme::of(cx);
        Some(
            div()
                .absolute()
                .bottom(px(22.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("toast")
                        .occlude()
                        .h(px(40.))
                        .pl(px(14.))
                        .pr(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(10.))
                        .bg(theme.overlay_bg)
                        .border_1()
                        .border_color(theme.overlay_border)
                        .shadow(ui::overlay_shadow(theme))
                        .text_size(px(13.))
                        .child(ui::icon(Icon::Check, px(15.), theme.accent))
                        .child(toast.message.clone())
                        .when(toast.undo.is_some(), |this| {
                            this.child(
                                div()
                                    .id("toast-undo")
                                    .h(px(28.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .rounded(px(7.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme.accent)
                                    .hover(|s| s.bg(theme.hover_bg))
                                    .child("Undo")
                                    .child(ui::keycap("⌘Z", theme))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(Undo), cx)
                                    }),
                            )
                        })
                        .when(toast.undo.is_none(), |this| this.pr(px(14.))),
                )
                .into_any_element(),
        )
    }

    fn render_overlay(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let content = if let Some((palette, _)) = &self.palette {
            palette.clone().into_any_element()
        } else if self.help_open {
            self.render_shortcuts_help(window, cx).into_any_element()
        } else {
            return None;
        };
        let theme = Theme::of(cx);
        Some(
            div()
                .id("overlay-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .map(|this| {
                    if self.help_open {
                        this.justify_center().p(px(40.))
                    } else {
                        this.pt(px(110.)).px(px(40.))
                    }
                })
                .bg(theme.backdrop)
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_overlays(window, cx)),
                )
                .child(content)
                .into_any_element(),
        )
    }

    fn resize_pane(
        &mut self,
        event: &DragMoveEvent<PaneResize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let x = event.event.position.x - RAIL_WIDTH;
        match event.drag(cx) {
            PaneResize::Sidebar => {
                self.sidebar_width = x.clamp(SIDEBAR_WIDTH.0, SIDEBAR_WIDTH.1);
            }
            PaneResize::List => {
                let sidebar = if self.sidebar_open {
                    self.sidebar_width
                } else {
                    px(0.)
                };
                self.list_width = (x - sidebar).clamp(LIST_WIDTH.0, LIST_WIDTH.1);
            }
        }
        cx.notify();
    }
}

impl Render for PigeonApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Whatever had focus went away (a closed draft, say): back to the panes.
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
        let theme = Theme::of(cx).clone();
        // Handlers sit on the root, so actions reach them from the overlays too.
        div()
            .id("pigeon")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::toggle_command_palette))
            .on_action(cx.listener(Self::toggle_shortcuts_help))
            .on_action(cx.listener(Self::open_thread))
            .on_action(cx.listener(Self::back_to_list))
            .on_action(cx.listener(Self::scroll_reader_down))
            .on_action(cx.listener(Self::scroll_reader_up))
            .on_action(cx.listener(Self::mark_read))
            .on_action(cx.listener(Self::compose_new))
            .on_action(cx.listener(Self::go_to_inbox))
            .on_action(cx.listener(Self::go_to_starred))
            .on_action(cx.listener(Self::go_to_sent))
            .on_action(cx.listener(Self::go_to_drafts))
            .on_action(cx.listener(Self::go_to_archive))
            .on_action(cx.listener(Self::go_to_spam))
            .on_action(cx.listener(Self::go_to_trash))
            .on_action(cx.listener(Self::go_to_label))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_search))
            .on_action(cx.listener(Self::toggle_archived_section))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::archive))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::toggle_star))
            .on_action(cx.listener(Self::mark_unread))
            .on_action(cx.listener(Self::reply))
            .on_action(cx.listener(Self::reply_all))
            .on_action(cx.listener(Self::forward))
            .on_action(cx.listener(Self::send_message))
            .on_action(cx.listener(Self::close_draft))
            .on_action(cx.listener(Self::discard_draft))
            .on_action(cx.listener(Self::show_cc_bcc))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::close_search))
            .on_action(cx.listener(Self::focus_results))
            .on_action(cx.listener(Self::next_message))
            .on_action(cx.listener(Self::prev_message))
            .on_drag_move(cx.listener(Self::resize_pane))
            .relative()
            .size_full()
            .overflow_hidden()
            .text_color(theme.text)
            .child(
                div()
                    .key_context(self.mailbox_key_context())
                    .track_focus(&self.focus_handle)
                    .size_full()
                    .flex()
                    .flex_row()
                    .child(ui::rail::render(&theme))
                    .when(self.sidebar_open, |this| {
                        this.child(self.render_sidebar(window, cx))
                    })
                    .child(
                        div()
                            .h_full()
                            .flex_none()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.reader_focused = false;
                                    if !this.search_input.focus_handle(cx).is_focused(window) {
                                        window.focus(&this.focus_handle, cx);
                                    }
                                    cx.notify();
                                }),
                            )
                            .child(self.render_thread_list(window, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(360.))
                            .h_full()
                            .flex()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.reader_focused = this.selected.is_some();
                                    cx.notify();
                                }),
                            )
                            .child(self.render_reader(window, cx)),
                    ),
            )
            .children(self.render_toast(cx))
            .children(self.render_overlay(window, cx))
    }
}

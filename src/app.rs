//! The root view. It owns all app state; the panes in `ui/` render from it.

use std::collections::HashSet;

use gpui::{
    AppContext as _, Context, DragMoveEvent, Entity, FocusHandle, Focusable as _,
    InteractiveElement, IntoElement, KeyContext, ListAlignment, ListState, MouseButton,
    ParentElement, Pixels, Render, ScrollHandle, Styled, Subscription, Window, div, point,
    prelude::FluentBuilder as _, px,
};

use crate::{
    actions::*,
    assets::Icon,
    data::{Folder, MailData, Thread},
    theme::Theme,
    ui::{
        self, PaneResize,
        command_palette::{CommandPalette, Leading, PaletteCommand, PaletteEvent, PaletteItem},
    },
};

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
    _appearance: Subscription,
}

impl PigeonApp {
    pub fn new(data: MailData, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync(window, cx);
            cx.notify();
        });

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
            _appearance: appearance,
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
        let mut rows: Vec<Row> = (0..threads.len())
            .filter(|&ix| self.nav_contains(self.nav, &threads[ix]))
            .map(Row::Thread)
            .collect();

        if self.nav == Nav::Folder(Folder::Inbox) {
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

    pub fn set_nav(&mut self, nav: Nav, cx: &mut Context<Self>) {
        if self.nav != nav {
            self.nav = nav;
            self.selected = None;
            self.reader_focused = false;
            self.refresh_rows(false);
        }
        cx.notify();
    }

    pub fn select_thread(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        self.data.threads[ix].unread = false;
        self.expanded_messages.clear();
        self.reader_scroll.set_offset(point(px(0.), px(0.)));
        if let Some(row) = self.rows.iter().position(|r| *r == Row::Thread(ix)) {
            self.list_state.scroll_to_reveal_item(row);
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

    fn select_adjacent(&mut self, forward: bool, cx: &mut Context<Self>) {
        let next = match self.selected_row() {
            Some(row) => self.neighbor_thread(row, forward),
            None => self.rows.iter().find_map(|r| match r {
                Row::Thread(ix) => Some(*ix),
                Row::ArchivedHeader { .. } => None,
            }),
        };
        if let Some(ix) = next {
            self.select_thread(ix, cx);
        }
    }

    /// Move the selected thread to `folder` and advance to its neighbor, like
    /// Gmail's auto-advance.
    fn move_selected(&mut self, folder: impl Fn(Folder) -> Folder, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let next = self.selected_row().and_then(|row| {
            self.neighbor_thread(row, true)
                .or_else(|| self.neighbor_thread(row, false))
        });
        let thread = &mut self.data.threads[ix];
        thread.folder = folder(thread.folder);
        self.refresh_rows(true);
        match next {
            Some(next) => self.select_thread(next, cx),
            None => self.selected = None,
        }
        cx.notify();
    }

    // Action handlers.

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    /// Search lands in a later phase; for now this only shows the bar.
    fn toggle_search(&mut self, _: &ToggleSearch, _: &mut Window, cx: &mut Context<Self>) {
        self.search_open = !self.search_open;
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

    fn select_next(&mut self, _: &SelectNextThread, _: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(true, cx);
    }

    fn select_prev(&mut self, _: &SelectPrevThread, _: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(false, cx);
    }

    fn archive(&mut self, _: &Archive, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selected(
            |f| match f {
                Folder::Archive => Folder::Inbox,
                _ => Folder::Archive,
            },
            cx,
        );
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selected(|_| Folder::Trash, cx);
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

    /// Seam for compose-in-reading-pane: open a draft for the selected thread
    /// below its messages in `ui/reader.rs`.
    fn compose(&mut self, _kind: ComposeKind, _: &mut Window, _: &mut Context<Self>) {}

    // Keyboard focus between the list and the reader.

    fn open_thread(&mut self, _: &OpenThread, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_none() {
            self.select_adjacent(true, cx);
        }
        self.reader_focused = self.selected.is_some();
        cx.notify();
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
    fn reveal_thread(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.rows.contains(&Row::Thread(ix)) {
            self.set_nav(Nav::Folder(self.data.threads[ix].folder), cx);
        }
        self.select_thread(ix, cx);
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
            [self.reader_focused, !self.reader_focused]
                .into_iter()
                .find_map(|reader| {
                    window.highest_precedence_binding_for_action_in_context(
                        action,
                        mailbox_context(reader),
                    )
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
            PaletteCommand::Thread(ix) => self.reveal_thread(ix, cx),
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
                                cx.listener(|this, _, _, cx| {
                                    this.reader_focused = false;
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
            .children(self.render_overlay(window, cx))
    }
}

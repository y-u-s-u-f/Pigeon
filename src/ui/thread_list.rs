//! The virtualized thread list.

use std::{collections::HashSet, rc::Rc};

use gpui::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, ElementId, Focusable as _,
    FontStyle, FontWeight, HighlightStyle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    StyledText, Window,
    base::motion::MotionReveal,
    base::spring,
    component::{input::Input, scroll::ScrollableElement as _},
    deferred, div, list,
    prelude::FluentBuilder as _,
    px, relative, rems,
};

use super::{
    HEADER_HEIGHT, avatar, binding_caps,
    controls::{ButtonKind, popover_surface, push_button},
    icon, icon_button, icon_button_small, is_secondary, label,
    motion::{self, SPRING_PANE, fade},
    status::{plural, thousands},
    tooltip, window_drag_area,
};
use crate::ui::style::*;
use crate::{
    actions::{
        ApplyLabel, Archive, CloseSearch, Compose, Delete, DismissNeedsReply,
        LoadOlderConversations, MarkRead, MarkUnread, SEARCH, SendLater, Snooze, SyncNow,
        ToggleArchivedSection, ToggleSearch, ToggleSidebar, ToggleStar,
    },
    app::{Density, Nav, PigeonApp, Row, SearchSuggestion},
    assets::Icon,
    data::{Folder, list_date_on},
    search::ServerState,
    theme::Theme,
};

/// Where a row's text starts: the row's margin plus its inset, which holds
/// the unread dot. The header and the result count line up with it.
const ROW_MARGIN: gpui::Pixels = SPACE_SM;
const ROW_INSET: gpui::Pixels = SPACE_XL;
/// Labels shown as dots in a row, at most; the reader shows them all.
const LABEL_DOTS: usize = 3;
const LABEL_DOT: gpui::Pixels = px(7.);
const UNREAD_DOT: gpui::Pixels = px(6.);
/// The bar on the selected row's leading edge while the list has focus.
const FOCUS_BAR: gpui::Pixels = px(3.);
/// The row's hover group: the date gives way to the quick actions, and the
/// checkbox shows, without the app keeping a hover state (ME-004).
const ROW_GROUP: &str = "thread-row";
/// The suggestions scroll past this (ME-015).
const SUGGESTIONS_MAX: gpui::Rems = rems(22.5);
/// How long after a sync its new rows fade in as they show (MD-024).
const FRESH_FADE: std::time::Duration = std::time::Duration::from_millis(400);

/// Conversations being dragged onto the sidebar (MA-087): the row's, or
/// every picked one when the row is one of them.
#[derive(Clone)]
pub struct DraggedThreads {
    pub ids: Vec<String>,
    title: SharedString,
}

impl Render for DraggedThreads {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        popover_surface(theme)
            .max_w(rems(16.))
            .h(CONTROL_MD)
            .px(SPACE_SM)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .text_size(TEXT_SM)
            .text_color(theme.text)
            .child(icon(Icon::Mail, ICON_MD, theme.text_muted))
            .child(div().min_w_0().truncate().child(self.title.clone()))
            .when(self.ids.len() > 1, |this| {
                this.child(
                    div()
                        .flex_none()
                        .px(SPACE_XS)
                        .rounded_full()
                        .bg(theme.accent)
                        .text_color(theme.accent_text)
                        .text_size(TEXT_XS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .font_features(tabular_figures())
                        .child(self.ids.len().to_string()),
                )
            })
    }
}

/// What every row needs, worked out once per frame.
struct RowContext {
    /// Threads with a send still waiting to go (MA-069).
    queued: HashSet<String>,
    /// Differentiate Without Color: label dots carry a letter (MA-024).
    no_color: bool,
    /// Read once per render, not per row (MC-043).
    theme: Theme,
    now: chrono::DateTime<chrono::Local>,
}

impl PigeonApp {
    /// The open view's name; a nested label's last part.
    pub fn nav_title(&self) -> SharedString {
        match self.nav {
            Nav::Folder(folder) => folder.title().into(),
            Nav::Starred => "Starred".into(),
            Nav::Snoozed => "Snoozed".into(),
            Nav::Scheduled => "Scheduled".into(),
            Nav::NeedsReply => "Needs Reply".into(),
            Nav::Label(ix) => {
                let name = &self.data.labels[ix].name;
                name.rsplit('/').next().unwrap_or(name).to_string().into()
            }
            Nav::Category(category) => category.name().into(),
        }
    }

    /// "Work › Clients › Acme": the parents muted, the leaf in full (ME-022).
    fn render_nav_title(&self, theme: &Theme) -> impl IntoElement {
        let parents: Vec<String> = match self.nav {
            Nav::Label(ix) => {
                let mut parts: Vec<&str> = self.data.labels[ix].name.split('/').collect();
                parts.pop();
                parts.into_iter().map(String::from).collect()
            }
            _ => Vec::new(),
        };
        let title = self.nav_title();
        let path = parents
            .iter()
            .map(String::as_str)
            .chain([title.as_ref()])
            .collect::<Vec<_>>()
            .join(" › ");
        div()
            .id("list-title")
            .role(gpui::Role::Heading)
            .aria_level(1)
            .aria_label(path)
            .min_w_0()
            .flex()
            .items_center()
            .text_size(TEXT_LG)
            .font_weight(FontWeight::SEMIBOLD)
            .when(!parents.is_empty(), |this| {
                this.child(
                    div()
                        .min_w_0()
                        .flex_shrink_1()
                        .truncate()
                        .text_color(theme.text_muted)
                        .font_weight(FontWeight::NORMAL)
                        .child(format!("{} › ", parents.join(" › "))),
                )
            })
            .child(div().flex_none().max_w_full().truncate().child(title))
    }

    pub(crate) fn render_thread_list(
        &self,
        width: gpui::Pixels,
        // How far the sidebar is shown, 0 to 1, as it slides (MD-012).
        sidebar: f32,
        counts: &crate::ui::sidebar::NavCounts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        // The sidebar's number, so the two agree (LST-6).
        let unread = counts.unread(self.nav);
        let thread_rows = self
            .rows
            .iter()
            .filter(|r| matches!(r, Row::Thread(_)))
            .count();
        let checked_rows = self
            .rows
            .iter()
            .filter(|r| matches!(r, Row::Thread(ix) if self.is_checked(*ix)))
            .count();
        let selecting = !self.checked.is_empty();
        // MD-016: the field opens on a spring, sampled open or not so it
        // never plays again on its own; it closes at once, with the list.
        let target = if self.search_open { 1. } else { 0. };
        let search = if cx.reduce_motion() {
            fade("search-open", target, target, window, cx)
        } else {
            let policy = SPRING_PANE.with_travel(self.search_open);
            spring("search-open", target, policy, window, cx)
        };

        let header = window_drag_area(div().id("list-header"))
            .h(super::HEADER_MIN)
            .min_h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .pr(SPACE_MD)
            .map(|this| {
                let inset = if selecting && !self.search_open {
                    ROW_MARGIN
                } else {
                    ROW_MARGIN + ROW_INSET
                };
                if sidebar >= 1. {
                    return this.pl(inset);
                }
                // The way back, in the sidebar's button's place. It fades as
                // the sidebar slides in, and its slot narrows with it, so the
                // title glides rather than jumps (MD-012).
                let p = sidebar.clamp(0., 1.);
                this.pl(SPACE_SM + (inset - SPACE_SM) * p).child(
                    div()
                        .flex_none()
                        .w(HIT_TARGET * (1. - p))
                        .mr(-SPACE_SM * p)
                        .overflow_hidden()
                        .opacity(1. - p)
                        .child(icon_button(
                            "show-sidebar",
                            Icon::PanelLeft,
                            "Show sidebar",
                            Box::new(ToggleSidebar),
                            &theme,
                        )),
                )
            })
            .map(|this| {
                if self.search_open {
                    return this.child(self.render_search_bar(search, &theme, window, cx));
                }
                this.when(selecting, |this| {
                    this.child(self.render_select_all(checked_rows, thread_rows, &theme, cx))
                })
                .child(self.render_nav_title(&theme))
                .when(unread > 0, |this| {
                    this.child(
                        div()
                            .flex_none()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_muted)
                            .font_features(tabular_figures())
                            .child(format!("{} unread", thousands(unread))),
                    )
                })
                .child(div().flex_1())
                .child(icon_button(
                    "compose",
                    Icon::SquarePen,
                    "New message",
                    Box::new(Compose),
                    &theme,
                ))
                .child(icon_button(
                    "search",
                    Icon::Search,
                    "Search",
                    Box::new(ToggleSearch),
                    &theme,
                ))
            });

        let searching = self.is_searching();
        let asking_gmail = self.server_search.state == ServerState::Waiting;
        let loaded = self.view_loaded(cx);
        // Only conversations count (ME-001): an "Archived" header or "Load
        // older" alone still leaves the view empty.
        let empty = thread_rows == 0;
        // The Inbox with nothing in it, whatever is archived under it.
        let inbox_zero = !searching
            && loaded
            && self.nav == Nav::Folder(Folder::Inbox)
            && !self
                .rows
                .iter()
                .take_while(|r| !matches!(r, Row::ArchivedHeader { .. }))
                .any(|r| matches!(r, Row::Thread(_)));
        // W4b (SRC-9): only Gmail answers these, and it can't be asked.
        let needs_gmail = searching
            && self.search_query.gmail_only()
            && (!self.server_search.available || self.server_search.state == ServerState::Offline);
        let row_cx = Rc::new(self.row_context(cx));
        let view = cx.entity();
        let list_view = |row_cx: Rc<RowContext>| {
            div()
                .id("thread-list")
                .role(gpui::Role::ListBox)
                .aria_label(self.nav_title())
                .relative()
                .size_full()
                .child(
                    list(self.list_state.clone(), move |ix, window, cx| {
                        view.update(cx, |this, cx| this.render_row(ix, &row_cx, window, cx))
                    })
                    .size_full()
                    .pb(SPACE_SM),
                )
                .vertical_scrollbar(&self.list_state)
        };
        let body = if empty && needs_gmail {
            centered(empty_view(
                Icon::Search,
                "Can't search Gmail offline",
                Some(muted_hint("after:, OR and -word need a connection", &theme)),
                &theme,
            ))
        } else if empty && searching && asking_gmail {
            // W9 (SRC-7): nothing here, but Gmail may have some.
            centered(empty_view(Icon::Search, "Searching Gmail…", None, &theme))
        } else if empty && searching {
            let narrow = self.search_query.scope.is_none() && !self.search_all;
            centered(
                empty_view(
                    Icon::Search,
                    format!("No results {}", self.search_scope_text()),
                    Some(muted_hint("Try fewer or different words", &theme)),
                    &theme,
                )
                // ME-016: offered, not explained.
                .when(narrow, |this| {
                    this.child(
                        push_button(
                            "search-all-mail",
                            "Search All Mail",
                            ButtonKind::Plain,
                            &theme,
                        )
                        .mt(SPACE_XS)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_search_scope(cx))),
                    )
                }),
            )
        } else if empty && !loaded && !searching {
            // Not "Nothing in …" before the first page is in (LST-9).
            match self.backend.read(cx).sync_error() {
                Some(error) => centered(
                    empty_view(
                        Icon::OctagonAlert,
                        format!("Couldn't load {}", self.nav_title()),
                        Some(muted_hint(
                            if error.starts_with("Couldn't reach") {
                                "Pigeon will try again when it can reach Gmail"
                            } else {
                                "Pigeon will try again shortly"
                            },
                            &theme,
                        )),
                        &theme,
                    )
                    // MA-076.
                    .child(
                        push_button("list-try-now", "Try Now", ButtonKind::Plain, &theme)
                            .mt(SPACE_XS)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(SyncNow), cx)
                            }),
                    ),
                ),
                None => render_skeleton(&theme),
            }
        } else if inbox_zero {
            // ME-001: a quiet line and what's next, the archived threads
            // still under it.
            let zero = empty_view(
                Icon::Check,
                "You’re all caught up",
                self.next_event_line(cx)
                    .map(|line| muted_hint(line, &theme)),
                &theme,
            );
            // Only rows on their way out: already the centered line, so it
            // doesn't jump there when they've gone.
            if self.rows.iter().all(|row| matches!(row, Row::Leaving(_))) {
                centered(zero)
            } else {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(zero.flex_none().pt(SPACE_2XL).pb(SPACE_LG))
                    .child(div().flex_1().min_h_0().child(list_view(row_cx)))
                    .into_any_element()
            }
        } else if empty {
            let (glyph, title, hint) = self.empty_copy(window, &theme);
            centered(empty_view(glyph, title, hint, &theme))
        } else {
            list_view(row_cx).into_any_element()
        };

        let summary_shown = searching
            && (!empty
                || matches!(
                    self.server_search.state,
                    ServerState::Offline | ServerState::Failed
                ));
        let target = if summary_shown { 1. } else { 0. };
        let policy = SPRING_PANE.with_travel(summary_shown);
        let summary = spring("search-summary", target, policy, window, cx);
        let summary_opacity = if cx.reduce_motion() {
            fade("search-summary", target, target, window, cx)
        } else {
            summary
        };
        let suggestions = self.search_suggestions(window, cx);
        div()
            .relative()
            .w(width)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.list_bg)
            .border_r_1()
            .border_color(theme.border)
            .child(header)
            .when(summary > 0., |this| {
                // MD-043: it opens the first time; its text then changes in
                // place, and clearing the search takes it away at once.
                this.child(gpui::base::motion::MotionReveal::new(
                    "search-summary-reveal",
                    summary,
                    self.render_search_summary(&theme, cx)
                        .opacity(summary_opacity)
                        .into_any_element(),
                ))
            })
            .child(div().flex_1().min_h_0().child(body))
            .when(
                empty && !inbox_zero && !self.rows.is_empty() && (searching || loaded),
                |this| {
                    this.child(
                        div()
                            .flex_none()
                            .pb(SPACE_SM)
                            .child(self.render_load_older(&theme, cx)),
                    )
                },
            )
            .when(!suggestions.is_empty(), |this| {
                this.child(self.render_search_suggestions(suggestions, &theme, window, cx))
            })
    }

    /// The row data that isn't per thread: queued sends, the system's
    /// Differentiate Without Color.
    fn row_context(&self, cx: &gpui::App) -> RowContext {
        RowContext {
            queued: self.queued_sends.clone(),
            no_color: crate::native::differentiate_without_color(),
            theme: Theme::of(cx).clone(),
            now: chrono::Local::now(),
        }
    }

    /// The header's checkbox while picking (MB-Q12): mixed while only some
    /// are picked; a click picks them all, or lets go when all are.
    fn render_select_all(
        &self,
        checked: usize,
        total: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let all = total > 0 && checked >= total;
        let state = if all {
            gpui::Toggled::True
        } else if checked > 0 {
            gpui::Toggled::Mixed
        } else {
            gpui::Toggled::False
        };
        focusable_control(div().id("select-all"), theme)
            .role(gpui::Role::CheckBox)
            .aria_label(format!("Select all, {checked} of {total} selected"))
            .aria_toggled(state)
            .size(HIT_TARGET)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(check_box(state, theme))
            .tooltip(move |window, cx| {
                tooltip(if all {
                    "Deselect all"
                } else {
                    "Select all conversations"
                })
                .build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if all {
                    this.clear_checked();
                    cx.notify();
                } else {
                    this.select_all_threads(cx);
                }
            }))
    }

    /// An empty view's glyph, title and next step, with the key that
    /// takes it (ME-001).
    fn empty_copy(
        &self,
        window: &Window,
        theme: &Theme,
    ) -> (Icon, SharedString, Option<AnyElement>) {
        let key = |before: &str, action: &dyn gpui::Action, after: &str| {
            self.key_hint(before, action, after, window, theme)
        };
        let say = |text: &'static str| Some(muted_hint(text, theme));
        match self.nav {
            Nav::Folder(Folder::Drafts) => (
                Icon::FilePen,
                "No drafts".into(),
                key("Press", &Compose, "to write a message"),
            ),
            Nav::Folder(Folder::Sent) => (
                Icon::Send,
                "No sent mail".into(),
                key("Press", &Compose, "to write a message"),
            ),
            Nav::Folder(Folder::Archive) => (
                Icon::Archive,
                "Nothing archived".into(),
                key("Press", &Archive, "on a conversation to archive it"),
            ),
            Nav::Folder(Folder::Spam) => (
                Icon::ShieldAlert,
                "No spam".into(),
                say("Gmail deletes spam after 30 days"),
            ),
            Nav::Folder(Folder::Trash) => (
                Icon::Trash,
                "Trash is empty".into(),
                say("Gmail deletes conversations in Trash after 30 days"),
            ),
            Nav::Folder(Folder::Inbox) => (Icon::Check, "You’re all caught up".into(), None),
            Nav::Starred => (
                Icon::Star,
                "No starred conversations".into(),
                key("Press", &ToggleStar, "on a conversation to star it"),
            ),
            Nav::Snoozed => (
                Icon::AlarmClock,
                "Nothing snoozed".into(),
                key("Press", &Snooze, "to bring a conversation back later"),
            ),
            Nav::Scheduled => (
                Icon::Clock,
                "Nothing scheduled".into(),
                key("Press", &SendLater, "in a draft to send it later"),
            ),
            Nav::NeedsReply => (
                Icon::Reply,
                "Nothing waits for your reply".into(),
                key(
                    "Press",
                    &DismissNeedsReply,
                    "on a conversation to clear its marker",
                ),
            ),
            Nav::Label(_) => (
                Icon::Tag,
                format!("Nothing labeled “{}”", self.nav_title()).into(),
                key("Press", &ApplyLabel, "on a conversation to label it"),
            ),
            Nav::Category(category) => (
                Icon::Inbox,
                format!("Nothing in {}", category.name()).into(),
                say("Move to Category files a conversation here"),
            ),
        }
    }

    /// "Press [C] to write a message", with the live key; nothing when the
    /// action has none.
    fn key_hint(
        &self,
        before: &str,
        action: &dyn gpui::Action,
        after: &str,
        window: &Window,
        theme: &Theme,
    ) -> Option<AnyElement> {
        let binding = self.binding_for(action, window)?;
        let keys: String = binding
            .keystrokes()
            .iter()
            .map(|k| k.key().to_uppercase())
            .collect::<Vec<_>>()
            .join(" ");
        Some(
            div()
                .id("empty-hint")
                .role(gpui::Role::Label)
                .aria_label(format!("{before} {keys} {after}"))
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
                .child(before.to_string())
                .child(binding_caps(&binding, theme))
                .child(after.to_string())
                .into_any_element(),
        )
    }

    /// Under inbox zero: the next event, today or tomorrow (ME-Q4 a).
    fn next_event_line(&self, cx: &gpui::App) -> Option<String> {
        use crate::{app::today, calendar::time};
        let data = &self.calendar.read(cx).data;
        let now = today::now();
        let agenda = today::agenda(data, now);
        let next = agenda
            .next
            .into_iter()
            .chain(agenda.later)
            .chain(agenda.tomorrow)
            .filter(|o| o.start >= now)
            .min_by_key(|o| o.start)?;
        let event = &data.events[next.event];
        let day = if next.start.date() == now.date() {
            "Next"
        } else {
            "Tomorrow"
        };
        let title = today::truncate(event.display_title(), 40);
        Some(if event.all_day {
            format!("{day}: {title}")
        } else {
            format!("{day}: {title} at {}", time::time_label(next.start.time()))
        })
    }

    /// Whether the view's first page has loaded from Gmail (STA-1).
    fn view_loaded(&self, cx: &gpui::App) -> bool {
        self.backend.read(cx).view_loaded(&self.view_key())
    }

    /// Where a search looks, as the scope chip names it: an `in:` in the
    /// query, else this view or All Mail.
    fn search_scope_name(&self) -> SharedString {
        use crate::search::Scope;
        match self.search_query.scope {
            None if self.search_all => "All Mail".into(),
            None => self.nav_title(),
            Some(Scope::All) => "All Mail".into(),
            Some(Scope::Anywhere) => "Anywhere".into(),
            Some(Scope::Folder(folder)) => folder.title().into(),
        }
    }

    /// "in Inbox", or where an `in:` operator or the scope chip points.
    fn search_scope_text(&self) -> String {
        use crate::search::Scope;
        match self.search_query.scope {
            Some(Scope::Anywhere) => "anywhere".into(),
            None if self.search_all => "in All Mail".into(),
            Some(Scope::All) => "in All Mail".into(),
            _ => format!("in {}", self.search_scope_name()),
        }
    }

    /// Under the search field: the operators it understood and how many
    /// threads match.
    fn render_search_summary(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let count = self
            .rows
            .iter()
            .filter(|r| matches!(r, Row::Thread(_)))
            .count();
        // W9: Gmail's estimate, when it knows of more (SRC-6), and how its
        // half of the search is going (SRC-7).
        let counted = result_count(
            count,
            self.server_search.estimate,
            self.server_search.has_more(),
        );
        let failed = self.server_search.state == ServerState::Failed;
        let note = match self.server_search.state {
            ServerState::Waiting => Some("Searching Gmail…"),
            ServerState::Offline if self.search_query.gmail_only() => {
                Some("Offline: this search needs a connection")
            }
            ServerState::Offline => Some("Offline: downloaded mail only"),
            ServerState::Failed => Some("Couldn't search Gmail"),
            _ => None,
        };
        div()
            .flex_none()
            .pl(ROW_MARGIN + ROW_INSET)
            .pr(SPACE_LG)
            .pb(SPACE_SM)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(SPACE_XS)
            .text_size(TEXT_SM)
            .text_color(theme.text_muted)
            // Status, not controls: no accent (MA-031).
            .children(self.search_query.operators().into_iter().map(|op| {
                div()
                    .h(CONTROL_XS)
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .rounded(RADIUS_SM)
                    .bg(theme.chip_bg)
                    .text_color(theme.text_muted)
                    .font_weight(FontWeight::MEDIUM)
                    .child(op)
            }))
            .child(
                label(
                    "search-count",
                    format!("{counted} {}", self.search_scope_text()),
                )
                .ml(SPACE_2XS)
                .font_features(tabular_figures()),
            )
            // MA-066: turning while Gmail is asked.
            .when(self.server_search.state == ServerState::Waiting, |this| {
                this.child(motion::spinner(
                    "search-gmail-spinner",
                    icon(Icon::RefreshCw, ICON_SM, theme.text_faint),
                ))
            })
            .children(note.map(|note| {
                super::live_text("search-gmail-note", note, gpui::accesskit::Live::Polite)
                    .ml(SPACE_2XS)
                    .text_color(if failed {
                        theme.text_danger
                    } else {
                        theme.text_faint
                    })
            }))
            // MA-076.
            .when(failed, |this| {
                this.child(
                    push_button("search-retry", "Retry", ButtonKind::Plain, theme)
                        .h(CONTROL_SM)
                        .min_h(CONTROL_SM)
                        .on_click(cx.listener(|this, _, _, cx| this.retry_search(cx))),
                )
            })
    }

    /// The search button, expanded into a field that fills the header. It
    /// grows out of the button's place on the right.
    fn render_search_bar(
        &self,
        open: f32,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Where the search looks; a click widens it to all mail. An `in:`
        // in the query decides instead, and the chip names it (ME-016).
        let fixed = self.search_query.scope.is_some();
        let scope_label = self.search_scope_name();
        let scope = div()
            .id("search-scope")
            .flex_none()
            .h(CONTROL_SM)
            .pl(SPACE_SM)
            .pr(if fixed { SPACE_SM } else { SPACE_XS })
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .rounded(RADIUS_SM)
            .bg(theme.chip_bg)
            .text_size(TEXT_SM)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .child(scope_label.clone())
            .map(|this| {
                if fixed {
                    this.role(gpui::Role::Label)
                        .aria_label(format!(
                            "Searching {scope_label}, set by in: in your search"
                        ))
                        .tooltip(|window, cx| {
                            tooltip("Set by in: in your search").build(window, cx)
                        })
                } else {
                    this.role(gpui::Role::Button)
                        .aria_label(format!("Search in {scope_label}"))
                        .child(icon(Icon::ChevronDown, ICON_SM, theme.text_faint))
                        .hover(|s| s.bg(theme.selected_bg).text_color(theme.text))
                        .active(|s| s.bg(theme.selected_bg).opacity(0.8))
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_search_scope(cx)))
                        .tooltip(|window, cx| {
                            tooltip("Switch between this folder and All Mail").build(window, cx)
                        })
                }
            });

        // MA-082: the field shows it has the keyboard; X clears the text
        // first and closes only an empty search.
        let focused = self.search_input.focus_handle(cx).is_focused(window);
        let has_text = !self.search_input.read(cx).value().is_empty();
        let clear = icon_button_small(
            "close-search",
            Icon::X,
            if has_text {
                "Clear search"
            } else {
                "Close search"
            },
            theme,
        )
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(cx.listener(move |this, _, window, cx| {
            if has_text {
                this.clear_search_text(window, cx);
            } else {
                window.dispatch_action(Box::new(CloseSearch), cx);
            }
        }));
        let field = super::search_field(Some(focused), theme)
            .key_context(SEARCH)
            .h(CONTROL_LG)
            .pl(SPACE_MD)
            .pr(SPACE_2XS)
            .overflow_hidden()
            .child(icon(Icon::Search, ICON_MD, theme.text_muted))
            .child(scope)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.search_input).appearance(false).px(px(0.))),
            )
            .child(clear)
            // From most of its width, since it opens often (MA-095); Reduce
            // Motion leaves the fade.
            .w(relative(if cx.reduce_motion() {
                1.
            } else {
                0.6 + 0.4 * open
            }))
            .opacity(open.clamp(0., 1.));
        div().flex_1().flex().justify_end().child(field)
    }

    /// The dropdown under the search field: recent searches and operators
    /// while it's empty, contacts after `from:` or `to:`. It scrolls past
    /// `SUGGESTIONS_MAX`, keeping the highlighted row in view (ME-015).
    fn render_search_suggestions(
        &self,
        suggestions: Vec<SearchSuggestion>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let section = |title: &'static str| {
            div()
                .px(SPACE_SM)
                .pt(SPACE_SM)
                .pb(SPACE_XS)
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_faint)
                .child(title)
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut highlighted_child = None;
        let mut last_kind = None;
        for (ix, suggestion) in suggestions.into_iter().enumerate() {
            let kind = std::mem::discriminant(&suggestion);
            let title = match suggestion {
                SearchSuggestion::Operator(..) => Some("Operators"),
                SearchSuggestion::Recent(_) => Some("Recent"),
                SearchSuggestion::Contact { .. } => Some("People"),
                SearchSuggestion::ClearRecent => None,
            };
            if let Some(title) = title
                && last_kind != Some(kind)
            {
                last_kind = Some(kind);
                rows.push(section(title).into_any_element());
            }
            let highlighted = self.search_highlight == Some(ix);
            if highlighted {
                highlighted_child = Some(rows.len());
            }
            let (content, name): (AnyElement, SharedString) = match &suggestion {
                SearchSuggestion::Operator(op, what) => (
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(TEXT_SM)
                                .text_color(theme.text)
                                .child(*op),
                        )
                        .child(div().text_color(theme.text_muted).child(*what))
                        .into_any_element(),
                    format!("{op} {what}").into(),
                ),
                SearchSuggestion::Recent(query) => (
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .min_w_0()
                        .child(icon(Icon::Clock, ICON_MD, theme.text_faint))
                        .child(div().min_w_0().truncate().child(query.clone()))
                        .into_any_element(),
                    query.clone().into(),
                ),
                SearchSuggestion::ClearRecent => (
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .text_color(theme.text_muted)
                        .child(icon(Icon::X, ICON_MD, theme.text_faint))
                        .child("Clear Recent Searches")
                        .into_any_element(),
                    "Clear Recent Searches".into(),
                ),
                SearchSuggestion::Contact {
                    operator,
                    name,
                    email,
                } => (
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .min_w_0()
                        .child(avatar(
                            crate::data::Contact {
                                name: name.clone(),
                                email: email.clone(),
                            }
                            .initials(),
                            theme.avatar_color(email),
                            AVATAR_XS,
                            theme,
                        ))
                        .child(div().flex_none().child(name.clone()))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_muted)
                                .child(format!("{operator}{email}")),
                        )
                        .into_any_element(),
                    format!("{name}, {operator}{email}").into(),
                ),
            };
            rows.push(
                div()
                    .id(("search-suggestion", ix))
                    .role(gpui::Role::ListBoxOption)
                    .aria_label(name)
                    .aria_selected(highlighted)
                    .when(highlighted, |d| d.aria_active_descendant())
                    .flex_none()
                    .h(ROW_SM)
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .rounded(RADIUS_SM)
                    .text_size(TEXT_BASE)
                    .when(highlighted, |d| d.bg(theme.selected_bg))
                    .hover(|d| d.bg(theme.hover_bg))
                    .child(content)
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_search_suggestion(suggestion.clone(), window, cx)
                    }))
                    .into_any_element(),
            );
        }
        // Scroll to the highlighted row when the arrows move it, not on
        // every frame, so the wheel still scrolls freely.
        let scroll = window.use_keyed_state("search-suggestions-scroll", cx, |_, _| {
            (ScrollHandle::new(), None::<usize>)
        });
        let handle = scroll.read(cx).0.clone();
        if scroll.read(cx).1 != highlighted_child {
            if let Some(child) = highlighted_child {
                handle.scroll_to_item(child);
            }
            scroll.update(cx, |s, _| s.1 = highlighted_child);
        }
        // MD-011: fades in as it opens; typing doesn't play it again.
        deferred(motion::enter(
            "search-suggestions-in",
            motion::Rise::None,
            popover_surface(theme)
                .id("search-suggestions")
                .role(gpui::Role::ListBox)
                .aria_label("Search suggestions")
                .occlude()
                .absolute()
                .top(super::header_height(window) - SPACE_XS)
                .left(ROW_MARGIN)
                .right(ROW_MARGIN)
                .max_h(SUGGESTIONS_MAX)
                .p(SPACE_XS)
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .track_scroll(&handle)
                .children(rows),
        ))
        .with_priority(1)
    }

    fn render_row(
        &self,
        ix: usize,
        row_cx: &RowContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &row_cx.theme;
        match self.rows[ix] {
            Row::Thread(thread_ix) => {
                let row = self.render_thread_row(thread_ix, row_cx, theme, window, cx);
                let id = &self.data.threads[thread_ix].id;
                // MD-017: Undo brings it back growing into place.
                if let Some(glide) = self.growing_rows.get(id) {
                    let p = glide.value().clamp(0., 1.);
                    return MotionReveal::new(
                        ElementId::Name(format!("growing-{id}").into()),
                        p,
                        div().opacity(p).child(row).into_any_element(),
                    )
                    .into_any_element();
                }
                // MD-024: one a sync or Gmail's search added fades in.
                let (fresh, at) = &self.fresh_rows;
                if at.is_some_and(|at| at.elapsed() < FRESH_FADE) && fresh.contains(id) {
                    return motion::enter(
                        ElementId::Name(format!("fresh-{id}").into()),
                        motion::Rise::None,
                        div().child(row),
                    )
                    .into_any_element();
                }
                row
            }
            Row::ArchivedHeader { count } => self.render_archived_header(count, theme, cx),
            Row::LoadOlder => self.render_load_older(theme, cx),
            Row::Leaving(key) => self.render_leaving_row(key, row_cx, theme, window, cx),
        }
    }

    /// MD-017: a conversation that left the view, collapsing and fading as
    /// the rows below close up. Inert: a cover takes its clicks.
    fn render_leaving_row(
        &self,
        key: u64,
        row_cx: &RowContext,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(leaving) = self.leaving_rows.iter().find(|row| row.key == key) else {
            return div().into_any_element();
        };
        let p = 1. - leaving.glide.value().clamp(0., 1.);
        let row = self
            .data
            .thread_ix(&leaving.id)
            .map(|ix| self.render_thread_row(ix, row_cx, theme, window, cx));
        div()
            .relative()
            .h(leaving.height * p)
            .overflow_hidden()
            .opacity(p)
            .children(row)
            .child(motion::inert_cover())
            .into_any_element()
    }

    /// W9 (LST-7): the end of the list, where older threads (or more of
    /// Gmail's results) load.
    fn render_load_older(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (label, enabled) = self.load_older_label(cx);
        div()
            .px(ROW_MARGIN)
            .pt(SPACE_SM)
            .child(
                focusable_control(div().id("load-older"), theme)
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .h(CONTROL_MD)
                    .pl(ROW_INSET)
                    .pr(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if enabled {
                        theme.text_muted
                    } else {
                        theme.text_disabled
                    })
                    .map(|this| {
                        // MA-066: turning while it loads.
                        let waiting = label.ends_with('…');
                        let glyph = if waiting {
                            Icon::RefreshCw
                        } else {
                            Icon::ChevronDown
                        };
                        let glyph = icon(glyph, ICON_MD, theme.text_faint);
                        if waiting {
                            this.child(motion::spinner("load-older-spinner", glyph))
                        } else {
                            this.child(glyph)
                        }
                    })
                    .child(label)
                    .when(enabled, |this| {
                        this.hover(|s| s.bg(theme.hover_bg))
                            .active(|s| s.bg(theme.selected_bg))
                            .tooltip(|window, cx| {
                                tooltip("Load older conversations")
                                    .action(&LoadOlderConversations, None)
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.activate_load_older(cx)))
                    }),
            )
            .into_any_element()
    }

    fn render_archived_header(
        &self,
        count: usize,
        theme: &Theme,
        _: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.archived_expanded;
        div()
            .px(ROW_MARGIN)
            .pt(SPACE_MD)
            .child(
                // A hairline sets the archived threads apart from the Inbox.
                div().mx(SPACE_SM).mb(SPACE_SM).h(px(1.)).bg(theme.border),
            )
            .child(
                focusable_control(div().id("archived-header"), theme)
                    .role(gpui::Role::Button)
                    .aria_label(format!("Archived, {}", thousands(count)))
                    .aria_expanded(expanded)
                    .h(ROW_SM)
                    .pl(SPACE_XS)
                    .pr(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.hover_bg))
                    .active(|s| s.bg(theme.selected_bg))
                    // MB-032: the disclosure triangle leads, in the inset.
                    .child(
                        div()
                            .flex_none()
                            .w(ROW_INSET - SPACE_XS)
                            .flex()
                            .justify_center()
                            // MD-021: one glyph, turned.
                            .child(motion::turn(
                                "chevron",
                                icon(Icon::ChevronRight, ICON_MD, theme.text_faint),
                                expanded,
                                1.,
                            )),
                    )
                    .child("Archived")
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .font_weight(FontWeight::NORMAL)
                            .font_features(tabular_figures())
                            .child(thousands(count)),
                    )
                    .tooltip(move |window, cx| {
                        tooltip(if expanded {
                            "Hide archived conversations"
                        } else {
                            "Show archived conversations"
                        })
                        .action(&ToggleArchivedSection, None)
                        .build(window, cx)
                    })
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ToggleArchivedSection), cx)
                    }),
            )
            .into_any_element()
    }

    fn render_thread_row(
        &self,
        thread_ix: usize,
        row_cx: &RowContext,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let thread = &self.data.threads[thread_ix];
        let last = thread.last_message();
        let selected = self.selected == Some(thread_ix);
        let checked = self.is_checked(thread_ix);
        let selecting = !self.checked.is_empty();
        let unread = thread.unread;
        let compact = self.density == Density::Compact;
        // Accent while the list has the keyboard in the key window, grey
        // while the reader (or another window) has it (MB-042).
        let list_focused = self.focus_handle.is_focused(window)
            && !self.reader_focused
            && window.is_window_active();
        let has_draft = thread.folder != Folder::Drafts && self.draft_replies.contains(&thread.id);
        let queued = row_cx.queued.contains(&thread.id);

        // Sent and drafts are about who they went to; everything else about
        // who wrote (ME-011).
        let who = match thread.folder {
            Folder::Sent | Folder::Drafts => thread.recipients_line(),
            _ => thread.senders_line(|c| self.data.is_me(c)),
        };
        let count = thread.messages.len();
        let subject: SharedString = thread.subject_or_placeholder().to_string().into();
        let no_subject = !thread.has_subject();
        // ACT-11, CMP-17: when it wakes or sends, in place of its date; a
        // woken one by when it came back.
        let later = thread.later.snoozed_until.or(thread.later.send_at);
        let date = match (later, thread.later.woke_at) {
            (Some(at), _) => {
                crate::later::short_label(crate::later::local_time(at), row_cx.now.naive_local())
            }
            (None, Some(woke)) if woke > last.internal_date => {
                let woke = chrono::DateTime::from_timestamp_millis(woke)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Local)
                    .fixed_offset();
                list_date_on(&woke, row_cx.now)
            }
            _ => list_date_on(&last.date, row_cx.now),
        };
        // AI-4, AI-7.
        let needs_reply = self.ai.needs_reply.contains(&thread.id);
        let ai_line = (self.ai.lines_on && thread.folder == Folder::Inbox)
            .then(|| crate::ai::pass::line(thread))
            .flatten();
        let snippet: SharedString = ai_line.unwrap_or(&last.snippet).to_string().into();
        let later_mark = if thread.later.snoozed_until.is_some() {
            Some((Icon::AlarmClock, "Snoozed until"))
        } else if thread.later.send_at.is_some() {
            Some((Icon::Clock, "Sends"))
        } else {
            None
        };

        let (primary, secondary) = if unread {
            (theme.text, theme.text_muted)
        } else {
            (theme.text_muted, theme.text_faint)
        };

        // Labels as dots (Linear-style): the subject keeps its room. The
        // reader shows them as chips.
        // W4b (FOL-10): only labels set to show in the conversation list.
        let labels: Vec<_> = thread
            .labels
            .iter()
            .filter_map(|id| self.data.label(id))
            .filter(|l| l.in_message_list)
            .collect();
        let label_names = labels
            .iter()
            .map(|l| l.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let label_dots = (!labels.is_empty()).then(|| {
            let names = label_names.clone();
            div()
                .id(("label-dots", thread_ix))
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_2XS)
                .children(labels.iter().take(LABEL_DOTS).map(|l| {
                    if row_cx.no_color {
                        // MA-024: a letter tells them apart without color.
                        div()
                            .px(SPACE_XS)
                            .rounded(RADIUS_SM)
                            .bg(theme.label_chip_bg(l.color))
                            .text_color(theme.label_chip_text(l.color))
                            .text_size(TEXT_XS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                l.name
                                    .rsplit('/')
                                    .next()
                                    .and_then(|leaf| leaf.chars().next())
                                    .map(|c| c.to_uppercase().to_string())
                                    .unwrap_or_default(),
                            )
                    } else {
                        div().size(LABEL_DOT).rounded_full().bg(l.color)
                    }
                }))
                .when(labels.len() > LABEL_DOTS, |d| {
                    d.child(
                        div()
                            .text_size(TEXT_XS)
                            .text_color(secondary)
                            .child(format!("+{}", labels.len() - LABEL_DOTS)),
                    )
                })
                .tooltip(move |window, cx| tooltip(names.clone()).build(window, cx))
        });

        // The date, and in its place on hover the quick actions (ME-004:
        // the row's hover group swaps them; nothing is re-rendered). Both
        // sit in one grid cell and swap visibility, never `display`: GPUI
        // hit-tests between prepaint and paint, so a hover style that
        // changes `display` can paint children it never prepainted.
        let meta = div()
            .id(("row-meta", thread_ix))
            .col_start(1)
            .row_start(1)
            .flex()
            .justify_end()
            .items_center()
            .gap(SPACE_SM)
            .group_hover(ROW_GROUP, |s| s.invisible())
            .when(queued, |this| {
                // MA-069.
                this.child(
                    div()
                        .id(("queued", thread_ix))
                        .role(gpui::Role::Image)
                        .aria_label("Waiting to send")
                        .child(icon(Icon::Clock, ICON_SM, secondary))
                        .tooltip(|window, cx| tooltip("Waiting to send").build(window, cx)),
                )
            })
            .when(thread.has_attachments(), |this| {
                this.child(icon(Icon::Paperclip, ICON_SM, secondary))
            })
            .when_some(later_mark, |this, (glyph, _)| {
                this.child(icon(glyph, ICON_SM, theme.accent))
            })
            .when(thread.important, |this| {
                // W4b (FOL-12): Gmail's Important marker.
                this.child(
                    div()
                        .id(("important", thread_ix))
                        .role(gpui::Role::Image)
                        .aria_label("Important")
                        .child(icon(Icon::Important, ICON_SM, theme.star))
                        .tooltip(|window, cx| tooltip("Important").build(window, cx)),
                )
            })
            .when(needs_reply, |this| {
                // AI-4: Jev finds it waits for your answer.
                this.child(
                    div()
                        .id(("needs-reply", thread_ix))
                        .role(gpui::Role::Image)
                        .aria_label("Needs reply")
                        .child(icon(Icon::Reply, ICON_SM, theme.accent))
                        .tooltip(|window, cx| tooltip("Needs reply").build(window, cx)),
                )
            })
            .when(thread.starred, |this| {
                let star = icon(Icon::StarFilled, ICON_SM, theme.star);
                // MD-027: a star set just now pops.
                this.child(if self.star_pops(&thread.id) {
                    motion::pop(("star-pop", thread_ix), star).into_any_element()
                } else {
                    star.into_any_element()
                })
            })
            .child(
                div()
                    .flex_none()
                    .text_size(TEXT_SM)
                    .text_color(secondary)
                    .font_features(tabular_figures())
                    .child(date.clone()),
            );
        let trailing = div()
            .flex_none()
            .grid()
            .grid_cols(1)
            .grid_rows(1)
            .items_center()
            .child(meta)
            .child(self.render_row_actions(thread_ix, theme, cx));

        // In the inset, clear of the focus bar: the unread dot on the sender
        // line, or a checkbox centered in the row while picking or hovered.
        let pinned_checkbox = checked || selecting;
        let marker = div()
            .absolute()
            .left(-ROW_INSET + SPACE_XS)
            .w(ROW_INSET - SPACE_XS)
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .when(unread && !pinned_checkbox, |this| {
                this.child(
                    div()
                        .id(("unread-dot", thread_ix))
                        .size(UNREAD_DOT)
                        .rounded_full()
                        .bg(theme.accent)
                        .group_hover(ROW_GROUP, |s| s.invisible()),
                )
            });
        let check = div()
            .absolute()
            .left(SPACE_XS)
            .w(ROW_INSET - SPACE_XS)
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                checkbox(thread_ix, checked, &subject, theme)
                    .when(!pinned_checkbox, |this| {
                        this.invisible().group_hover(ROW_GROUP, |s| s.visible())
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_row_checked(thread_ix, cx);
                    })),
            );

        let sender = div()
            .min_w_0()
            .truncate()
            .text_size(TEXT_BASE)
            .font_weight(if unread {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_color(primary)
            .child(who.clone());
        let draft_mark = has_draft.then(|| {
            div()
                .flex_none()
                .text_size(TEXT_SM)
                .text_color(theme.text_danger)
                .child("Draft")
        });
        let message_count = (count > 1).then(|| {
            div()
                .flex_none()
                .text_size(TEXT_SM)
                .text_color(secondary)
                .font_features(tabular_figures())
                .child(format!("({count})"))
        });

        // MA-048: what the row shows, said.
        let mut spoken = Vec::new();
        if unread {
            spoken.push("Unread".to_string());
        }
        spoken.push(who.clone());
        if count > 1 {
            spoken.push(format!("{count} messages"));
        }
        if has_draft {
            spoken.push("Draft".into());
        }
        spoken.push(subject.to_string());
        match later_mark {
            Some((_, what)) => spoken.push(format!("{what} {date}")),
            None => spoken.push(date.clone()),
        }
        for (on, what) in [
            (thread.starred, "Starred"),
            (thread.important, "Important"),
            (needs_reply, "Needs reply"),
            (thread.has_attachments(), "Has attachments"),
            (queued, "Waiting to send"),
        ] {
            if on {
                spoken.push(what.into());
            }
        }
        if !label_names.is_empty() {
            spoken.push(format!("Labels: {label_names}"));
        }
        if let Some(line) = ai_line {
            spoken.push(format!("AI summary: {line}"));
        }
        let line_mark = ai_line.map(|_| ai_mark(thread_ix, secondary));

        let subject_weight = if unread {
            FontWeight::MEDIUM
        } else {
            FontWeight::NORMAL
        };
        let row = div()
            .id(("thread", thread_ix))
            .group(ROW_GROUP)
            .role(gpui::Role::ListBoxOption)
            .aria_label(spoken.join(", "))
            .aria_selected(selected || checked)
            .when(selected, |this| this.aria_active_descendant())
            .relative()
            .pl(ROW_INSET)
            .pr(SPACE_MD)
            .rounded(RADIUS_MD)
            .line_height(relative(1.4))
            .map(|this| {
                if checked {
                    this.bg(theme.checked_bg)
                } else if selected && list_focused {
                    this.bg(theme.selected_focused_bg)
                } else if selected {
                    this.bg(theme.selected_bg)
                } else {
                    this.hover(|s| s.bg(theme.hover_bg))
                }
            })
            .map(|this| {
                if compact {
                    this.py(SPACE_SM).child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .child(marker)
                            .child(
                                div()
                                    .flex_none()
                                    // A third of the row, so a narrow list
                                    // still leaves the subject room.
                                    .w(relative(0.3))
                                    .max_w(rems(11.25))
                                    .flex()
                                    .gap(SPACE_XS)
                                    .child(sender)
                                    .children(draft_mark)
                                    .children(message_count),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(TEXT_BASE)
                                    .text_color(primary)
                                    .font_weight(subject_weight)
                                    .child(subject_and_snippet(
                                        &subject,
                                        no_subject,
                                        &snippet,
                                        secondary,
                                        theme.text_faint,
                                    )),
                            )
                            .children(line_mark)
                            .children(label_dots)
                            .child(trailing),
                    )
                } else {
                    this.py(SPACE_MD)
                        .flex()
                        .flex_col()
                        .gap(SPACE_2XS)
                        // Sender, icons, date.
                        .child(
                            div()
                                .relative()
                                .flex()
                                .items_center()
                                .gap(SPACE_SM)
                                .child(marker)
                                .child(sender)
                                .children(draft_mark)
                                .children(message_count)
                                .child(div().flex_1())
                                .child(trailing),
                        )
                        // Subject and labels.
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_SM)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(TEXT_BASE)
                                        .map(|this| {
                                            if no_subject {
                                                // ME-021: a placeholder, not a subject.
                                                this.italic().text_color(theme.text_faint)
                                            } else {
                                                this.font_weight(subject_weight).text_color(primary)
                                            }
                                        })
                                        .child(subject),
                                )
                                .children(label_dots),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_XS)
                                .text_size(TEXT_SM)
                                .text_color(secondary)
                                .children(line_mark)
                                .child(div().flex_1().min_w_0().truncate().child(snippet)),
                        )
                }
            })
            // The keyboard's place: a short pill on the leading edge.
            .when(selected && list_focused, |this| {
                this.child(
                    div()
                        .absolute()
                        .left(SPACE_2XS)
                        .top(relative(0.25))
                        .bottom(relative(0.25))
                        .w(FOCUS_BAR)
                        .rounded_full()
                        .bg(theme.focus_bar),
                )
            })
            .when(self.is_context_target(thread_ix), |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .rounded(RADIUS_MD)
                        .border_1()
                        .border_color(theme.context_ring),
                )
            })
            .child(check)
            // MA-087: onto a folder, label or category in the sidebar.
            .on_drag(
                self.dragged_threads(thread_ix, checked),
                |dragged, _, _, cx| cx.new(|_| dragged.clone()),
            )
            .on_any_mouse_down(cx.listener(
                move |this, event: &gpui::MouseDownEvent, window, cx| {
                    if is_secondary(event) {
                        this.row_context_menu(thread_ix, event.position, window, cx)
                    }
                },
            ))
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    if !event.is_right_click() && !event.modifiers().control {
                        this.click_row(thread_ix, event.modifiers(), window, cx)
                    }
                }),
            );

        div()
            .px(ROW_MARGIN)
            .pt(SPACE_2XS)
            .child(row)
            .into_any_element()
    }

    /// What dragging the row carries: the picked threads when it's one of
    /// them, else the row's.
    fn dragged_threads(&self, thread_ix: usize, checked: bool) -> DraggedThreads {
        let thread = &self.data.threads[thread_ix];
        let ids: Vec<String> = if checked {
            self.rows
                .iter()
                .filter_map(|r| match r {
                    Row::Thread(ix) if self.is_checked(*ix) => {
                        Some(self.data.threads[*ix].id.clone())
                    }
                    _ => None,
                })
                .collect()
        } else {
            vec![thread.id.clone()]
        };
        let title = match ids.len() {
            1 => thread.subject_or_placeholder().to_string(),
            n => format!("{n} conversations"),
        };
        DraggedThreads {
            ids,
            title: title.into(),
        }
    }

    /// Archive, delete, read or unread and star, for the row under the
    /// mouse; they act on that row alone. Shown on the row's hover.
    fn render_row_actions(
        &self,
        thread_ix: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let thread = &self.data.threads[thread_ix];
        // ME-024: each teaches its key.
        let pop = self.star_pops(&thread.id);
        let button = |id: &'static str,
                      glyph: Icon,
                      color,
                      tip: &'static str,
                      action: Box<dyn gpui::Action>| {
            let glyph = icon(glyph, ICON_MD, color);
            div()
                .id((id, thread_ix))
                .role(gpui::Role::Button)
                .aria_label(tip)
                .size(HIT_TARGET)
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM)
                .hover(|s| s.bg(theme.selected_bg))
                .active(|s| s.bg(theme.selected_bg).opacity(0.8))
                .child(if pop && id == "row-star" {
                    motion::pop(("row-star-pop", thread_ix), glyph).into_any_element()
                } else {
                    glyph.into_any_element()
                })
                .tooltip(move |window, cx| {
                    tooltip(tip).action(action.as_ref(), None).build(window, cx)
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        div()
            .id(("row-actions", thread_ix))
            .col_start(1)
            .row_start(1)
            .flex()
            .justify_end()
            .my(-SPACE_XS)
            .invisible()
            .group_hover(ROW_GROUP, |s| s.visible())
            .items_center()
            .gap(SPACE_2XS)
            .when_some(thread.folder.archive_title(), |this, title| {
                this.child(
                    button(
                        "row-archive",
                        if thread.folder == Folder::Inbox {
                            Icon::Archive
                        } else {
                            Icon::Inbox
                        },
                        theme.icon,
                        title,
                        Box::new(Archive),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.archive_threads(vec![thread_ix], window, cx)
                    })),
                )
            })
            .when(thread.folder != Folder::Trash, |this| {
                this.child(
                    button(
                        "row-delete",
                        Icon::Trash,
                        theme.icon,
                        if thread.folder == Folder::Drafts {
                            "Discard draft"
                        } else {
                            "Delete"
                        },
                        Box::new(Delete),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.delete_threads(vec![thread_ix], window, cx)
                    })),
                )
            })
            .child(
                if thread.unread {
                    button(
                        "row-read",
                        Icon::MailOpen,
                        theme.icon,
                        "Mark as read",
                        Box::new(MarkRead),
                    )
                } else {
                    button(
                        "row-unread",
                        Icon::MailUnread,
                        theme.icon,
                        "Mark as unread",
                        Box::new(MarkUnread),
                    )
                }
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let unread = !this.data.threads[thread_ix].unread;
                    this.set_unread(vec![thread_ix], unread, cx)
                })),
            )
            .child(
                if thread.starred {
                    button(
                        "row-star",
                        Icon::StarFilled,
                        theme.star,
                        "Unstar",
                        Box::new(ToggleStar),
                    )
                } else {
                    button(
                        "row-star",
                        Icon::Star,
                        theme.icon,
                        "Star",
                        Box::new(ToggleStar),
                    )
                }
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.star_threads(vec![thread_ix], cx)
                })),
            )
    }
}

/// An empty view: a glyph, a title VoiceOver reads, and a next step.
fn empty_view(
    glyph: Icon,
    title: impl Into<SharedString>,
    hint: Option<AnyElement>,
    theme: &Theme,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(SPACE_SM)
        .px(SPACE_LG)
        .line_height(LINE_UI)
        .child(icon(glyph, ICON_HERO, theme.text_faint))
        .child(
            label("empty-title", title)
                .text_size(TEXT_BASE)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted),
        )
        .children(hint)
}

/// `content` in the middle of the list's pane.
fn centered(content: gpui::Div) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(content)
        .into_any_element()
}

fn muted_hint(text: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    label("empty-hint", text)
        .text_size(TEXT_SM)
        .text_color(theme.text_faint)
        .into_any_element()
}

/// AI-7: the small mark on a row whose snippet is the AI's line.
fn ai_mark(thread_ix: usize, color: gpui::Hsla) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("ai-line", thread_ix))
        .flex_none()
        .role(gpui::Role::Image)
        .aria_label("AI summary")
        .text_size(TEXT_XS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child("AI")
        .tooltip(|window, cx| tooltip("Summary by AI").build(window, cx))
}

/// "Subject — snippet", the snippet dimmer, on one line; "(no subject)"
/// faint and italic (ME-021).
fn subject_and_snippet(
    subject: &str,
    placeholder: bool,
    snippet: &str,
    dim: gpui::Hsla,
    faint: gpui::Hsla,
) -> StyledText {
    let text = format!("{subject} — {snippet}");
    let snippet_range = subject.len()..text.len();
    let mut highlights = Vec::new();
    if placeholder {
        highlights.push((
            0..subject.len(),
            HighlightStyle {
                color: Some(faint),
                font_style: Some(FontStyle::Italic),
                font_weight: Some(FontWeight::NORMAL),
                ..Default::default()
            },
        ));
    }
    highlights.push((
        snippet_range,
        HighlightStyle {
            color: Some(dim),
            font_weight: Some(FontWeight::NORMAL),
            ..Default::default()
        },
    ));
    StyledText::new(text).with_highlights(highlights)
}

/// The box of a checkbox: empty, checked, or mixed.
fn check_box(state: gpui::Toggled, theme: &Theme) -> gpui::Div {
    div()
        .size(ICON_MD)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .map(|d| match state {
            gpui::Toggled::True => {
                d.bg(theme.accent)
                    .child(icon(Icon::Check, ICON_SM, theme.accent_text))
            }
            gpui::Toggled::Mixed => {
                d.bg(theme.accent)
                    .child(icon(Icon::Minus, ICON_SM, theme.accent_text))
            }
            _ => d.border_1().border_color(theme.text_faint),
        })
}

/// A row's checkbox for picking it with others.
fn checkbox(
    thread_ix: usize,
    checked: bool,
    subject: &str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let state = if checked {
        gpui::Toggled::True
    } else {
        gpui::Toggled::False
    };
    div()
        .id(("row-check", thread_ix))
        .role(gpui::Role::CheckBox)
        .aria_label(format!("Select {subject}"))
        .aria_toggled(state)
        .size(HIT_TARGET)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(check_box(state, theme))
}

/// Placeholder rows while a view's first page loads (STA-1).
fn render_skeleton(theme: &Theme) -> AnyElement {
    let bar = |width: gpui::DefiniteLength, height: gpui::Rems| {
        div()
            .w(width)
            .h(height)
            .rounded(RADIUS_SM)
            .bg(theme.chip_bg)
    };
    let row = |ix: usize| {
        div()
            .pl(ROW_MARGIN + ROW_INSET)
            .pr(SPACE_LG)
            .py(SPACE_MD)
            .flex()
            .flex_col()
            .gap(SPACE_SM)
            // Fainter down the list.
            .opacity(1. - ix as f32 * 0.12)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(bar(relative(0.35), TEXT_BASE * 0.75))
                    .child(bar(relative(0.12), TEXT_BASE * 0.75)),
            )
            .child(bar(relative(0.7), TEXT_SM * 0.75))
            .child(bar(relative(0.9), TEXT_SM * 0.75))
    };
    // MA-066: only once the load takes a moment, so a fast one never
    // flashes it.
    motion::after_loading_delay(
        "thread-list-loading-in",
        div().size_full().child(
            div()
                .id("thread-list-loading")
                .role(gpui::Role::Status)
                .aria_label("Loading conversations")
                .size_full()
                .pt(SPACE_XS)
                .flex()
                .flex_col()
                .children((0..6).map(row))
                .with_animation(
                    "thread-list-loading",
                    // MC-041: it keeps pulsing (MC-Q9 a), at 30 frames a second.
                    Animation::new(std::time::Duration::from_millis(1200))
                        .repeat()
                        .with_max_fps(30.)
                        .with_easing(gpui::pulsating_between(0.5, 1.)),
                    |this, delta| this.opacity(delta),
                ),
        ),
    )
    .into_any_element()
}

/// "12 results", or approximate while Gmail has more pages (SRC-6): its
/// estimate when larger, else "12+ results". Exact once everything is in.
fn result_count(count: usize, estimate: Option<usize>, more: bool) -> String {
    match estimate.filter(|&n| n > count) {
        Some(n) if more => format!("About {} results", thousands(n)),
        _ if more => format!("{}+ results", thousands(count)),
        _ => plural(count, "result", "results"),
    }
}

#[cfg(test)]
mod tests {
    use super::result_count;

    #[test]
    fn result_count_is_approximate_while_gmail_has_more() {
        assert_eq!(result_count(1, None, false), "1 result");
        assert_eq!(result_count(1124, Some(1124), false), "1,124 results");
        assert_eq!(result_count(1124, Some(1200), true), "About 1,200 results");
        assert_eq!(result_count(1124, Some(1000), true), "1,124+ results");
        assert_eq!(result_count(1124, None, true), "1,124+ results");
        // Everything in: exact, whatever the estimate said.
        assert_eq!(result_count(30, Some(41), false), "30 results");
        assert_eq!(result_count(0, None, false), "0 results");
    }
}

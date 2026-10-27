//! The virtualized thread list.

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, FontWeight, HighlightStyle,
    InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Window,
    component::{input::Input, scroll::ScrollableElement as _, tooltip::Tooltip},
    deferred, div, ease_out_quint, list,
    prelude::FluentBuilder as _,
    px, relative,
};

use super::{HEADER_HEIGHT, avatar, empty_state, icon, icon_button, window_drag_area};
use crate::ui::style::*;
use crate::{
    actions::{
        CloseSearch, LoadOlderConversations, SEARCH, ToggleArchivedSection, ToggleSearch,
        ToggleSidebar,
    },
    app::{Density, Nav, PigeonApp, Row, SearchSuggestion},
    assets::Icon,
    data::{Folder, list_date},
    search::ServerState,
    theme::Theme,
};

/// Where a row's text starts: the row's margin plus its inset, which holds
/// the unread dot. The header and the result count line up with it.
const ROW_MARGIN: gpui::Pixels = SPACE_SM;
const ROW_INSET: gpui::Pixels = SPACE_XL;
/// Labels shown as dots in a row, at most; the reader shows them all.
const LABEL_DOTS: usize = 3;

impl PigeonApp {
    pub fn nav_title(&self) -> SharedString {
        match self.nav {
            Nav::Folder(folder) => folder.title().into(),
            Nav::Starred => "Starred".into(),
            Nav::Label(ix) => self.data.labels[ix].name.clone().into(),
        }
    }

    pub(crate) fn render_thread_list(
        &self,
        width: gpui::Pixels,
        sidebar_shown: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        // The sidebar's number, so the two agree (LST-6).
        let unread = self.view_unread(self.nav);

        let header = window_drag_area(div().id("list-header"))
            .h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .pr(SPACE_MD)
            .map(|this| {
                if sidebar_shown {
                    this.pl(ROW_MARGIN + ROW_INSET)
                } else {
                    // The way back, in the sidebar's button's place.
                    this.pl(SPACE_SM).child(icon_button(
                        "show-sidebar",
                        Icon::PanelLeft,
                        "Show sidebar",
                        Box::new(ToggleSidebar),
                        &theme,
                    ))
                }
            })
            .map(|this| {
                if self.search_open {
                    return this.child(self.render_search_bar(&theme, cx));
                }
                this.child(
                    div()
                        .text_size(TEXT_LG)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.nav_title()),
                )
                .when(unread > 0, |this| {
                    this.child(
                        div()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_muted)
                            .font_features(tabular_figures())
                            .child(format!("{unread} unread")),
                    )
                })
                .child(div().flex_1())
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
        // "Load older" alone (W9) still leaves the view empty.
        let empty = self.rows.iter().all(|r| *r == Row::LoadOlder);
        let body = if empty && searching && asking_gmail {
            // W9 (SRC-7): nothing here, but Gmail may have some.
            empty_state(Icon::Search, "Searching Gmail…", None, &theme)
        } else if empty && searching {
            empty_state(
                Icon::Search,
                format!("No results {}", self.search_scope_text()),
                Some(if self.search_query.scope.is_some() || self.search_all {
                    "Try fewer or different words"
                } else {
                    "Try fewer words, or search all mail with in:all"
                }),
                &theme,
            )
        } else if empty && !self.view_loaded(cx) {
            // Not "Nothing in …" before the first page is in (LST-9).
            match self.backend.read(cx).sync_error() {
                Some(error) => empty_state(
                    Icon::OctagonAlert,
                    format!("Couldn't load {}", self.nav_title()),
                    Some(if error.starts_with("Couldn't reach") {
                        "Pigeon will try again when it can reach Gmail"
                    } else {
                        "Pigeon will try again shortly"
                    }),
                    &theme,
                ),
                None => render_skeleton(&theme),
            }
        } else if empty {
            empty_state(
                Icon::Inbox,
                format!("Nothing in {}", self.nav_title()),
                None,
                &theme,
            )
        } else {
            let view = cx.entity();
            div()
                .id("thread-list")
                .role(gpui::Role::ListBox)
                .aria_label(self.nav_title())
                .relative()
                .size_full()
                .child(
                    list(self.list_state.clone(), move |ix, window, cx| {
                        view.update(cx, |this, cx| this.render_row(ix, window, cx))
                    })
                    .size_full()
                    .pb(SPACE_SM),
                )
                .vertical_scrollbar(&self.list_state)
                .into_any_element()
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
            .when(
                searching && (!empty || self.server_search.state == ServerState::Offline),
                |this| this.child(self.render_search_summary(&theme)),
            )
            .child(div().flex_1().min_h_0().child(body))
            .when(
                empty && !self.rows.is_empty() && (searching || self.view_loaded(cx)),
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
                this.child(self.render_search_suggestions(suggestions, &theme, cx))
            })
    }

    /// Whether the view's first page has loaded from Gmail (STA-1).
    fn view_loaded(&self, cx: &gpui::App) -> bool {
        let view = match self.nav {
            Nav::Folder(folder) => crate::backend::ViewKey::Folder(folder),
            Nav::Starred => crate::backend::ViewKey::Starred,
            Nav::Label(ix) => crate::backend::ViewKey::Label(self.data.labels[ix].id.clone()),
        };
        self.backend.read(cx).view_loaded(&view)
    }

    /// "in Inbox", or where an `in:` operator or the scope chip points.
    fn search_scope_text(&self) -> String {
        use crate::search::Scope;
        match self.search_query.scope {
            None if self.search_all => "in all mail".into(),
            None => format!("in {}", self.nav_title()),
            Some(Scope::All) => "in all mail".into(),
            Some(Scope::Anywhere) => "anywhere".into(),
            Some(Scope::Folder(folder)) => format!("in {}", folder.title()),
        }
    }

    /// Under the search field: the operators it understood, tinted, and how
    /// many threads match.
    fn render_search_summary(&self, theme: &Theme) -> impl IntoElement {
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
        let note = match self.server_search.state {
            ServerState::Waiting => Some("Searching Gmail…"),
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
            .children(self.search_query.operators().into_iter().map(|op| {
                div()
                    .h(px(20.))
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .rounded(RADIUS_SM)
                    .bg(theme.accent.alpha(0.12))
                    .text_color(theme.accent)
                    .font_weight(FontWeight::MEDIUM)
                    .child(op)
            }))
            .child(
                div()
                    .ml(SPACE_2XS)
                    .font_features(tabular_figures())
                    .child(format!("{counted} {}", self.search_scope_text())),
            )
            .children(note.map(|note| {
                div()
                    .id("search-gmail-note")
                    .role(gpui::Role::Status)
                    .aria_label(note)
                    .ml(SPACE_2XS)
                    .text_color(theme.text_faint)
                    .child(format!("· {note}"))
            }))
    }

    /// The search button, expanded into a field that fills the header. It
    /// grows out of the button's place on the right.
    fn render_search_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // Where the search looks; a click widens it to all mail. An `in:`
        // in the query decides instead.
        let fixed = self.search_query.scope.is_some();
        let scope_label: SharedString = if fixed {
            "in:".into()
        } else if self.search_all {
            "All Mail".into()
        } else {
            self.nav_title()
        };
        let scope = div()
            .id("search-scope")
            .role(gpui::Role::Button)
            .aria_label(format!("Search in {scope_label}"))
            .flex_none()
            .h(px(22.))
            .pl(SPACE_SM)
            .pr(SPACE_XS)
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .rounded(RADIUS_SM)
            .bg(theme.chip_bg)
            .text_size(TEXT_SM)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .child(scope_label)
            .when(!fixed, |this| {
                this.child(icon(Icon::ChevronDown, px(12.), theme.text_faint))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.selected_bg).text_color(theme.text))
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_search_scope(cx)))
                    .tooltip(|window, cx| {
                        Tooltip::new("Switch between this folder and all mail").build(window, cx)
                    })
            });

        let field = div()
            .key_context(SEARCH)
            .h(px(32.))
            .pl(SPACE_MD)
            .pr(SPACE_XS)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .bg(theme.search_bg)
            .border_1()
            .border_color(theme.border)
            .text_size(TEXT_BASE)
            .overflow_hidden()
            .child(icon(Icon::Search, px(14.), theme.text_muted))
            .child(scope)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.search_input).appearance(false).px(px(0.))),
            )
            .child(
                icon_button(
                    "close-search",
                    Icon::X,
                    "Clear search",
                    Box::new(CloseSearch),
                    theme,
                )
                .size(HIT_TARGET),
            )
            .with_animation(
                "search-expand",
                Animation::new(MOTION_PANE).with_easing(ease_out_quint()),
                |field, delta| {
                    field
                        .w(relative(0.12 + 0.88 * delta))
                        .opacity(0.4 + 0.6 * delta)
                },
            );
        div().flex_1().flex().justify_end().child(field)
    }

    /// The dropdown under the search field: operators and recent searches
    /// while it's empty, contacts after `from:` or `to:`.
    fn render_search_suggestions(
        &self,
        suggestions: Vec<SearchSuggestion>,
        theme: &Theme,
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
        let mut last_kind = None;
        for (ix, suggestion) in suggestions.into_iter().enumerate() {
            let kind = std::mem::discriminant(&suggestion);
            if last_kind != Some(kind) {
                last_kind = Some(kind);
                rows.push(
                    section(match suggestion {
                        SearchSuggestion::Operator(..) => "Narrow it down",
                        SearchSuggestion::Recent(_) => "Recent",
                        SearchSuggestion::Contact { .. } => "People",
                    })
                    .into_any_element(),
                );
            }
            let highlighted = self.search_highlight == Some(ix);
            let content: AnyElement = match &suggestion {
                SearchSuggestion::Operator(op, what) => div()
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .child(
                        div()
                            .font_family("Menlo")
                            .text_size(TEXT_SM)
                            .text_color(theme.accent)
                            .child(*op),
                    )
                    .child(div().text_color(theme.text_muted).child(*what))
                    .into_any_element(),
                SearchSuggestion::Recent(query) => div()
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .min_w_0()
                    .child(icon(Icon::Clock, px(14.), theme.text_faint))
                    .child(div().min_w_0().truncate().child(query.clone()))
                    .into_any_element(),
                SearchSuggestion::Contact {
                    operator,
                    name,
                    email,
                } => div()
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
                        px(20.),
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
            };
            rows.push(
                div()
                    .id(("search-suggestion", ix))
                    .role(gpui::Role::ListBoxOption)
                    .aria_selected(highlighted)
                    .h(px(30.))
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
        deferred(
            div()
                .id("search-suggestions")
                .role(gpui::Role::ListBox)
                .aria_label("Search suggestions")
                .occlude()
                .absolute()
                .top(HEADER_HEIGHT - SPACE_XS)
                .left(ROW_MARGIN)
                .right(ROW_MARGIN)
                .p(SPACE_XS)
                .flex()
                .flex_col()
                .rounded(RADIUS_MD)
                .bg(theme.overlay_bg)
                .border_1()
                .border_color(theme.overlay_border)
                .shadow(super::popover_shadow(theme))
                .children(rows),
        )
        .with_priority(1)
    }

    fn render_row(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        match self.rows[ix] {
            Row::Thread(thread_ix) => self.render_thread_row(thread_ix, &theme, window, cx),
            Row::ArchivedHeader { count } => self.render_archived_header(count, &theme, cx),
            Row::LoadOlder => self.render_load_older(&theme, cx),
        }
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
                    .h(HIT_TARGET)
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
                        theme.text_faint
                    })
                    .child(icon(Icon::ChevronDown, px(14.), theme.text_faint))
                    .child(label)
                    .when(enabled, |this| {
                        this.cursor_pointer()
                            .hover(|s| s.bg(theme.hover_bg))
                            .tooltip(|window, cx| {
                                Tooltip::new("Load older conversations")
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
                    .aria_label(format!("Archived, {count}"))
                    .aria_expanded(expanded)
                    .h(px(30.))
                    .pl(ROW_INSET)
                    .pr(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(icon(Icon::Archive, px(14.), theme.text_faint))
                    .child("Archived")
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .font_weight(FontWeight::NORMAL)
                            .font_features(tabular_figures())
                            .child(count.to_string()),
                    )
                    .child(div().flex_1())
                    .child(icon(
                        if expanded {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        },
                        px(14.),
                        theme.text_faint,
                    ))
                    .tooltip(move |window, cx| {
                        Tooltip::new(if expanded {
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
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let thread = &self.data.threads[thread_ix];
        let last = thread.last_message();
        let selected = self.selected == Some(thread_ix);
        let checked = self.is_checked(thread_ix);
        let selecting = !self.checked.is_empty();
        let hovered = self.hovered_thread == Some(thread_ix);
        let unread = thread.unread;
        let compact = self.density == Density::Compact;
        // Accent while the list has the keyboard, grey while the reader
        // (or anything else) has it.
        let list_focused = self.focus_handle.is_focused(window) && !self.reader_focused;

        // Sent and drafts are about who they went to; everything else about
        // who wrote last.
        let who = match thread.folder {
            Folder::Sent | Folder::Drafts => last
                .to
                .iter()
                .chain(&last.cc)
                .chain(&last.bcc)
                .next()
                .map(|c| format!("To: {}", c.display_name()))
                .unwrap_or_else(|| "(no recipients)".into()),
            _ => thread
                .messages
                .iter()
                .rev()
                .map(|m| &m.from)
                .find(|c| !self.data.is_me(c))
                .unwrap_or(&last.from)
                .display_name()
                .to_string(),
        };
        let count = thread.messages.len();
        let subject: SharedString = if thread.subject.is_empty() {
            "(no subject)".into()
        } else {
            thread.subject.clone().into()
        };

        let (primary, secondary) = if unread {
            (theme.text, theme.text_muted)
        } else {
            (theme.text_dim, theme.text_faint)
        };

        // Labels as dots (Linear-style): the subject keeps its room. The
        // reader shows them as chips.
        let labels: Vec<_> = thread
            .labels
            .iter()
            .filter_map(|id| self.data.label(id))
            .collect();
        let label_dots = (!labels.is_empty()).then(|| {
            let names = labels
                .iter()
                .map(|l| l.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            div()
                .id(("label-dots", thread_ix))
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_2XS)
                .children(
                    labels
                        .iter()
                        .take(LABEL_DOTS)
                        .map(|l| div().size(px(7.)).rounded_full().bg(l.color)),
                )
                .when(labels.len() > LABEL_DOTS, |d| {
                    d.child(
                        div()
                            .text_size(TEXT_XS)
                            .text_color(secondary)
                            .child(format!("+{}", labels.len() - LABEL_DOTS)),
                    )
                })
                .tooltip(move |window, cx| Tooltip::new(names.clone()).build(window, cx))
        });

        // The date, or on hover the quick actions in its place.
        let trailing = if hovered {
            self.render_row_actions(thread_ix, theme, cx)
                .into_any_element()
        } else {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .when(thread.has_attachments(), |this| {
                    this.child(icon(Icon::Paperclip, px(13.), secondary))
                })
                .when(thread.starred, |this| {
                    this.child(icon(Icon::StarFilled, px(13.), theme.star))
                })
                .child(
                    div()
                        .flex_none()
                        .text_size(TEXT_SM)
                        .text_color(secondary)
                        .font_features(tabular_figures())
                        .child(list_date(&last.date)),
                )
                .into_any_element()
        };

        // In the inset, clear of the focus bar: the unread dot on the sender
        // line, or a checkbox centered in the row while picking.
        let show_checkbox = checked || selecting || hovered;
        let marker = div()
            .absolute()
            .left(-ROW_INSET + SPACE_XS)
            .w(ROW_INSET - SPACE_XS)
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .when(unread && !show_checkbox, |this| {
                this.child(div().size(px(6.)).rounded_full().bg(theme.accent))
            });
        let check = show_checkbox.then(|| {
            div()
                .absolute()
                .left(SPACE_XS)
                .w(ROW_INSET - SPACE_XS)
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .justify_center()
                .child(checkbox(thread_ix, checked, theme).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_row_checked(thread_ix, cx);
                    },
                )))
        });

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
        let message_count = (count > 1).then(|| {
            div()
                .flex_none()
                .text_size(TEXT_SM)
                .text_color(secondary)
                .child(count.to_string())
        });

        let row = div()
            .id(("thread", thread_ix))
            .role(gpui::Role::ListBoxOption)
            .aria_label(format!(
                "{}{who}, {}, {}",
                if unread { "Unread, " } else { "" },
                thread.subject,
                list_date(&last.date)
            ))
            .aria_selected(selected || checked)
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
                                    .max_w(px(180.))
                                    .flex()
                                    .gap(SPACE_XS)
                                    .child(sender)
                                    .children(message_count),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(TEXT_BASE)
                                    .text_color(primary)
                                    .font_weight(if unread {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .child(subject_and_snippet(&subject, &last.snippet, secondary)),
                            )
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
                                        .font_weight(if unread {
                                            FontWeight::MEDIUM
                                        } else {
                                            FontWeight::NORMAL
                                        })
                                        .text_color(primary)
                                        .child(subject),
                                )
                                .children(label_dots),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(TEXT_SM)
                                .text_color(secondary)
                                .child(last.snippet.clone()),
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
                        .w(px(3.))
                        .rounded_full()
                        .bg(theme.focus_bar),
                )
            })
            .children(check)
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_thread = Some(thread_ix);
                } else if this.hovered_thread == Some(thread_ix) {
                    this.hovered_thread = None;
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.row_context_menu(thread_ix, event.position, window, cx)
                }),
            )
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    if !event.is_right_click() {
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

    /// Archive, delete, read or unread and star, for the row under the
    /// mouse; they act on that row alone.
    fn render_row_actions(
        &self,
        thread_ix: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let thread = &self.data.threads[thread_ix];
        let button = |id: &'static str, glyph: Icon, color, tip: &'static str| {
            div()
                .id((id, thread_ix))
                .role(gpui::Role::Button)
                .aria_label(tip)
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM)
                .hover(|s| s.bg(theme.selected_bg))
                .active(|s| s.opacity(0.7))
                .child(icon(glyph, px(15.), color))
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        div()
            .flex_none()
            .my(-SPACE_XS)
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .when_some(thread.folder.archive_target(), |this, target| {
                this.child(
                    button(
                        "row-archive",
                        if target == Folder::Inbox {
                            Icon::Inbox
                        } else {
                            Icon::Archive
                        },
                        theme.icon,
                        if target == Folder::Inbox {
                            "Move to Inbox"
                        } else {
                            "Archive"
                        },
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
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.delete_threads(vec![thread_ix], window, cx)
                    })),
                )
            })
            .child(
                if thread.unread {
                    button("row-read", Icon::MailOpen, theme.icon, "Mark as read")
                } else {
                    button("row-unread", Icon::MailUnread, theme.icon, "Mark as unread")
                }
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let unread = !this.data.threads[thread_ix].unread;
                    this.set_unread(vec![thread_ix], unread, cx)
                })),
            )
            .child(
                if thread.starred {
                    button("row-star", Icon::StarFilled, theme.star, "Unstar")
                } else {
                    button("row-star", Icon::Star, theme.icon, "Star")
                }
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.star_threads(vec![thread_ix], cx)
                })),
            )
    }
}

/// "Subject — snippet", the snippet dimmer, on one line.
fn subject_and_snippet(subject: &str, snippet: &str, dim: gpui::Hsla) -> StyledText {
    let text = format!("{subject}  {snippet}");
    let range = subject.len()..text.len();
    StyledText::new(text).with_highlights([(
        range,
        HighlightStyle {
            color: Some(dim),
            font_weight: Some(FontWeight::NORMAL),
            ..Default::default()
        },
    )])
}

/// A row's checkbox for picking it with others.
fn checkbox(thread_ix: usize, checked: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("row-check", thread_ix))
        .role(gpui::Role::CheckBox)
        .aria_label("Select")
        .aria_toggled(if checked {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM)
                .map(|d| {
                    if checked {
                        d.bg(theme.accent)
                            .child(icon(Icon::Check, px(11.), theme.accent_text))
                    } else {
                        d.border_1().border_color(theme.text_faint)
                    }
                }),
        )
}

/// Placeholder rows while a view's first page loads (STA-1).
fn render_skeleton(theme: &Theme) -> AnyElement {
    let bar = |width: gpui::DefiniteLength, height: gpui::Pixels| {
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
                    .child(bar(relative(0.35), px(10.)))
                    .child(bar(relative(0.12), px(10.))),
            )
            .child(bar(relative(0.7), px(9.)))
            .child(bar(relative(0.9), px(9.)))
    };
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
            Animation::new(std::time::Duration::from_millis(1200))
                .repeat()
                .with_easing(gpui::pulsating_between(0.5, 1.)),
            |this, delta| this.opacity(delta),
        )
        .into_any_element()
}

/// "12 results", or approximate while Gmail has more pages (SRC-6): its
/// estimate when larger, else "12+ results". Exact once everything is in.
fn result_count(count: usize, estimate: Option<usize>, more: bool) -> String {
    use super::status::thousands;
    match estimate.filter(|&n| n > count) {
        Some(n) if more => format!("About {} results", thousands(n)),
        _ if more => format!("{}+ results", thousands(count)),
        _ => format!(
            "{} result{}",
            thousands(count),
            if count == 1 { "" } else { "s" }
        ),
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

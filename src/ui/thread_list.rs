//! The virtualized thread list.

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
    component::{input::Input, scroll::ScrollableElement as _},
    div, ease_out_quint, list,
    prelude::FluentBuilder as _,
    px, relative,
};

use super::{HEADER_HEIGHT, empty_state, icon, icon_button, label_chip, window_drag_area};
use crate::ui::style::*;
use crate::{
    actions::{CloseSearch, SEARCH, ToggleArchivedSection, ToggleSearch, ToggleSidebar},
    app::{Nav, PigeonApp, Row},
    assets::Icon,
    data::{Folder, list_date},
    theme::Theme,
};

/// Where a row's text starts: the row's margin plus its inset, which holds
/// the unread dot. The header and the result count line up with it.
const ROW_MARGIN: gpui::Pixels = SPACE_SM;
const ROW_INSET: gpui::Pixels = SPACE_XL;

impl PigeonApp {
    pub fn nav_title(&self) -> SharedString {
        match self.nav {
            Nav::Folder(folder) => match folder {
                Folder::Inbox => "Inbox",
                Folder::Archive => "Archive",
                Folder::Sent => "Sent",
                Folder::Drafts => "Drafts",
                Folder::Spam => "Spam",
                Folder::Trash => "Trash",
            }
            .into(),
            Nav::Starred => "Starred".into(),
            Nav::Label(ix) => self.data.labels[ix].name.clone().into(),
        }
    }

    pub(crate) fn render_thread_list(
        &self,
        width: gpui::Pixels,
        sidebar_shown: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let unread = self
            .rows
            .iter()
            .filter(|row| match row {
                Row::Thread(ix) => {
                    let t = &self.data.threads[*ix];
                    t.unread && self.nav_contains(self.nav, t)
                }
                Row::ArchivedHeader { .. } => false,
            })
            .count();

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
                    return this.child(self.render_search_bar(&theme));
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
                            .text_color(theme.text_faint)
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
        let body = if self.rows.is_empty() && searching {
            empty_state(
                Icon::Search,
                format!("No results {}", self.search_scope_text()),
                Some(if self.search_query.scope.is_some() {
                    "Try fewer or different words"
                } else {
                    "Try fewer words, or in:all to look in every folder"
                }),
                &theme,
            )
        } else if self.rows.is_empty() {
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
            .when(searching, |this| {
                let count = self.rows.len();
                this.child(
                    div()
                        .flex_none()
                        .pl(ROW_MARGIN + ROW_INSET)
                        .pr(SPACE_LG)
                        .pb(SPACE_SM)
                        .text_size(TEXT_SM)
                        .text_color(theme.text_faint)
                        .child(format!(
                            "{count} result{} {}",
                            if count == 1 { "" } else { "s" },
                            self.search_scope_text()
                        )),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
    }

    /// "in Inbox", or where an `in:` operator points.
    fn search_scope_text(&self) -> String {
        use crate::search::Scope;
        match self.search_query.scope {
            None => format!("in {}", self.nav_title()),
            Some(Scope::All) => "in all mail".into(),
            Some(Scope::Anywhere) => "anywhere".into(),
            Some(Scope::Folder(folder)) => {
                let title = match folder {
                    Folder::Inbox => "Inbox",
                    Folder::Archive => "Archive",
                    Folder::Sent => "Sent",
                    Folder::Drafts => "Drafts",
                    Folder::Spam => "Spam",
                    Folder::Trash => "Trash",
                };
                format!("in {title}")
            }
        }
    }

    /// The search button, expanded into a field that fills the header. It
    /// grows out of the button's place on the right.
    fn render_search_bar(&self, theme: &Theme) -> impl IntoElement {
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

    fn render_row(&self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        match self.rows[ix] {
            Row::Thread(thread_ix) => self.render_thread_row(thread_ix, &theme, cx),
            Row::ArchivedHeader { count } => self.render_archived_header(count, &theme, cx),
        }
    }

    fn render_archived_header(
        &self,
        count: usize,
        theme: &Theme,
        _: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .px(ROW_MARGIN)
            .pt(SPACE_MD)
            .child(
                div()
                    .id("archived-header")
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
                    .child("Archived")
                    .child(
                        div()
                            .text_color(theme.text_faint)
                            .font_weight(FontWeight::NORMAL)
                            .child(count.to_string()),
                    )
                    .child(icon(
                        if self.archived_expanded {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        },
                        px(14.),
                        theme.text_faint,
                    ))
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let thread = &self.data.threads[thread_ix];
        let last = thread.last_message();
        let selected = self.selected == Some(thread_ix);
        let unread = thread.unread;

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

        let (primary, secondary) = if unread {
            (theme.text, theme.text_muted)
        } else {
            (theme.text_dim, theme.text_faint)
        };

        let chips = thread
            .labels
            .iter()
            .filter_map(|id| self.data.label(id))
            .map(|label| label_chip(label, theme));

        div()
            .px(ROW_MARGIN)
            .pt(SPACE_2XS)
            .child(
                div()
                    .id(("thread", thread_ix))
                    .role(gpui::Role::ListBoxOption)
                    .aria_label(format!(
                        "{}{who}, {}, {}",
                        if unread { "Unread, " } else { "" },
                        thread.subject,
                        list_date(&last.date)
                    ))
                    .aria_selected(selected)
                    .relative()
                    .pl(ROW_INSET)
                    .pr(SPACE_MD)
                    .py(SPACE_MD)
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .line_height(relative(1.4))
                    .rounded(RADIUS_MD)
                    .map(|this| {
                        if selected && self.reader_focused {
                            this.bg(theme.selected_inactive_bg)
                        } else if selected {
                            this.bg(theme.selected_bg)
                        } else {
                            this.hover(|s| s.bg(theme.hover_bg))
                        }
                    })
                    // Sender, icons, date.
                    .child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .when(unread, |this| {
                                // Centered in the inset, level with this line
                                // at any text size.
                                this.child(
                                    div()
                                        .absolute()
                                        .left(px(-15.))
                                        .top_0()
                                        .bottom_0()
                                        .flex()
                                        .items_center()
                                        .child(div().size(px(6.)).rounded_full().bg(theme.accent)),
                                )
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(TEXT_BASE)
                                    .font_weight(if unread {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(primary)
                                    .child(who),
                            )
                            .when(count > 1, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .text_size(TEXT_SM)
                                        .text_color(secondary)
                                        .child(count.to_string()),
                                )
                            })
                            .child(div().flex_1())
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
                                    .child(list_date(&last.date)),
                            ),
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
                                    .child(if thread.subject.is_empty() {
                                        "(no subject)".into()
                                    } else {
                                        thread.subject.clone()
                                    }),
                            )
                            .children(chips),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(TEXT_SM)
                            .text_color(secondary)
                            .child(last.snippet.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_thread(thread_ix, window, cx)
                    })),
            )
            .into_any_element()
    }
}

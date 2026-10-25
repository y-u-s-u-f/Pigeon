//! The virtualized thread list.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, component::scroll::ScrollableElement as _, div,
    list, prelude::FluentBuilder as _, px,
};

use super::{
    HEADER_HEIGHT, PaneResize, empty_state, icon, icon_button, label_chip, pane_divider,
    window_drag_area,
};
use crate::{
    actions::{ToggleArchivedSection, ToggleSearch, ToggleSidebar},
    app::{Nav, PigeonApp, RAIL_WIDTH, Row},
    assets::Icon,
    data::{Folder, list_date},
    theme::Theme,
};

/// Where the traffic lights end, measured from the window's left edge.
const TRAFFIC_LIGHTS_END: gpui::Pixels = px(84.);

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
            .gap(px(8.))
            .pr(px(10.))
            .map(|this| {
                if self.sidebar_open {
                    this.pl(px(18.))
                } else {
                    // Clear the traffic lights, then offer the way back.
                    this.pl(TRAFFIC_LIGHTS_END - RAIL_WIDTH).child(icon_button(
                        "show-sidebar",
                        Icon::PanelLeft,
                        "Show sidebar",
                        Box::new(ToggleSidebar),
                        &theme,
                    ))
                }
            })
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.nav_title()),
            )
            .when(unread > 0, |this| {
                this.child(
                    div()
                        .text_size(px(12.))
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
            ));

        let body = if self.rows.is_empty() {
            empty_state(
                Icon::Inbox,
                format!("Nothing in {}", self.nav_title()),
                None,
                &theme,
            )
        } else {
            let view = cx.entity();
            div()
                .relative()
                .size_full()
                .child(
                    list(self.list_state.clone(), move |ix, window, cx| {
                        view.update(cx, |this, cx| this.render_row(ix, window, cx))
                    })
                    .size_full()
                    .pb(px(8.)),
                )
                .vertical_scrollbar(&self.list_state)
                .into_any_element()
        };

        div()
            .relative()
            .w(self.list_width)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.list_bg)
            .border_r_1()
            .border_color(theme.border)
            .child(header)
            .when(self.search_open, |this| {
                this.child(self.render_search_bar(&theme))
            })
            .child(div().flex_1().min_h_0().child(body))
            .child(pane_divider(PaneResize::List))
    }

    /// Seam for search: replace with the real query input.
    fn render_search_bar(&self, theme: &Theme) -> impl IntoElement {
        div()
            .mx(px(12.))
            .mb(px(8.))
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(8.))
            .bg(theme.chip_bg)
            .border_1()
            .border_color(theme.border)
            .text_size(px(13.))
            .text_color(theme.text_faint)
            .child(icon(Icon::Search, px(14.), theme.text_faint))
            .child("Search mail")
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
            .px(px(8.))
            .pt(px(10.))
            .child(
                div()
                    .id("archived-header")
                    .h(px(30.))
                    .pl(px(22.))
                    .pr(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(7.))
                    .text_size(px(12.))
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
                .first()
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
            .px(px(8.))
            .pt(px(2.))
            .child(
                div()
                    .id(("thread", thread_ix))
                    .relative()
                    .pl(px(22.))
                    .pr(px(12.))
                    .py(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .line_height(px(18.))
                    .rounded(px(9.))
                    .map(|this| {
                        if selected {
                            this.bg(theme.selected_bg)
                        } else {
                            this.hover(|s| s.bg(theme.hover_bg))
                        }
                    })
                    .when(unread, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(9.))
                                .top(px(16.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(theme.accent),
                        )
                    })
                    // Sender, icons, date.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.5))
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
                                        .text_size(px(11.5))
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
                                    .text_size(px(12.))
                                    .text_color(secondary)
                                    .child(list_date(&last.date)),
                            ),
                    )
                    // Subject and labels.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(if unread {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(primary)
                                    .child(thread.subject.clone()),
                            )
                            .children(chips),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(secondary)
                            .child(last.snippet.clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_thread(thread_ix, cx))),
            )
            .into_any_element()
    }
}

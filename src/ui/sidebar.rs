//! Account card, system folders and labels.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};

use super::{HEADER_HEIGHT, PaneResize, avatar, icon, icon_button, pane_divider, window_drag_area};
use crate::{
    actions::ToggleSidebar,
    app::{Nav, PigeonApp},
    assets::Icon,
    data::{Contact, Folder},
    theme::Theme,
};

const FOLDERS: [(Nav, &str, Icon); 7] = [
    (Nav::Folder(Folder::Inbox), "Inbox", Icon::Inbox),
    (Nav::Starred, "Starred", Icon::Star),
    (Nav::Folder(Folder::Sent), "Sent", Icon::Send),
    (Nav::Folder(Folder::Drafts), "Drafts", Icon::FilePen),
    (Nav::Folder(Folder::Archive), "Archive", Icon::Archive),
    (Nav::Folder(Folder::Spam), "Spam", Icon::OctagonAlert),
    (Nav::Folder(Folder::Trash), "Trash", Icon::Trash),
];

/// Leading visual of a nav row.
enum Marker {
    Icon(Icon),
    Dot(gpui::Hsla),
}

impl PigeonApp {
    /// The number shown next to a nav item: total drafts, unread elsewhere,
    /// nothing for folders where an unread count means little.
    fn nav_count(&self, nav: Nav) -> usize {
        let threads = self
            .data
            .threads
            .iter()
            .filter(|t| self.nav_contains(nav, t));
        match nav {
            Nav::Folder(Folder::Drafts) => threads.count(),
            Nav::Folder(Folder::Sent | Folder::Archive | Folder::Trash) | Nav::Starred => 0,
            _ => threads.filter(|t| t.unread).count(),
        }
    }

    pub(crate) fn render_sidebar(
        &self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let account = Contact {
            name: self.data.account.name.clone(),
            email: self.data.account.email.clone(),
        };

        let folders = FOLDERS
            .iter()
            .map(|&(nav, name, glyph)| {
                self.nav_item(nav, name.into(), Marker::Icon(glyph), &theme, cx)
            })
            .collect::<Vec<_>>();
        let labels = self
            .data
            .labels
            .iter()
            .enumerate()
            .map(|(ix, label)| {
                self.nav_item(
                    Nav::Label(ix),
                    label.name.clone().into(),
                    Marker::Dot(label.color),
                    &theme,
                    cx,
                )
            })
            .collect::<Vec<_>>();

        div()
            .relative()
            .w(self.sidebar_width)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.sidebar_bg)
            .border_r_1()
            .border_color(theme.border)
            .child(
                window_drag_area(div().id("sidebar-header"))
                    .h(HEADER_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(px(10.))
                    .child(icon_button(
                        "toggle-sidebar",
                        Icon::PanelLeft,
                        "Hide sidebar",
                        Box::new(ToggleSidebar),
                        &theme,
                    )),
            )
            .child(
                // Account card.
                div()
                    .mx(px(10.))
                    .mb(px(14.))
                    .p(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(10.))
                    .bg(theme.card_bg)
                    .border_1()
                    .border_color(theme.border)
                    .child(avatar(
                        account.initials(),
                        theme.avatar_color(&account.email),
                        px(32.),
                        &theme,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(account.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(theme.text_muted)
                                    .child(account.email.clone()),
                            ),
                    ),
            )
            .child(
                div()
                    .id("sidebar-nav")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .children(folders)
                    .child(
                        div()
                            .pt(px(18.))
                            .pb(px(6.))
                            .px(px(10.))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_faint)
                            .child("Labels"),
                    )
                    .children(labels),
            )
            .child(pane_divider(PaneResize::Sidebar))
    }

    fn nav_item(
        &self,
        nav: Nav,
        name: gpui::SharedString,
        marker: Marker,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.nav == nav;
        let count = self.nav_count(nav);
        let id = match nav {
            Nav::Folder(folder) => ("nav-folder", folder as usize),
            Nav::Starred => ("nav-starred", 0),
            Nav::Label(ix) => ("nav-label", ix),
        };

        div()
            .id(id)
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(7.))
            .text_size(px(13.))
            .map(|this| {
                if active {
                    this.bg(theme.nav_active_bg)
                        .text_color(theme.text)
                        .font_weight(FontWeight::MEDIUM)
                } else {
                    this.text_color(theme.text_muted)
                        .hover(|s| s.bg(theme.hover_bg))
                }
            })
            .child(match marker {
                Marker::Icon(glyph) => icon(
                    glyph,
                    px(16.),
                    if active {
                        theme.icon_active
                    } else {
                        theme.icon
                    },
                )
                .into_any_element(),
                Marker::Dot(color) => div()
                    .size(px(16.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(8.)).rounded_full().bg(color))
                    .into_any_element(),
            })
            .child(div().flex_1().truncate().child(name))
            .when(count > 0, |this| {
                this.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(if active {
                            theme.text_muted
                        } else {
                            theme.text_faint
                        })
                        .child(count.to_string()),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| this.set_nav(nav, cx)))
            .into_any_element()
    }
}

//! Account card, system folders and labels.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, component::tooltip::Tooltip, div,
    prelude::FluentBuilder as _, px,
};

use super::{HEADER_HEIGHT, avatar, icon, icon_button, window_drag_area};
use crate::ui::style::*;
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
        width: gpui::Pixels,
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
            .w(width)
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
                    .px(SPACE_MD)
                    .child(icon_button(
                        "toggle-sidebar",
                        Icon::PanelLeft,
                        "Hide sidebar",
                        Box::new(ToggleSidebar),
                        &theme,
                    )),
            )
            .child(
                // The account: who this mail is for. Display only; the
                // address is in its tooltip.
                div()
                    .id("account")
                    .role(gpui::Role::Group)
                    .aria_label(format!("{}, {}", account.name, account.email))
                    .mx(SPACE_MD)
                    .mb(SPACE_SM)
                    .h(HIT_TARGET)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(avatar(
                        account.initials(),
                        theme.avatar_color(&account.email),
                        px(18.),
                        &theme,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(TEXT_BASE)
                            .font_weight(FontWeight::MEDIUM)
                            .child(account.name.clone()),
                    )
                    .tooltip({
                        let email = account.email.clone();
                        move |window, cx| Tooltip::new(email.clone()).build(window, cx)
                    }),
            )
            .child(
                div()
                    .id("sidebar-nav")
                    .role(gpui::Role::Navigation)
                    .aria_label("Mailboxes")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(SPACE_MD)
                    .pb(SPACE_MD)
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .children(folders)
                    .child(
                        div()
                            .pt(SPACE_LG)
                            .pb(SPACE_SM)
                            .px(SPACE_MD)
                            .text_size(TEXT_XS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_faint)
                            .child("Labels"),
                    )
                    .children(labels),
            )
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

        focusable_control(div().id(id), theme)
            .role(gpui::Role::Link)
            .aria_label(name.clone())
            .aria_selected(active)
            .h(px(30.))
            .px(SPACE_MD)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .text_size(TEXT_BASE)
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
                // Unread counts (and Drafts' total); the Inbox's stands out.
                let inbox = nav == Nav::Folder(Folder::Inbox);
                this.child(
                    div()
                        .text_size(TEXT_SM)
                        .font_features(tabular_figures())
                        .font_weight(if inbox {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if inbox { theme.text } else { theme.text_muted })
                        .child(count.to_string()),
                )
            })
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    this.set_nav(nav, cx);
                    // A click hands the keyboard back to the list; Enter on a
                    // focused row keeps focus on the row.
                    if !event.is_keyboard() {
                        window.focus(&this.focus_handle, cx);
                    }
                }),
            )
            .into_any_element()
    }
}

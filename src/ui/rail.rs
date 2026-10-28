//! The activity rail on the far left: Mail and Calendar, and Settings at
//! the bottom.

use gpui::{
    Action, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, div, prelude::FluentBuilder as _, px,
};

use super::{HEADER_HEIGHT, icon, window_drag_area};
use crate::ui::style::*;
use crate::{
    actions::{GoToCalendar, GoToMail, OpenSettings},
    app::{Mode, RAIL_WIDTH},
    assets::Icon,
    theme::Theme,
};

/// `mode`: the app showing, none while Settings is. `unread`: unread
/// threads in the Inbox, shown as a dot on Mail so new mail is visible from
/// Calendar too.
pub fn render(mode: Option<Mode>, unread: usize, theme: &Theme) -> impl IntoElement {
    let item = |id: &'static str,
                glyph: Icon,
                item_mode: Option<Mode>,
                tooltip: SharedString,
                action: Box<dyn Action>| {
        let active = mode == item_mode;
        let tooltip_action = action.boxed_clone();
        let label = tooltip.clone();
        focusable_control(div().id(id), theme)
            .role(gpui::Role::Tab)
            .aria_label(label)
            .aria_selected(active)
            .relative()
            .size(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_MD)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            .tooltip(move |window, cx| {
                super::tooltip(tooltip.clone())
                    .action(tooltip_action.as_ref(), None)
                    .build(window, cx)
            })
            .map(|this| {
                if active {
                    this.bg(theme.nav_active_bg)
                } else {
                    this.hover(|s| s.bg(theme.hover_bg))
                }
            })
            .child(icon(
                glyph,
                ICON_RAIL,
                if active {
                    theme.icon_active
                } else {
                    theme.icon
                },
            ))
    };

    window_drag_area(
        div()
            .id("rail")
            .role(gpui::Role::TabList)
            .aria_label("Apps")
            .w(RAIL_WIDTH)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(SPACE_SM)
            .pt(HEADER_HEIGHT)
            .bg(theme.rail_bg)
            .border_r_1()
            .border_color(theme.border),
    )
    .child(
        item(
            "rail-mail",
            Icon::Mail,
            Some(Mode::Mail),
            match unread {
                0 => "Mail".into(),
                n => format!("Mail · {n} unread").into(),
            },
            Box::new(GoToMail),
        )
        .when(unread > 0, |this| {
            // Off the envelope's top-right corner.
            this.child(
                div()
                    .absolute()
                    .top(px(5.))
                    .right(px(4.))
                    .size(px(8.))
                    .rounded_full()
                    .bg(theme.accent),
            )
        }),
    )
    .child(item(
        "rail-calendar",
        Icon::Calendar,
        Some(Mode::Calendar),
        "Calendar".into(),
        Box::new(GoToCalendar),
    ))
    .child(div().flex_1())
    .child(div().pb(SPACE_LG).child(item(
        "rail-settings",
        Icon::Settings,
        None,
        "Settings".into(),
        Box::new(OpenSettings),
    )))
}

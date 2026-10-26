//! The activity rail on the far left: Mail and Calendar.

use gpui::{
    Action, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement as _, Styled, component::tooltip::Tooltip, div,
    prelude::FluentBuilder as _, px,
};

use super::{HEADER_HEIGHT, icon, window_drag_area};
use crate::ui::style::*;
use crate::{
    actions::{GoToCalendar, GoToMail},
    app::{Mode, RAIL_WIDTH},
    assets::Icon,
    theme::Theme,
};

pub fn render(mode: Mode, theme: &Theme) -> impl IntoElement {
    let item = |id: &'static str,
                glyph: Icon,
                item_mode: Mode,
                tooltip: &'static str,
                action: Box<dyn Action>| {
        let active = mode == item_mode;
        let tooltip_action = action.boxed_clone();
        focusable_control(div().id(id), theme)
            .role(gpui::Role::Tab)
            .aria_label(tooltip)
            .aria_selected(active)
            .size(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_MD)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            .tooltip(move |window, cx| {
                Tooltip::new(tooltip)
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
                px(18.),
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
    .child(item(
        "rail-mail",
        Icon::Mail,
        Mode::Mail,
        "Mail",
        Box::new(GoToMail),
    ))
    .child(item(
        "rail-calendar",
        Icon::Calendar,
        Mode::Calendar,
        "Calendar",
        Box::new(GoToCalendar),
    ))
}

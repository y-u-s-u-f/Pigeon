//! The activity rail on the far left: Mail, plus Calendar as a placeholder.

use gpui::{
    InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, div,
    prelude::FluentBuilder as _, px,
};

use super::{HEADER_HEIGHT, icon, window_drag_area};
use crate::{app::RAIL_WIDTH, assets::Icon, theme::Theme};

pub fn render(theme: &Theme) -> impl IntoElement {
    let item = |id: &'static str, glyph: Icon, active: bool| {
        div()
            .id(id)
            .size(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
            .w(RAIL_WIDTH)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .pt(HEADER_HEIGHT)
            .bg(theme.rail_bg)
            .border_r_1()
            .border_color(theme.border),
    )
    .child(item("rail-mail", Icon::Mail, true))
    .child(item("rail-calendar", Icon::Calendar, false))
}

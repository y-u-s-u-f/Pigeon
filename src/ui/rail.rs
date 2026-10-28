//! The activity rail on the far left: Mail and Calendar, the sync status
//! and Settings at the bottom.

use gpui::{
    Action, AnyElement, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    SharedString, StatefulInteractiveElement as _, Styled, div, prelude::FluentBuilder as _, px,
};

use super::{HEADER_HEIGHT, icon, window_drag_area};
use crate::ui::style::*;
use crate::{
    actions::{GoToCalendar, GoToMail, OpenSettings, SignIn},
    app::{Mode, RAIL_WIDTH},
    assets::Icon,
    backend::SyncStatus,
    theme::Theme,
};

/// A rail button, square.
const ITEM: Pixels = px(36.);
/// The unread dot on Mail, off the envelope's top-right corner.
const DOT: Pixels = px(8.);
const DOT_TOP: Pixels = px(5.);
const DOT_RIGHT: Pixels = px(4.);

/// `mode`: the app showing, none while Settings is. `unread`: unread
/// threads in the Inbox, shown as a dot on Mail so new mail is visible from
/// Calendar too.
pub fn render(mode: Option<Mode>, unread: usize, theme: &Theme) -> impl IntoElement {
    render_with_status(mode, unread, None, theme)
}

/// `render`, with the sync status at the bottom (MA-068): in Calendar or
/// with the sidebar hidden it's the only place it shows.
pub fn render_with_status(
    mode: Option<Mode>,
    unread: usize,
    status: Option<&SyncStatus>,
    theme: &Theme,
) -> impl IntoElement {
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
            .size(ITEM)
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
                        .active(|s| s.bg(theme.selected_bg))
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
                n => format!("Mail, {} unread", super::status::thousands(n)).into(),
            },
            Box::new(GoToMail),
        )
        .when(unread > 0, |this| {
            this.child(
                div()
                    .absolute()
                    .top(DOT_TOP)
                    .right(DOT_RIGHT)
                    .size(DOT)
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
    .children(status.and_then(|status| status_glyph(status, theme)))
    .child(div().pb(SPACE_LG).child(item(
        "rail-settings",
        Icon::Settings,
        None,
        "Settings".into(),
        Box::new(OpenSettings),
    )))
}

/// MA-068: syncing, offline (with what's waiting) or signed out, as one
/// glyph with the status line's words in its tooltip. Nothing while all is
/// well. "Sign in again" is a button.
fn status_glyph(status: &SyncStatus, theme: &Theme) -> Option<AnyElement> {
    let (glyph, color) = match status {
        SyncStatus::Syncing { .. } | SyncStatus::Waiting => (Icon::RefreshCw, theme.text_faint),
        SyncStatus::Offline { .. } => (Icon::OctagonAlert, theme.text_warning),
        SyncStatus::AuthExpired => (Icon::LogIn, theme.text_danger),
        SyncStatus::Demo | SyncStatus::SignedOut | SyncStatus::Synced { .. } => return None,
    };
    let text: SharedString = super::status::label(status, 0).into();
    let tip = text.clone();
    let el = div()
        .id("rail-status")
        .size(ITEM)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .child(icon(glyph, ICON_LG, color))
        .tooltip(move |window, cx| super::tooltip(tip.clone()).build(window, cx));
    Some(if *status == SyncStatus::AuthExpired {
        focusable_control(el, theme)
            .role(gpui::Role::Button)
            .aria_label(text)
            .hover(|s| s.bg(theme.hover_bg))
            .active(|s| s.bg(theme.selected_bg))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, window, cx| window.dispatch_action(Box::new(SignIn), cx))
            .into_any_element()
    } else {
        el.role(gpui::Role::Status)
            .aria_label(text)
            .into_any_element()
    })
}

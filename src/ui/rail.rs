//! The activity rail on the far left, under the top bar: Mail and Calendar
//! at the top, Settings at the bottom. It's the bar's material with no line
//! between them, so the two make one L-shaped frame around the panes. A
//! pane of its own (`app/panes.rs`), so its hovers redraw it alone.

use gpui::{
    Action, AnyElement, App, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, SharedString, StatefulInteractiveElement as _, Styled, Window, base::spring, div,
    prelude::FluentBuilder as _, px,
};

use super::motion::{SPRING_CONTROL, SPRING_EXIT, fade};
use super::style::*;
use super::{icon, status, tooltip, window_drag_area};
use crate::{
    actions::{GoToCalendar, GoToMail, OpenSettings},
    app::{Mode, Nav, PigeonApp, RAIL_WIDTH},
    assets::Icon,
    backend::Source,
    data::Folder,
    theme::Theme,
};

/// The unread dot on Mail, over the envelope's top-right corner.
const DOT: Pixels = px(7.);

impl PigeonApp {
    pub(crate) fn render_rail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let settings_open = self.settings_page.open;
        let signed_out = self.backend.read(cx).source() == Source::SignedOut;
        // Neither app is chosen while Settings shows.
        let mode = (!settings_open && !signed_out).then_some(self.mode);
        let unread = self.counts.unread(Nav::Folder(Folder::Inbox));
        let mail_label: SharedString = match unread {
            0 => "Mail".into(),
            n => format!("Mail, {} unread", status::thousands(n)).into(),
        };
        let dot = unread_dot(unread > 0, &theme, window, cx);

        let apps = div()
            .id("rail-apps")
            .role(gpui::Role::TabList)
            .aria_label("Apps")
            .flex()
            .flex_col()
            .items_center()
            .gap(SPACE_XS)
            .child(
                rail_item(
                    "rail-mail",
                    Icon::Mail,
                    mail_label,
                    "Mail",
                    Box::new(GoToMail),
                    mode == Some(Mode::Mail),
                    gpui::Role::Tab,
                    &theme,
                )
                .children(dot),
            )
            .child(rail_item(
                "rail-calendar",
                Icon::Calendar,
                "Calendar".into(),
                "Calendar",
                Box::new(GoToCalendar),
                mode == Some(Mode::Calendar),
                gpui::Role::Tab,
                &theme,
            ));

        window_drag_area(div().id("rail"))
            .role(gpui::Role::Navigation)
            .aria_label("Rail")
            .w(RAIL_WIDTH)
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .pt(SPACE_SM)
            .pb(SPACE_MD)
            .bg(theme.bar_bg)
            .border_r_1()
            .border_color(theme.border)
            .when(!signed_out, |this| this.child(apps))
            .child(div().flex_1())
            .child(
                rail_item(
                    "rail-settings",
                    Icon::Settings,
                    "Settings".into(),
                    "Settings",
                    Box::new(OpenSettings),
                    settings_open,
                    gpui::Role::Button,
                    &theme,
                )
                .aria_toggled(if settings_open {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                }),
            )
            .into_any_element()
    }
}

/// A rail button: square, highlighted while its app shows.
#[allow(clippy::too_many_arguments)]
fn rail_item(
    id: &'static str,
    glyph: Icon,
    label: SharedString,
    tip: &'static str,
    action: Box<dyn Action>,
    active: bool,
    role: gpui::Role,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let tip_action = action.boxed_clone();
    focusable_control(div().id(id), theme)
        .role(role)
        .aria_label(label)
        .when(role == gpui::Role::Tab, |this| this.aria_selected(active))
        .when_some(super::shortcut_text(action.as_ref()), |this, keys| {
            this.aria_keyshortcuts(keys)
        })
        .relative()
        .size(HIT_TARGET + SPACE_SM)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
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
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .tooltip(move |window, cx| {
            tooltip(tip)
                .action(tip_action.as_ref(), None)
                .build(window, cx)
        })
}

/// The unread dot (MD-029): it pops in when the Inbox gets unread mail and
/// fades out when it has none, so new mail shows from Calendar and Settings
/// too. Not at launch: the spring starts where it's going.
fn unread_dot(shown: bool, theme: &Theme, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let (size, opacity) = if cx.reduce_motion() {
        let target = if shown { 1. } else { 0. };
        (DOT, fade("unread-dot", target, target, window, cx))
    } else {
        let (target, policy) = if shown {
            (DOT, SPRING_CONTROL)
        } else {
            (px(0.), SPRING_EXIT)
        };
        let value = spring("unread-dot", target, policy, window, cx);
        if shown {
            (value, 1.)
        } else {
            (DOT, value / DOT)
        }
    };
    (size > px(0.) && opacity > 0.).then(|| {
        // Grows from its center.
        let inset = (DOT - size) / 2.;
        div()
            .absolute()
            .top(SPACE_SM - SPACE_2XS + inset)
            .right(SPACE_SM - SPACE_2XS + inset)
            .size(size)
            .rounded_full()
            .bg(theme.accent)
            .opacity(opacity.clamp(0., 1.))
            .into_any_element()
    })
}

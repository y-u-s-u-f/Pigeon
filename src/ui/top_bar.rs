//! The window's top bar, across Mail, Calendar and Settings: the traffic
//! lights, the sidebar's toggle and the Mail / Calendar switch on the left,
//! the command field in the middle, and Refresh, the app's main action,
//! Settings and the account on the right. Empty space moves the window.
//! It's a pane of its own (`app/panes.rs`), so its hovers and its spinner
//! redraw it alone.

use gpui::{
    Action, AnyElement, App, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, SharedString, StatefulInteractiveElement as _, Styled, Toggled, Window,
    base::spring, div, point, prelude::FluentBuilder as _, px,
};

use super::context_menu::{ContextMenu, Item};
use super::controls::{ButtonKind, Segment, push_button_frame, segmented_with};
use super::motion::{SPRING_CONTROL, SPRING_EXIT, fade, spinner};
use super::style::*;
use super::{
    account_avatar, icon, icon_button, keycap, shortcut_text, status, tooltip, window_drag_area,
};
use crate::{
    actions::{
        Compose, GoToCalendar, GoToMail, NewEvent, OpenSettings, OpenSettingsSection, SignIn,
        SignOut, SyncNow, ToggleCommandPalette, ToggleSidebar,
    },
    app::{Mode, Nav, PigeonApp, shell},
    assets::Icon,
    backend::{Source, SyncStatus},
    data::{Contact, Folder},
    settings::Section,
    theme::Theme,
};

/// The command field, at most; it narrows with the window.
const SEARCH_WIDTH: gpui::Rems = type_size(360.);
/// The unread dot on Mail.
const DOT: Pixels = px(7.);

impl PigeonApp {
    pub(crate) fn render_top_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let settings_open = self.settings_page.open;
        let (source, status, can_sign_out, photo) = {
            let b = self.backend.read(cx);
            (
                b.source(),
                b.status().clone(),
                b.source() == Source::Google || b.demo_by_choice(),
                b.profile_photo().map(|p| p.to_path_buf()),
            )
        };
        let signed_out = source == Source::SignedOut;
        // Neither app is chosen while Settings shows.
        let mode = (!settings_open && !signed_out).then_some(self.mode);
        let unread = self.counts.unread(Nav::Folder(Folder::Inbox));
        let sidebar_shown = self.pane_layout(window.viewport_size().width).sidebar > px(0.);
        let lights = shell::traffic_lights_inset(window.is_fullscreen());

        let leading = div()
            .flex_1()
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .pl(lights.max(SPACE_MD))
            .map(|this| {
                // Kept in place in Calendar and Settings, so the switch
                // doesn't move.
                if mode == Some(Mode::Mail) {
                    this.child(
                        icon_button(
                            "toggle-sidebar",
                            Icon::PanelLeft,
                            if sidebar_shown {
                                "Hide sidebar"
                            } else {
                                "Show sidebar"
                            },
                            Box::new(ToggleSidebar),
                            &theme,
                        )
                        .aria_expanded(sidebar_shown),
                    )
                } else {
                    this.child(div().flex_none().size(HIT_TARGET))
                }
            })
            .when(!signed_out, |this| {
                this.child(app_switch(mode, unread, &theme, window, cx))
            });

        let center = div()
            .flex_none()
            .flex_shrink(1.)
            .min_w_0()
            .w(SEARCH_WIDTH)
            .flex()
            .justify_center()
            .when(!signed_out && !settings_open, |this| {
                this.child(command_field(&theme))
            });

        let primary = mode.map(|mode| match mode {
            Mode::Mail => primary_button(
                "top-bar-compose",
                Icon::SquarePen,
                "Compose",
                "New message",
                Box::new(Compose),
                &theme,
            ),
            Mode::Calendar => primary_button(
                "top-bar-new-event",
                Icon::Plus,
                "New Event",
                "New event",
                Box::new(NewEvent),
                &theme,
            ),
        });

        let account = Contact {
            name: self.data.account.name.clone(),
            email: self.data.account.email.clone(),
        };
        let trailing = div()
            .flex_1()
            .flex()
            .items_center()
            .justify_end()
            .gap(SPACE_XS)
            .pr(SPACE_MD)
            .children(status_glyph(&status, &theme))
            .when(source == Source::Demo, |this| this.child(demo_chip(&theme)))
            .when(!signed_out, |this| {
                let spinning = self.refresh_spin.is_some()
                    || matches!(status, SyncStatus::Syncing { .. } | SyncStatus::Waiting);
                this.child(refresh_button(
                    spinning,
                    status.clone(),
                    self.demo_synced_at,
                    &theme,
                ))
            })
            .children(primary.map(|button| div().pl(SPACE_XS).pr(SPACE_XS).child(button)))
            .child(settings_button(settings_open, &theme))
            .child(account_button(
                &account,
                photo,
                source,
                can_sign_out,
                &theme,
            ));

        window_drag_area(div().id("top-bar"))
            .role(gpui::Role::Toolbar)
            .aria_label("Top bar")
            .size_full()
            .flex()
            .items_center()
            .bg(theme.bar_bg)
            .border_b_1()
            .border_color(theme.border)
            .child(leading)
            .child(center)
            .child(trailing)
            .into_any_element()
    }
}

/// Mail and Calendar, with the unread dot on Mail (MD-029) so new mail
/// shows from Calendar and Settings too. ←/→ switch while it has focus.
fn app_switch(
    mode: Option<Mode>,
    unread: usize,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let mail_label: SharedString = match unread {
        0 => "Mail".into(),
        n => format!("Mail, {} unread", status::thousands(n)).into(),
    };
    let segments = vec![
        Segment::new("Mail")
            .icon(Icon::Mail)
            .label(mail_label)
            .badge(unread_dot(unread > 0, theme, window, cx))
            .tooltip("Show mail", Box::new(GoToMail)),
        Segment::new("Calendar")
            .icon(Icon::Calendar)
            .tooltip("Show calendar", Box::new(GoToCalendar)),
    ];
    let selected = mode.map(|mode| match mode {
        Mode::Mail => 0,
        Mode::Calendar => 1,
    });
    let go = |ix: usize| -> Box<dyn Action> {
        if ix == 0 {
            Box::new(GoToMail)
        } else {
            Box::new(GoToCalendar)
        }
    };
    segmented_with(
        "app-switch",
        "App",
        segments,
        selected,
        theme,
        window,
        cx,
        move |ix, window, cx| window.dispatch_action(go(ix), cx),
    )
    .on_key_down(move |event, window, cx| {
        let ix = match event.keystroke.key.as_str() {
            "left" => 0,
            "right" => 1,
            _ => return,
        };
        cx.stop_propagation();
        window.dispatch_action(go(ix), cx);
    })
}

/// The unread dot: it pops in when the Inbox gets unread mail and fades out
/// when it has none. Not at launch: the spring starts where it's going.
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
        // Grows from its center, over the envelope's top-right corner.
        let inset = (DOT - size) / 2.;
        div()
            .absolute()
            .top(-SPACE_XS + inset)
            .right(-SPACE_XS + inset)
            .size(size)
            .rounded_full()
            .bg(theme.accent)
            .opacity(opacity.clamp(0., 1.))
            .into_any_element()
    })
}

/// Looks like a search field; opens ⌘K, which finds commands, views and
/// conversations. Mail's own search stays in the list's header (⌘F).
fn command_field(theme: &Theme) -> impl IntoElement {
    let keys = shortcut_text(&ToggleCommandPalette);
    focusable_control(div().id("top-bar-command"), theme)
        .role(gpui::Role::Button)
        .aria_label("Search or jump to")
        .when_some(keys.clone(), |this, keys| this.aria_keyshortcuts(keys))
        .w_full()
        .min_w_0()
        .h(CONTROL_MD)
        .pl(SPACE_SM)
        .pr(SPACE_XS)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .bg(theme.search_bg)
        .border_1()
        .border_color(theme.border)
        .text_size(TEXT_SM)
        .text_color(theme.text_faint)
        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text_muted))
        .active(|s| s.bg(theme.selected_bg))
        .child(icon(Icon::Search, ICON_MD, theme.text_faint))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child("Search or jump to…"),
        )
        .children(keys.map(|keys| keycap(keys, theme)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleCommandPalette), cx))
        .tooltip(|window, cx| {
            tooltip("Search and run commands")
                .action(&ToggleCommandPalette, None)
                .build(window, cx)
        })
}

/// "Refresh · Synced 4 min ago", worked out as the tooltip shows.
fn refresh_tip(status: &SyncStatus, demo_synced_at: Option<i64>, now_ms: i64) -> String {
    let state = match (status, demo_synced_at) {
        (SyncStatus::Demo, Some(at)) => format!("Demo data · Synced {}", status::ago(at, now_ms)),
        _ => status::label(status, now_ms),
    };
    format!("Refresh · {state}")
}

/// Refresh (`SyncNow`): turns while a sync runs, and for a moment after a
/// click even when there was nothing to fetch.
fn refresh_button(
    spinning: bool,
    status: SyncStatus,
    demo_synced_at: Option<i64>,
    theme: &Theme,
) -> impl IntoElement {
    let glyph = icon(Icon::RefreshCw, ICON_TOOLBAR, theme.icon);
    let state: SharedString = status::label(&status, status::now_ms()).into();
    focusable_control(div().id("top-bar-refresh"), theme)
        .role(gpui::Role::Button)
        .aria_label("Refresh")
        .aria_description(state)
        .when_some(shortcut_text(&SyncNow), |this, keys| {
            this.aria_keyshortcuts(keys)
        })
        .size(HIT_TARGET)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .map(|this| {
            if spinning {
                this.child(spinner("refresh-spinner", glyph))
            } else {
                this.child(glyph)
            }
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, window, cx| window.dispatch_action(Box::new(SyncNow), cx))
        .tooltip(move |window, cx| {
            tooltip(refresh_tip(&status, demo_synced_at, status::now_ms()))
                .action(&SyncNow, None)
                .build(window, cx)
        })
}

/// MA-068: offline (with what's waiting) or signed out, as one glyph with
/// the status line's words in its tooltip. Nothing while all is well;
/// syncing turns Refresh instead. "Sign in again" is a button.
fn status_glyph(status: &SyncStatus, theme: &Theme) -> Option<AnyElement> {
    let (glyph, color) = match status {
        SyncStatus::Offline { .. } => (Icon::OctagonAlert, theme.text_warning),
        SyncStatus::AuthExpired => (Icon::LogIn, theme.text_danger),
        _ => return None,
    };
    let text: SharedString = status::label(status, 0).into();
    let tip = text.clone();
    let el = div()
        .id("top-bar-status")
        .size(HIT_TARGET)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .child(icon(glyph, ICON_TOOLBAR, color))
        .tooltip(move |window, cx| tooltip(tip.clone()).build(window, cx));
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

/// On demo data: says so, and signs in.
fn demo_chip(theme: &Theme) -> impl IntoElement {
    focusable_control(div().id("demo-sign-in"), theme)
        .role(gpui::Role::Button)
        .aria_label("Demo data. Sign in…")
        .flex_none()
        .h(CONTROL_SM)
        .px(SPACE_SM)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .text_size(TEXT_XS)
        .text_color(theme.text_faint)
        .bg(theme.chip_bg)
        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text_muted))
        .active(|s| s.bg(theme.selected_bg))
        .child("Demo Data")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, window, cx| window.dispatch_action(Box::new(SignIn), cx))
        .tooltip(|window, cx| {
            tooltip("Sign in with Google")
                .action(&SignIn, None)
                .build(window, cx)
        })
}

/// Compose in Mail, New Event in Calendar.
fn primary_button(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    tip: &'static str,
    action: Box<dyn Action>,
    theme: &Theme,
) -> impl IntoElement {
    let tip_action = action.boxed_clone();
    push_button_frame(id, label, ButtonKind::Primary, theme)
        .when_some(shortcut_text(action.as_ref()), |this, keys| {
            this.aria_keyshortcuts(keys)
        })
        .pl(SPACE_SM)
        .gap(SPACE_XS)
        .child(icon(glyph, ICON_MD, theme.accent_text))
        .child(label)
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .tooltip(move |window, cx| {
            tooltip(tip)
                .action(tip_action.as_ref(), None)
                .build(window, cx)
        })
}

/// Settings, highlighted while it shows; again, back to the app.
fn settings_button(open: bool, theme: &Theme) -> impl IntoElement {
    icon_button(
        "top-bar-settings",
        Icon::Settings,
        "Settings",
        Box::new(OpenSettings),
        theme,
    )
    .aria_toggled(if open { Toggled::True } else { Toggled::False })
    .when(open, |this| this.bg(theme.nav_active_bg))
}

/// The account: who this mail is for. Opens a menu with Account Settings
/// (ME-029) and signing in or out; the address is in its tooltip.
fn account_button(
    account: &Contact,
    photo: Option<std::path::PathBuf>,
    source: Source,
    can_sign_out: bool,
    theme: &Theme,
) -> impl IntoElement {
    let label = format!("{}, {}", account.name, account.email);
    let tip: SharedString = match source {
        Source::Demo => format!("{} · Demo data", account.email).into(),
        Source::SignedOut => "Not signed in".into(),
        Source::Google => account.email.clone().into(),
    };
    let email: SharedString = account.email.clone().into();
    focusable_control(div().id("account"), theme)
        .role(gpui::Role::Button)
        .aria_label(label)
        .aria_description("Opens the account menu")
        .flex_none()
        .min_w(HIT_TARGET)
        .min_h(HIT_TARGET)
        .p(SPACE_2XS)
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(account_avatar(
            photo.as_deref(),
            account.initials(),
            theme.avatar_color(&account.email),
            AVATAR_XS,
            theme,
        ))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_: &ClickEvent, window, cx| {
            account_menu(email.clone(), source, can_sign_out).show(menu_anchor(window), window, cx)
        })
        .tooltip(move |window, cx| tooltip(tip.clone()).build(window, cx))
}

/// Under the bar's right end: the menu lines up with the window's edge.
fn menu_anchor(window: &Window) -> gpui::Point<Pixels> {
    point(
        window.viewport_size().width - SPACE_SM,
        super::top_bar_height(window),
    )
}

fn account_menu(email: SharedString, source: Source, can_sign_out: bool) -> ContextMenu {
    let menu = ContextMenu::new()
        .label("Account")
        .header(email)
        .item(
            Item::new(
                "Account Settings…",
                Box::new(OpenSettingsSection(Section::Accounts)),
            )
            .icon(Icon::User),
        )
        .item(Item::new("Settings…", Box::new(OpenSettings)).icon(Icon::Settings))
        .separator();
    // On demo data opened with PIGEON_DEMO, it says how to sign in instead.
    let menu = if source != Source::Google {
        menu.item(Item::new("Sign In…", Box::new(SignIn)).icon(Icon::LogIn))
    } else {
        menu
    };
    if can_sign_out {
        menu.item(Item::new("Sign Out", Box::new(SignOut)).icon(Icon::LogOut))
    } else {
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_tip_says_when_it_last_synced() {
        let now = 10 * 60_000;
        assert_eq!(
            refresh_tip(&SyncStatus::Synced { at: now }, None, now),
            "Refresh · Synced just now"
        );
        // Demo data: after a refresh, when that was.
        assert_eq!(
            refresh_tip(&SyncStatus::Demo, None, now),
            "Refresh · Demo data"
        );
        assert_eq!(
            refresh_tip(&SyncStatus::Demo, Some(now - 4 * 60_000), now),
            "Refresh · Demo data · Synced 4 min ago"
        );
    }
}

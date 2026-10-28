//! The sign-in screen (ACC-3): shown while nobody is signed in. Return
//! signs in with Google in the browser, D tries the demo data, Esc cancels a
//! sign-in that's waiting for the browser; the keys are bound in the
//! `Onboarding` context (MC-089).
//!
//! It shows the app icon, says what Pigeon asks Google for and where it
//! goes (MB-057), offers Open Again while waiting, and lets people choose
//! the menu bar icon before it appears (MB-Q1).

use std::{path::PathBuf, sync::OnceLock};

use gpui::{
    AnyElement, App, Context, Div, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    accesskit::Live, actions, div, img, prelude::FluentBuilder as _, px,
};

use super::{icon, keycap_on_accent, keys_caps, live_text, style::*, window_drag_area};
use crate::{
    actions::{CancelSignIn, ONBOARDING, SignIn, UseDemoData},
    app::{PigeonApp, menu_bar},
    assets::Icon,
    backend::SignInState,
    settings,
    theme::Theme,
};

// Return and D on the screen. Their own actions, so the menu items for
// SignIn and UseDemoData carry no bare key (W2-R15).
actions!(onboarding, [Confirm, TryDemo]);

/// The wide buttons, and the error above them.
const BUTTON_WIDTH: Pixels = px(260.);
const ERROR_WIDTH: Pixels = px(360.);
/// The app icon, as macOS shows it in Finder's large icons (MB-008).
const APP_ICON: Pixels = px(64.);

/// What Pigeon asks for, as Settings › Accounts › Access says it (MB-057).
const ACCESS_NOTE: &str = "Opens Google in your browser. Pigeon asks for your mail, calendar and contacts; they stay on this Mac and go only to Google.";

/// The bundle's `AppIcon.png` (scripts/bundle.sh); none under `cargo run`.
fn app_icon_path() -> Option<&'static PathBuf> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let path = exe.parent()?.parent()?.join("Resources/AppIcon.png");
        path.is_file().then_some(path)
    })
    .as_ref()
}

impl PigeonApp {
    pub(crate) fn render_onboarding(
        &self,
        state: &SignInState,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let waiting = *state == SignInState::Waiting;
        let error = match state {
            SignInState::Failed(message) => Some(message.clone()),
            _ => None,
        };

        let body = if waiting {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(SPACE_LG)
                // MA-066: turning while the browser has it.
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child(super::motion::spinner(
                            "sign-in-spinner",
                            icon(Icon::RefreshCw, ICON_MD, theme.text_faint),
                        ))
                        .child(
                            live_text("sign-in-waiting", "Waiting for your browser…", Live::Polite)
                                .text_size(TEXT_MD)
                                .text_color(theme.text_muted),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_SM)
                        .child(button(
                            "cancel-sign-in",
                            "Cancel",
                            keys_caps("escape", &theme),
                            false,
                            &theme,
                            |_, window, cx| window.dispatch_action(Box::new(CancelSignIn), cx),
                        ))
                        .child({
                            // A closed tab: a fresh sign-in in a new one.
                            let backend = self.backend.clone();
                            button("open-again", "Open Again", div(), false, &theme, {
                                move |_, _, cx| {
                                    backend.update(cx, |b, cx| {
                                        b.cancel_sign_in(cx);
                                        b.sign_in(cx);
                                    })
                                }
                            })
                        }),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(SPACE_SM)
                // MD-042: the error fades in.
                .children(error.map(|message| {
                    super::motion::enter(
                        "sign-in-error-in",
                        super::motion::Rise::None,
                        div()
                            .id("sign-in-error")
                            .role(gpui::Role::Alert)
                            .aria_label(SharedString::from(message.clone()))
                            .max_w(ERROR_WIDTH)
                            .mb(SPACE_SM)
                            .text_center()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_danger)
                            .child(message),
                    )
                }))
                .child(button(
                    "sign-in",
                    "Sign In with Google",
                    keycap_on_accent("↩", &theme),
                    true,
                    &theme,
                    |_, window, cx| window.dispatch_action(Box::new(SignIn), cx),
                ))
                .child(button(
                    "use-demo",
                    "Try with Demo Data",
                    keys_caps("d", &theme),
                    false,
                    &theme,
                    |_, window, cx| window.dispatch_action(Box::new(UseDemoData), cx),
                ))
                .child(
                    super::label("access-note", ACCESS_NOTE)
                        .max_w(ERROR_WIDTH)
                        .mt(SPACE_SM)
                        .text_center()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted),
                )
                .child(menu_bar_check(settings::get(cx).menu_bar.icon, &theme))
        };

        window_drag_area(div().id("onboarding"))
            .key_context(ONBOARDING)
            .track_focus(focus)
            .role(gpui::Role::Main)
            .aria_label("Sign in to Pigeon")
            // While the browser is open, Return and D do nothing; the app's
            // handlers take them otherwise.
            .on_action(cx.listener(move |_, _: &SignIn, _, cx| {
                if !waiting {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(move |_, _: &UseDemoData, _, cx| {
                if !waiting {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(move |_, _: &Confirm, window, cx| {
                if !waiting {
                    window.dispatch_action(Box::new(SignIn), cx);
                }
            }))
            .on_action(cx.listener(move |_, _: &TryDemo, window, cx| {
                if !waiting {
                    window.dispatch_action(Box::new(UseDemoData), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CancelSignIn, _, cx| {
                this.backend.update(cx, |b, cx| b.cancel_sign_in(cx));
            }))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(SPACE_XL)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(SPACE_SM)
                    .child(match app_icon_path() {
                        Some(path) => div()
                            .id("app-icon")
                            .role(gpui::Role::Image)
                            .aria_label("Pigeon app icon")
                            .child(img(path.clone()).size(APP_ICON))
                            .into_any_element(),
                        None => icon(Icon::Mail, ICON_HERO, theme.icon).into_any_element(),
                    })
                    .child(
                        div()
                            .id("onboarding-title")
                            .role(gpui::Role::Heading)
                            .aria_level(1)
                            .aria_label("Pigeon")
                            .text_size(TEXT_TITLE1)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Pigeon"),
                    )
                    .child(
                        div()
                            .text_size(TEXT_BASE)
                            .text_color(theme.text_muted)
                            .child("Mail and calendar for your Google account."),
                    ),
            )
            .child(body)
            .into_any_element()
    }
}

/// MB-Q1: the menu bar icon is on unless turned off here, before it
/// shows; Settings › Menu Bar changes it later.
fn menu_bar_check(on: bool, theme: &Theme) -> impl IntoElement {
    let label = "Show Pigeon in the menu bar";
    focusable_control(div().id("menu-bar-check"), theme)
        .role(gpui::Role::CheckBox)
        .aria_label(label)
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .mt(SPACE_SM)
        .px(SPACE_SM)
        .h(CONTROL_MD)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .text_size(TEXT_SM)
        .text_color(theme.text)
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(
            div()
                .flex_none()
                .size(ICON_LG)
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM / 2.)
                .map(|d| {
                    if on {
                        d.bg(theme.switch_on).child(icon(
                            Icon::Check,
                            ICON_SM,
                            theme.on_fill(theme.switch_on),
                        ))
                    } else {
                        d.bg(theme.control_track)
                            .border_1()
                            .border_color(theme.border)
                    }
                }),
        )
        .child(label)
        .on_click(|_, _, cx| {
            let mut prefs = settings::get(cx).menu_bar;
            prefs.icon = !prefs.icon;
            menu_bar::set_prefs(prefs, cx);
            cx.refresh_windows();
        })
}

/// A wide button with its key beside the label.
fn button(
    id: &'static str,
    label: &'static str,
    key: Div,
    primary: bool,
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    focusable_control(div().id(id), theme)
        .role(gpui::Role::Button)
        .aria_label(label)
        .w(BUTTON_WIDTH)
        .min_h(CONTROL_LG)
        .py(SPACE_SM)
        .px(SPACE_LG)
        .flex()
        .items_center()
        .justify_between()
        .rounded(RADIUS_MD)
        .text_size(TEXT_BASE)
        .font_weight(FontWeight::MEDIUM)
        .map(|this| {
            if primary {
                this.bg(theme.accent)
                    .text_color(theme.accent_text)
                    .hover(|s| s.opacity(0.9))
                    .active(|s| s.opacity(0.8))
            } else {
                this.bg(theme.chip_bg)
                    .text_color(theme.text)
                    .hover(|s| s.bg(theme.hover_bg))
                    .active(|s| s.bg(theme.selected_bg))
            }
        })
        .child(label)
        .child(key)
        .on_click(on_click)
}

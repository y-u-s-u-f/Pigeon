//! The sign-in screen (ACC-3): shown while nobody is signed in. Return
//! signs in with Google in the browser, D tries the demo data, Esc cancels a
//! sign-in that's waiting for the browser; the keys are bound in the
//! `Onboarding` context (MC-089).

use gpui::{
    AnyElement, App, Context, Div, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    accesskit::Live, actions, div, prelude::FluentBuilder as _, px,
};

use super::{icon, keycap_on_accent, keys_caps, live_text, style::*, window_drag_area};
use crate::{
    actions::{CancelSignIn, ONBOARDING, SignIn, UseDemoData},
    app::PigeonApp,
    assets::Icon,
    backend::SignInState,
    theme::Theme,
};

// Return and D on the screen. Their own actions, so the menu items for
// SignIn and UseDemoData carry no bare key (W2-R15).
actions!(onboarding, [Confirm, TryDemo]);

/// The wide buttons, and the error above them.
const BUTTON_WIDTH: Pixels = px(260.);
const ERROR_WIDTH: Pixels = px(360.);

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
                .child(
                    live_text("sign-in-waiting", "Waiting for your browser…", Live::Polite)
                        .text_size(TEXT_MD)
                        .text_color(theme.text_muted),
                )
                .child(button(
                    "cancel-sign-in",
                    "Cancel",
                    keys_caps("escape", &theme),
                    false,
                    &theme,
                    |_, window, cx| window.dispatch_action(Box::new(CancelSignIn), cx),
                ))
        } else {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(SPACE_SM)
                .children(error.map(|message| {
                    div()
                        .id("sign-in-error")
                        .role(gpui::Role::Alert)
                        .aria_label(SharedString::from(message.clone()))
                        .max_w(ERROR_WIDTH)
                        .mb(SPACE_SM)
                        .text_center()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_danger)
                        .child(message)
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
                    .child(icon(Icon::Mail, ICON_HERO, theme.icon))
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

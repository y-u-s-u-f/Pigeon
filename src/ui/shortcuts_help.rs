//! The keyboard shortcuts sheet (`?`). It reads the keymap, so it always
//! shows the bindings that are actually live.

use gpui::{
    AnyElement, AsKeystroke as _, Context, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, ParentElement, StatefulInteractiveElement as _, Styled, Window, div, px,
};

use super::{binding_caps, keycap, overlay_shadow};
use crate::{
    actions::{COMMANDS, Group, HELP, lookup_contexts},
    app::PigeonApp,
    theme::Theme,
};

/// Groups per column.
const COLUMNS: [&[Group]; 2] = [
    &[Group::Navigation, Group::GoTo],
    &[Group::Conversation, Group::Compose, Group::App],
];

impl PigeonApp {
    pub(crate) fn render_shortcuts_help(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let columns = COLUMNS.map(|groups| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(22.))
                .children(
                    groups
                        .iter()
                        .map(|&group| render_group(group, window, &theme)),
                )
        });

        div()
            .id("shortcuts-help")
            .key_context(HELP)
            .track_focus(&self.help_focus)
            .w(px(780.))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(14.))
            .bg(theme.overlay_bg)
            .border_1()
            .border_color(theme.overlay_border)
            .shadow(overlay_shadow(&theme))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(52.))
                    .flex_none()
                    .px(px(22.))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Keyboard Shortcuts"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .text_color(theme.text_faint)
                            .child(keycap("Esc", &theme))
                            .child("to close"),
                    ),
            )
            .child(
                div()
                    .id("shortcuts-help-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(22.))
                    .pt(px(18.))
                    .pb(px(22.))
                    .flex()
                    .gap(px(36.))
                    .children(columns),
            )
    }
}

fn render_group(group: Group, window: &Window, theme: &Theme) -> AnyElement {
    let rows = COMMANDS
        .iter()
        .filter(|c| c.group == group)
        .filter_map(|command| {
            let bindings = live_bindings(&*(command.action)(), window);
            (!bindings.is_empty()).then(|| {
                div()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(theme.text_muted)
                            .child(command.name),
                    )
                    .children(bindings.iter().enumerate().flat_map(|(ix, binding)| {
                        let or = (ix > 0).then(|| {
                            div()
                                .text_size(px(11.))
                                .text_color(theme.text_faint)
                                .child("or")
                                .into_any_element()
                        });
                        or.into_iter()
                            .chain([binding_caps(binding, theme).into_any_element()])
                    }))
            })
        });

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .pb(px(6.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_faint)
                .child(group.title().to_uppercase()),
        )
        .children(rows)
        .into_any_element()
}

/// The action's bindings in the list, reader, draft and search contexts, highest precedence
/// first, without duplicates.
fn live_bindings(action: &dyn gpui::Action, window: &Window) -> Vec<KeyBinding> {
    let mut bindings: Vec<KeyBinding> = Vec::new();
    for context in lookup_contexts(false) {
        for binding in window
            .bindings_for_action_in_context(action, context)
            .into_iter()
            .rev()
        {
            let same = |other: &KeyBinding| {
                other.keystrokes().len() == binding.keystrokes().len()
                    && other
                        .keystrokes()
                        .iter()
                        .zip(binding.keystrokes())
                        .all(|(a, b)| a.as_keystroke() == b.as_keystroke())
            };
            if !bindings.iter().any(same) {
                bindings.push(binding);
            }
        }
    }
    bindings
}

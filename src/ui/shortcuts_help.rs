//! The keyboard shortcuts sheet (`?`). It reads the keymap, so it always
//! shows the bindings that are actually live. Typing filters it.

use gpui::{
    AnyElement, AsKeystroke as _, Context, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, ParentElement, StatefulInteractiveElement as _, Styled, Window,
    component::input::Input, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _,
    px,
};

use super::{binding_caps, icon, keycap, overlay_shadow};
use crate::ui::style::*;
use crate::{
    actions::{COMMANDS, Command, Group, HELP, Scope, lookup_contexts},
    app::{Mode, PigeonApp},
    assets::Icon,
    theme::Theme,
};

/// The groups of each app, in reading order: the global ones first.
const MAIL_GROUPS: &[Group] = &[
    Group::App,
    Group::Navigation,
    Group::GoTo,
    Group::Conversation,
    Group::Labels,
    Group::Selection,
    Group::Compose,
    Group::Invite,
];
const CALENDAR_GROUPS: &[Group] = &[
    Group::App,
    Group::CalendarNavigation,
    Group::CalendarView,
    Group::CalendarEvents,
];

/// A command with its live bindings.
type HelpRow = (&'static Command, Vec<KeyBinding>);

/// From this window width the sheet has three columns, below it two.
const THREE_COLUMNS: f32 = 1200.;

impl PigeonApp {
    pub(crate) fn render_shortcuts_help(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = Theme::of(cx).clone();
        let (groups, scope) = match self.mode {
            Mode::Mail => (MAIL_GROUPS, Scope::Mail),
            Mode::Calendar => (CALENDAR_GROUPS, Scope::Calendar),
        };
        let filter = self.help_filter.read(cx).value().trim().to_lowercase();

        // Each group with its rows that match the filter; empty groups go.
        let groups: Vec<(Group, Vec<HelpRow>)> = groups
            .iter()
            .map(|&group| {
                let rows = COMMANDS
                    .iter()
                    .filter(|c| c.group == group)
                    .filter(|c| filter.is_empty() || c.name.to_lowercase().contains(&filter))
                    .filter_map(|command| {
                        let bindings = live_bindings(&*(command.action)(), scope, window);
                        (!bindings.is_empty()).then_some((command, bindings))
                    })
                    .collect::<Vec<_>>();
                (group, rows)
            })
            .filter(|(_, rows)| !rows.is_empty())
            .collect();

        // Fill the columns in order, starting the next one once a column
        // holds its share of rows.
        let wide = window.viewport_size().width >= px(THREE_COLUMNS);
        let column_count = if wide { 3 } else { 2 };
        let total: usize = groups.iter().map(|(_, rows)| rows.len() + 2).sum();
        let share = total.div_ceil(column_count);
        let mut columns: Vec<Vec<AnyElement>> = (0..column_count).map(|_| Vec::new()).collect();
        let (mut column, mut filled) = (0, 0);
        for (group, rows) in groups.iter() {
            if filled >= share && column + 1 < column_count {
                column += 1;
                filled = 0;
            }
            filled += rows.len() + 2;
            columns[column].push(render_group(*group, rows, &theme));
        }
        let empty = groups.is_empty();

        div()
            .id("shortcuts-help")
            .key_context(HELP)
            .track_focus(&self.help_focus)
            .w(px(if wide { 1040. } else { 780. }))
            // A fixed height: the sheet keeps its size while filtering.
            .h(px(640.))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(RADIUS_LG)
            .bg(theme.overlay_bg)
            .border_1()
            .border_color(theme.overlay_border)
            .shadow(overlay_shadow(&theme))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(52.))
                    .flex_none()
                    .px(SPACE_XL)
                    .flex()
                    .items_center()
                    .gap(SPACE_LG)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(TEXT_LG)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Keyboard Shortcuts"),
                    )
                    .child(
                        div()
                            .w(px(220.))
                            .h(px(30.))
                            .px(SPACE_SM)
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .rounded(RADIUS_MD)
                            .bg(theme.search_bg)
                            .text_size(TEXT_BASE)
                            .child(icon(Icon::Search, px(13.), theme.text_faint))
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.help_filter).appearance(false).px(px(0.)),
                                ),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .text_size(TEXT_SM)
                            .text_color(theme.text_faint)
                            .child(keycap("Esc", &theme))
                            .child("to close"),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("shortcuts-help-body")
                            .size_full()
                            .overflow_y_scroll()
                            .px(SPACE_XL)
                            .pt(SPACE_LG)
                            .pb(SPACE_2XL)
                            .flex()
                            .gap(SPACE_2XL)
                            .map(|this| {
                                if empty {
                                    this.justify_center().child(
                                        div()
                                            .py(SPACE_XL)
                                            .text_size(TEXT_BASE)
                                            .text_color(theme.text_faint)
                                            .child("No shortcuts match"),
                                    )
                                } else {
                                    this.children(columns.into_iter().map(|groups| {
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap(SPACE_XL)
                                            .children(groups)
                                    }))
                                }
                            }),
                    )
                    // More below: the last rows fade out. It lies over the
                    // bottom padding, so with nothing below it shows nothing.
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(SPACE_2XL)
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(theme.overlay_bg.alpha(0.), 0.),
                                linear_color_stop(theme.overlay_bg, 1.),
                            )),
                    ),
            )
    }
}

fn render_group(group: Group, rows: &[HelpRow], theme: &Theme) -> AnyElement {
    let rows = rows.iter().map(|(command, bindings)| {
        div()
            .h(px(30.))
            .flex()
            .items_center()
            .gap(SPACE_MD)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(TEXT_BASE)
                    .text_color(theme.text_muted)
                    .child(command.name),
            )
            .children(bindings.iter().enumerate().flat_map(|(ix, binding)| {
                let or = (ix > 0).then(|| {
                    div()
                        .text_size(TEXT_XS)
                        .text_color(theme.text_faint)
                        .child("or")
                        .into_any_element()
                });
                or.into_iter()
                    .chain([binding_caps(binding, theme).into_any_element()])
            }))
    });

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .pb(SPACE_SM)
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_faint)
                .child(group.title().to_uppercase()),
        )
        .children(rows)
        .into_any_element()
}

/// The action's bindings in the scope's contexts (see `lookup_contexts`),
/// highest precedence first, without duplicates.
pub(crate) fn live_bindings(
    action: &dyn gpui::Action,
    scope: Scope,
    window: &Window,
) -> Vec<KeyBinding> {
    let mut bindings: Vec<KeyBinding> = Vec::new();
    for context in lookup_contexts(scope, false) {
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

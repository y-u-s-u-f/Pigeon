//! The keyboard shortcuts sheet (`?`). It reads the keymap, so it always
//! shows the bindings that are actually live. Typing filters it.

use gpui::{
    AnyElement, AsKeystroke as _, Context, Focusable as _, FontWeight, Global, InteractiveElement,
    IntoElement, KeyBinding, KeymapVersion, MouseButton, ParentElement, Pixels,
    StatefulInteractiveElement as _, Styled, Window,
    component::{input::Input, scroll::ScrollableElement as _},
    div, linear_color_stop, linear_gradient,
    prelude::FluentBuilder as _,
    px,
};

use super::{controls::sheet_surface, icon, keys_caps, label};
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
const WIDE: Pixels = px(1040.);
const NARROW: Pixels = px(780.);
/// A fixed height: the sheet keeps its size while filtering.
const HEIGHT: Pixels = px(640.);
const FILTER_WIDTH: gpui::Rems = type_size(220.);

/// MC-040: every command's live bindings in one scope, looked up once per
/// keymap (a preset switch rebuilds it) instead of on every frame.
struct HelpBindings {
    version: KeymapVersion,
    scope: Scope,
    /// By index into `COMMANDS`, with what the filter searches: the name,
    /// the group and the keys, with their spelled-out names (ME-082).
    rows: Vec<(usize, Vec<KeyBinding>, String)>,
}

impl Global for HelpBindings {}

/// "⇧⌘d shift cmd command d": what a filter for a binding can type.
fn keys_text(binding: &KeyBinding) -> String {
    let mut out = String::new();
    for k in binding.keystrokes() {
        let k = k.as_keystroke();
        let m = &k.modifiers;
        for (on, words) in [
            (m.control, "⌃ ctrl control"),
            (m.alt, "⌥ opt option alt"),
            (m.shift, "⇧ shift"),
            (m.platform, "⌘ cmd command"),
        ] {
            if on {
                out.push_str(words);
                out.push(' ');
            }
        }
        out.push_str(&k.key);
        out.push(' ');
    }
    out
}

/// Whether every word of `filter` (lowercase) is in `haystack`.
fn matches_filter(haystack: &str, filter: &str) -> bool {
    filter
        .split_whitespace()
        .all(|word| haystack.contains(word))
}

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

        let version = cx.key_bindings().borrow().version();
        let fresh = cx
            .try_global::<HelpBindings>()
            .is_some_and(|b| b.version == version && b.scope == scope);
        if !fresh {
            let rows = COMMANDS
                .iter()
                .enumerate()
                .filter(|(_, c)| groups.contains(&c.group))
                .filter_map(|(ix, command)| {
                    let bindings = live_bindings(&*(command.action)(), scope, window);
                    if bindings.is_empty() {
                        return None;
                    }
                    let mut text = format!("{} {} ", command.name, command.group.title());
                    for binding in &bindings {
                        text.push_str(&keys_text(binding));
                    }
                    Some((ix, bindings, text.to_lowercase()))
                })
                .collect();
            cx.set_global(HelpBindings {
                version,
                scope,
                rows,
            });
        }
        let cached = cx.global::<HelpBindings>();

        // Each group with its rows that match the filter; empty groups go.
        let groups: Vec<(Group, Vec<HelpRow>)> = groups
            .iter()
            .map(|&group| {
                let rows = cached
                    .rows
                    .iter()
                    .filter(|(ix, _, text)| {
                        COMMANDS[*ix].group == group && matches_filter(text, &filter)
                    })
                    .map(|(ix, bindings, _)| (&COMMANDS[*ix], bindings.clone()))
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

        sheet_surface(&theme)
            .id("shortcuts-help")
            .key_context(HELP)
            .track_focus(&self.help_focus)
            .role(gpui::Role::Dialog)
            .aria_label("Keyboard Shortcuts")
            .w(if wide { WIDE } else { NARROW })
            .h(HEIGHT)
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .py(SPACE_MD)
                    .px(SPACE_XL)
                    .flex()
                    .items_center()
                    .gap(SPACE_LG)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .id("shortcuts-title")
                            .role(gpui::Role::Heading)
                            .aria_level(1)
                            .aria_label("Keyboard Shortcuts")
                            .text_size(TEXT_LG)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Keyboard Shortcuts"),
                    )
                    .child(
                        super::search_field(
                            Some(self.help_filter.focus_handle(cx).is_focused(window)),
                            &theme,
                        )
                        .w(FILTER_WIDTH)
                        .h(CONTROL_MD)
                        .px(SPACE_SM)
                        .child(icon(Icon::Search, ICON_SM, theme.text_faint))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.help_filter)
                                    .appearance(false)
                                    .cleanable(true)
                                    .px(px(0.)),
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
                            .child("Close")
                            .child(keys_caps("escape", &theme)),
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
                            .px(SPACE_XL)
                            .pt(SPACE_LG)
                            .pb(SPACE_2XL)
                            .flex()
                            .gap(SPACE_2XL)
                            .map(|this| {
                                if empty {
                                    // ME-001: what to try instead.
                                    this.justify_center().child(
                                        div()
                                            .py(SPACE_XL)
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap(SPACE_XS)
                                            .child(
                                                label("help-empty", "No shortcuts match")
                                                    .text_size(TEXT_BASE)
                                                    .text_color(theme.text_muted),
                                            )
                                            .child(
                                                label(
                                                    "help-empty-hint",
                                                    "Try a key like ⌘ or shift, or a group \
                                                     like Navigation",
                                                )
                                                .text_size(TEXT_SM)
                                                .text_color(theme.text_faint),
                                            ),
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
                            })
                            .overflow_y_scrollbar(),
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
    let rows = rows.iter().enumerate().map(|(ix, (command, bindings))| {
        div()
            .h(ROW_SM)
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
                    .child(label(("help-row", ix), command.name)),
            )
            .children(super::bindings_caps(bindings.iter(), theme))
    });

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .id(("help-group", group as usize))
                .role(gpui::Role::Heading)
                .aria_level(2)
                .aria_label(group.title())
                .pb(SPACE_SM)
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_faint)
                .child(group.title()),
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

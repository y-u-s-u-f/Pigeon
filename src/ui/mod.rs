//! Panes and the small building blocks they share.

pub mod calendar;
pub mod command_palette;
pub mod composer;
pub mod html_view;
pub mod rail;
pub mod reader;
pub mod shortcuts_help;
pub mod sidebar;
pub mod thread_list;

use gpui::{
    Action, AnyElement, App, AppContext as _, AsKeystroke as _, Axis, Context, Div, ElementId,
    FontWeight, Hsla, InteractiveElement, IntoElement, KeyBinding, Keystroke, MouseButton,
    ParentElement, Pixels, Render, SharedString, Stateful, StatefulInteractiveElement, Styled, Svg,
    Window, component::tooltip::Tooltip, div, px, svg,
};

use crate::{assets::Icon, data::Label, theme::Theme};

/// Height of the top bar of every pane; the traffic lights sit in it.
pub const HEADER_HEIGHT: Pixels = px(52.);

pub fn icon(icon: Icon, size: Pixels, color: Hsla) -> Svg {
    svg()
        .path(icon.path())
        .size(size)
        .flex_none()
        .text_color(color)
}

/// A square, borderless toolbar button that dispatches `action`, so clicking it
/// does exactly what the action's keyboard shortcut does.
pub fn icon_button(
    id: impl Into<ElementId>,
    glyph: Icon,
    tooltip: &'static str,
    action: Box<dyn Action>,
    theme: &Theme,
) -> Stateful<Div> {
    icon_button_tinted(id, glyph, theme.icon, tooltip, action, theme)
}

pub fn icon_button_tinted(
    id: impl Into<ElementId>,
    glyph: Icon,
    color: Hsla,
    tooltip: &'static str,
    action: Box<dyn Action>,
    theme: &Theme,
) -> Stateful<Div> {
    let tooltip_action = action.boxed_clone();
    div()
        .id(id)
        .size(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(icon(glyph, px(16.), color))
        // Keep the press from starting a window drag in the header behind it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .tooltip(move |window, cx| {
            Tooltip::new(tooltip)
                .action(tooltip_action.as_ref(), None)
                .build(window, cx)
        })
}

/// Makes empty header space drag the window, and double clicks zoom it, like a
/// native titlebar.
pub fn window_drag_area(el: Stateful<Div>) -> Stateful<Div> {
    el.on_mouse_down(MouseButton::Left, |event, window, _| {
        if event.click_count == 2 {
            window.titlebar_double_click();
        } else {
            window.start_window_move();
        }
    })
}

pub fn avatar(initials: impl Into<SharedString>, color: Hsla, size: Pixels, theme: &Theme) -> Div {
    div()
        .size(size)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(color)
        .text_color(theme.avatar_text)
        .text_size(size * 0.4)
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials.into())
}

pub fn label_chip(label: &Label, theme: &Theme) -> Div {
    div()
        .flex_none()
        .h(px(18.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .bg(theme.label_chip_bg(label.color))
        .text_color(theme.label_chip_text(label.color))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .child(label.name.clone())
}

/// Centered placeholder for empty panes.
pub fn empty_state(
    glyph: Icon,
    title: impl Into<SharedString>,
    hint: Option<&'static str>,
    theme: &Theme,
) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .child(icon(glyph, px(28.), theme.text_faint))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted)
                .child(title.into()),
        )
        .children(hint.map(|hint| {
            div()
                .text_size(px(12.))
                .text_color(theme.text_faint)
                .child(hint)
        }))
        .into_any_element()
}

/// Drag payload for the pane dividers; `PigeonApp::resize_pane` follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneResize {
    Sidebar,
    List,
}

impl Render for PaneResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// The draggable divider on a pane's trailing edge.
pub fn pane_divider(kind: PaneResize) -> impl IntoElement {
    gpui::component::resize_handle::<PaneResize, PaneResize>(
        match kind {
            PaneResize::Sidebar => "sidebar-divider",
            PaneResize::List => "list-divider",
        },
        Axis::Horizontal,
    )
    .inside(gpui::component::resizable::HandleEdge::Trailing)
    .on_drag(kind, |kind, _, _, cx: &mut App| cx.new(|_| *kind))
}

/// The keycaps of one keystroke: each modifier, then the key.
fn keystroke_caps(keystroke: &Keystroke) -> Vec<SharedString> {
    let m = keystroke.modifiers;
    let mut caps: Vec<SharedString> = [
        (m.control, "⌃"),
        (m.alt, "⌥"),
        (m.shift, "⇧"),
        (m.platform, "⌘"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, symbol)| symbol.into())
    .collect();
    caps.push(match keystroke.key.as_str() {
        "escape" => "Esc".into(),
        "enter" => "↵".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        "backspace" => "⌫".into(),
        "delete" => "⌦".into(),
        key => key.to_uppercase().into(),
    });
    caps
}

pub fn keycap(label: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex_none()
        .h(px(20.))
        .min_w(px(20.))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(theme.kbd_bg)
        .border_1()
        .border_color(theme.kbd_border)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.kbd_text)
        .child(label.into())
}

/// A binding as keycaps; sequences like `g i` read "G then I".
pub fn binding_caps(binding: &KeyBinding, theme: &Theme) -> Div {
    let mut row = div().flex_none().flex().items_center().gap(px(3.));
    for (ix, keystroke) in binding.keystrokes().iter().enumerate() {
        if ix > 0 {
            row = row.child(
                div()
                    .px(px(2.))
                    .text_size(px(11.))
                    .text_color(theme.text_faint)
                    .child("then"),
            );
        }
        row = row.children(
            keystroke_caps(keystroke.as_keystroke())
                .into_iter()
                .map(|cap| keycap(cap, theme)),
        );
    }
    row
}

/// The shadow under floating panels.
pub fn overlay_shadow(theme: &Theme) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: theme.overlay_shadow,
        offset: gpui::point(px(0.), px(18.)),
        blur_radius: px(48.),
        spread_radius: px(-8.),
        inset: false,
    }]
}

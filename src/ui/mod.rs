//! Panes and the small building blocks they share.

pub mod calendar;
pub mod command_palette;
pub mod composer;
pub mod contact_card;
pub mod controls;
pub mod html_view;
pub mod menu_bar;
pub mod motion;
pub mod onboarding;
pub mod rail;
pub mod reader;
pub mod rich_editor;
pub mod selectable;
pub mod settings;
pub mod shortcuts_help;
pub mod sidebar;
pub mod status;
pub mod style;
pub mod thread_list;

use gpui::{
    AbsoluteLength, Action, AnyElement, App, AppContext as _, AsKeystroke as _, Context, Div,
    ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, KeyBinding, Keystroke,
    MouseButton, MouseDownEvent, ObjectFit, ParentElement, Pixels, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, StyledImage as _, Svg, Window, accesskit::Live,
    component::tooltip::Tooltip, div, img, prelude::FluentBuilder as _, px, rems, svg,
};

use std::path::Path;

use self::style::*;
use crate::{
    assets::{Icon, Stroke},
    data::{Attachment, FileKind, Label},
    theme::Theme,
};

/// Height of the top bar of every pane; the traffic lights sit in it.
pub const HEADER_HEIGHT: Pixels = px(52.);

/// `size` at a text scale of 1, in pixels.
fn nominal_px(size: AbsoluteLength) -> f32 {
    match size {
        AbsoluteLength::Pixels(px) => px.as_f32(),
        AbsoluteLength::Rems(rems) => rems.0 * BASE_REM,
    }
}

/// A glyph `size` square. Inline icons take a rem size (`ICON_*`) so they
/// grow with the text; the stroke gets heavier for small sizes.
pub fn icon(icon: Icon, size: impl Into<AbsoluteLength>, color: Hsla) -> Svg {
    let size = size.into();
    svg()
        .path(icon.path(Stroke::for_size(nominal_px(size))))
        .size(size)
        .flex_none()
        .text_color(color)
}

/// A tooltip at the size macOS uses for them.
pub fn tooltip(text: impl Into<SharedString>) -> Tooltip {
    Tooltip::new(text.into()).text_size(TEXT_XS)
}

/// Text VoiceOver reads: plain strings never reach the accessibility tree.
/// For empty states, pane titles, summaries and confirmations.
pub fn label(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    let text = text.into();
    div()
        .id(id)
        .role(gpui::Role::Label)
        .aria_value(text.clone())
        .child(text)
}

/// Status text VoiceOver announces when it changes: toasts, save state,
/// load and sync states. `Live::Assertive` for errors, else `Live::Polite`.
pub fn live_text(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    live: Live,
) -> Stateful<Div> {
    label(id, text).a11y_synthetic_children(move |builder| builder.parent_node().set_live(live))
}

/// A right click or a Control-click: what opens a context menu on macOS.
pub fn is_secondary(event: &MouseDownEvent) -> bool {
    event.button == MouseButton::Right
        || (event.button == MouseButton::Left && event.modifiers.control)
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
    style::focusable_control(div().id(id), theme)
        .aria_label(tooltip)
        .role(gpui::Role::Button)
        .size(HIT_TARGET)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(icon(glyph, ICON_TOOLBAR, color))
        // Keep the press from starting a window drag in the header behind it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .tooltip(move |window, cx| {
            self::tooltip(tooltip)
                .action(tooltip_action.as_ref(), None)
                .build(window, cx)
        })
}

/// The compact icon button inside rows, chips and fields (remove, fold,
/// clear): a small glyph in a box that's still `HIT_TARGET` to click.
/// The caller adds `on_click`, and a negative margin where space is tight.
pub fn icon_button_small(
    id: impl Into<ElementId>,
    glyph: Icon,
    label: impl Into<SharedString>,
    theme: &Theme,
) -> Stateful<Div> {
    const GROUP: &str = "icon-button-small";
    let label = label.into();
    let tip = label.clone();
    let (hover, pressed) = (theme.hover_bg, theme.selected_bg);
    div()
        .id(id)
        .group(GROUP)
        .role(gpui::Role::Button)
        .aria_label(label)
        .min_w(HIT_TARGET)
        .min_h(HIT_TARGET)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            // The visible button; the box around it only takes the click.
            div()
                .id("face")
                .size(CONTROL_XS)
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_SM)
                .group_hover(GROUP, |s| s.bg(hover))
                .group_active(GROUP, |s| s.bg(pressed))
                .child(icon(glyph, ICON_SM, theme.text_faint)),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(move |window, cx| self::tooltip(tip.clone()).build(window, cx))
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

/// A colored circle with initials. Sizes come from `AVATAR_*`; at
/// `AVATAR_XS` and below only the first initial fits, and the text never
/// goes below 10pt.
pub fn avatar(
    initials: impl Into<SharedString>,
    color: Hsla,
    size: impl Into<AbsoluteLength>,
    theme: &Theme,
) -> Div {
    let size = size.into();
    let mut initials: SharedString = initials.into();
    let nominal = nominal_px(size);
    if nominal <= AVATAR_XS.0 * BASE_REM {
        initials = initials
            .chars()
            .next()
            .map(String::from)
            .unwrap_or_default()
            .into();
    }
    let text = rems((nominal * 0.4).max(10.) / BASE_REM);
    div()
        .size(size)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(color)
        .text_color(theme.on_fill(color))
        .text_size(text)
        .line_height(LINE_UI)
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials)
}

/// The user's own avatar: their profile photo when there is one, in a
/// circle; the initials while it loads, if it fails, and without a photo.
pub fn account_avatar(
    photo: Option<&Path>,
    initials: impl Into<SharedString>,
    color: Hsla,
    size: impl Into<AbsoluteLength>,
    theme: &Theme,
) -> Div {
    let size = size.into();
    let initials: SharedString = initials.into();
    let Some(photo) = photo else {
        return avatar(initials, color, size, theme);
    };
    let shown = move || {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(initials.clone())
            .into_any_element()
    };
    avatar("", color, size, theme).child(
        img(photo.to_path_buf())
            .size_full()
            .rounded_full()
            .object_fit(ObjectFit::Cover)
            .with_loading(shown.clone())
            .with_fallback(shown),
    )
}

/// A file's icon, tinted by its type.
pub fn attachment_icon(attachment: &Attachment, theme: &Theme) -> Svg {
    let (glyph, color) = match attachment.kind() {
        FileKind::Pdf => (Icon::FileText, theme.danger),
        FileKind::Image => (Icon::Image, theme.avatar_palette[1]),
        FileKind::Archive => (Icon::FileArchive, theme.star),
        FileKind::Document => (Icon::FileText, theme.accent),
        FileKind::Spreadsheet => (Icon::FileSpreadsheet, theme.success),
        FileKind::Other => (Icon::File, theme.icon),
    };
    icon(glyph, ICON_LG, color)
}

pub fn label_chip(label: &Label, theme: &Theme) -> Div {
    div()
        .flex_none()
        .py(SPACE_2XS)
        .px(SPACE_SM)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .line_height(LINE_UI)
        .bg(theme.label_chip_bg(label.color))
        .text_color(theme.label_chip_text(label.color))
        .text_size(TEXT_XS)
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
        .gap(SPACE_SM)
        .line_height(LINE_UI)
        .child(icon(glyph, ICON_HERO, theme.text_faint))
        .child(
            label("empty-title", title)
                .text_size(TEXT_BASE)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted),
        )
        .children(hint.map(|hint| {
            label("empty-hint", hint)
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
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

/// The draggable divider over the border at `x`: a 6pt band centered on
/// it, lit with a 1px accent line on hover and while `active` (dragged).
/// Double-clicking calls `on_reset`. Drawn over the panes, after them, so
/// both halves of the band take the mouse.
pub fn pane_divider(
    kind: PaneResize,
    x: Pixels,
    active: bool,
    theme: &Theme,
    on_reset: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (id, group) = match kind {
        PaneResize::Sidebar => ("sidebar-divider", "sidebar-divider"),
        PaneResize::List => ("list-divider", "list-divider"),
    };
    let accent = theme.accent;
    div()
        .id(id)
        .group(group)
        .absolute()
        .top_0()
        .bottom_0()
        .left(x - px(3.))
        .w(px(6.))
        .occlude()
        .cursor_col_resize()
        .child(
            // Over the pane's own 1px border, the band's third pixel.
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(2.))
                .w(px(1.))
                .when(active, |d| d.bg(accent))
                .group_hover(group, |s| s.bg(accent)),
        )
        .on_drag(kind, |kind, _, _, cx: &mut App| cx.new(|_| *kind))
        .on_click(move |event, window, cx| {
            if event.click_count() == 2 {
                on_reset(window, cx);
            }
        })
}

/// The unshifted key a US keyboard types `symbol` with, for symbols that
/// need Shift: `cmd-&` reads ⇧⌘7, as in macOS menus.
fn shifted_base(symbol: &str) -> Option<&'static str> {
    Some(match symbol {
        "~" => "`",
        "!" => "1",
        "@" => "2",
        "#" => "3",
        "$" => "4",
        "%" => "5",
        "^" => "6",
        "&" => "7",
        "*" => "8",
        "(" => "9",
        ")" => "0",
        "_" => "-",
        "+" => "=",
        "{" => "[",
        "}" => "]",
        "|" => "\\",
        ":" => ";",
        "\"" => "'",
        "<" => ",",
        ">" => ".",
        "?" => "/",
        _ => return None,
    })
}

/// The keycaps of one keystroke: each modifier, then the key. A shifted
/// symbol with ⌘, ⌃ or ⌥ reads as ⇧ plus its base key; on its own (`?`,
/// `+`) it stays as typed.
fn keystroke_caps(keystroke: &Keystroke) -> Vec<SharedString> {
    let m = keystroke.modifiers;
    let base = shifted_base(&keystroke.key).filter(|_| m.platform || m.control || m.alt);
    let shift = m.shift || base.is_some();
    let mut caps: Vec<SharedString> = [
        (m.control, "⌃"),
        (m.alt, "⌥"),
        (shift, "⇧"),
        (m.platform, "⌘"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, symbol)| symbol.into())
    .collect();
    caps.push(match base.unwrap_or(keystroke.key.as_str()) {
        "escape" => "⎋".into(),
        "enter" => "↩".into(),
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
        .h(CONTROL_XS)
        .min_w(CONTROL_XS)
        .px(SPACE_XS)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .bg(theme.kbd_bg)
        .border_1()
        .border_color(theme.kbd_border)
        .text_size(TEXT_XS)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.kbd_text)
        .child(label.into())
}

/// A keycap on an accent-filled button, where the standard one would
/// vanish into the fill.
pub fn keycap_on_accent(label: impl Into<SharedString>, theme: &Theme) -> Div {
    let text = theme.accent_text;
    div()
        .flex_none()
        .h(CONTROL_XS)
        .min_w(CONTROL_XS)
        .px(SPACE_XS)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .bg(text.alpha(0.16))
        .text_size(TEXT_XS)
        .font_weight(FontWeight::MEDIUM)
        .text_color(text.alpha(0.85))
        .child(label.into())
}

/// Keystrokes as keycaps; sequences like `g i` read "G then I".
fn keystrokes_caps<'a>(keystrokes: impl IntoIterator<Item = &'a Keystroke>, theme: &Theme) -> Div {
    let mut row = div().flex_none().flex().items_center().gap(SPACE_XS);
    for (ix, keystroke) in keystrokes.into_iter().enumerate() {
        if ix > 0 {
            row = row.child(
                div()
                    .px(SPACE_2XS)
                    .text_size(TEXT_XS)
                    .text_color(theme.text_faint)
                    .child("then"),
            );
        }
        row = row.children(
            keystroke_caps(keystroke)
                .into_iter()
                .map(|cap| keycap(cap, theme)),
        );
    }
    row
}

/// A binding as keycaps.
pub fn binding_caps(binding: &KeyBinding, theme: &Theme) -> Div {
    keystrokes_caps(binding.keystrokes().iter().map(|k| k.as_keystroke()), theme)
}

/// Keystrokes written as in the keymap (`"cmd-shift-d"`, `"g i"`) as
/// keycaps, so fixed hints read the same as live bindings.
pub fn keys_caps(keys: &str, theme: &Theme) -> Div {
    let keystrokes: Vec<Keystroke> = keys
        .split_whitespace()
        .filter_map(|k| Keystroke::parse(k).ok())
        .collect();
    keystrokes_caps(&keystrokes, theme)
}

/// The shadow under popovers and dropdowns: a tight one that lifts the
/// edge off whatever is under it, and a soft one around.
pub fn popover_shadow(theme: &Theme) -> Vec<gpui::BoxShadow> {
    vec![
        gpui::BoxShadow {
            color: theme.overlay_shadow.alpha(theme.overlay_shadow.a * 0.6),
            offset: gpui::point(px(0.), px(2.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
            inset: false,
        },
        gpui::BoxShadow {
            color: theme.overlay_shadow,
            offset: gpui::point(px(0.), px(12.)),
            blur_radius: px(32.),
            spread_radius: px(-4.),
            inset: false,
        },
    ]
}

/// A 1px lift for small controls (the chosen segment, the toggle knob) that
/// stays inside the track they sit in.
pub fn segment_shadow(theme: &Theme) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: theme.overlay_shadow.alpha(0.18),
        offset: gpui::point(px(0.), px(1.)),
        blur_radius: px(2.),
        spread_radius: px(0.),
        inset: false,
    }]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(keys: &str) -> String {
        keystroke_caps(&Keystroke::parse(keys).unwrap()).concat()
    }

    #[test]
    fn keycaps_read_like_macos_menus() {
        assert_eq!(caps("enter"), "↩");
        assert_eq!(caps("escape"), "⎋");
        assert_eq!(caps("cmd-shift-d"), "⇧⌘D");
        assert_eq!(caps("cmd-&"), "⇧⌘7");
        assert_eq!(caps("cmd-alt-?"), "⌥⇧⌘/");
        assert_eq!(caps("?"), "?");
        assert_eq!(caps("+"), "+");
    }
}

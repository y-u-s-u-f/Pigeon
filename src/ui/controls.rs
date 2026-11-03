//! Shared controls: buttons, switches, segmented controls, steppers,
//! sliders, chips, menus and popovers. Each has hover, press, keyboard
//! focus, a role and a label built in; callers add the handler and keep
//! their own sizes where they differ.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Bounds, ClickEvent, Context, Div, ElementId, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled, Toggled, Window, anchored, base::spring,
    canvas, deferred, div, prelude::FluentBuilder as _, px, relative,
};

use super::motion::{SPRING_CONTROL, SPRING_POP, fade};
use super::style::*;
use super::{icon, overlay_shadow, popover_shadow, segment_shadow};
use crate::{assets::Icon, data::Contact, theme::Theme};

fn toggled(on: bool) -> Toggled {
    if on { Toggled::True } else { Toggled::False }
}

/// Keeps a press from starting a window drag in a header behind it.
fn stop_mouse_down<E: InteractiveElement>(el: E) -> E {
    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// A filled, neutral button.
    Plain,
    /// The default action: filled with the accent.
    Primary,
    /// Red text on a neutral fill: it asks before it acts.
    Danger,
    /// Filled red: the confirmation of a danger button.
    Destructive,
    /// No fill until hovered; secondary actions in a footer.
    Quiet,
}

/// A text button, `CONTROL_MD` tall (grows with text) and never under
/// `HIT_TARGET`.
pub fn push_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    theme: &Theme,
) -> Stateful<Div> {
    let label = label.into();
    push_button_frame(id, label.clone(), kind, theme).child(label)
}

/// `push_button` without its label, for a caller that lays out its own.
pub fn push_button_frame(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    theme: &Theme,
) -> Stateful<Div> {
    let label = label.into();
    let el = focusable_control(div().id(id), theme)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .flex_none()
        .h(CONTROL_MD)
        .min_h(HIT_TARGET)
        .px(SPACE_MD)
        .flex()
        .items_center()
        .justify_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .text_size(TEXT_SM)
        .font_weight(FontWeight::MEDIUM)
        .map(|el| match kind {
            ButtonKind::Plain | ButtonKind::Danger => el
                .bg(theme.control_track)
                .text_color(if kind == ButtonKind::Danger {
                    theme.text_danger
                } else {
                    theme.text
                })
                .hover(|s| s.bg(theme.selected_bg))
                .active(|s| s.bg(theme.selected_bg).opacity(0.8)),
            ButtonKind::Primary => el
                .bg(theme.accent)
                .text_color(theme.accent_text)
                .hover(|s| s.opacity(0.9))
                .active(|s| s.opacity(0.8)),
            ButtonKind::Destructive => el
                .bg(theme.danger)
                .text_color(theme.danger_text)
                .hover(|s| s.opacity(0.9))
                .active(|s| s.opacity(0.8)),
            ButtonKind::Quiet => el
                .text_color(theme.text_muted)
                .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                .active(|s| s.bg(theme.selected_bg).text_color(theme.text)),
        });
    stop_mouse_down(el)
}

/// The event panel's footer buttons: Save or Done, and Discard.
pub fn panel_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let kind = if primary {
        ButtonKind::Primary
    } else {
        ButtonKind::Quiet
    };
    push_button(id, label, kind, theme)
}

const SWITCH_WIDTH: Pixels = px(32.);
const SWITCH_HEIGHT: Pixels = px(18.);
const SWITCH_KNOB: Pixels = px(14.);
const SWITCH_TRAVEL: Pixels = px(14.);

/// An on/off switch. The knob springs across (`SPRING_CONTROL`) and the
/// track's color follows it; under Reduce Motion the knob snaps and the
/// color fades.
pub fn switch(
    id: impl Into<ElementId>,
    on: bool,
    label: impl Into<SharedString>,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let target = if on { SWITCH_TRAVEL } else { px(0.) };
    let knob = spring((id.clone(), "knob"), target, SPRING_CONTROL, window, cx);
    let tint = if cx.reduce_motion() {
        let to = if on { 1. } else { 0. };
        fade((id.clone(), "tint"), to, to, window, cx)
    } else {
        (knob / SWITCH_TRAVEL).clamp(0., 1.)
    };
    let el = focusable_control(div().id(id), theme)
        .role(gpui::Role::Switch)
        .aria_label(label.into())
        .aria_toggled(toggled(on))
        .flex_none()
        .h(HIT_TARGET)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .active(|s| s.opacity(0.8))
        .child(
            div()
                .relative()
                .w(SWITCH_WIDTH)
                .h(SWITCH_HEIGHT)
                .rounded_full()
                .bg(theme.control_track)
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded_full()
                        .bg(theme.switch_on)
                        .opacity(tint),
                )
                .child(
                    div()
                        .absolute()
                        .top(SPACE_2XS)
                        .left(SPACE_2XS + knob)
                        .size(SWITCH_KNOB)
                        .rounded_full()
                        .bg(theme.control_knob)
                        .shadow(segment_shadow(theme)),
                ),
        );
    stop_mouse_down(el)
}

/// A row of equal-width options with a thumb under the chosen one that
/// slides (`SPRING_POP`) when the choice changes. Keyboard changes apply at
/// once; only the thumb glides.
#[allow(clippy::too_many_arguments)]
pub fn segmented(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    options: &[&'static str],
    selected: usize,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let count = options.len().max(1) as f32;
    let at = spring(
        (id.clone(), "thumb"),
        selected as f32,
        SPRING_POP,
        window,
        cx,
    );
    let on_select = Rc::new(on_select);
    let segments = options.iter().enumerate().map(|(option, name)| {
        let chosen = option == selected;
        let on_select = on_select.clone();
        div()
            .id(("segment", option))
            .role(gpui::Role::RadioButton)
            .aria_label(*name)
            .aria_toggled(toggled(chosen))
            .aria_position_in_set(option + 1)
            .aria_size_of_set(options.len())
            .h(CONTROL_SM)
            .px(SPACE_MD)
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .text_size(TEXT_SM)
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .map(|el| {
                if chosen {
                    el.text_color(theme.text)
                } else {
                    el.text_color(theme.text_muted)
                        .hover(|s| s.text_color(theme.text))
                        .active(|s| s.bg(theme.hover_bg))
                }
            })
            .child(*name)
            .on_click(move |_: &ClickEvent, window, cx| {
                cx.stop_propagation();
                on_select(option, window, cx)
            })
    });
    let el = focusable_control(div().id(id), theme)
        .role(gpui::Role::RadioGroup)
        .aria_label(label.into())
        .flex_none()
        .p(SPACE_2XS)
        .rounded(RADIUS_MD)
        .bg(theme.control_track)
        .child(
            div()
                .relative()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(at / count))
                        .w(relative(1. / count))
                        .rounded(RADIUS_SM)
                        .bg(theme.segment_bg)
                        .shadow(segment_shadow(theme)),
                )
                // Columns as wide as the widest option.
                .child(
                    div()
                        .grid()
                        .grid_cols_min_content(options.len() as u16)
                        .children(segments),
                ),
        );
    stop_mouse_down(el)
}

/// − value + : steps a value; a button does nothing at its bound.
pub fn stepper(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    (can_down, can_up): (bool, bool),
    theme: &Theme,
    on_step: impl Fn(i32, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let value = value.into();
    let on_step = Rc::new(on_step);
    let hover = theme.selected_bg;
    // A CONTROL_SM face in a HIT_TARGET box, like `icon_button_small`.
    let button = |name: &'static str, glyph: Icon, delta: i32, enabled: bool| {
        let on_step = on_step.clone();
        div()
            .id(name)
            .group(name)
            .role(gpui::Role::Button)
            .aria_label(name)
            .size(HIT_TARGET)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("face")
                    .size(CONTROL_SM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_SM)
                    .bg(theme.control_track)
                    .when(enabled, |el| {
                        el.group_hover(name, |s| s.bg(hover))
                            .group_active(name, |s| s.bg(hover).opacity(0.8))
                    })
                    .child(icon(
                        glyph,
                        ICON_MD,
                        if enabled {
                            theme.icon
                        } else {
                            theme.text_disabled
                        },
                    )),
            )
            .when(enabled, |el| {
                el.on_click(move |_: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    on_step(delta, window, cx)
                })
            })
    };
    let el = focusable_control(div().id(id), theme)
        .role(gpui::Role::SpinButton)
        .aria_label(label.into())
        .aria_value(value.clone())
        .flex_none()
        .flex()
        .items_center()
        .gap(SPACE_XS)
        .rounded(RADIUS_SM)
        .child(button("Decrease", Icon::Minus, -1, can_down))
        .child(
            div()
                .min_w(STEPPER_VALUE_MIN)
                .flex()
                .justify_center()
                .text_size(TEXT_SM)
                .font_weight(FontWeight::MEDIUM)
                .font_features(tabular_figures())
                .child(value),
        )
        .child(button("Increase", Icon::Plus, 1, can_up));
    stop_mouse_down(el)
}

/// A stepper's value, so the buttons don't move as it changes.
const STEPPER_VALUE_MIN: Pixels = px(44.);
const SLIDER_WIDTH: Pixels = px(160.);
const SLIDER_TRACK: Pixels = px(4.);
const SLIDER_KNOB: Pixels = px(16.);

/// What a slider drag carries, so a move knows which slider it's for.
struct SliderDrag(ElementId);

impl Render for SliderDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// A horizontal slider over 0…1. Click or drag anywhere on it; `on_change`
/// gets the new value. `value_text` is what VoiceOver reads ("40%").
pub fn slider(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: f32,
    value_text: impl Into<SharedString>,
    theme: &Theme,
    on_change: impl Fn(f32, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let value = value.clamp(0., 1.);
    let bounds = Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
    let on_change = Rc::new(on_change);
    let at = |x: Pixels, bounds: Bounds<Pixels>| {
        if bounds.size.width <= px(0.) {
            return 0.;
        }
        ((x - bounds.left()) / bounds.size.width).clamp(0., 1.)
    };
    let track = div()
        .id("track")
        .relative()
        .flex_1()
        .h_full()
        .flex()
        .items_center()
        .child({
            let bounds = bounds.clone();
            canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                .absolute()
                .size_full()
        })
        .child(
            div()
                .w_full()
                .h(SLIDER_TRACK)
                .rounded_full()
                .bg(theme.control_track)
                .child(
                    div()
                        .h_full()
                        .w(relative(value))
                        .rounded_full()
                        .bg(theme.switch_on),
                ),
        )
        .child(
            div()
                .absolute()
                .left(relative(value))
                .ml(-SLIDER_KNOB / 2.)
                .size(SLIDER_KNOB)
                .rounded_full()
                .bg(theme.control_knob)
                .shadow(segment_shadow(theme)),
        )
        .on_mouse_down(MouseButton::Left, {
            let on_change = on_change.clone();
            move |event, window, cx| {
                cx.stop_propagation();
                on_change(at(event.position.x, bounds.get()), window, cx)
            }
        })
        .on_drag(SliderDrag(id.clone()), |drag, _, _, cx| {
            cx.new(|_| SliderDrag(drag.0.clone()))
        })
        .on_drag_move::<SliderDrag>({
            let id = id.clone();
            move |event, window, cx| {
                if event.drag(cx).0 == id {
                    on_change(at(event.event.position.x, event.bounds), window, cx)
                }
            }
        });
    focusable_control(div().id(id), theme)
        .role(gpui::Role::Slider)
        .aria_label(label.into())
        .aria_value(value_text.into())
        .aria_numeric_value(value as f64)
        .aria_min_numeric_value(0.)
        .aria_max_numeric_value(1.)
        .flex_none()
        .w(SLIDER_WIDTH)
        .h(HIT_TARGET)
        .px(SLIDER_KNOB / 2.)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .active(|s| s.opacity(0.9))
        .child(track)
}

/// A small toggle in a row of options (repeat days, reminders).
pub fn toggle_chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let label = label.into();
    focusable_control(div().id(id), theme)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .aria_toggled(toggled(active))
        .h(CONTROL_SM)
        .px(SPACE_SM)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .text_size(TEXT_SM)
        .font_weight(FontWeight::MEDIUM)
        .map(|el| {
            if active {
                el.bg(theme.accent)
                    .text_color(theme.accent_text)
                    .active(|s| s.opacity(0.8))
            } else {
                el.bg(theme.chip_bg)
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                    .active(|s| s.bg(theme.selected_bg).text_color(theme.text))
            }
        })
        .child(label)
}

/// A checkbox's box; callers make the row around it the control.
pub fn check_box(on: bool, theme: &Theme) -> Div {
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
        })
}

/// The leading check column of a menu row, empty when not chosen, so
/// every pop-up puts the check in the same place.
pub fn menu_check(chosen: bool, theme: &Theme) -> Div {
    div()
        .flex_none()
        .w(SPACE_LG)
        .flex()
        .items_center()
        .justify_center()
        .when(chosen, |el| {
            el.child(icon(Icon::Check, ICON_MD, theme.accent))
        })
}

/// A row of a pop-up menu or dropdown. Callers add `menu_check` and the
/// content.
pub fn menu_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    chosen: bool,
    highlighted: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(gpui::Role::MenuItemRadio)
        .aria_label(label.into())
        .aria_toggled(toggled(chosen))
        .h(ROW_SM)
        .px(SPACE_SM)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_SM)
        .text_size(TEXT_BASE)
        .text_color(theme.text)
        .when(highlighted, |el| el.bg(theme.selected_bg))
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
}

/// The chrome of anything anchored to a control: dropdowns, pickers,
/// suggestion lists, toasts.
pub fn popover_surface(theme: &Theme) -> Div {
    div()
        .rounded(RADIUS_LG)
        .bg(theme.overlay_bg)
        .border_1()
        .border_color(theme.overlay_border)
        .shadow(popover_shadow(theme))
}

/// The chrome of a sheet over the whole window: the palette, the shortcuts
/// sheet, the contact card.
pub fn sheet_surface(theme: &Theme) -> Div {
    div()
        .rounded(RADIUS_LG)
        .bg(theme.overlay_bg)
        .border_1()
        .border_color(theme.overlay_border)
        .shadow(overlay_shadow(theme))
}

/// A dropdown's list: a popover of rows, `width` wide.
pub fn dropdown(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    width: gpui::Rems,
    theme: &Theme,
) -> Stateful<Div> {
    popover_surface(theme)
        .id(id)
        .role(gpui::Role::Menu)
        .aria_label(label.into())
        .occlude()
        .w(width)
        .p(SPACE_XS)
        .flex()
        .flex_col()
}

/// Shows `content` floating under the element before it: a zero-height
/// anchor in the flow, so it starts where the next sibling would.
pub fn popup(content: impl IntoElement) -> Div {
    div().w_full().h(px(0.)).child(
        deferred(
            anchored()
                .snap_to_window_with_margin(SPACE_SM)
                .child(content),
        )
        .with_priority(2),
    )
}

/// Fill, text and hover colors of an all-day or multi-day bar.
pub fn bar_colors(color: Hsla, past: bool, selected: bool, theme: &Theme) -> (Hsla, Hsla, Hsla) {
    let (fill, text) = if selected {
        let fill = theme.event_selected_fill(color);
        (fill, theme.on_fill(fill))
    } else {
        (theme.event_fill(color, past), theme.event_text(color, past))
    };
    (fill, text, theme.event_fill_hover(color))
}

/// A contact in a suggestion list: avatar, name and address.
pub fn person_row(
    id: impl Into<ElementId>,
    contact: &Contact,
    highlighted: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let name = contact.display_name().to_string();
    div()
        .id(id)
        .role(gpui::Role::ListBoxOption)
        .aria_label(format!("{name}, {}", contact.email))
        .aria_selected(highlighted)
        .py(SPACE_XS)
        .px(SPACE_SM)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_SM)
        .when(highlighted, |el| el.bg(theme.selected_bg))
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(super::avatar(
            contact.initials(),
            theme.avatar_color(&contact.email),
            AVATAR_SM,
            theme,
        ))
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_size(TEXT_BASE)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .child(contact.email.clone()),
                ),
        )
}

/// An inline error under a field: read by VoiceOver and selectable, to
/// copy into a search or a report (MB-035).
pub fn error_line(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    theme: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let text = text.into();
    div()
        .id(id.clone())
        .role(gpui::Role::Label)
        .aria_value(text.clone())
        .flex()
        .items_center()
        .gap(SPACE_XS)
        .text_size(TEXT_SM)
        .text_color(theme.text_danger)
        .child(icon(Icon::OctagonAlert, ICON_SM, theme.text_danger))
        .child(
            super::selectable::Selectable::plain((id, "text"), text, theme.text_selection).silent(),
        )
}

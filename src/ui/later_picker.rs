//! The time picker Snooze (ACT-11) and Send Later (CMP-17) share: presets
//! still ahead, and a field that reads a typed time like quick add, said
//! back as it's typed. ↑↓ choose, ↩ picks, Esc closes.

use chrono::NaiveDateTime;
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, actions,
    component::input::{Input, InputEvent, InputState},
    div,
    prelude::FluentBuilder as _,
    px,
};

use super::{controls::menu_row, controls::sheet_surface, icon, keycap, keys_caps, live_text};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    later::{self, Preset, Typed},
    theme::Theme,
};

actions!(later_picker, [SelectNext, SelectPrev, Dismiss]);

/// Key context of the picker.
pub const LATER_PICKER: &str = "LaterPicker";

const WIDTH: f32 = 380.;

pub enum LaterEvent {
    /// The time picked, Unix ms.
    Picked(i64),
    Dismiss,
}

pub struct LaterPicker {
    title: SharedString,
    /// Who it's for: "3 conversations", the draft's subject.
    subtitle: SharedString,
    now: NaiveDateTime,
    presets: Vec<(Preset, NaiveDateTime)>,
    input: Entity<InputState>,
    typed: Typed,
    /// The highlighted row: a preset, or past them the typed time.
    selected: usize,
    focus_handle: FocusHandle,
    _input_events: Subscription,
}

impl EventEmitter<LaterEvent> for LaterPicker {}

impl Focusable for LaterPicker {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl LaterPicker {
    pub fn new(
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let now = later::now_local();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Or type a time, like “fri 9am” or “in 2 hours”")
        });
        let events = cx.subscribe_in(&input, window, Self::on_input_event);
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            now,
            presets: later::presets(now),
            input,
            typed: Typed::Empty,
            selected: 0,
            focus_handle: cx.focus_handle(),
            _input_events: events,
        }
    }

    /// The field takes the keys, so typing a time works at once.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn typed_row(&self) -> usize {
        self.presets.len()
    }

    fn rows(&self) -> usize {
        self.presets.len() + usize::from(matches!(self.typed, Typed::At(_)))
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.now = later::now_local();
                self.typed = later::parse(self.input.read(cx).value().as_ref(), self.now);
                // A time that reads is the one Return picks.
                self.selected = match self.typed {
                    Typed::At(_) => self.typed_row(),
                    _ => self.selected.min(self.presets.len().saturating_sub(1)),
                };
                cx.notify();
            }
            // Typed text that isn't a time ahead picks nothing, rather
            // than the highlighted preset.
            InputEvent::PressEnter { .. } => {
                let empty = self.input.read(cx).value().trim().is_empty();
                if empty || matches!(self.typed, Typed::At(_)) {
                    self.pick(self.selected, cx)
                }
            }
            _ => {}
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected + 1 < self.rows() {
            self.selected += 1;
            cx.notify();
        }
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected > 0 {
            self.selected -= 1;
            cx.notify();
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(LaterEvent::Dismiss);
    }

    /// The time on `row`, unless it has passed while the picker was open:
    /// the presets and the typed time are read again then.
    fn pick(&mut self, row: usize, cx: &mut Context<Self>) {
        let at = match (self.presets.get(row), self.typed) {
            (Some((_, at)), _) => *at,
            (None, Typed::At(at)) => at,
            _ => return,
        };
        let now = later::now_local();
        if at > now {
            cx.emit(LaterEvent::Picked(later::local_ms(at)));
            return;
        }
        self.now = now;
        self.presets = later::presets(now);
        self.typed = later::parse(self.input.read(cx).value().as_ref(), now);
        self.selected = self.selected.min(self.rows().saturating_sub(1));
        cx.notify();
    }

    fn hover(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.selected != row {
            self.selected = row;
            cx.notify();
        }
    }
}

impl Render for LaterPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let now = self.now;
        let rows = self
            .presets
            .iter()
            .enumerate()
            .map(|(ix, (preset, at))| {
                let when = later::when_label(*at, now);
                let selected = ix == self.selected;
                menu_row(
                    ("later-preset", ix),
                    format!("{}, {when}", preset.title()),
                    false,
                    selected,
                    &theme,
                )
                .role(gpui::Role::ListBoxOption)
                .aria_selected(selected)
                .when(selected, |d| d.aria_active_descendant())
                .h(ROW_PALETTE)
                .px(SPACE_MD)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(preset.title()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .font_features(tabular_figures())
                        .child(when),
                )
                .on_mouse_move(cx.listener(move |this, _, _, cx| this.hover(ix, cx)))
                .on_click(cx.listener(move |this, _, _, cx| this.pick(ix, cx)))
            })
            .collect::<Vec<_>>();
        let typed_row = self.typed_row();
        let typed_selected = self.selected == typed_row;
        let (readback, color) = match self.typed {
            Typed::Empty => (None, theme.text_faint),
            Typed::At(at) => (Some(later::when_label(at, now)), theme.text),
            Typed::Past(_) => (Some("That time has passed".to_string()), theme.text_faint),
            Typed::Unknown => (Some("Not a time yet".to_string()), theme.text_faint),
        };
        let can_pick_typed = matches!(self.typed, Typed::At(_));

        let hint = |label: &'static str, caps: gpui::Div| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .child(label)
                .child(caps)
        };

        sheet_surface(&theme)
            .id("later-picker")
            .key_context(LATER_PICKER)
            .track_focus(&self.focus_handle)
            .role(gpui::Role::Dialog)
            .aria_label(self.title.clone())
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::dismiss))
            .w(px(WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(SPACE_LG)
                    .pt(SPACE_LG)
                    .pb(SPACE_SM)
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .child(
                        div()
                            .id("later-title")
                            .role(gpui::Role::Heading)
                            .aria_label(self.title.clone())
                            .text_size(TEXT_LG)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title.clone()),
                    )
                    .when(!self.subtitle.is_empty(), |d| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_muted)
                                .child(self.subtitle.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .id("later-presets")
                    .role(gpui::Role::ListBox)
                    .aria_label("Times")
                    .px(SPACE_SM)
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .child(
                div()
                    .px(SPACE_LG)
                    .pt(SPACE_SM)
                    .pb(SPACE_LG)
                    .flex()
                    .flex_col()
                    .gap(SPACE_XS)
                    .child(
                        div()
                            .id("later-typed")
                            .role(gpui::Role::Group)
                            .aria_label("Type a time")
                            .h(CONTROL_LG)
                            .px(SPACE_SM)
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .rounded(RADIUS_MD)
                            .bg(theme.search_bg)
                            .border_1()
                            .border_color(if typed_selected {
                                theme.accent.alpha(0.55)
                            } else {
                                theme.border
                            })
                            .child(icon(Icon::Clock, ICON_MD, theme.text_faint))
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.input)
                                        .appearance(false)
                                        .px(px(0.))
                                        .text_size(TEXT_BASE),
                                ),
                            ),
                    )
                    .children(readback.map(|text| {
                        live_text("later-readback", text, gpui::accesskit::Live::Polite)
                            .px(SPACE_SM)
                            .min_h(ROW_SM)
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .rounded(RADIUS_SM)
                            .text_size(TEXT_SM)
                            .text_color(color)
                            .when(typed_selected && can_pick_typed, |d| {
                                d.bg(theme.selected_bg)
                            })
                            .when(can_pick_typed, |d| {
                                d.cursor_pointer()
                                    .on_mouse_move(
                                        cx.listener(move |this, _, _, cx| {
                                            this.hover(typed_row, cx)
                                        }),
                                    )
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.pick(typed_row, cx)),
                                    )
                            })
                    })),
            )
            .child(
                div()
                    .py(SPACE_SM)
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_LG)
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(hint(
                        "Choose",
                        div()
                            .flex()
                            .gap(SPACE_XS)
                            .child(keycap("↑", &theme))
                            .child(keycap("↓", &theme)),
                    ))
                    .child(hint("Pick", keys_caps("enter", &theme)))
                    .child(hint("Close", keys_caps("escape", &theme))),
            )
    }
}

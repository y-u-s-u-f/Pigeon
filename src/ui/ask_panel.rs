//! Ask Pigeon (AI-14): the floating panel over the window, with the
//! question, the streamed answer and its numbered sources. The app does
//! the asking (`app/ai_palette.rs`); this shows where it is.

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement as _,
    Styled, Subscription, Window,
    accesskit::Live,
    component::input::{Input, InputEvent, InputState},
    div,
    prelude::FluentBuilder as _,
    px,
};

use super::{
    command_palette::{Confirm, Dismiss},
    controls::{error_line, sheet_surface},
    icon, keys_caps, live_text, section_caption,
};
use crate::{
    actions::PALETTE,
    ai::{Source, SourceId},
    assets::Icon,
    theme::Theme,
    ui::style::*,
};

const WIDTH: f32 = 620.;

#[derive(Clone, Debug, PartialEq)]
pub enum AskStatus {
    Idle,
    Searching,
    Writing,
    Done,
    Failed(SharedString),
}

pub enum AskEvent {
    Submit(String),
    Dismiss,
    /// A source, by its index in the list.
    Open(usize),
}

pub struct AskPanel {
    input: Entity<InputState>,
    status: AskStatus,
    answer: SharedString,
    sources: Vec<Source>,
    _input_events: Subscription,
}

impl EventEmitter<AskEvent> for AskPanel {}

impl Focusable for AskPanel {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl AskPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Ask about your mail and calendar"));
        let input_events = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        });
        Self {
            input,
            status: AskStatus::Idle,
            answer: SharedString::default(),
            sources: Vec::new(),
            _input_events: input_events,
        }
    }

    /// Where the question is: its status, the answer so far and the
    /// sources sent.
    pub fn show(
        &mut self,
        status: AskStatus,
        answer: impl Into<SharedString>,
        sources: Vec<Source>,
        cx: &mut Context<Self>,
    ) {
        self.status = status;
        self.answer = answer.into();
        self.sources = sources;
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        let question = self.input.read(cx).value().trim().to_string();
        if !question.is_empty() {
            cx.emit(AskEvent::Submit(question));
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(AskEvent::Dismiss);
    }

    fn render_source(
        &self,
        ix: usize,
        source: &Source,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let glyph = match source.id {
            SourceId::Mail(_) => Icon::Mail,
            SourceId::Event { .. } => Icon::Calendar,
        };
        focusable_control(div().id(("ask-source", ix)), theme)
            .role(gpui::Role::Link)
            .aria_label(format!(
                "Source {}: {}, {}",
                source.n, source.title, source.date
            ))
            .min_h(ROW_SM)
            .px(SPACE_SM)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .text_size(TEXT_SM)
            .hover(|s| s.bg(theme.hover_bg))
            .active(|s| s.bg(theme.selected_bg))
            .child(
                div()
                    .flex_none()
                    .text_color(theme.text_faint)
                    .font_features(tabular_figures())
                    .child(format!("[{}]", source.n)),
            )
            .child(icon(glyph, ICON_MD, theme.icon))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text)
                    .child(source.title.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(theme.text_faint)
                    .child(source.date.clone()),
            )
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(AskEvent::Open(ix))))
            .into_any_element()
    }
}

impl Render for AskPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let status: Option<SharedString> = match &self.status {
            AskStatus::Idle => {
                Some("Ask about your mail and events, like “When is the dentist?”".into())
            }
            AskStatus::Searching => Some("Searching…".into()),
            AskStatus::Writing if self.answer.is_empty() => Some("Writing…".into()),
            AskStatus::Writing | AskStatus::Done | AskStatus::Failed(_) => None,
        };
        let sources: Vec<gpui::AnyElement> = self
            .sources
            .iter()
            .enumerate()
            .map(|(ix, s)| self.render_source(ix, s, &theme, cx))
            .collect();

        sheet_surface(&theme)
            .id("ask-pigeon")
            .key_context(PALETTE)
            .role(gpui::Role::Dialog)
            .aria_label("Ask Pigeon")
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .min_h(CONTROL_LG)
                    .py(SPACE_MD)
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_MD)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(icon(Icon::MessagesSquare, ICON_XL, theme.text_faint))
                    .child(
                        div()
                            .flex_1()
                            .text_size(TEXT_LG)
                            .child(Input::new(&self.input).appearance(false)),
                    ),
            )
            .child(
                div()
                    .id("ask-body")
                    .max_h(gpui::rems(ROW_PALETTE.0 * 10.))
                    .overflow_y_scroll()
                    .py(SPACE_MD)
                    .px(SPACE_LG)
                    .flex()
                    .flex_col()
                    .gap(SPACE_MD)
                    .children(status.map(|text| {
                        live_text("ask-status", text, Live::Polite)
                            .text_size(TEXT_BASE)
                            .text_color(theme.text_faint)
                    }))
                    .when(!self.answer.is_empty(), |this| {
                        this.child(
                            div()
                                .id("ask-answer")
                                .role(gpui::Role::Document)
                                .aria_label(format!("AI answer: {}", self.answer))
                                .text_size(TEXT_BASE)
                                .text_color(theme.text)
                                .child(self.answer.clone()),
                        )
                    })
                    .when_some(
                        match &self.status {
                            AskStatus::Failed(why) => Some(why.clone()),
                            _ => None,
                        },
                        |this, why| this.child(error_line("ask-error", why, &theme)),
                    )
                    .when(!sources.is_empty(), |this| {
                        this.child(
                            div()
                                .id("ask-sources")
                                .role(gpui::Role::List)
                                .aria_label("Sources")
                                .flex()
                                .flex_col()
                                .gap(SPACE_XS)
                                .child(section_caption("Sources", &theme).px(SPACE_SM))
                                .children(sources),
                        )
                    }),
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
                    .child("Searched mail and events on this Mac")
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .child("Ask")
                            .child(keys_caps("enter", &theme)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .child("Close")
                            .child(keys_caps("escape", &theme)),
                    ),
            )
    }
}

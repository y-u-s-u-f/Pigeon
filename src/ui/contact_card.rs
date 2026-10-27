//! The contact card (CON-5, sprint 2 W6): who someone is, from the local
//! cache, and the latest conversations with them. Opened from a sender or
//! recipient in the reader, a guest in the event panel, or "Show Contact".

use gpui::{
    Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Window, actions, div, prelude::FluentBuilder as _, px,
};

use super::{avatar, icon, keycap, overlay_shadow};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    contacts::ContactCard,
    data::{Contact, list_date},
    theme::Theme,
};

actions!(
    contact_card,
    [
        SelectNext,
        SelectPrev,
        Confirm,
        Dismiss,
        ComposeTo,
        CopyAddress
    ]
);

/// Key context of the card.
pub const CONTACT_CARD: &str = "ContactCard";

const WIDTH: f32 = 440.;
const ROW_HEIGHT: f32 = 36.;

pub enum CardEvent {
    OpenThread(String),
    Compose(Contact),
    Copy(String),
    Dismiss,
}

pub struct ContactCardView {
    card: ContactCard,
    /// The highlighted conversation.
    selected: usize,
    focus_handle: FocusHandle,
}

impl EventEmitter<CardEvent> for ContactCardView {}

impl Focusable for ContactCardView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ContactCardView {
    pub fn new(card: ContactCard, cx: &mut Context<Self>) -> Self {
        Self {
            card,
            selected: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected + 1 < self.card.threads.len() {
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

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.open(self.selected, cx);
    }

    fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(thread) = self.card.threads.get(ix) {
            cx.emit(CardEvent::OpenThread(thread.id.clone()));
        }
    }

    fn compose(&mut self, _: &ComposeTo, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CardEvent::Compose(self.card.contact()));
    }

    fn copy(&mut self, _: &CopyAddress, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CardEvent::Copy(self.card.contact().email));
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CardEvent::Dismiss);
    }

    /// A line of what the card knows: an icon, the value and its label.
    fn detail(glyph: Icon, value: String, label: &str, theme: &Theme) -> gpui::Div {
        div()
            .min_h(px(ROW_HEIGHT - 8.))
            .flex()
            .items_center()
            .gap(SPACE_MD)
            .child(icon(glyph, px(14.), theme.text_faint))
            .child(div().min_w_0().truncate().text_size(TEXT_BASE).child(value))
            .when(!label.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_faint)
                        .child(label.to_string()),
                )
            })
    }

    fn action_button(
        id: &'static str,
        glyph: Icon,
        label: &'static str,
        key: &'static str,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label)
            .h(HIT_TARGET)
            .px(SPACE_MD)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .bg(theme.chip_bg)
            .cursor_pointer()
            .hover(|s| s.bg(theme.selected_bg))
            .text_size(TEXT_SM)
            .font_weight(FontWeight::MEDIUM)
            .child(icon(glyph, px(13.), theme.icon))
            .child(label)
            .child(keycap(key, theme))
    }
}

impl Render for ContactCardView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let card = &self.card;
        let contact = card.contact();
        let name: SharedString = contact.display_name().to_string().into();
        let role_line = [
            card.details.title.as_str(),
            card.details.organization.as_str(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        let subtitle = if !role_line.is_empty() {
            role_line
        } else if card.is_contact {
            String::new()
        } else {
            "Not in your contacts".into()
        };

        let mut details: Vec<gpui::Div> = Vec::new();
        for email in &card.emails {
            details.push(Self::detail(Icon::Mail, email.clone(), "", &theme));
        }
        for phone in &card.details.phones {
            details.push(Self::detail(
                Icon::Phone,
                phone.number.clone(),
                &phone.label,
                &theme,
            ));
        }

        let threads = card.threads.iter().enumerate().map(|(ix, thread)| {
            let selected = ix == self.selected;
            let date = chrono::DateTime::from_timestamp_millis(thread.last_date)
                .map(|d| list_date(&d.fixed_offset()))
                .unwrap_or_default();
            let subject = if thread.subject.trim().is_empty() {
                "(no subject)".to_string()
            } else {
                thread.subject.clone()
            };
            div()
                .id(("contact-thread", ix))
                .role(gpui::Role::ListBoxOption)
                .aria_label(format!("{subject}, {date}"))
                .aria_selected(selected)
                .h(px(ROW_HEIGHT))
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_MD)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .when(selected, |d| d.bg(theme.selected_bg))
                .child(icon(
                    Icon::Mail,
                    px(14.),
                    if selected {
                        theme.icon_active
                    } else {
                        theme.icon
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(TEXT_BASE)
                        .child(subject),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_faint)
                        .child(date),
                )
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.selected != ix {
                        this.selected = ix;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.open(ix, cx)))
        });
        let no_threads = card.threads.is_empty();

        let hint = |keys: &'static [&'static str], label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .children(keys.iter().map(|k| keycap(*k, &theme)))
                .child(div().ml(SPACE_2XS).child(label))
        };

        div()
            .id("contact-card")
            .key_context(CONTACT_CARD)
            .track_focus(&self.focus_handle)
            .role(gpui::Role::Dialog)
            .aria_label(format!("Contact: {name}"))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::compose))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::dismiss))
            .w(px(WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(RADIUS_LG)
            .bg(theme.overlay_bg)
            .border_1()
            .border_color(theme.overlay_border)
            .shadow(overlay_shadow(&theme))
            // Keep clicks inside from reaching the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .p(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_MD)
                    .child(avatar(
                        contact.initials(),
                        theme.avatar_color(&contact.email),
                        px(48.),
                        &theme,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(SPACE_2XS)
                            .child(
                                div()
                                    .id("contact-name")
                                    .role(gpui::Role::Heading)
                                    .aria_label(name.clone())
                                    .truncate()
                                    .text_size(TEXT_XL)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name.clone()),
                            )
                            .when(!subtitle.is_empty(), |d| {
                                d.child(
                                    div()
                                        .truncate()
                                        .text_size(TEXT_SM)
                                        .text_color(theme.text_muted)
                                        .child(subtitle),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .px(SPACE_LG)
                    .pb(SPACE_MD)
                    .flex()
                    .flex_col()
                    .children(details),
            )
            .child(
                div()
                    .px(SPACE_LG)
                    .pb(SPACE_LG)
                    .flex()
                    .gap(SPACE_SM)
                    .child(
                        Self::action_button(
                            "contact-compose",
                            Icon::SquarePen,
                            "Compose",
                            "C",
                            &theme,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.compose(&ComposeTo, window, cx)),
                        ),
                    )
                    .child(
                        Self::action_button(
                            "contact-copy",
                            Icon::Copy,
                            "Copy Address",
                            "⌘C",
                            &theme,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.copy(&CopyAddress, window, cx)),
                        ),
                    ),
            )
            .child(
                div()
                    .px(SPACE_LG)
                    .pt(SPACE_MD)
                    .pb(SPACE_XS)
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child("Recent conversations"),
            )
            .child(
                div()
                    .id("contact-threads")
                    .role(gpui::Role::ListBox)
                    .aria_label("Recent conversations")
                    .px(SPACE_SM)
                    .pb(SPACE_SM)
                    .flex()
                    .flex_col()
                    .children(threads)
                    .when(no_threads, |d| {
                        d.child(
                            div()
                                .h(px(ROW_HEIGHT))
                                .px(SPACE_MD)
                                .flex()
                                .items_center()
                                .text_size(TEXT_BASE)
                                .text_color(theme.text_faint)
                                .child("No conversations in this Mac’s cache"),
                        )
                    }),
            )
            .child(
                div()
                    .h(px(34.))
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_LG)
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(hint(&["↑", "↓"], "Navigate"))
                    .child(hint(&["↵"], "Open"))
                    .child(hint(&["Esc"], "Close")),
            )
    }
}

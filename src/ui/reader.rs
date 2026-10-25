//! The reading pane: toolbar, subject, and the thread as a conversation.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, component::scroll::ScrollableElement as _, div,
    prelude::FluentBuilder as _, px, relative,
};

use super::{
    HEADER_HEIGHT, avatar, empty_state, icon, icon_button, icon_button_tinted, label_chip,
    window_drag_area,
};
use crate::{
    actions::*,
    app::PigeonApp,
    assets::Icon,
    data::{Contact, Folder, Message, Thread, file_size, full_date, html_to_text},
    theme::Theme,
};

impl PigeonApp {
    pub(crate) fn render_reader(&self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let thread = self.selected.map(|ix| &self.data.threads[ix]);

        div()
            .flex_1()
            .min_w(px(360.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.reader_bg)
            .child(self.render_toolbar(thread, &theme))
            .child(match thread {
                Some(thread) => self.render_conversation(thread, &theme, cx),
                None => empty_state(
                    Icon::Mail,
                    "No conversation selected",
                    Some("Pick a thread from the list to read it here"),
                    &theme,
                ),
            })
    }

    fn render_toolbar(&self, thread: Option<&Thread>, theme: &Theme) -> impl IntoElement {
        let header = window_drag_area(div().id("reader-toolbar"))
            .h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(12.));
        let Some(thread) = thread else {
            return header;
        };

        let separator = || div().w(px(1.)).h(px(16.)).mx(px(6.)).bg(theme.border);
        let archived = thread.folder == Folder::Archive;

        header
            .child(icon_button(
                "archive",
                Icon::Archive,
                if archived { "Move to Inbox" } else { "Archive" },
                Box::new(Archive),
                theme,
            ))
            .child(icon_button(
                "delete",
                Icon::Trash,
                "Delete",
                Box::new(Delete),
                theme,
            ))
            .child(icon_button(
                "mark-unread",
                Icon::MailUnread,
                "Mark as unread",
                Box::new(MarkUnread),
                theme,
            ))
            .child(if thread.starred {
                icon_button_tinted(
                    "star",
                    Icon::StarFilled,
                    theme.star,
                    "Unstar",
                    Box::new(ToggleStar),
                    theme,
                )
            } else {
                icon_button("star", Icon::Star, "Star", Box::new(ToggleStar), theme)
            })
            .child(div().flex_1())
            .child(icon_button(
                "reply",
                Icon::Reply,
                "Reply",
                Box::new(Reply),
                theme,
            ))
            .child(icon_button(
                "reply-all",
                Icon::ReplyAll,
                "Reply all",
                Box::new(ReplyAll),
                theme,
            ))
            .child(separator())
            .child(icon_button(
                "forward",
                Icon::Forward,
                "Forward",
                Box::new(Forward),
                theme,
            ))
    }

    fn render_conversation(
        &self,
        thread: &Thread,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips = thread
            .labels
            .iter()
            .filter_map(|id| self.data.label(id))
            .map(|label| label_chip(label, theme));
        let last = thread.messages.len() - 1;

        let messages = thread.messages.iter().enumerate().map(|(ix, message)| {
            if ix == last || self.expanded_messages.contains(&ix) {
                self.render_message(ix, message, ix != last, theme, cx)
            } else {
                self.render_collapsed_message(ix, message, theme, cx)
            }
        });

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("conversation")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.reader_scroll)
                    .px(px(28.))
                    .pt(px(6.))
                    .pb(px(40.))
                    .child(
                        div()
                            .text_size(px(20.))
                            .line_height(relative(1.3))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(thread.subject.clone()),
                    )
                    .child(
                        div()
                            .mt(px(10.))
                            .mb(px(18.))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.))
                            .children(chips)
                            .child(div().text_size(px(12.)).text_color(theme.text_faint).child(
                                match thread.messages.len() {
                                    1 => "1 message".to_string(),
                                    n => format!("{n} messages"),
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.card_bg)
                            .overflow_hidden()
                            .children(messages),
                    ),
            )
            .vertical_scrollbar(&self.reader_scroll)
            .into_any_element()
    }

    fn render_collapsed_message(
        &self,
        ix: usize,
        message: &Message,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(("message", ix))
            .h(px(46.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(10.))
            .when(ix > 0, |this| this.border_t_1())
            .border_color(theme.border)
            .hover(|s| s.bg(theme.hover_bg))
            .child(self.sender_avatar(&message.from, px(24.), theme))
            .child(
                div()
                    .flex_none()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.contact_name(&message.from).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.5))
                    .text_color(theme.text_faint)
                    .child(message.snippet.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(theme.text_faint)
                    .child(full_date(&message.date)),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_message(ix, cx)))
            .into_any_element()
    }

    fn render_message(
        &self,
        ix: usize,
        message: &Message,
        collapsible: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let recipients = |prefix: &str, contacts: &[Contact]| {
            (!contacts.is_empty()).then(|| {
                let names: Vec<&str> = contacts.iter().map(|c| self.contact_name(c)).collect();
                format!("{prefix} {}", names.join(", "))
            })
        };
        let to_line = [recipients("to", &message.to), recipients("cc", &message.cc)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");

        let attachments = message.attachments.iter().map(|attachment| {
            div()
                .h(px(34.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(8.))
                .bg(theme.chip_bg)
                .border_1()
                .border_color(theme.border)
                .child(icon(Icon::File, px(15.), theme.icon))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .child(attachment.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_faint)
                        .child(file_size(attachment.size_bytes)),
                )
        });

        let header = div()
            .id(("message-header", ix))
            .flex()
            .items_start()
            .gap(px(12.))
            .child(self.sender_avatar(&message.from, px(34.), theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.contact_name(&message.from).to_string()),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(theme.text_faint)
                                    .child(message.from.email.clone()),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme.text_muted)
                            .child(to_line),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(theme.text_faint)
                    .child(full_date(&message.date)),
            )
            .when(collapsible, |this| {
                this.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_message(ix, cx)))
            });

        div()
            .px(px(16.))
            .pt(px(16.))
            .pb(px(20.))
            .flex()
            .flex_col()
            .when(ix > 0, |this| this.border_t_1())
            .border_color(theme.border)
            .child(header)
            .child(
                div()
                    .mt(px(14.))
                    .pl(px(46.))
                    .child(render_body(message, theme)),
            )
            .when(!message.attachments.is_empty(), |this| {
                this.child(
                    div()
                        .mt(px(16.))
                        .pl(px(46.))
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .children(attachments),
                )
            })
            .into_any_element()
    }

    fn contact_name<'a>(&self, contact: &'a Contact) -> &'a str {
        if self.data.is_me(contact) {
            "me"
        } else {
            contact.display_name()
        }
    }

    fn sender_avatar(&self, contact: &Contact, size: gpui::Pixels, theme: &Theme) -> gpui::Div {
        avatar(
            contact.initials(),
            theme.avatar_color(&contact.email),
            size,
            theme,
        )
    }
}

/// Seam for the HTML renderer: turn `message.html` into elements. For now the
/// HTML is flattened to plain text.
fn render_body(message: &Message, theme: &Theme) -> impl IntoElement {
    div()
        .text_size(px(14.))
        .line_height(relative(1.55))
        .text_color(theme.text)
        .whitespace_normal()
        .child(html_to_text(&message.html))
}

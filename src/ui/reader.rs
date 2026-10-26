//! The reading pane: toolbar, subject, and the thread as a conversation.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, component::scroll::ScrollableElement as _, div,
    prelude::FluentBuilder as _, px, relative,
};

use super::{
    HEADER_HEIGHT, avatar, empty_state, html_view, icon, icon_button, icon_button_tinted,
    label_chip, window_drag_area,
};
use crate::ui::style::*;
use crate::{
    actions::*,
    app::PigeonApp,
    assets::Icon,
    data::{Contact, Folder, Message, Thread, file_size, full_date},
    html::Document,
    theme::Theme,
};

/// Children of the conversation's scroll view before its first message: the
/// subject and the line under it.
pub const MESSAGES_START: usize = 2;

impl PigeonApp {
    pub(crate) fn render_reader(&self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let thread = self.selected.map(|ix| &self.data.threads[ix]);
        let standalone = self
            .composer
            .as_ref()
            .filter(|_| !self.composer_inline(cx))
            .map(|(composer, _)| composer.clone());

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.reader_bg)
            .map(|this| match standalone {
                Some(composer) => this
                    .child(self.render_compose_toolbar(&composer, &theme, cx))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .child(
                                div()
                                    .id("compose-scroll")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.reader_scroll)
                                    .px(SPACE_XL)
                                    .pt(SPACE_2XS)
                                    .pb(SPACE_2XL)
                                    .child(composer),
                            )
                            .vertical_scrollbar(&self.reader_scroll),
                    ),
                None => this
                    .child(self.render_toolbar(thread, &theme))
                    .child(match thread {
                        Some(thread) => self.render_conversation(thread, &theme, cx),
                        None => empty_state(
                            Icon::Mail,
                            "No conversation selected",
                            Some("Pick a thread from the list to read it here"),
                            &theme,
                        ),
                    }),
            })
    }

    fn render_compose_toolbar(
        &self,
        composer: &gpui::Entity<crate::ui::composer::Composer>,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let composer = composer.read(cx);
        let title = if composer.draft_id.is_some() {
            "Draft"
        } else {
            match composer.kind {
                crate::app::ComposeKind::Forward => "Forward",
                crate::app::ComposeKind::New => "New Message",
                _ => "Reply",
            }
        };
        window_drag_area(div().id("compose-toolbar"))
            .h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .pl(SPACE_XL)
            .pr(SPACE_MD)
            .child(
                div()
                    .text_size(TEXT_LG)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(div().flex_1())
            .child(icon_button(
                "compose-discard",
                Icon::Trash,
                "Discard draft",
                Box::new(DiscardDraft),
                theme,
            ))
            .child(icon_button(
                "compose-close",
                Icon::X,
                "Close and save draft",
                Box::new(CloseDraft),
                theme,
            ))
    }

    fn render_toolbar(&self, thread: Option<&Thread>, theme: &Theme) -> impl IntoElement {
        let header = window_drag_area(div().id("reader-toolbar"))
            .h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .px(SPACE_MD);
        let Some(thread) = thread else {
            return header;
        };

        let separator = || div().w(px(1.)).h(px(16.)).mx(SPACE_SM).bg(theme.border);

        header
            .when_some(thread.folder.archive_target(), |this, target| {
                this.child(icon_button(
                    "archive",
                    if target == Folder::Inbox {
                        Icon::Inbox
                    } else {
                        Icon::Archive
                    },
                    if target == Folder::Inbox {
                        "Move to Inbox"
                    } else {
                        "Archive"
                    },
                    Box::new(Archive),
                    theme,
                ))
            })
            .when(thread.folder != Folder::Trash, |this| {
                this.child(icon_button(
                    "delete",
                    Icon::Trash,
                    if thread.folder == Folder::Drafts {
                        "Discard draft"
                    } else {
                        "Delete"
                    },
                    Box::new(Delete),
                    theme,
                ))
            })
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

        // Each message is a child of the scroll view, so `n` and `p` can
        // scroll to it; together they read as one card.
        let messages = thread.messages.iter().enumerate().map(|(ix, message)| {
            let focused = self.reader_focused && self.focused_message == Some(ix);
            let body = if ix == last || self.expanded_messages.contains(&ix) {
                self.render_message(ix, message, ix != last, theme, cx)
            } else {
                self.render_collapsed_message(ix, message, theme, cx)
            };
            div()
                .id(("message-card", ix))
                .role(gpui::Role::Article)
                .aria_label(format!(
                    "{}, {}: {}",
                    self.contact_name(&message.from),
                    full_date(&message.date),
                    message.snippet
                ))
                .relative()
                .bg(theme.card_bg)
                .border_x_1()
                .border_t_1()
                .border_color(theme.border)
                .when(ix == 0, |d| d.rounded_t(RADIUS_LG))
                .when(ix == last, |d| d.border_b_1().rounded_b(RADIUS_LG))
                .when(focused, |d| d.bg(theme.card_focused_bg))
                .child(body)
                .when(focused, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(-1.))
                            .top(px(10.))
                            .bottom(px(10.))
                            .w(px(3.))
                            .rounded_r(RADIUS_SM)
                            .bg(theme.accent),
                    )
                })
        });
        let messages = messages.collect::<Vec<_>>();
        let composer = self
            .composer
            .as_ref()
            .filter(|_| self.composer_inline(cx))
            .map(|(composer, _)| {
                div()
                    .mt(SPACE_LG)
                    .px(SPACE_LG)
                    .pt(SPACE_MD)
                    .pb(SPACE_LG)
                    .rounded(RADIUS_LG)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.card_bg)
                    .child(composer.clone())
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
                    .px(SPACE_XL)
                    .pt(SPACE_SM)
                    .pb(SPACE_2XL)
                    .child(
                        div()
                            .id("subject")
                            .role(gpui::Role::Heading)
                            .aria_label(thread.subject.clone())
                            .text_size(TEXT_XL)
                            .line_height(relative(1.3))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(thread.subject.clone()),
                    )
                    .child(
                        div()
                            .mt(SPACE_MD)
                            .mb(SPACE_LG)
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(SPACE_SM)
                            .children(chips)
                            .child(div().text_size(TEXT_SM).text_color(theme.text_faint).child(
                                match thread.messages.len() {
                                    1 => "1 message".to_string(),
                                    n => format!("{n} messages"),
                                },
                            )),
                    )
                    .children(messages)
                    .children(composer),
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
            .px(SPACE_LG)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_LG)
            .hover(|s| s.bg(theme.hover_bg))
            .child(self.sender_avatar(&message.from, px(24.), theme))
            .child(
                div()
                    .flex_none()
                    .text_size(TEXT_BASE)
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.contact_name(&message.from).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(message.snippet.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(TEXT_SM)
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

        let body = self.html_cache.get(&message.html);

        let attachments = message.attachments.iter().map(|attachment| {
            div()
                .h(px(34.))
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .bg(theme.chip_bg)
                .child(icon(Icon::File, px(15.), theme.icon))
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .font_weight(FontWeight::MEDIUM)
                        .child(attachment.name.clone()),
                )
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_faint)
                        .child(file_size(attachment.size_bytes)),
                )
        });

        let header = div()
            .id(("message-header", ix))
            .flex()
            .items_start()
            .gap(SPACE_MD)
            .child(self.sender_avatar(&message.from, px(34.), theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(SPACE_SM)
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(TEXT_BASE)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.contact_name(&message.from).to_string()),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(TEXT_SM)
                                    .text_color(theme.text_faint)
                                    .child(message.from.email.clone()),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_muted)
                            .child(to_line),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(full_date(&message.date)),
            )
            .when(collapsible, |this| {
                this.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_message(ix, cx)))
            });

        div()
            .px(SPACE_LG)
            .pt(SPACE_LG)
            .pb(SPACE_XL)
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id(("message-body", ix))
                    .mt(SPACE_LG)
                    .pl(px(46.))
                    .children(
                        message
                            .invite
                            .as_ref()
                            .map(|invite| self.render_invite(invite, theme, cx)),
                    )
                    .child(render_body(&body, theme)),
            )
            .when(!message.attachments.is_empty(), |this| {
                this.child(
                    div()
                        .mt(SPACE_LG)
                        .pl(px(46.))
                        .flex()
                        .flex_wrap()
                        .gap(SPACE_SM)
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

/// The message's HTML, rendered natively. Parsed once per message and cached.
fn render_body(doc: &Document, theme: &Theme) -> impl IntoElement {
    div()
        .text_size(TEXT_MD)
        .line_height(relative(1.55))
        .text_color(theme.text)
        .child(html_view::render(doc, theme))
}

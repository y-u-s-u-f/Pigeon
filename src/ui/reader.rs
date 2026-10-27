//! The reading pane: toolbar, subject, and the thread as a conversation.

use std::rc::Rc;

use gpui::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, base::TextSelection,
    component::scroll::ScrollableElement as _, div, prelude::FluentBuilder as _, px, relative,
};

use super::{
    HEADER_HEIGHT, avatar, empty_state, html_view, icon, icon_button, icon_button_tinted,
    label_chip,
    selectable::{ORDER_BASE, OnClear, Selectable},
    window_drag_area,
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
    pub(crate) fn render_reader(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                // The draft scrolls itself, above its send row.
                Some(composer) => this
                    .child(self.render_compose_toolbar(&composer, &theme, cx))
                    .child(div().flex_1().min_h_0().child(composer)),
                None if !self.checked.is_empty() => this
                    .child(window_drag_area(div().id("reader-toolbar")).h(HEADER_HEIGHT))
                    .child(self.render_bulk_summary(&theme, window)),
                None => this
                    .child(self.render_toolbar(thread, &theme, cx))
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

    fn render_toolbar(
        &self,
        thread: Option<&Thread>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            .child(more_button(theme).on_click(cx.listener(
                |this, event: &gpui::ClickEvent, window, cx| {
                    this.show_thread_menu(event.position(), false, window, cx)
                },
            )))
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
            // Keyed by thread, so the text inside keeps no selection state
            // from another conversation.
            div()
                .id(ElementId::NamedInteger(thread.id.clone().into(), ix as u64))
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
                            .child(
                                Selectable::plain(
                                    ElementId::Name(format!("subject-{}", thread.id).into()),
                                    thread.subject.clone(),
                                    theme.text_selection,
                                )
                                .order(ORDER_BASE),
                            ),
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

        let body = self.html_cache.get(message.html());
        // The invite card above says when, where and who; the lines saying
        // it again in the body fold away.
        let has_card = message.invite.as_ref().is_some_and(|invite| {
            self.calendar
                .read(cx)
                .data
                .event_ix(&invite.event_id)
                .is_some()
        });
        let folds = has_card && body.blocks.iter().any(super::invite::is_invite_summary);
        let show_all = !folds || self.invite_details_open;

        let attachments = message
            .attachments
            .iter()
            .enumerate()
            .map(|(n, attachment)| {
                div()
                    .id(("attachment", ix * 100 + n))
                    .role(gpui::Role::Group)
                    .aria_label(format!(
                        "{}, {}",
                        attachment.name,
                        file_size(attachment.size_bytes)
                    ))
                    .h(px(34.))
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .bg(theme.chip_bg)
                    .child(super::attachment_icon(attachment, theme))
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

        let mine = self.data.is_me(&message.from);
        let order = ORDER_BASE + ((ix as u64 + 1) << 20);
        let header = div()
            .id(("message-header", ix))
            .group("message-header")
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
                                    .child(
                                        Selectable::plain(
                                            "sender-name",
                                            self.contact_name(&message.from).to_string(),
                                            theme.text_selection,
                                        )
                                        .order(order),
                                    ),
                            )
                            // Others' addresses on hover; never my own.
                            .when(!mine, |this| {
                                this.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(TEXT_SM)
                                        .text_color(theme.text_faint)
                                        .opacity(0.)
                                        .group_hover("message-header", |s| s.opacity(1.))
                                        .child(
                                            Selectable::plain(
                                                "sender-address",
                                                message.from.email.clone(),
                                                theme.text_selection,
                                            )
                                            .order(order + 1),
                                        ),
                                )
                            }),
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // Not when the click ended selecting the sender.
                        if TextSelection::selected_text(window, cx).is_empty() {
                            this.toggle_message(ix, cx)
                        }
                    }))
            });
        let selection = html_view::Selection {
            order: order + 2,
            all: (self.select_all_text == Some(ix)).then(|| {
                let app = cx.entity().downgrade();
                Rc::new(move |cx: &mut gpui::App| {
                    _ = app.update(cx, |this, cx| {
                        if this.select_all_text.take().is_some() {
                            cx.notify();
                        }
                    });
                }) as OnClear
            }),
        };

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
                    .child(render_body(&body, show_all, selection, theme))
                    .when(folds, |this| {
                        let open = self.invite_details_open;
                        this.child(
                            div()
                                .id(("invite-details", ix))
                                .mt(SPACE_SM)
                                .flex()
                                .items_center()
                                .gap(SPACE_XS)
                                .text_size(TEXT_SM)
                                .text_color(theme.text_muted)
                                .cursor_pointer()
                                .hover(|s| s.text_color(theme.text))
                                .child(icon(
                                    if open {
                                        Icon::ChevronDown
                                    } else {
                                        Icon::ChevronRight
                                    },
                                    px(13.),
                                    theme.text_faint,
                                ))
                                .child(if open {
                                    "Hide invitation details"
                                } else {
                                    "Show invitation details"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.invite_details_open = !this.invite_details_open;
                                    cx.notify();
                                })),
                        )
                    }),
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

    /// In place of a conversation while several are selected: what they
    /// are, and what can be done to all of them.
    fn render_bulk_summary(&self, theme: &Theme, window: &Window) -> AnyElement {
        let targets = self.targets();
        let count = targets.len();
        let all_unread = targets.iter().all(|&ix| self.data.threads[ix].unread);
        let all_starred = targets.iter().all(|&ix| self.data.threads[ix].starred);
        let archive_label = match targets
            .first()
            .and_then(|&ix| self.data.threads[ix].folder.archive_target())
        {
            Some(Folder::Inbox) => "Move to Inbox",
            _ => "Archive",
        };
        let subjects = targets.iter().take(5).map(|&ix| {
            let thread = &self.data.threads[ix];
            div()
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .min_w_0()
                .child(icon(Icon::Mail, px(13.), theme.text_faint))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(if thread.subject.is_empty() {
                            "(no subject)".to_string()
                        } else {
                            thread.subject.clone()
                        }),
                )
        });
        let action =
            |id: &'static str, glyph: Icon, label: &'static str, action: Box<dyn gpui::Action>| {
                let caps = self
                    .binding_for(action.as_ref(), window)
                    .map(|b| super::binding_caps(&b, theme));
                focusable_control(div().id(id), theme)
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .h(px(36.))
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .bg(theme.chip_bg)
                    .hover(|s| s.bg(theme.selected_bg))
                    .active(|s| s.opacity(0.8))
                    .text_size(TEXT_BASE)
                    .child(icon(glyph, px(15.), theme.icon))
                    .child(div().flex_1().child(label))
                    .children(caps)
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            };
        div()
            .id("bulk-summary")
            .role(gpui::Role::Region)
            .aria_label(format!("{count} conversations selected"))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(72.))
            .px(SPACE_XL)
            .child(
                div()
                    .w_full()
                    .max_w(px(440.))
                    .flex()
                    .flex_col()
                    .gap(SPACE_LG)
                    .child(
                        div()
                            .text_size(TEXT_XL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(match count {
                                1 => "1 conversation selected".to_string(),
                                n => format!("{n} conversations selected"),
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SPACE_XS)
                            .text_size(TEXT_SM)
                            .text_color(theme.text_muted)
                            .children(subjects)
                            .when(count > 5, |d| {
                                d.child(
                                    div()
                                        .pl(px(21.))
                                        .text_color(theme.text_faint)
                                        .child(format!("and {} more", count - 5)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SPACE_XS)
                            .child(action(
                                "bulk-archive",
                                Icon::Archive,
                                archive_label,
                                Box::new(Archive),
                            ))
                            .child(action(
                                "bulk-delete",
                                Icon::Trash,
                                "Delete",
                                Box::new(Delete),
                            ))
                            .child(if all_unread {
                                action(
                                    "bulk-read",
                                    Icon::MailOpen,
                                    "Mark as Read",
                                    Box::new(MarkRead),
                                )
                            } else {
                                action(
                                    "bulk-unread",
                                    Icon::MailUnread,
                                    "Mark as Unread",
                                    Box::new(MarkUnread),
                                )
                            })
                            .child(action(
                                "bulk-star",
                                Icon::Star,
                                if all_starred { "Unstar" } else { "Star" },
                                Box::new(ToggleStar),
                            ))
                            .child(action(
                                "bulk-move",
                                Icon::FolderInput,
                                "Move to…",
                                Box::new(MoveTo),
                            ))
                            .child(action(
                                "bulk-label",
                                Icon::Tag,
                                "Label…",
                                Box::new(ApplyLabel),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .text_size(TEXT_SM)
                            .text_color(theme.text_faint)
                            .child(super::keycap("Esc", theme))
                            .child("to clear the selection"),
                    ),
            )
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
/// Without `all`, the lines an invite card summarizes are left out.
fn render_body(
    doc: &Document,
    all: bool,
    selection: html_view::Selection,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .text_size(TEXT_MD)
        .line_height(relative(1.55))
        .text_color(theme.text)
        .child(html_view::render_filtered(
            doc,
            theme,
            Some(selection),
            |block| all || !super::invite::is_invite_summary(block),
        ))
}

/// The toolbar's "…": the rest of the thread actions, in a menu.
fn more_button(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    focusable_control(div().id("more-actions"), theme)
        .aria_label("More actions")
        .role(gpui::Role::Button)
        .size(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(icon(Icon::Ellipsis, px(16.), theme.icon))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(|window, cx| {
            gpui::component::tooltip::Tooltip::new("More: move, label, mark read").build(window, cx)
        })
}

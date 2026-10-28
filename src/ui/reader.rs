//! The reading pane: toolbar, subject, and the thread as a conversation.

use std::{rc::Rc, sync::Arc};

use gpui::{
    AnyElement, Context, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, UniformListScrollHandle,
    Window, base::TextSelection, component::scroll::ScrollableElement as _, div,
    prelude::FluentBuilder as _, px, relative, uniform_list,
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
    data::{Body, Contact, Folder, Message, Thread, file_size, full_date},
    html::Document,
    settings::RemoteImages,
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
        // W2 (RDR-8): the source, while its conversation is selected.
        let original = self
            .original
            .as_ref()
            .filter(|o| thread.is_some_and(|t| t.id == o.thread_id));
        if self.original.is_some() && original.is_none() {
            cx.defer_in(window, |this, window, cx| {
                if let Some(original) = this.original.take() {
                    if original.focus.contains_focused(window, cx) {
                        window.focus(&this.focus_handle, cx);
                    }
                    cx.notify();
                }
            });
        }

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
                None if original.is_some() => {
                    this.child(render_original(original.expect("checked"), &theme))
                }
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
            .child(if thread.important {
                // W4b (FOL-12).
                icon_button_tinted(
                    "important",
                    Icon::Important,
                    theme.star,
                    "Mark as not important",
                    Box::new(ToggleImportant),
                    theme,
                )
            } else {
                icon_button(
                    "important",
                    Icon::Important,
                    "Mark as important",
                    Box::new(ToggleImportant),
                    theme,
                )
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
        // W6: each name opens its contact card (CON-5).
        let mut to_line: Vec<AnyElement> = Vec::new();
        for (group, (prefix, contacts)) in [("to", &message.to), ("cc", &message.cc)]
            .into_iter()
            .filter(|(_, contacts)| !contacts.is_empty())
            .enumerate()
        {
            if group > 0 {
                to_line.push(div().mx(SPACE_XS).child("·").into_any_element());
            }
            to_line.push(div().mr(SPACE_XS).child(prefix).into_any_element());
            for (n, contact) in contacts.iter().enumerate() {
                let name = self.contact_name(contact).to_string();
                let last = n + 1 == contacts.len();
                to_line.push(
                    self.contact_link(
                        ("recipient", ix * 10_000 + group * 5_000 + n),
                        contact,
                        &name,
                        theme,
                        cx,
                    )
                    .child(if last { name } else { format!("{name},") })
                    .when(!last, |d| d.mr(SPACE_XS))
                    .into_any_element(),
                );
            }
        }

        let body = self.html_cache.get(message.html());
        let images = self.message_images(message, &body, cx);

        let attachments = message
            .attachments
            .iter()
            .enumerate()
            .map(|(n, attachment)| {
                let size = file_size(attachment.size_bytes);
                focusable_control(div().id(("attachment", ix * 100 + n)), theme)
                    .role(gpui::Role::Button)
                    .aria_label(format!("Open {}, {size}", attachment.name))
                    .h(px(34.))
                    .pl(SPACE_MD)
                    .pr(SPACE_XS)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .bg(theme.chip_bg)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.selected_bg))
                    .child(super::attachment_icon(attachment, theme))
                    .child(
                        div()
                            .max_w(px(260.))
                            .truncate()
                            .text_size(TEXT_SM)
                            .font_weight(FontWeight::MEDIUM)
                            .child(attachment.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_faint)
                            .child(size),
                    )
                    .child(
                        div()
                            .id(("attachment-save", ix * 100 + n))
                            .role(gpui::Role::Button)
                            .aria_label(format!("Save {}", attachment.name))
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS_SM)
                            .hover(|s| s.bg(theme.hover_bg))
                            .child(icon(Icon::Download, px(13.), theme.text_faint))
                            .tooltip(|window, cx| {
                                gpui::component::tooltip::Tooltip::new("Save…").build(window, cx)
                            })
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                window.dispatch_action(Box::new(SaveAttachmentAt(ix, n)), cx)
                            }),
                    )
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(OpenAttachmentAt(ix, n)), cx)
                    })
            });

        let mine = self.data.is_me(&message.from);
        let order = ORDER_BASE + ((ix as u64 + 1) << 20);
        let header = div()
            .id(("message-header", ix))
            .group("message-header")
            .flex()
            .items_start()
            .gap(SPACE_MD)
            .child(
                self.contact_link(
                    ("sender-avatar", ix),
                    &message.from,
                    self.contact_name(&message.from),
                    theme,
                    cx,
                )
                .flex_none()
                .child(self.sender_avatar(&message.from, px(34.), theme)),
            )
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
                                self.contact_link(
                                    ("sender-name", ix),
                                    &message.from,
                                    self.contact_name(&message.from),
                                    theme,
                                    cx,
                                )
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
                            .flex()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_muted)
                            .children(to_line),
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
                    .when_some(images.ask, |this, hidden| {
                        this.child(render_images_bar(ix, &message.id, hidden, theme, cx))
                    })
                    .child(match &message.body {
                        Body::Loaded { .. } => render_body(
                            &body,
                            selection,
                            html_view::ShowImages {
                                loaded: &self.images,
                                remote: images.show,
                            },
                            theme,
                        )
                        .into_any_element(),
                        Body::Failed(reason) => render_body_failed(ix, reason, theme),
                        Body::NotLoaded | Body::Loading => render_body_loading(theme),
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

    /// W6: a name or avatar that opens `contact`'s card (CON-5); not when
    /// the click ended selecting text.
    fn contact_link(
        &self,
        id: impl Into<ElementId>,
        contact: &Contact,
        name: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let contact = contact.clone();
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(format!("Show contact: {name}"))
            .cursor_pointer()
            .rounded(RADIUS_SM)
            .hover(|s| s.text_color(theme.text))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if TextSelection::selected_text(window, cx).is_empty() {
                    this.show_contact(contact.clone(), window, cx);
                }
            }))
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
fn render_body(
    doc: &Document,
    selection: html_view::Selection,
    images: html_view::ShowImages,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .text_size(TEXT_MD)
        .line_height(relative(1.55))
        .text_color(theme.text)
        .child(html_view::render_with(
            doc,
            theme,
            Some(selection),
            Some(images),
        ))
}

// Remote images (W10, ATT-1).

impl PigeonApp {
    /// A message's remote images: fetched (after this frame) once the user
    /// let them load, or always per the setting.
    fn message_images(
        &self,
        message: &Message,
        body: &Document,
        cx: &mut Context<Self>,
    ) -> ImageConsent {
        if !matches!(message.body, Body::Loaded { .. }) {
            return ImageConsent::default();
        }
        // W2 (ATT-2): inline images, part of the mail, need no consent.
        let inline: Vec<SharedString> = html_view::cid_images(body)
            .into_iter()
            .filter(|url| !self.images.requested.contains(url))
            .collect();
        if !inline.is_empty()
            && let Some(thread) = self.selected.map(|ix| self.data.threads[ix].id.clone())
        {
            let this = cx.entity().downgrade();
            let message_id = message.id.clone();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| {
                    this.load_inline_images(&thread, &message_id, inline, cx)
                })
                .ok();
            });
        }
        if self.backend.read(cx).is_demo() {
            return ImageConsent::default();
        }
        let remote = crate::html::remote_images(body);
        let consent = image_consent(
            self.remote_images_setting(),
            self.images.allowed.contains(&message.id),
            remote.len(),
        );
        if !consent.show {
            return consent;
        }
        let missing: Vec<SharedString> = remote
            .into_iter()
            .filter(|url| !self.images.requested.contains(url))
            .collect();
        if !missing.is_empty() {
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.fetch_images(missing, cx))
                    .ok();
            });
        }
        consent
    }

    /// Let a message's remote images load.
    pub(crate) fn allow_images(&mut self, message_id: &str, cx: &mut Context<Self>) {
        if self.images.allowed.insert(message_id.to_string()) {
            cx.notify();
        }
    }

    fn fetch_images(&mut self, mut urls: Vec<SharedString>, cx: &mut Context<Self>) {
        urls.retain(|url| self.images.requested.insert(url.clone()));
        if urls.is_empty() {
            return;
        }
        let task = self.backend.update(cx, |b, cx| {
            b.fetch_images(urls.iter().map(|u| u.to_string()).collect(), cx)
        });
        cx.spawn(async move |this, cx| {
            let fetched = task.await;
            this.update(cx, |this, cx| this.images_ready(fetched, cx))
                .ok();
        })
        .detach();
    }

    /// W2 (ATT-2): a message's inline images, from the cache.
    fn load_inline_images(
        &mut self,
        thread_id: &str,
        message_id: &str,
        mut urls: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        urls.retain(|url| self.images.requested.insert(url.clone()));
        if urls.is_empty() {
            return;
        }
        let task = self.backend.update(cx, |b, cx| {
            b.inline_images(
                thread_id,
                message_id,
                urls.iter().map(|u| u.to_string()).collect(),
                cx,
            )
        });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            this.update(cx, |this, cx| this.images_ready(found, cx))
                .ok();
        })
        .detach();
    }

    fn images_ready(
        &mut self,
        images: Vec<(String, Option<crate::backend::images::FetchedImage>)>,
        cx: &mut Context<Self>,
    ) {
        for (url, image) in images {
            let Some(image) = image else { continue };
            let Some(format) = gpui::ImageFormat::from_mime_type(image.mime) else {
                continue;
            };
            self.images.ready.insert(
                url.into(),
                html_view::ReadyImage {
                    image: Arc::new(gpui::Image::from_bytes(format, image.bytes)),
                    width: image.width as f32,
                    height: image.height as f32,
                },
            );
        }
        cx.notify();
    }
}

/// Whether a message shows its remote images, and how many the "Load images"
/// bar says are hidden. Consent is per message: an image another message
/// already loaded stays hidden here until this one is allowed, so the count
/// is always true.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ImageConsent {
    show: bool,
    ask: Option<usize>,
}

fn image_consent(setting: RemoteImages, allowed: bool, remote: usize) -> ImageConsent {
    if remote == 0 {
        return ImageConsent::default();
    }
    let show = setting == RemoteImages::Always || allowed;
    ImageConsent {
        show,
        ask: (!show && setting == RemoteImages::Ask).then_some(remote),
    }
}

/// Above a message whose remote images wait for the user (ATT-1).
fn render_images_bar(
    ix: usize,
    message_id: &str,
    hidden: usize,
    theme: &Theme,
    cx: &mut Context<PigeonApp>,
) -> AnyElement {
    let id = message_id.to_string();
    div()
        .id(("images-bar", ix))
        .role(gpui::Role::Region)
        .aria_label("Remote images hidden")
        .mb(SPACE_MD)
        .px(SPACE_MD)
        .py(SPACE_SM)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .bg(theme.chip_bg)
        .text_size(TEXT_SM)
        .text_color(theme.text_muted)
        .child(icon(Icon::Image, px(14.), theme.text_faint))
        .child(div().flex_1().min_w_0().child(match hidden {
            1 => "1 remote image is hidden to protect your privacy".to_string(),
            n => format!("{n} remote images are hidden to protect your privacy"),
        }))
        .child(
            focusable_control(div().id(("images-load", ix)), theme)
                .role(gpui::Role::Button)
                .aria_label("Load images")
                .flex_none()
                .h(HIT_TARGET)
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .hover(|s| s.bg(theme.selected_bg))
                .text_color(theme.text)
                .font_weight(FontWeight::MEDIUM)
                .child("Load images")
                .child(super::keycap("⇧⌘I", theme))
                .on_click(cx.listener(move |this, _, _, cx| this.allow_images(&id, cx))),
        )
        .into_any_element()
}

// The original message (W2, RDR-8).

/// A message's source, read-only, in place of its conversation.
pub struct Original {
    pub thread_id: String,
    pub message_id: String,
    pub subject: String,
    pub state: OriginalState,
    pub focus: FocusHandle,
    pub scroll: UniformListScrollHandle,
}

pub enum OriginalState {
    Loading,
    Failed(SharedString),
    Loaded {
        /// The whole message, for Save as .eml.
        raw: Arc<Vec<u8>>,
        /// Its first `ORIGINAL_SHOWN` bytes as text, for Copy.
        shown: SharedString,
        lines: Arc<Vec<SharedString>>,
        truncated: bool,
    },
}

impl Original {
    pub fn new(thread_id: String, message_id: String, subject: String, cx: &mut gpui::App) -> Self {
        Self {
            thread_id,
            message_id,
            subject,
            state: OriginalState::Loading,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        }
    }
}

impl OriginalState {
    pub fn loaded(raw: Vec<u8>) -> Self {
        let limit = crate::backend::attachments::ORIGINAL_SHOWN;
        let truncated = raw.len() > limit;
        let shown = String::from_utf8_lossy(&raw[..raw.len().min(limit)]).into_owned();
        OriginalState::Loaded {
            lines: Arc::new(source_lines(&shown)),
            shown: shown.into(),
            raw: Arc::new(raw),
            truncated,
        }
    }
}

/// The source as lines to show: tabs as spaces, long lines broken.
fn source_lines(text: &str) -> Vec<SharedString> {
    const WRAP: usize = 100;
    let mut out = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r').replace('\t', "    ");
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            out.push(SharedString::default());
        }
        for chunk in chars.chunks(WRAP) {
            out.push(chunk.iter().collect::<String>().into());
        }
    }
    if text.ends_with('\n') {
        out.pop();
    }
    out
}

fn render_original(original: &Original, theme: &Theme) -> AnyElement {
    let toolbar = window_drag_area(div().id("original-toolbar"))
        .h(HEADER_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .gap(SPACE_2XS)
        .pl(SPACE_XL)
        .pr(SPACE_MD)
        .child(
            div()
                .flex_none()
                .text_size(TEXT_LG)
                .font_weight(FontWeight::SEMIBOLD)
                .child("Original Message"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .pl(SPACE_SM)
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
                .child(original.subject.clone()),
        )
        .child(icon_button(
            "original-copy",
            Icon::FileText,
            "Copy",
            Box::new(CopyOriginal),
            theme,
        ))
        .child(icon_button(
            "original-save",
            Icon::Download,
            "Save as .eml",
            Box::new(SaveOriginal),
            theme,
        ))
        .child(icon_button(
            "original-close",
            Icon::X,
            "Close original",
            Box::new(CloseOriginal),
            theme,
        ));
    let note = |text: SharedString| {
        div()
            .px(SPACE_XL)
            .py(SPACE_SM)
            .text_size(TEXT_SM)
            .text_color(theme.text_faint)
            .child(text)
    };
    let body = match &original.state {
        OriginalState::Loading => note("Loading the original…".into()).into_any_element(),
        OriginalState::Failed(reason) => {
            note(format!("Couldn't load the original. {reason}").into()).into_any_element()
        }
        OriginalState::Loaded {
            lines, truncated, ..
        } => {
            let lines = lines.clone();
            let count = lines.len();
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .when(*truncated, |this| {
                    this.child(note(
                        "Showing the first 1 MB. Save as .eml for the whole message.".into(),
                    ))
                })
                .child(
                    div().flex_1().min_h_0().px(SPACE_XL).pb(SPACE_LG).child(
                        uniform_list("original-lines", count, move |range, _, _| {
                            range
                                .map(|ix| {
                                    div()
                                        .whitespace_nowrap()
                                        .truncate()
                                        .child(lines[ix].clone())
                                })
                                .collect::<Vec<_>>()
                        })
                        .track_scroll(&original.scroll)
                        .size_full(),
                    ),
                )
                .into_any_element()
        }
    };
    div()
        .id("original")
        .key_context(ORIGINAL)
        .track_focus(&original.focus)
        .role(gpui::Role::Region)
        .aria_label("Original message")
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(toolbar)
        .child(
            div()
                .id("original-source")
                .role(gpui::Role::Document)
                .aria_label("Message source")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .font_family("Menlo")
                .text_size(TEXT_SM)
                .line_height(relative(1.5))
                .text_color(theme.text_muted)
                .child(body),
        )
        .into_any_element()
}

/// In place of a body still on its way (RDR-6).
fn render_body_loading(theme: &Theme) -> AnyElement {
    div()
        .id("message-loading")
        .role(gpui::Role::Status)
        .aria_label("Loading message")
        .text_size(TEXT_BASE)
        .text_color(theme.text_faint)
        .child("Loading…")
        .into_any_element()
}

/// A body that didn't load, with why and a way to try again.
fn render_body_failed(ix: usize, reason: &str, theme: &Theme) -> AnyElement {
    div()
        .id(("message-failed", ix))
        .role(gpui::Role::Alert)
        .aria_label(format!("Couldn't load this message. {reason}"))
        .flex()
        .flex_col()
        .items_start()
        .gap(SPACE_SM)
        .child(
            div()
                .text_size(TEXT_BASE)
                .text_color(theme.text_muted)
                .child("Couldn't load this message"),
        )
        .child(
            div()
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
                .child(reason.to_string()),
        )
        .child(
            focusable_control(div().id(("message-retry", ix)), theme)
                .role(gpui::Role::Button)
                .aria_label("Retry loading message")
                .h(HIT_TARGET)
                .px(SPACE_MD)
                .flex()
                .items_center()
                .rounded(RADIUS_MD)
                .bg(theme.chip_bg)
                .hover(|s| s.bg(theme.selected_bg))
                .text_size(TEXT_SM)
                .child("Retry")
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(RetryLoadingMessage), cx)
                }),
        )
        .into_any_element()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_show_only_with_consent_for_this_message() {
        let ask = image_consent(RemoteImages::Ask, false, 5);
        assert_eq!(
            ask,
            ImageConsent {
                show: false,
                ask: Some(5)
            }
        );
        let allowed = image_consent(RemoteImages::Ask, true, 5);
        assert_eq!(
            allowed,
            ImageConsent {
                show: true,
                ask: None
            }
        );
        let always = image_consent(RemoteImages::Always, false, 5);
        assert_eq!(
            always,
            ImageConsent {
                show: true,
                ask: None
            }
        );
        let never = image_consent(RemoteImages::Never, false, 5);
        assert_eq!(never, ImageConsent::default());
        assert_eq!(
            image_consent(RemoteImages::Ask, false, 0),
            ImageConsent::default()
        );
    }
}

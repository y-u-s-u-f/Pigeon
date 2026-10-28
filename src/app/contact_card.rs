//! Sprint 2 W6: the contact card (CON-5) over the panes, from the local
//! cache only (`Backend::contact_card`). The card itself is
//! `ui::contact_card`.

use gpui::{ClipboardItem, Context, Div, Entity, Focusable as _, Stateful, Window, prelude::*};

use super::{ComposeKind, Mode, PigeonApp, ToastKind};
use crate::{
    actions::ShowContact,
    data::Contact,
    ui::contact_card::{CardEvent, ContactCardView},
};

impl PigeonApp {
    /// Registers the handlers of this module's actions on the root.
    pub(super) fn on_contact_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(|this, _: &ShowContact, window, cx| {
            if let Some(contact) = this.contact_target() {
                this.show_contact(contact, window, cx);
            }
        }))
    }

    /// Who "Show Contact" is about: the sender of the focused message
    /// while the reader has focus, else of the newest; for the user's own
    /// mail, the first other person on it.
    fn contact_target(&self) -> Option<Contact> {
        if self.mode != Mode::Mail {
            return None;
        }
        let thread = &self.data.threads[self.selected?];
        let last = thread.messages.len().checked_sub(1)?;
        let m = self
            .focused_message
            .filter(|&m| self.reader_focused && m <= last)
            .unwrap_or(last);
        let message = &thread.messages[m];
        if !self.data.is_me(&message.from) {
            return Some(message.from.clone());
        }
        message
            .to
            .iter()
            .chain(&message.cc)
            .find(|c| !self.data.is_me(c))
            .or(Some(&message.from))
            .cloned()
    }

    /// The card for `contact`, over everything.
    pub(crate) fn show_contact(
        &mut self,
        contact: Contact,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let card = self
            .backend
            .read(cx)
            .contact_card(&contact.email, &contact.name);
        self.palette = None;
        self.help_open = false;
        let view = cx.new(|cx| ContactCardView::new(card, cx));
        let events = cx.subscribe_in(&view, window, Self::on_card_event);
        window.focus(&view.focus_handle(cx), cx);
        self.contact_card = Some((view, events));
        cx.notify();
    }

    fn on_card_event(
        &mut self,
        _: &Entity<ContactCardView>,
        event: &CardEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlays(window, cx);
        match event {
            CardEvent::OpenThread(id) => {
                self.set_mode(Mode::Mail, window, cx);
                if let Some(ix) = self.data.thread_ix(id) {
                    self.reveal_thread(ix, window, cx);
                }
            }
            CardEvent::Compose(contact) => self.compose_to(contact.clone(), window, cx),
            CardEvent::Copy(email) => {
                cx.write_to_clipboard(ClipboardItem::new_string(email.clone()));
                self.show_toast(ToastKind::Done, format!("Copied {email}"), None, cx);
            }
            CardEvent::Dismiss => {}
        }
    }

    /// A new message to `contact`, writing from the subject on.
    fn compose_to(&mut self, contact: Contact, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Mail, window, cx);
        self.compose(ComposeKind::New, window, cx);
        let Some((composer, _)) = &self.composer else {
            return;
        };
        let composer = composer.clone();
        composer.update(cx, |c, cx| {
            let mut content = c.content(cx);
            content.to = vec![contact];
            c.load(content, cx);
        });
        // The recipient is in once the composer has rendered with it.
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                composer.update(cx, |c, cx| c.focus_first(window, cx));
            });
        });
    }
}

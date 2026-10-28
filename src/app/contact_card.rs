//! Sprint 2 W6: the contact card (CON-5) over the panes, from the local
//! cache only (`Backend::contact_card`). The card itself is
//! `ui::contact_card`.

use gpui::{
    ClipboardItem, Context, Div, Entity, Focusable as _, Pixels, Point, Stateful, Window,
    prelude::*,
};

use super::{Mode, PigeonApp, ToastKind};
use crate::{
    actions::{SearchFromContact, ShowContact},
    data::Contact,
    ui::{
        composer::DraftContent,
        contact_card::{CardEvent, ContactCardView},
    },
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
                this.show_contact(contact, None, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &SearchFromContact, window, cx| {
            let Some((card, _)) = &this.contact_card else {
                return;
            };
            let email = card.read(cx).contact().email;
            this.search_from(&email, window, cx);
        }))
    }

    /// ME-033: the card's Search Mail, as "from:" them across all mail.
    fn search_from(&mut self, email: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        self.set_mode(Mode::Mail, window, cx);
        self.search_open = true;
        self.search_all = true;
        self.set_search_text(&format!("from:{email} "), window, cx);
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
        self.remember_search(cx);
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

    /// The card for `contact`, over everything; hung at `position` when a
    /// click on a name opened it (MB-069).
    pub(crate) fn show_contact(
        &mut self,
        contact: Contact,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let card = self
            .backend
            .read(cx)
            .contact_card(&contact.email, &contact.name);
        self.palette = None;
        self.help_open = false;
        let view = cx.new(|cx| {
            let view = ContactCardView::new(card, cx);
            match position {
                Some(position) => view.anchored_at(position),
                None => view,
            }
        });
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
        let content = DraftContent {
            to: vec![contact],
            ..Default::default()
        };
        self.compose_with(content, window, cx);
    }
}

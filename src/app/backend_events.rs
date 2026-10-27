//! What the backend reports, put into effect. One arm per `BackendEvent`
//! variant, each calling its own function below, so workstreams change only
//! theirs: mail (W3, W5, W6), calendar (W4, W7), contacts (W8), status and
//! sign-in (W2, W11). Demo mode sends only `OpCommitted` and `Status`.

use gpui::{Context, Entity};

use super::{Nav, PigeonApp};
use crate::backend::{Backend, BackendEvent, OpId, SyncStatus};

impl PigeonApp {
    pub(super) fn on_backend_event(
        &mut self,
        _: Entity<Backend>,
        event: &BackendEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            BackendEvent::ThreadsChanged(ids) => self.on_threads_changed(ids, cx),
            BackendEvent::ThreadsRemoved(ids) => self.on_threads_removed(ids, cx),
            BackendEvent::IdReplaced { old, new } => self.on_id_replaced(old, new, cx),
            BackendEvent::LabelsChanged => self.on_labels_changed(cx),
            BackendEvent::CountsChanged => self.on_counts_changed(cx),
            BackendEvent::EventsChanged(ids) => self.on_events_changed(ids, cx),
            BackendEvent::EventsRemoved(ids) => self.on_events_removed(ids, cx),
            BackendEvent::CalendarsChanged => self.on_calendars_changed(cx),
            BackendEvent::ContactsChanged => self.on_contacts_changed(cx),
            BackendEvent::Status(status) => self.on_status(status, cx),
            BackendEvent::OpCommitted(op) => self.on_op_committed(*op, cx),
            BackendEvent::OpFailed { op, message } => self.on_op_failed(*op, message, cx),
            BackendEvent::SignedIn => self.on_signed_in(cx),
            BackendEvent::SignedOut => self.on_signed_out(cx),
        }
    }

    // Mail.

    /// Reload the threads from the store, in place; new ones join the list.
    /// The selection stays on its thread.
    fn on_threads_changed(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let threads = self.backend.read(cx).load_threads(ids);
        let gone: Vec<String> = ids
            .iter()
            .filter(|id| !threads.iter().any(|t| &t.id == *id))
            .cloned()
            .collect();
        // W11 (STA-5): the open thread keeps its expanded and focused messages.
        let open = self.open_messages(ids);
        self.update_threads(|data| data.upsert_threads(threads));
        self.restore_open_messages(open);
        if !gone.is_empty() {
            self.on_threads_removed(&gone, cx);
        }
        // W6: a Gmail draft's text arrived for the open composer.
        self.refresh_open_draft(ids, cx);
        cx.notify();
    }

    fn on_threads_removed(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let selected_goes = self
            .selected
            .is_some_and(|ix| ids.contains(&self.data.threads[ix].id));
        if selected_goes {
            self.selected = None;
        }
        self.update_threads(|data| data.threads.retain(|t| !ids.contains(&t.id)));
        self.checked.retain(|id| !ids.contains(id));
        cx.notify();
    }

    /// A local id the server replaced: the thread, and whatever points at
    /// it, take the new one.
    fn on_id_replaced(&mut self, old: &str, new: &str, cx: &mut Context<Self>) {
        if let Some(ix) = self.data.thread_ix(old) {
            self.data.threads[ix].id = new.to_string();
        }
        for thread in &mut self.data.threads {
            if let Some(origin) = &mut thread.origin
                && origin.reply_to.as_deref() == Some(old)
            {
                origin.reply_to = Some(new.to_string());
            }
        }
        if self.checked.remove(old) {
            self.checked.insert(new.to_string());
        }
        if let Some((composer, _)) = &self.composer {
            composer.update(cx, |c, _| {
                for id in [&mut c.draft_id, &mut c.reply_to].into_iter().flatten() {
                    if id == old {
                        *id = new.to_string();
                    }
                }
            });
        }
        cx.notify();
    }

    fn on_labels_changed(&mut self, cx: &mut Context<Self>) {
        let labels = self.backend.read(cx).load_labels();
        // A label view stays on its label, by id.
        let nav_label = match self.nav {
            Nav::Label(ix) => self.data.labels.get(ix).map(|l| l.id.clone()),
            _ => None,
        };
        self.data.labels = labels;
        if let Some(id) = nav_label {
            match self.data.labels.iter().position(|l| l.id == id) {
                Some(ix) => self.nav = Nav::Label(ix),
                None => self.set_nav(Nav::Folder(crate::data::Folder::Inbox), cx),
            }
        }
        // Labels come with the account's aliases (sendAs), which `is_me`
        // checks.
        if let Some(account) = self.backend.read(cx).account() {
            self.data.account.aliases = account.aliases.clone();
        }
        self.refresh_server_counts(cx);
        cx.notify();
    }

    /// Gmail's counts changed, or a view finished loading.
    fn on_counts_changed(&mut self, cx: &mut Context<Self>) {
        self.refresh_server_counts(cx);
        cx.notify();
    }

    // Calendar.

    /// Reload the events from the store, in place; new ones join, and ones
    /// no longer stored go. An event shared into several calendars has a
    /// copy in each, under one id. Selection and the view stay put: they
    /// go by id and date.
    fn on_events_changed(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let mut fresh = self.backend.read(cx).load_events(ids);
        let gone: Vec<String> = ids
            .iter()
            .filter(|id| !fresh.iter().any(|e| &e.id == *id))
            .cloned()
            .collect();
        self.calendar.update(cx, |calendar, cx| {
            calendar.data.events.retain_mut(|event| {
                if !ids.contains(&event.id) {
                    return true;
                }
                match fresh.iter().position(|e| e.id == event.id) {
                    Some(ix) => {
                        *event = fresh.remove(ix);
                        true
                    }
                    None => false,
                }
            });
            calendar.data.events.append(&mut fresh);
            cx.notify();
        });
        if !gone.is_empty() {
            self.on_events_removed(&gone, cx);
        }
    }

    fn on_events_removed(&mut self, ids: &[String], cx: &mut Context<Self>) {
        self.calendar.update(cx, |calendar, cx| {
            calendar.data.events.retain(|e| !ids.contains(&e.id));
            if calendar
                .selected
                .as_ref()
                .is_some_and(|s| ids.contains(&s.event_id))
            {
                calendar.selected = None;
            }
            cx.notify();
        });
    }

    fn on_calendars_changed(&mut self, cx: &mut Context<Self>) {
        let calendars = self.backend.read(cx).load_calendar().calendars;
        self.calendar.update(cx, |calendar, cx| {
            calendar.data.calendars = calendars;
            cx.notify();
        });
    }

    // Contacts. Composers and the event panel share the book, so open
    // ones see the change at once (CON-1).

    fn on_contacts_changed(&mut self, cx: &mut Context<Self>) {
        let contacts = self.backend.read(cx).contacts();
        self.contact_book.borrow_mut().update(contacts);
    }

    // Status, the outbox and signing in.

    /// Also the account's name, which the profile check may have updated.
    fn on_status(&mut self, _status: &SyncStatus, cx: &mut Context<Self>) {
        if let Some(account) = self.backend.read(cx).account().cloned() {
            self.calendar.update(cx, |calendar, _| {
                calendar.data.account = account.clone();
            });
            self.data.account = account;
        }
        cx.notify();
    }

    fn on_op_committed(&mut self, op: OpId, cx: &mut Context<Self>) {
        // W6: "Sending…" becomes "Message sent".
        self.on_send_committed(op, cx);
    }

    /// The backend put the server's state back already (as `ThreadsChanged`
    /// or `EventsChanged`); say why.
    fn on_op_failed(&mut self, op: OpId, message: &str, cx: &mut Context<Self>) {
        self.sends.retain(|s| *s != op);
        self.show_error_toast(message.to_string(), cx);
    }

    /// Signed in, signed out or switched to the demo data: everything
    /// comes from the new source.
    fn on_signed_in(&mut self, cx: &mut Context<Self>) {
        self.reload_all(cx);
    }

    fn on_signed_out(&mut self, cx: &mut Context<Self>) {
        self.mode = super::Mode::Mail;
        self.reload_all(cx);
    }

    fn reload_all(&mut self, cx: &mut Context<Self>) {
        let (data, calendar, contacts) = {
            let b = self.backend.read(cx);
            (b.load_mail(), b.load_calendar(), b.contacts())
        };
        self.composer = None;
        self.selected = None;
        self.focused_message = None;
        self.expanded_messages.clear();
        self.checked.clear();
        self.toast = None;
        self.data = data;
        self.calendar.update(cx, |view, cx| {
            view.data = calendar;
            view.selected = None;
            cx.notify();
        });
        self.contact_book.borrow_mut().update(contacts);
        // W9: Gmail search and paging, signed in.
        self.server_search = Default::default();
        self.server_search.available = self.backend.read(cx).can_search();
        self.refresh_rows(false);
        cx.notify();
    }

    // W11 (STA-5): the reader's message state goes by index; across a
    // reload it goes by message id, so a new or removed message doesn't
    // shift it.

    /// The open thread's expanded, focused and last message ids, when it's
    /// among `ids`.
    fn open_messages(&self, ids: &[String]) -> Option<OpenMessages> {
        let thread = &self.data.threads[self.selected?];
        if !ids.contains(&thread.id) || thread.messages.iter().any(|m| m.id.is_empty()) {
            return None;
        }
        let id = |ix: usize| thread.messages.get(ix).map(|m| m.id.clone());
        Some(OpenMessages {
            thread: thread.id.clone(),
            expanded: self
                .expanded_messages
                .iter()
                .filter_map(|&ix| id(ix))
                .collect(),
            focused: self.focused_message.and_then(id),
            last: thread.messages.last().map(|m| m.id.clone()),
        })
    }

    fn restore_open_messages(&mut self, open: Option<OpenMessages>) {
        let Some(open) = open else {
            return;
        };
        let Some(thread) = self.selected.map(|ix| &self.data.threads[ix]) else {
            return;
        };
        if thread.id != open.thread {
            return;
        }
        let ix = |id: &String| thread.messages.iter().position(|m| &m.id == id);
        // The last message is always shown open; it stays open once a newer
        // one arrives.
        self.expanded_messages = open
            .expanded
            .iter()
            .chain(&open.last)
            .filter_map(ix)
            .collect();
        self.focused_message = match &open.focused {
            Some(id) => ix(id).or(Some(thread.messages.len().saturating_sub(1))),
            None => None,
        };
    }
}

struct OpenMessages {
    thread: String,
    expanded: Vec<String>,
    focused: Option<String>,
    last: Option<String>,
}

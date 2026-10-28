//! W2: opening and saving attachments (ATT-4), a forward's files (ATT-7)
//! and the original message (RDR-8). The files come from
//! `backend::attachments`; the source shows in the reader
//! (`ui::reader::Original`).

use std::{path::PathBuf, sync::Arc};

use gpui::{
    ClipboardItem, Context, Div, Stateful, Window, component::native_menu::NativeMenu, point,
    prelude::*,
};

use super::{PigeonApp, ToastAction, ToastKind, toast::plain_error};
use crate::{
    actions::*,
    backend::attachments::{self, DEMO_NO_BYTES},
    data::{Attachment, file_size},
    ui::reader::{Original, OriginalState},
};

/// What a save writes.
enum Saving {
    File(PathBuf),
    Bytes(Arc<Vec<u8>>),
}

impl PigeonApp {
    /// Registers the handlers of this module's actions on the root.
    pub(super) fn on_attachment_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(|this, _: &OpenAttachment, window, cx| {
            this.pick_attachment(false, window, cx)
        }))
        .on_action(cx.listener(|this, _: &SaveAttachment, window, cx| {
            this.pick_attachment(true, window, cx)
        }))
        .on_action(
            cx.listener(|this, a: &OpenAttachmentAt, _, cx| this.open_attachment(a.0, a.1, cx)),
        )
        .on_action(
            cx.listener(|this, a: &SaveAttachmentAt, _, cx| this.save_attachment(a.0, a.1, cx)),
        )
        .on_action(cx.listener(|this, _: &ShowOriginal, window, cx| this.show_original(window, cx)))
        .on_action(cx.listener(|this, _: &SaveOriginal, _, cx| this.save_original(cx)))
        .on_action(cx.listener(|this, _: &CopyOriginal, _, cx| this.copy_original(cx)))
        .on_action(
            cx.listener(|this, _: &CloseOriginal, window, cx| this.close_original(window, cx)),
        )
    }

    /// The attachments the shortcuts act on, as (message, attachment):
    /// the focused message's while the reader has focus, else all of them.
    fn attachment_targets(&self) -> Vec<(usize, usize)> {
        let Some(thread) = self.selected.map(|ix| &self.data.threads[ix]) else {
            return Vec::new();
        };
        let of = |m: usize| (0..thread.messages[m].attachments.len()).map(move |a| (m, a));
        if self.reader_focused
            && let Some(m) = self.focused_message.filter(|&m| {
                thread
                    .messages
                    .get(m)
                    .is_some_and(|message| !message.attachments.is_empty())
            })
        {
            return of(m).collect();
        }
        (0..thread.messages.len()).flat_map(of).collect()
    }

    fn attachment_at(&self, m: usize, a: usize) -> Option<(String, Attachment)> {
        let message = self.data.threads[self.selected?].messages.get(m)?;
        Some((message.id.clone(), message.attachments.get(a)?.clone()))
    }

    /// Open or save the conversation's attachment, or with several, pick
    /// one from a menu.
    fn pick_attachment(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.attachment_targets();
        match targets.as_slice() {
            [] => self.show_toast(
                ToastKind::Info,
                "No attachments in this conversation",
                None,
                cx,
            ),
            [(m, a)] if save => self.save_attachment(*m, *a, cx),
            [(m, a)] => self.open_attachment(*m, *a, cx),
            _ => {
                let thread = &self.data.threads[self.selected.expect("has targets")];
                let mut menu = NativeMenu::new();
                for &(m, a) in &targets {
                    let attachment = &thread.messages[m].attachments[a];
                    let label =
                        format!("{} — {}", attachment.name, file_size(attachment.size_bytes));
                    let action: Box<dyn gpui::Action> = if save {
                        Box::new(SaveAttachmentAt(m, a))
                    } else {
                        Box::new(OpenAttachmentAt(m, a))
                    };
                    menu = menu.menu(label, action);
                }
                let size = window.viewport_size();
                menu.show(point(size.width * 0.55, size.height * 0.3), window, cx);
            }
        }
    }

    /// The file for an attachment, then `then` with its path. A failure
    /// offers to try again (`save` or open), while the conversation is
    /// still the one selected.
    fn with_attachment_file(
        &mut self,
        m: usize,
        a: usize,
        save: bool,
        then: impl FnOnce(&mut Self, attachments::Ready, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some((message_id, attachment)) = self.attachment_at(m, a) else {
            return;
        };
        let thread_id = self.selected.map(|ix| self.data.threads[ix].id.clone());
        let retry = ToastAction::run("Try Again", move |this, _, cx| {
            if this.selected.map(|ix| &this.data.threads[ix].id) != thread_id.as_ref() {
                return;
            }
            if save {
                this.save_attachment(m, a, cx)
            } else {
                this.open_attachment(m, a, cx)
            }
        });
        let task = self
            .backend
            .update(cx, |b, cx| b.attachment_file(&message_id, &attachment, cx));
        let name = attachment.name.clone();
        if !self.backend.read(cx).is_demo() {
            self.show_toast(ToastKind::Working, format!("Getting “{name}”…"), None, cx);
        }
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(ready) => {
                    if this
                        .toast
                        .as_ref()
                        .is_some_and(|t| t.kind == ToastKind::Working)
                    {
                        this.dismiss_toast(cx);
                    }
                    then(this, ready, cx)
                }
                Err(e) if e.to_string() == DEMO_NO_BYTES => {
                    this.show_toast(ToastKind::Info, DEMO_NO_BYTES, None, cx)
                }
                Err(e) => {
                    let message = plain_error(&format!("Couldn't get “{name}”"), &e);
                    this.show_toast(ToastKind::Error, message, Some(retry), cx)
                }
            })
            .ok();
        })
        .detach();
    }

    /// Open with the default app; something that could run code is shown in
    /// Finder instead (ATT-4).
    pub(crate) fn open_attachment(&mut self, m: usize, a: usize, cx: &mut Context<Self>) {
        let Some((_, attachment)) = self.attachment_at(m, a) else {
            return;
        };
        let name = attachment.name;
        self.with_attachment_file(
            m,
            a,
            false,
            move |this, ready, cx| {
                if ready.executable {
                    cx.reveal_path(&ready.path);
                    this.show_toast(
                        ToastKind::Info,
                        format!("“{name}” could run code, so it's shown in Finder instead"),
                        None,
                        cx,
                    );
                } else {
                    cx.open_with_system(&ready.path);
                }
            },
            cx,
        );
    }

    pub(crate) fn save_attachment(&mut self, m: usize, a: usize, cx: &mut Context<Self>) {
        let Some((_, attachment)) = self.attachment_at(m, a) else {
            return;
        };
        let name = attachment.name;
        self.with_attachment_file(
            m,
            a,
            true,
            move |this, ready, cx| this.save_as(name, Saving::File(ready.path), cx),
            cx,
        );
    }

    /// The save panel, starting in Downloads; the file written is
    /// quarantined like a download.
    fn save_as(&mut self, name: String, what: Saving, cx: &mut Context<Self>) {
        let name = attachments::sanitize_name(&name);
        let write = move |dest: PathBuf| -> anyhow::Result<PathBuf> {
            match &what {
                Saving::File(path) => {
                    std::fs::copy(path, &dest)?;
                }
                Saving::Bytes(bytes) => std::fs::write(&dest, bytes.as_slice())?,
            }
            attachments::quarantine(&dest);
            Ok(dest)
        };
        let chosen: Chosen = match attachments::dev_save_dir() {
            Some(dir) => {
                std::fs::create_dir_all(&dir).ok();
                let dest = attachments::unique_path(&dir, &name);
                Box::pin(async move { Some(dest) })
            }
            None => {
                let rx = cx.prompt_for_new_path(&attachments::downloads_dir(), Some(&name));
                Box::pin(async move { rx.await.ok().and_then(Result::ok).flatten() })
            }
        };
        cx.spawn(async move |this, cx| {
            let Some(dest) = chosen.await else {
                return;
            };
            let written = cx
                .background_executor()
                .spawn(async move { write(dest) })
                .await;
            this.update(cx, |this, cx| match written {
                Ok(dest) => this.show_toast(
                    ToastKind::Done,
                    format!("Saved to {}", dest.display()),
                    None,
                    cx,
                ),
                Err(e) => {
                    this.show_error_toast(plain_error(&format!("Couldn't save “{name}”"), &e), cx)
                }
            })
            .ok();
        })
        .detach();
    }

    /// The message the original is for: the focused one while the reader
    /// has focus, else the newest.
    fn original_target(&self) -> Option<usize> {
        let thread = &self.data.threads[self.selected?];
        let last = thread.messages.len().checked_sub(1)?;
        Some(
            self.focused_message
                .filter(|&m| self.reader_focused && m <= last)
                .unwrap_or(last),
        )
    }

    /// RDR-8: the source in place of the conversation; again closes it.
    fn show_original(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.original_target() else {
            return;
        };
        let thread = &self.data.threads[self.selected.expect("has a target")];
        if self
            .original
            .as_ref()
            .is_some_and(|o| o.thread_id == thread.id)
        {
            return self.close_original(window, cx);
        }
        let message = &thread.messages[m];
        let subject = thread.subject.clone();
        let task = self
            .backend
            .update(cx, |b, cx| b.raw_message(message, &subject, cx));
        let id = message.id.clone();
        let original = Original::new(thread.id.clone(), id.clone(), subject, cx);
        window.focus(&original.focus, cx);
        self.original = Some(original);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if let Some(original) = this.original.as_mut().filter(|o| o.message_id == id) {
                    original.state = match result {
                        Ok(raw) => OriginalState::loaded(raw),
                        Err(e) => OriginalState::Failed(e.to_string().into()),
                    };
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn close_original(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.original.take().is_some() {
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    fn copy_original(&mut self, cx: &mut Context<Self>) {
        let Some(Original {
            state: OriginalState::Loaded {
                shown, truncated, ..
            },
            ..
        }) = &self.original
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(shown.to_string()));
        let message = if *truncated {
            "Copied the first 1 MB of the original"
        } else {
            "Copied the original"
        };
        self.show_toast(ToastKind::Done, message, None, cx);
    }

    /// The whole message as an `.eml` file: the one showing, else the
    /// selected conversation's.
    fn save_original(&mut self, cx: &mut Context<Self>) {
        let Some(m) = self.original_target() else {
            return;
        };
        let thread = &self.data.threads[self.selected.expect("has a target")];
        let subject = thread.subject.clone();
        let name = format!(
            "{}.eml",
            if subject.trim().is_empty() {
                "Message"
            } else {
                subject.trim()
            }
        );
        if let Some(Original {
            state: OriginalState::Loaded { raw, .. },
            thread_id,
            ..
        }) = &self.original
            && *thread_id == thread.id
        {
            let raw = raw.clone();
            return self.save_as(name, Saving::Bytes(raw), cx);
        }
        let message = &thread.messages[m];
        let task = self
            .backend
            .update(cx, |b, cx| b.raw_message(message, &subject, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(raw) => this.save_as(name, Saving::Bytes(Arc::new(raw)), cx),
                Err(e) => this.show_error_toast(plain_error("Couldn't get the message", &e), cx),
            })
            .ok();
        })
        .detach();
    }
}

/// Where a save goes, once the user has chosen.
type Chosen = std::pin::Pin<Box<dyn std::future::Future<Output = Option<PathBuf>>>>;

//! W2: opening and saving attachments (ATT-4), a forward's files (ATT-7)
//! and the original message (RDR-8). The files come from
//! `backend::attachments`; the source shows in the reader
//! (`ui::reader::Original`). Also the reader's menu actions: links, names
//! and ⌥Tab through a message's links (MA-003, MB-010).

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use gpui::{
    ClipboardItem, Context, Div, KeyDownEvent, PathPromptOptions, Stateful, Window,
    component::native_menu::NativeMenu, point, prelude::*,
};

use super::{PigeonApp, ToastAction, ToastKind, toast::plain_error};
use crate::{
    actions::*,
    backend::attachments::{self, DEMO_NO_BYTES},
    data::{Attachment, file_size},
    native,
    ui::{
        html_view::{self, CopyText, OpenLink},
        reader::{
            MESSAGES_START, Original, OriginalState, PrefetchAttachmentAt, QuickLookAttachmentAt,
            ReaderUi, RevealAttachmentAt, ShowContactFor,
        },
    },
};

/// What a save writes.
enum Saving {
    File(PathBuf),
    Bytes(Arc<Vec<u8>>),
}

/// What to do with an attachment's file once it's here.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    Open,
    Save,
    QuickLook,
    Reveal,
    /// Only fetch it, quietly: for a drag to Finder.
    Fetch,
}

impl PigeonApp {
    /// Registers the handlers of this module's actions on the root.
    pub(super) fn on_attachment_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(|this, _: &OpenAttachment, window, cx| {
            this.pick_attachment(Use::Open, window, cx)
        }))
        .on_action(cx.listener(|this, _: &SaveAttachment, window, cx| {
            this.pick_attachment(Use::Save, window, cx)
        }))
        .on_action(cx.listener(|this, _: &QuickLookAttachment, window, cx| {
            this.pick_attachment(Use::QuickLook, window, cx)
        }))
        .on_action(cx.listener(|this, a: &OpenAttachmentAt, _, cx| {
            this.use_attachment(a.0, a.1, Use::Open, cx)
        }))
        .on_action(cx.listener(|this, a: &SaveAttachmentAt, _, cx| {
            this.use_attachment(a.0, a.1, Use::Save, cx)
        }))
        .on_action(cx.listener(|this, a: &QuickLookAttachmentAt, _, cx| {
            this.use_attachment(a.0, a.1, Use::QuickLook, cx)
        }))
        .on_action(cx.listener(|this, a: &RevealAttachmentAt, _, cx| {
            this.use_attachment(a.0, a.1, Use::Reveal, cx)
        }))
        .on_action(cx.listener(|this, a: &PrefetchAttachmentAt, _, cx| {
            this.use_attachment(a.0, a.1, Use::Fetch, cx)
        }))
        .on_action(cx.listener(|this, _: &SaveAllAttachments, _, cx| this.save_all(cx)))
        .on_action(cx.listener(|this, _: &ShowOriginal, window, cx| this.show_original(window, cx)))
        .on_action(cx.listener(|this, _: &SaveOriginal, _, cx| this.save_original(cx)))
        .on_action(cx.listener(|this, _: &CopyOriginal, _, cx| this.copy_original(cx)))
        .on_action(
            cx.listener(|this, _: &CloseOriginal, window, cx| this.close_original(window, cx)),
        )
        .on_action(cx.listener(|this, a: &ShowContactFor, window, cx| {
            this.show_contact(a.0.clone(), a.1, window, cx)
        }))
        .on_action(|a: &OpenLink, _, cx| html_view::open_link(&a.0, cx))
        .on_action(cx.listener(|this, a: &CopyText, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(a.0.to_string()));
            this.show_toast(ToastKind::Done, "Copied", None, cx);
        }))
        .on_action(cx.listener(|this, _: &CycleMessageLinks, _, cx| this.cycle_links(cx)))
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            // ↵ opens the link ⌥Tab is on.
            let keys = &event.keystroke;
            if keys.key == "enter"
                && !keys.modifiers.modified()
                && this.reader_focused
                && this.focus_handle.is_focused(window)
                && let Some(url) = this.focused_link(cx)
            {
                cx.stop_propagation();
                html_view::open_link(&url, cx);
            }
        }))
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

    /// Use the conversation's attachment, or with several, pick one from a
    /// menu under the first chip (MB-016).
    fn pick_attachment(&mut self, what: Use, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.attachment_targets();
        match targets.as_slice() {
            [] => self.show_toast(
                ToastKind::Info,
                "No attachments in this conversation",
                None,
                cx,
            ),
            [(m, a)] => self.use_attachment(*m, *a, what, cx),
            _ => {
                let thread = &self.data.threads[self.selected.expect("has targets")];
                let mut menu = NativeMenu::new();
                for &(m, a) in &targets {
                    let attachment = &thread.messages[m].attachments[a];
                    let label =
                        format!("{} — {}", attachment.name, file_size(attachment.size_bytes));
                    let action: Box<dyn gpui::Action> = match what {
                        Use::Save => Box::new(SaveAttachmentAt(m, a)),
                        Use::QuickLook => Box::new(QuickLookAttachmentAt(m, a)),
                        _ => Box::new(OpenAttachmentAt(m, a)),
                    };
                    menu = menu.menu(label, action);
                }
                let position = cx
                    .default_global::<ReaderUi>()
                    .attachment_anchor
                    .get()
                    .map(|b| b.bottom_left())
                    .unwrap_or_else(|| {
                        let size = window.viewport_size();
                        point(size.width * 0.55, size.height * 0.3)
                    });
                menu.show(position, window, cx);
            }
        }
    }

    /// Esc in the reader: stop the open conversation's attachment
    /// downloads (MA-067). Whether there were any.
    pub(super) fn cancel_attachment_fetches(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.selected else {
            return false;
        };
        let ids: HashSet<String> = self.data.threads[ix]
            .messages
            .iter()
            .map(|m| m.id.clone())
            .collect();
        let ui = cx.default_global::<ReaderUi>();
        let before = ui.fetching.len();
        ui.fetching.retain(|(id, _), _| !ids.contains(id));
        let cancelled = ui.fetching.len() < before;
        if cancelled {
            cx.notify();
        }
        cancelled
    }

    /// The file for an attachment, then `what` with it. Asking again while
    /// it's on its way cancels it (MA-067); the chip pulses meanwhile. A
    /// failure offers to try again, while the conversation is still the one
    /// selected.
    fn use_attachment(&mut self, m: usize, a: usize, what: Use, cx: &mut Context<Self>) {
        let Some((message_id, attachment)) = self.attachment_at(m, a) else {
            return;
        };
        let key = (message_id.clone(), a);
        let ui = cx.default_global::<ReaderUi>();
        if ui.fetching.contains_key(&key) {
            if what != Use::Fetch {
                ui.fetching.remove(&key);
                cx.notify();
            }
            return;
        }
        if what == Use::Fetch && ui.files.contains_key(&key) {
            return;
        }
        let thread_id = self.selected.map(|ix| self.data.threads[ix].id.clone());
        let task = self
            .backend
            .update(cx, |b, cx| b.attachment_file(&message_id, &attachment, cx));
        let name = attachment.name.clone();
        let fetch_key = key.clone();
        let fetch = cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let ui = cx.default_global::<ReaderUi>();
                if let Some(task) = ui.fetching.remove(&fetch_key) {
                    task.detach();
                }
                match result {
                    Ok(ready) => {
                        ui.files.insert(fetch_key, ready.clone());
                        this.use_file(name, ready, what, cx);
                    }
                    Err(_) if what == Use::Fetch => {}
                    Err(e) if e.to_string() == DEMO_NO_BYTES => {
                        this.show_toast(ToastKind::Info, DEMO_NO_BYTES, None, cx)
                    }
                    Err(e) => {
                        let retry = ToastAction::run("Try Again", move |this, _, cx| {
                            if this.selected.map(|ix| &this.data.threads[ix].id)
                                == thread_id.as_ref()
                            {
                                this.use_attachment(m, a, what, cx)
                            }
                        });
                        let message = plain_error(&format!("Couldn't get “{name}”"), &e);
                        this.show_toast(ToastKind::Error, message, Some(retry), cx)
                    }
                }
                cx.notify();
            })
            .ok();
        });
        cx.default_global::<ReaderUi>().fetching.insert(key, fetch);
        cx.notify();
    }

    /// Something that could run code is never opened or previewed: Finder
    /// shows it instead (ATT-4).
    fn use_file(
        &mut self,
        name: String,
        ready: attachments::Ready,
        what: Use,
        cx: &mut Context<Self>,
    ) {
        match what {
            Use::Fetch => {}
            Use::Save => self.save_as(name, Saving::File(ready.path), cx),
            Use::Reveal => cx.reveal_path(&ready.path),
            Use::Open | Use::QuickLook if ready.executable => {
                cx.reveal_path(&ready.path);
                self.show_toast(
                    ToastKind::Info,
                    format!("“{name}” could run code, so it's shown in Finder instead"),
                    None,
                    cx,
                );
            }
            Use::Open => cx.open_with_system(&ready.path),
            Use::QuickLook => native::quick_look(&ready.path),
        }
    }

    /// The save panel, starting where the last save went (MB-060), else in
    /// Downloads; the file written is quarantined like a download.
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
        let dev_dir = attachments::dev_save_dir();
        let start = cx
            .default_global::<ReaderUi>()
            .save_dir
            .clone()
            .unwrap_or_else(attachments::downloads_dir);
        let prompt = dev_dir
            .is_none()
            .then(|| cx.prompt_for_new_path(&start, Some(&name)));
        cx.spawn(async move |this, cx| {
            let dest = match (dev_dir, prompt) {
                (Some(dir), _) => {
                    std::fs::create_dir_all(&dir).ok();
                    attachments::unique_path(&dir, &name)
                }
                (None, Some(prompt)) => match prompt.await.ok().and_then(Result::ok).flatten() {
                    Some(dest) => dest,
                    None => return,
                },
                (None, None) => return,
            };
            let written = cx
                .background_executor()
                .spawn(async move { write(dest) })
                .await;
            this.update(cx, |this, cx| match written {
                Ok(dest) => {
                    cx.default_global::<ReaderUi>().save_dir = dest.parent().map(PathBuf::from);
                    let file = dest.file_name().unwrap_or_default().to_string_lossy();
                    this.show_toast(
                        ToastKind::Done,
                        format!("Saved “{file}”"),
                        Some(ToastAction::run("Show in Finder", move |_, _, cx| {
                            cx.reveal_path(&dest)
                        })),
                        cx,
                    )
                }
                Err(e) => {
                    this.show_error_toast(plain_error(&format!("Couldn't save “{name}”"), &e), cx)
                }
            })
            .ok();
        })
        .detach();
    }

    /// MB-059: every attachment the shortcuts act on, into a folder picked
    /// once; names that are taken get a number.
    fn save_all(&mut self, cx: &mut Context<Self>) {
        let targets = self.attachment_targets();
        if targets.is_empty() {
            return self.show_toast(
                ToastKind::Info,
                "No attachments in this conversation",
                None,
                cx,
            );
        }
        let files: Vec<_> = targets
            .iter()
            .filter_map(|&(m, a)| self.attachment_at(m, a))
            .map(|(message_id, attachment)| {
                let task = self
                    .backend
                    .update(cx, |b, cx| b.attachment_file(&message_id, &attachment, cx));
                (attachment.name, task)
            })
            .collect();
        let dev_dir = attachments::dev_save_dir();
        let prompt = dev_dir.is_none().then(|| {
            cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Save All".into()),
            })
        });
        cx.spawn(async move |this, cx| {
            let dir = match (dev_dir, prompt) {
                (Some(dir), _) => dir,
                (None, Some(prompt)) => match prompt.await.ok().and_then(Result::ok).flatten() {
                    Some(mut dirs) if !dirs.is_empty() => dirs.remove(0),
                    _ => return,
                },
                (None, None) => return,
            };
            let mut saved = 0;
            let mut failed: Option<String> = None;
            for (name, task) in files {
                let ready = match task.await {
                    Ok(ready) => ready,
                    Err(e) => {
                        failed.get_or_insert(if e.to_string() == DEMO_NO_BYTES {
                            DEMO_NO_BYTES.to_string()
                        } else {
                            plain_error(&format!("Couldn't get “{name}”"), &e)
                        });
                        continue;
                    }
                };
                let dir = dir.clone();
                let file = name.clone();
                let written = cx
                    .background_executor()
                    .spawn(async move {
                        std::fs::create_dir_all(&dir)?;
                        let dest =
                            attachments::unique_path(&dir, &attachments::sanitize_name(&file));
                        std::fs::copy(&ready.path, &dest)?;
                        attachments::quarantine(&dest);
                        anyhow::Ok(())
                    })
                    .await;
                match written {
                    Ok(()) => saved += 1,
                    Err(e) => {
                        failed.get_or_insert(plain_error(&format!("Couldn't save “{name}”"), &e));
                    }
                }
            }
            this.update(cx, |this, cx| {
                if saved > 0 {
                    cx.default_global::<ReaderUi>().save_dir = Some(dir.clone());
                }
                match failed {
                    Some(error) if saved == 0 => this.show_toast(
                        if error == DEMO_NO_BYTES {
                            ToastKind::Info
                        } else {
                            ToastKind::Error
                        },
                        error,
                        None,
                        cx,
                    ),
                    Some(error) => this.show_error_toast(error, cx),
                    None => this.show_toast(
                        ToastKind::Done,
                        match saved {
                            1 => "Saved 1 attachment".to_string(),
                            n => format!("Saved {n} attachments"),
                        },
                        Some(ToastAction::run("Show in Finder", move |_, _, cx| {
                            cx.reveal_path(&dir)
                        })),
                        cx,
                    ),
                }
            })
            .ok();
        })
        .detach();
    }

    /// ⌥Tab (MA-003): the focused message's next link, ringed; ↵ opens it.
    /// Links in folded quoted text are skipped.
    fn cycle_links(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.selected.map(|ix| &self.data.threads[ix]) else {
            return;
        };
        let Some(last) = thread.messages.len().checked_sub(1) else {
            return;
        };
        let m = self.focused_message.filter(|&m| m <= last).unwrap_or(last);
        let message = &thread.messages[m];
        let Some(doc) = self.html_cache.get(&message.id, message.html()) else {
            return;
        };
        let thread_id = thread.id.clone();
        let ui = cx.default_global::<ReaderUi>();
        let end = match crate::html::trailing_quote_start(&doc) {
            Some(start) if !ui.open_quotes.contains(&message.id) => start,
            _ => doc.blocks.len(),
        };
        let count = crate::html::links(&doc.blocks[..end]).len();
        if count == 0 {
            ui.link = None;
            return self.show_toast(ToastKind::Info, "No links in this message", None, cx);
        }
        let next = match &ui.link {
            Some((t, lm, n)) if *t == thread_id && *lm == m => (n + 1) % count,
            _ => 0,
        };
        let moved = !matches!(&ui.link, Some((t, lm, _)) if *t == thread_id && *lm == m);
        ui.link = Some((thread_id, m, next));
        self.focused_message = Some(m);
        if moved {
            self.reader_scroll.scroll_to_item(MESSAGES_START + m);
        }
        cx.notify();
    }

    /// The link ⌥Tab is on, while it's showing.
    fn focused_link(&self, cx: &mut Context<Self>) -> Option<gpui::SharedString> {
        let thread = &self.data.threads[self.selected?];
        let (t, m, n) = cx.default_global::<ReaderUi>().link.clone()?;
        if t != thread.id {
            return None;
        }
        let message = thread.messages.get(m)?;
        let doc = self.html_cache.get(&message.id, message.html())?;
        crate::html::links(&doc.blocks).get(n).cloned()
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
            // Up to 1 MB to decode and split: off the main thread (MC-054).
            let state = cx
                .background_executor()
                .spawn(async move {
                    match result {
                        Ok(raw) => OriginalState::loaded(raw),
                        Err(e) => OriginalState::Failed(e.to_string().into()),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(original) = this.original.as_mut().filter(|o| o.message_id == id) {
                    original.state = state;
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

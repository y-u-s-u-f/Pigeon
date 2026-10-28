//! A draft in a window of its own (MB-047), from "Open Draft in New
//! Window". The `Composer` moves here from the reading pane, so the main
//! window can show other mail while it's written. `PigeonApp` keeps the
//! list of these windows and does the saving, sending and discarding; this
//! view only hosts the composer and hands its actions over.

use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions,
    component::input, div, px, size,
};

use super::{PigeonApp, RAIL_WIDTH, shell};
use crate::{
    actions::*,
    assets::Icon,
    settings,
    theme::Theme,
    ui::{
        self, HEADER_HEIGHT,
        composer::{Composer, ComposerEvent, Quoted},
        controls::{ButtonKind, push_button},
        style::*,
    },
};

/// Key context of a draft window's root.
const DRAFT_WINDOW: &str = "DraftWindow";

/// A draft window the app keeps, with the app's subscription to its
/// composer.
pub(crate) struct DraftWindow {
    pub window: AnyWindowHandle,
    pub composer: Entity<Composer>,
    pub _events: Subscription,
}

pub(crate) struct DraftHost {
    composer: Entity<Composer>,
    app: WeakEntity<PigeonApp>,
    /// The window's title as last set.
    title: SharedString,
    _events: Subscription,
    /// Light and Dark follow the system while it's open.
    _appearance: Subscription,
}

impl DraftHost {
    /// The composer titles the window, and its subject once it has one.
    fn sync_title(&mut self, window: &mut Window, cx: &gpui::App) {
        let composer = self.composer.read(cx);
        let subject = composer.content(cx).subject;
        let title: SharedString = if subject.trim().is_empty() {
            composer.title().into()
        } else {
            subject.trim().to_string().into()
        };
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
    }

    /// Run `f` on the app; false once the main window has gone.
    fn with_app(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut PigeonApp, &mut Context<PigeonApp>) -> bool,
    ) -> bool {
        self.app.update(cx, f).unwrap_or(false)
    }

    fn send(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.composer.clone();
        // The checks show their errors here, in the draft.
        if !super::compose::ready_to_send(&composer, window, cx) {
            return;
        }
        composer.update(cx, |c, cx| c.remember_focus(window, cx));
        if self.with_app(cx, |app, cx| {
            app.send_composer(composer, cx);
            true
        }) {
            window.remove_window();
        }
    }

    fn discard(&mut self, _: &DiscardDraft, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.composer.clone();
        composer.update(cx, |c, cx| c.remember_focus(window, cx));
        if self.with_app(cx, |app, cx| {
            app.discard_composer(composer, cx);
            true
        }) {
            window.remove_window();
        }
    }

    /// Escape, ⌘W and the close button: the draft goes to Drafts.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.composer.clone();
        self.with_app(cx, |app, cx| {
            app.close_draft_window(&composer, cx);
            true
        });
        window.remove_window();
    }

    fn save(&mut self, _: &SaveDraft, _: &mut Window, cx: &mut Context<Self>) {
        let composer = self.composer.clone();
        self.with_app(cx, |app, cx| {
            app.save_draft(&composer, true, cx);
            app.show_toast(super::ToastKind::Done, "Saved to Drafts", None, cx);
            true
        });
    }
}

impl Render for DraftHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let title = self.composer.read(cx).title();
        div()
            .id("draft-window")
            .key_context(DRAFT_WINDOW)
            .role(gpui::Role::Group)
            .aria_label(self.title.clone())
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(Self::discard))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(|this, _: &CloseDraft, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &ZoomWindow, window, _| window.zoom_window()))
            .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
            // The Edit menu, to the field with focus.
            .on_action(cx.listener(|_, _: &EditUndo, window, cx| {
                window.dispatch_action(Box::new(input::Undo), cx)
            }))
            .on_action(cx.listener(|_, _: &EditRedo, window, cx| {
                window.dispatch_action(Box::new(input::Redo), cx)
            }))
            .on_action(cx.listener(|_, _: &EditCopy, window, cx| {
                window.dispatch_action(Box::new(input::Copy), cx)
            }))
            .on_action(cx.listener(|_, _: &EditSelectAll, window, cx| {
                window.dispatch_action(Box::new(input::SelectAll), cx)
            }))
            .on_action(cx.listener(|this, _: &ChangeFrom, window, cx| {
                this.composer
                    .update(cx, |c, cx| c.open_from_menu(window, cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleSignature, _, cx| {
                this.composer.update(cx, |c, cx| c.toggle_signature(cx))
            }))
            .on_action(cx.listener(|this, _: &ShowCcBcc, window, cx| {
                this.composer.update(cx, |c, cx| c.show_cc_bcc(window, cx))
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.reader_bg)
            .text_color(theme.text)
            .line_height(LINE_UI)
            .child(
                // Like the reading pane's draft toolbar, past the traffic
                // lights.
                ui::window_drag_area(div().id("draft-window-toolbar"))
                    .h(HEADER_HEIGHT)
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(SPACE_2XS)
                    .pl(RAIL_WIDTH)
                    .pr(SPACE_MD)
                    .child(
                        ui::label("draft-window-title", title)
                            .role(gpui::Role::Heading)
                            .aria_level(1)
                            .text_size(TEXT_LG)
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(div().flex_1())
                    .child(
                        push_button("draft-window-send", "Send", ButtonKind::Primary, &theme)
                            .mr(SPACE_SM)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(SendMessage), cx)
                            }),
                    )
                    .child(ui::icon_button(
                        "draft-window-discard",
                        Icon::Trash,
                        "Discard draft",
                        Box::new(DiscardDraft),
                        &theme,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.composer.clone()),
            )
    }
}

impl PigeonApp {
    /// "Open Draft in New Window" (MB-047): the open draft leaves the
    /// reading pane for a window of its own. Its text fields belong to the
    /// window they were made in (focus and the caret's blink are per
    /// window), so the draft is saved if it changed and a new composer
    /// takes it up there, as saved.
    pub(super) fn open_draft_in_new_window(
        &mut self,
        _: &OpenDraftInNewWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((old, _)) = self.composer.take() else {
            return;
        };
        if !old.read(cx).is_pristine(cx) {
            self.save_draft(&old, true, cx);
        }
        old.update(cx, |c, _| c.set_closed(true));
        let seed = self.composer_seed(&old, cx);
        let draft_id = seed.draft_id.clone();
        // A draft picked in Drafts leaves the reading pane empty.
        if draft_id.is_some()
            && self.selected.map(|ix| &self.data.threads[ix].id) == draft_id.as_ref()
        {
            self.selected = None;
        }
        let app = cx.weak_entity();
        let options = window_options(window, cx);
        let opened = gpui::open_window(options, cx, move |window, cx| {
            Theme::sync_for(settings::get(cx).appearance, window, cx);
            let composer = seed.build(false, window, cx);
            let host = cx.new(|cx| {
                // The title follows the subject as the draft saves.
                let events = cx.subscribe_in(
                    &composer,
                    window,
                    |this: &mut DraftHost, _, event, window, cx| {
                        if let ComposerEvent::Autosave = event {
                            this.sync_title(window, cx);
                        }
                    },
                );
                let appearance = cx.observe_window_appearance(window, |_, window, cx| {
                    Theme::sync_for(settings::get(cx).appearance, window, cx);
                });
                let mut host = DraftHost {
                    composer: composer.clone(),
                    app: app.clone(),
                    title: SharedString::default(),
                    _events: events,
                    _appearance: appearance,
                };
                host.sync_title(window, cx);
                host
            });
            let closing = composer.clone();
            window.on_window_should_close(cx, move |_, cx| {
                app.update(cx, |this, cx| this.close_draft_window(&closing, cx))
                    .ok();
                true
            });
            composer.update(cx, |c, cx| c.focus_first(window, cx));
            host
        });
        let (draft_window, host) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                // It's in Drafts, if it said anything.
                crate::logging::info(&format!("draft window: couldn't open: {e}"));
                self.show_error_toast("Couldn't open a window for the draft", cx);
                return;
            }
        };
        let composer = host.read(cx).composer.clone();
        self.windowed.insert(composer.entity_id());
        let events = cx.subscribe_in(&composer, window, |this, composer, event, window, cx| {
            match event {
                ComposerEvent::Edited { .. } => {}
                ComposerEvent::Autosave => this.save_draft(composer, false, cx),
                // In the main window, where the card lives.
                ComposerEvent::ShowContact(contact, _) => {
                    this.show_contact(contact.clone(), None, window, cx)
                }
            }
        });
        self.draft_windows.push(DraftWindow {
            window: draft_window,
            composer,
            _events: events,
        });
        self.reader_focused = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// The draft window holding the draft `id`, or the reply to the
    /// conversation `id`: brought forward. Whether there was one.
    pub(super) fn raise_draft_window(&self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(found) = self.draft_windows.iter().find(|d| {
            let c = d.composer.read(cx);
            c.draft_id.as_deref() == Some(id) || c.reply_to.as_deref() == Some(id)
        }) else {
            return false;
        };
        found
            .window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    }

    /// A draft window closes: its draft goes to Drafts, if it says
    /// anything. Nothing once the draft is gone (sent or discarded).
    pub(super) fn close_draft_window(
        &mut self,
        composer: &Entity<Composer>,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self
            .draft_windows
            .iter()
            .position(|d| d.composer == *composer)
        else {
            return;
        };
        let closed = self.draft_windows.remove(ix);
        self.stash(closed.composer, cx);
    }

    /// The main window closes: so do the draft windows, their drafts saved.
    pub(super) fn close_draft_windows(&mut self, cx: &mut Context<Self>) {
        for closed in std::mem::take(&mut self.draft_windows) {
            self.stash(closed.composer, cx);
            closed
                .window
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }

    /// What it takes to make `composer` again, in another window.
    fn composer_seed(&self, composer: &Entity<Composer>, cx: &gpui::App) -> ComposerSeed {
        let c = composer.read(cx);
        ComposerSeed {
            kind: c.kind,
            content: c.content(cx),
            reply_to: c.reply_to.clone(),
            answers: c.answers.clone(),
            draft_id: c.draft_id.clone(),
            quoted: c
                .quoted
                .as_ref()
                .map(|q| (q.header.clone(), q.html.clone())),
            awaiting_body: c.awaiting_body,
            dropped_files: c.dropped_files.clone(),
            identities: self.data.account.identities(),
            book: self.contact_book.clone(),
        }
    }

    /// A draft from a draft window coming back to the reading pane (Undo
    /// Send, Undo Discard): made again in this window, and saved as it is,
    /// so closing it later keeps what it says.
    pub(super) fn adopt_composer(
        &mut self,
        composer: Entity<Composer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Composer> {
        if !self.windowed.remove(&composer.entity_id()) {
            return composer;
        }
        let inline = composer.read(cx).reply_to.is_some();
        let fresh = self.composer_seed(&composer, cx).build(inline, window, cx);
        self.save_draft(&fresh, true, cx);
        fresh
    }

    /// Forget the window of a draft sent or discarded from it.
    pub(super) fn forget_draft_window(&mut self, composer: &Entity<Composer>) {
        self.draft_windows.retain(|d| d.composer != *composer);
    }
}

/// A composer's state, to make it again in another window.
struct ComposerSeed {
    kind: super::ComposeKind,
    content: crate::ui::composer::DraftContent,
    reply_to: Option<String>,
    answers: String,
    draft_id: Option<String>,
    quoted: Option<(Vec<(String, String)>, String)>,
    awaiting_body: bool,
    dropped_files: Vec<String>,
    identities: Vec<crate::data::SendAs>,
    book: crate::contacts::SharedContactBook,
}

impl ComposerSeed {
    fn build(self, inline: bool, window: &mut Window, cx: &mut gpui::App) -> Entity<Composer> {
        cx.new(|cx| {
            let quoted = self.quoted.map(|(header, html)| Quoted::new(header, html));
            let mut c = Composer::new(
                self.kind,
                self.content,
                self.reply_to,
                self.draft_id,
                quoted,
                inline,
                self.book,
                window,
                cx,
            )
            .with_identities(self.identities);
            c.awaiting_body = self.awaiting_body;
            c.answers = self.answers;
            c.dropped_files = self.dropped_files;
            c
        })
    }
}

/// A window the size of a roomy draft, centered on the main window's
/// display, in its glass.
fn window_options(main: &Window, cx: &gpui::App) -> WindowOptions {
    let display_id = main.display(cx).map(|d| d.id());
    let bounds = gpui::Bounds::centered(display_id, DRAFT_WINDOW_SIZE, cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        window_min_size: Some(DRAFT_WINDOW_MIN),
        titlebar: Some(TitlebarOptions {
            title: Some("New Message".into()),
            appears_transparent: true,
            traffic_light_position: Some(shell::traffic_lights()),
        }),
        app_owns_titlebar_drag: true,
        window_background: shell::window_background(cx),
        ..Default::default()
    }
}

const DRAFT_WINDOW_SIZE: gpui::Size<gpui::Pixels> = size(px(720.), px(640.));
const DRAFT_WINDOW_MIN: gpui::Size<gpui::Pixels> = size(px(480.), px(360.));

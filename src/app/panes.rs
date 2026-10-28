//! The mail panes as views of their own (MC-036). Each draws from
//! `PigeonApp`'s state, but as its own entity: a hover, a spring or a caret
//! inside one redraws that pane alone, and the others are reused through
//! `cached` instead of being laid out and painted again.
//!
//! Invalidation: every notify of `PigeonApp` notifies every pane
//! (`PigeonApp::new` observes itself), so a pane never shows state older
//! than the app's. Notifies of a pane's own (hover, scroll, its springs, a
//! child view such as the composer or an input) redraw only that pane and
//! the app above it. The calendar notifies the list (its inbox-zero line).

use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Entity, IntoElement, ParentElement as _,
    Render, StyleRefinement, Styled as _, WeakEntity, Window,
};

use super::PigeonApp;

type Draw = fn(&mut PigeonApp, &mut Window, &mut Context<PigeonApp>) -> AnyElement;

pub(crate) struct Pane {
    app: WeakEntity<PigeonApp>,
    draw: Draw,
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let draw = self.draw;
        self.app
            .update(cx, |app, cx| draw(app, window, cx))
            .unwrap_or_else(|_| gpui::Empty.into_any_element())
    }
}

/// The cached panes, and the layer above them (toast, tip, overlays),
/// which isn't cached: it's a view so its springs redraw it, not the app.
pub(crate) struct Panes {
    pub sidebar: Entity<Pane>,
    pub list: Entity<Pane>,
    pub reader: Entity<Pane>,
    pub layer: Entity<Pane>,
}

impl Panes {
    pub fn new(
        sidebar: Draw,
        list: Draw,
        reader: Draw,
        layer: Draw,
        cx: &mut Context<PigeonApp>,
    ) -> Self {
        let app = cx.weak_entity();
        let mut pane = |draw| {
            let app = app.clone();
            cx.new(|_| Pane { app, draw })
        };
        Self {
            sidebar: pane(sidebar),
            list: pane(list),
            reader: pane(reader),
            layer: pane(layer),
        }
    }

    /// The app's state changed: every cached pane draws again.
    pub fn notify(&self, cx: &mut App) {
        for pane in [&self.sidebar, &self.list, &self.reader] {
            pane.update(cx, |_, cx| cx.notify());
        }
    }
}

/// `pane` filling `slot`, which gives it its size. With `cache`, reused
/// while neither it nor its bounds changed; never while something above it
/// fades (a reused pane keeps the opacity it was painted with), nor with
/// VoiceOver on: the accessibility tree is built from each frame's drawing,
/// so a reused pane would drop out of it.
pub(crate) fn embed(pane: &Entity<Pane>, slot: Div, cache: bool, window: &Window) -> Div {
    let pane = pane.clone();
    slot.child(if !cache || window.is_a11y_active() {
        pane.into_any_element()
    } else {
        pane.cached(StyleRefinement::default().size_full())
            .into_any_element()
    })
}

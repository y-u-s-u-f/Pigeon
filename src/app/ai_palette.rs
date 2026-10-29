//! ⌘K's AI (W4): plain words, Best Matches, Ask Pigeon and Do It for Me,
//! with their commands.

use gpui::{Context, Div, Stateful, Window, prelude::*};

use super::PigeonApp;
use crate::actions::{AskPigeon, DoItForMe};

#[derive(Default)]
pub struct PaletteAi {}

impl PigeonApp {
    /// Registers the handlers of this module's actions on the root.
    pub(super) fn on_ai_palette_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(Self::ask_pigeon))
            .on_action(cx.listener(Self::do_it_for_me))
    }

    fn ask_pigeon(&mut self, _: &AskPigeon, _: &mut Window, _: &mut Context<Self>) {}

    fn do_it_for_me(&mut self, _: &DoItForMe, _: &mut Window, _: &mut Context<Self>) {}
}

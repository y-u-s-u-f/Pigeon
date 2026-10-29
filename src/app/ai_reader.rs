//! The reader's AI (W3): label suggestion, summary and catch-up, Clean
//! View, quick replies and Add to Calendar, with their commands.

use gpui::{Context, Div, Stateful, Window, prelude::*};

use super::PigeonApp;
use crate::actions::{
    AddToCalendar, ApplySuggestedLabel, QuickReply, ReportBadCleanView, RetryCleanView,
    SummarizeConversation, ToggleCleanView,
};

#[derive(Default)]
pub struct ReaderAi {}

impl PigeonApp {
    /// Registers the handlers of this module's actions on the root.
    pub(super) fn on_ai_reader_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(Self::apply_suggested_label))
            .on_action(cx.listener(Self::summarize_conversation))
            .on_action(cx.listener(Self::toggle_clean_view))
            .on_action(cx.listener(Self::retry_clean_view))
            .on_action(cx.listener(Self::report_bad_clean_view))
            .on_action(cx.listener(Self::quick_reply))
            .on_action(cx.listener(Self::add_to_calendar))
    }

    fn apply_suggested_label(
        &mut self,
        _: &ApplySuggestedLabel,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    fn summarize_conversation(
        &mut self,
        _: &SummarizeConversation,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    fn toggle_clean_view(&mut self, _: &ToggleCleanView, _: &mut Window, _: &mut Context<Self>) {}

    fn retry_clean_view(&mut self, _: &RetryCleanView, _: &mut Window, _: &mut Context<Self>) {}

    fn report_bad_clean_view(
        &mut self,
        _: &ReportBadCleanView,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    fn quick_reply(&mut self, _: &QuickReply, _: &mut Window, _: &mut Context<Self>) {}

    fn add_to_calendar(&mut self, _: &AddToCalendar, _: &mut Window, _: &mut Context<Self>) {}
}

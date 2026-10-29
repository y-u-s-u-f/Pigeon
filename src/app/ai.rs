//! AI in the app (AI-1): Settings › AI's prefs put into effect, the note
//! and what each row shows, the paused toast, Needs Reply's commands, and
//! which AI commands ⌘K offers. The reader's commands live in `ai_reader`,
//! ⌘K's, Ask Pigeon's and Do It for Me's in `ai_palette`.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use gpui::{App, Context, Div, Stateful, Window, prelude::*};

use super::{Mode, PigeonApp, ToastAction, ToastKind, ai_palette, ai_reader, shell};
use crate::{
    actions::{AiNeed, DismissNeedsReply, GoToNeedsReply, OpenSettingsSection},
    ai::{AiPrefs, Feature, Provider, ProviderInfo, queue::Health},
    data::Folder,
    settings::{self, Section},
};

#[derive(Default)]
pub struct AiState {
    #[allow(dead_code)]
    pub reader: ai_reader::ReaderAi,
    #[allow(dead_code)]
    pub palette: ai_palette::PaletteAi,
    /// Test Connection was pressed and its answer isn't in yet.
    pub testing: Vec<Provider>,
}

/// The paused toast shows once per launch.
static PAUSED_SHOWN: AtomicBool = AtomicBool::new(false);

/// Settings › AI changed: saved, and handed to the backend.
pub fn set_prefs(prefs: AiPrefs, cx: &mut App) {
    shell::change_app_settings(cx, |s| s.ai = prefs.clone());
    shell::backend_of(cx).update(cx, |b, cx| b.ai_apply(&prefs, cx));
    cx.refresh_windows();
}

/// What Settings › AI's main row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainRow {
    /// Neither model is set in .env.
    Unavailable,
    Off,
    /// Turning on, with the note to confirm.
    Asking,
    /// On, but the note was confirmed for somewhere else: nothing is sent
    /// until it's confirmed again.
    Paused,
    On,
}

pub fn main_row(prefs: &AiPrefs, fingerprint: &str, configured: bool, asking: bool) -> MainRow {
    match () {
        _ if !configured => MainRow::Unavailable,
        _ if prefs.consented(fingerprint) => MainRow::On,
        _ if prefs.enabled => MainRow::Paused,
        _ if asking => MainRow::Asking,
        _ => MainRow::Off,
    }
}

/// What is sent where, confirmed before AI turns on.
pub fn note(writing_host: Option<&str>, jev_host: Option<&str>) -> String {
    let jev = jev_host.map(|host| {
        if host.ends_with("openrouter.ai") {
            "OpenRouter (TypeSafe)".to_string()
        } else {
            host.to_string()
        }
    });
    let to = match (writing_host, jev) {
        (Some(writing), Some(jev)) => format!("to {writing} for writing and to {jev} for Jev"),
        (Some(writing), None) => format!("to {writing} for writing"),
        (None, Some(jev)) => format!("to {jev} for Jev"),
        (None, None) => "to the models set in .env".to_string(),
    };
    format!(
        "When AI is on, Pigeon sends message text, subjects and sender names, your label names, \
         what you type in ⌘K and search (with the subjects and snippets of matching \
         conversations), your Ask Pigeon questions and matching calendar events {to}. Nothing \
         is sent while AI is off, and text already sent can't be taken back."
    )
}

/// A model row's status line, and whether it's a problem; `None` when the
/// model isn't set in .env.
pub fn provider_status(info: &ProviderInfo, on: bool, testing: bool) -> Option<(String, bool)> {
    let health = info.health?;
    Some(match (health, &info.last_test) {
        _ if !on => (
            "Turn on AI to use it and test the connection.".into(),
            false,
        ),
        (_, None) if testing => ("Testing…".into(), false),
        (Health::Stopped, _) => ("Check the key in .env.".into(), true),
        (Health::Waiting(wait), _) => (
            format!("Not answering. Trying again in {}.", wait_text(wait)),
            true,
        ),
        (_, Some(Err(why))) => (format!("Test failed: {why}"), true),
        (_, Some(Ok(()))) => ("Connected.".into(), false),
        (Health::Ready, None) => ("Ready.".into(), false),
    })
}

fn wait_text(wait: Duration) -> String {
    let secs = wait.as_secs().max(1);
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{} min", secs.div_ceil(60))
    }
}

/// A feature as the activity log names it.
pub fn feature_label(feature: Feature) -> &'static str {
    match feature {
        Feature::Important => "Important",
        Feature::Signals => "Mail signals",
        Feature::Label => "Label suggestion",
        Feature::Palette => "⌘K suggestion",
        Feature::Rerank => "Best Matches",
        Feature::Line => "List summary",
        Feature::Summary => "Summary",
        Feature::CatchUp => "Catch-up",
        Feature::Replies => "Quick replies",
        Feature::Clean | Feature::CleanRejected => "Clean View",
        Feature::AskTerms => "Ask Pigeon search",
        Feature::Ask => "Ask Pigeon",
        Feature::DoIt => "Do It for Me",
        Feature::Event => "Add to Calendar",
        Feature::NeedsReplyDismissed => "Needs Reply",
        Feature::Test => "Test Connection",
    }
}

impl PigeonApp {
    /// Registers the handlers of this module's actions on the root.
    pub(super) fn on_ai_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(Self::go_to_needs_reply))
            .on_action(cx.listener(Self::dismiss_needs_reply))
    }

    fn go_to_needs_reply(&mut self, _: &GoToNeedsReply, _: &mut Window, _: &mut Context<Self>) {}

    fn dismiss_needs_reply(
        &mut self,
        _: &DismissNeedsReply,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// `When::Ai`: whether ⌘K offers an AI command now.
    pub(crate) fn ai_command_available(&self, need: AiNeed, cx: &App) -> bool {
        let b = self.backend.read(cx);
        let allows = |feature| b.ai_allows(feature);
        let mail = self.mode == Mode::Mail;
        let one = mail && self.checked.is_empty() && self.selected.is_some();
        let replyable = one
            && self
                .selected
                .is_some_and(|ix| self.data.threads[ix].folder != Folder::Drafts);
        let needs_reply = allows(Feature::Signals) && b.ai_prefs().needs_reply;
        match need {
            AiNeed::NeedsReply => mail && needs_reply,
            AiNeed::NeedsReplyThread => one && needs_reply,
            AiNeed::Label => one && allows(Feature::Label),
            AiNeed::Summary => one && allows(Feature::Summary),
            AiNeed::CleanView => one && allows(Feature::Clean) && allows(Feature::Signals),
            AiNeed::QuickReply => replyable && needs_reply && allows(Feature::Replies),
            AiNeed::Event => one && allows(Feature::Signals),
            AiNeed::Ask => allows(Feature::Ask) && allows(Feature::Rerank),
            AiNeed::DoIt => mail && allows(Feature::DoIt),
        }
    }

    /// Settings › AI's Test Connection.
    pub(crate) fn test_ai_connection(&mut self, provider: Provider, cx: &mut Context<Self>) {
        if !self.ai.testing.contains(&provider) {
            self.ai.testing.push(provider);
        }
        self.backend.update(cx, |b, cx| b.ai_test(provider, cx));
        cx.notify();
    }

    /// A provider's status or the log changed: Settings › AI shows it.
    pub(super) fn on_ai_changed(&mut self, cx: &mut Context<Self>) {
        let providers = self.backend.read(cx).ai_providers();
        self.ai.testing.retain(|p| {
            let info = match p {
                Provider::Writing => &providers.writing,
                Provider::Jev => &providers.jev,
            };
            info.health.is_some() && info.last_test.is_none()
        });
        self.check_ai_paused(cx);
        if self.settings_page.open {
            cx.notify();
        }
    }

    /// Once per launch: AI is on but its note was confirmed for somewhere
    /// else, so nothing is sent until it's reviewed.
    pub(super) fn check_ai_paused(&mut self, cx: &mut Context<Self>) {
        let b = self.backend.read(cx);
        let providers = b.ai_providers();
        let configured = providers.writing.health.is_some() || providers.jev.health.is_some();
        let row = main_row(
            &settings::get(cx).ai,
            &b.ai_fingerprint(),
            configured,
            false,
        );
        if row != MainRow::Paused || PAUSED_SHOWN.swap(true, Ordering::Relaxed) {
            return;
        }
        let review = ToastAction::run("Review", |this, window, cx| {
            this.open_settings_section(&OpenSettingsSection(Section::Ai), window, cx)
        });
        self.show_toast(
            ToastKind::Info,
            "AI is paused until you review what is sent",
            Some(review),
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: &str = "gmail|llm.example.ts.net|openrouter.ai";

    fn prefs(enabled: bool, ack: Option<&str>) -> AiPrefs {
        AiPrefs {
            enabled,
            note_ack: ack.map(str::to_string),
            ..AiPrefs::default()
        }
    }

    #[test]
    fn the_main_row_follows_the_note_fingerprint() {
        // Off by default; turning on asks.
        assert_eq!(
            main_row(&prefs(false, None), HERE, true, false),
            MainRow::Off
        );
        assert_eq!(
            main_row(&prefs(false, None), HERE, true, true),
            MainRow::Asking
        );
        // Confirmed here: on.
        assert_eq!(
            main_row(&prefs(true, Some(HERE)), HERE, true, false),
            MainRow::On
        );
        // Confirmed for another host, or the demo: paused, the note again.
        let elsewhere = "gmail|other.example.com|openrouter.ai";
        assert_eq!(
            main_row(&prefs(true, Some(elsewhere)), HERE, true, false),
            MainRow::Paused
        );
        assert_eq!(
            main_row(
                &prefs(true, Some(HERE)),
                "demo|llm.example.ts.net|openrouter.ai",
                true,
                false
            ),
            MainRow::Paused
        );
        assert_eq!(
            main_row(&prefs(true, None), HERE, true, true),
            MainRow::Paused
        );
        // Turned off after confirming: off, and on again without asking.
        assert_eq!(
            main_row(&prefs(false, Some(HERE)), HERE, true, false),
            MainRow::Off
        );
        // No model in .env.
        assert_eq!(
            main_row(&prefs(true, Some(HERE)), HERE, false, false),
            MainRow::Unavailable
        );
    }

    #[test]
    fn the_note_names_where_text_goes() {
        let both = note(Some("llm.example.ts.net"), Some("openrouter.ai"));
        assert!(
            both.contains(
                "to llm.example.ts.net for writing and to OpenRouter (TypeSafe) for Jev."
            )
        );
        assert!(both.ends_with("text already sent can't be taken back."));
        assert!(note(None, Some("openrouter.ai")).contains("to OpenRouter (TypeSafe) for Jev."));
        assert!(note(Some("h.example"), None).contains("to h.example for writing."));
    }

    #[test]
    fn provider_status_reads_health_and_tests() {
        let info = |health, last_test| ProviderInfo {
            host: Some("h".into()),
            model: Some("m".into()),
            health,
            last_test,
        };
        assert_eq!(provider_status(&info(None, None), true, false), None);
        let status = |health, test, testing| {
            provider_status(&info(Some(health), test), true, testing).unwrap()
        };
        assert_eq!(status(Health::Ready, None, false), ("Ready.".into(), false));
        assert_eq!(
            status(Health::Ready, None, true),
            ("Testing…".into(), false)
        );
        assert_eq!(status(Health::Ready, Some(Ok(())), true).0, "Connected.");
        assert!(status(Health::Stopped, Some(Err("HTTP 401".into())), false).1);
        assert_eq!(
            status(Health::Waiting(Duration::from_secs(90)), None, false).0,
            "Not answering. Trying again in 2 min."
        );
        assert!(
            !provider_status(&info(Some(Health::Stopped), None), false, false)
                .unwrap()
                .1
        );
    }
}

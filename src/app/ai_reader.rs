//! The reader's AI (W3): label suggestion, summary and catch-up, Clean
//! View, quick replies and Add to Calendar, with their commands. What's
//! asked and what's shown is decided here; `ui::reader` draws it.

use std::collections::{HashMap, HashSet};

use chrono::{Local, TimeZone as _, Utc};
use gpui::{App, Context, Div, SharedString, Stateful, Window, prelude::*};

use super::{ComposeKind, Mode, PigeonApp, ToastKind};
use crate::{
    actions::{
        AddToCalendar, ApplySuggestedLabel, QuickReply, ReportBadCleanView, RetryCleanView,
        SummarizeConversation, ToggleCleanView,
    },
    ai::{Done, EventCandidate, Feature, Job, Work, reading, text},
    backend::gmail,
    data::Thread,
    ui::reader::ReaderUi,
};

/// What the reader's AI may show now, from the backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReaderOn {
    pub label: bool,
    pub summary: bool,
    pub catch_up: bool,
    pub clean: bool,
    pub replies: bool,
    pub events: bool,
}

/// AI-6's question for the open conversation, worked out once per open.
struct LabelAsk {
    message: usize,
    hash: String,
    options: Vec<(String, String)>,
}

#[derive(Default)]
pub struct ReaderAi {
    pub on: ReaderOn,
    /// The conversation opened last.
    open: Option<String>,
    /// AI-9: its messages unread when it opened, if an older one was read.
    snapshot: Option<Vec<String>>,
    /// Asked since it opened, so a failure isn't asked again at once.
    asked: HashSet<(Feature, String)>,
    /// ⇧S: the summary shown or hidden, against its default, by
    /// conversation.
    summary: HashMap<String, bool>,
    /// AI-8 shows on its own; `None` until its words can be counted.
    summary_due: Option<bool>,
    /// `None` until worked out; `Some(None)` when there's nothing to ask.
    label: Option<Option<LabelAsk>>,
    /// AI-10: newsletters too long for a Clean View.
    over_budget: HashSet<String>,
    /// Clean Views asked again: the one made before (its `created`) no
    /// longer counts.
    retrying: HashMap<String, i64>,
}

impl ReaderAi {
    /// A conversation opened, with its messages' (id, unread). False when
    /// it's the one already open, so re-clicks, auto-advance and undo keep
    /// its snapshot.
    pub fn open(&mut self, thread_id: &str, messages: &[(&str, bool)]) -> bool {
        if self.open.as_deref() == Some(thread_id) {
            return false;
        }
        self.open = Some(thread_id.to_string());
        self.snapshot = reading::catch_up_snapshot(messages);
        self.asked.clear();
        self.summary_due = None;
        self.label = None;
        self.retrying.clear();
        true
    }

    /// The conversation open, in a folder the reader's AI is for.
    fn is_open(&self, thread: &Thread) -> bool {
        self.open.as_deref() == Some(thread.id.as_str()) && reading::reader_allowed(thread)
    }

    /// The summary block shows: catch-up by default, AI-8 when it's due,
    /// either as ⇧S last left it.
    fn summary_shown(&self, thread_id: &str) -> bool {
        let default = self.snapshot.is_some() || self.summary_due == Some(true);
        self.summary.get(thread_id).copied().unwrap_or(default)
    }
}

/// The summary block (AI-8, AI-9) as drawn.
pub(crate) struct SummaryView {
    /// A catch-up, of this many new messages.
    pub catch_up: Option<usize>,
    /// The answer, or as much as has streamed.
    pub text: Option<String>,
    pub failed: bool,
}

/// A message's Clean View (AI-10).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CleanView {
    /// Not a newsletter, reported, too long, or Clean View is off.
    Off,
    /// Not made yet: the original.
    Waiting,
    /// Made while the original showed: the original, with the bar.
    Ready,
    /// Its HTML.
    Clean(SharedString),
    /// The user went back to the original.
    Original,
    Failed,
}

impl CleanView {
    fn made(&self) -> bool {
        matches!(
            self,
            CleanView::Ready | CleanView::Clean(_) | CleanView::Original
        )
    }
}

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

    /// `select_thread`, opening another conversation: the last one's jobs
    /// go, the catch-up snapshot is taken (before its messages are marked
    /// read), and this one's are asked for at Interactive.
    pub(super) fn reader_ai_open(&mut self, ix: usize, cx: &mut Context<Self>) {
        let thread = &self.data.threads[ix];
        let messages: Vec<(&str, bool)> = thread
            .messages
            .iter()
            .map(|m| {
                let unread = m.label_ids.iter().any(|l| l == gmail::UNREAD);
                (m.id.as_str(), unread)
            })
            .collect();
        if !self.ai.reader.open(&thread.id, &messages) {
            return;
        }
        let id = thread.id.clone();
        self.backend
            .update(cx, |b, _| b.ai_cancel(crate::ai::JobGroup::Open));
        cx.default_global::<ReaderUi>().clean_waiting.clear();
        let backend = self.backend.clone();
        self.ai.request_b(&id, &backend, cx);
        self.reader_ai_jobs(cx);
    }

    /// `AiChanged` or `BodiesReady`: what may show, and the open
    /// conversation's jobs that can go now.
    pub(super) fn reader_ai_changed(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let open = self
            .ai
            .reader
            .open
            .as_ref()
            .is_some_and(|id| ids.is_empty() || ids.contains(id));
        if open {
            self.reader_ai_jobs(cx);
        } else {
            self.refresh_reader_on(cx);
        }
    }

    fn refresh_reader_on(&mut self, cx: &mut Context<Self>) {
        let b = self.backend.read(cx);
        let allows = |f| b.ai_allows(f);
        self.ai.reader.on = ReaderOn {
            label: allows(Feature::Label),
            summary: allows(Feature::Summary),
            catch_up: allows(Feature::CatchUp),
            clean: allows(Feature::Clean) && allows(Feature::Signals),
            replies: self.ai.needs_reply_on && allows(Feature::Replies),
            events: allows(Feature::Signals),
        };
    }

    /// The open conversation's reader jobs that are missing, haven't been
    /// asked since it opened, and have the bodies they need.
    fn reader_ai_jobs(&mut self, cx: &mut Context<Self>) {
        self.refresh_reader_on(cx);
        let Some(ix) = self
            .ai
            .reader
            .open
            .as_ref()
            .and_then(|id| self.data.thread_ix(id))
        else {
            return;
        };
        let replies_blocked = self.has_reply_draft(ix);
        let needs_reply = self.ai.needs_reply.contains(&self.data.threads[ix].id);
        let thread = &self.data.threads[ix];
        if !reading::reader_allowed(thread) {
            return;
        }
        let r = &mut self.ai.reader;
        let on = r.on;
        let mut jobs: Vec<Job> = Vec::new();
        let mut ask =
            |r: &mut ReaderAi, feature, key: &str, hash: &str, build: &dyn Fn() -> Option<Job>| {
                let retrying = feature == Feature::Clean && r.retrying.contains_key(key);
                if (!retrying && thread.ai.fresh(feature, key, hash).is_some())
                    || r.asked.contains(&(feature, key.to_string()))
                {
                    return;
                }
                if let Some(job) = build() {
                    r.asked.insert((feature, key.to_string()));
                    jobs.push(job);
                }
            };

        // AI-6.
        if on.label {
            if r.label.is_none() {
                let now = Utc::now().timestamp_millis();
                let shortlist = reading::shortlist(&self.data.labels, &self.data.threads, now);
                r.label = Some(
                    reading::label_message(thread, &self.data.account)
                        .filter(|_| !shortlist.is_empty())
                        .map(|message| {
                            let ids: Vec<&str> = shortlist
                                .iter()
                                .map(|&l| self.data.labels[l].id.as_str())
                                .collect();
                            LabelAsk {
                                message,
                                hash: reading::label_hash(&thread.messages[message].id, &ids),
                                options: reading::label_options(
                                    &self.data.labels,
                                    &shortlist,
                                    &self.data.threads,
                                    &thread.id,
                                ),
                            }
                        }),
                );
            }
            if let Some(Some(label)) = &r.label {
                let (hash, message, options) =
                    (label.hash.clone(), label.message, label.options.clone());
                ask(r, Feature::Label, &thread.id, &hash, &|| {
                    let msg = text::Message::from_thread(thread, message)?;
                    let request = reading::label_request(&msg, options.clone());
                    Some(reading::job(
                        Feature::Label,
                        &thread.id,
                        thread,
                        hash.clone(),
                        Work::Jev(request),
                    ))
                });
            }
        }

        // AI-8, AI-9.
        if r.summary_due.is_none() {
            r.summary_due = reading::summary_due(thread);
        }
        if r.summary_shown(&thread.id) {
            match r.snapshot.clone() {
                Some(snapshot) if on.catch_up => {
                    let hash = reading::catch_up_hash(&snapshot);
                    ask(r, Feature::CatchUp, &thread.id, &hash, &|| {
                        reading::catch_up_job(thread, &snapshot)
                    });
                }
                None if on.summary => {
                    if let Some(hash) = reading::summary_hash(thread) {
                        ask(r, Feature::Summary, &thread.id, &hash, &|| {
                            reading::summary_job(thread)
                        });
                    }
                }
                _ => {}
            }
        }

        // AI-10.
        if on.clean {
            for (mix, m) in thread.messages.iter().enumerate() {
                let hash = reading::clean_hash(&m.id);
                let made = !r.retrying.contains_key(&m.id)
                    && thread.ai.fresh(Feature::Clean, &m.id, &hash).is_some();
                if made
                    || r.asked.contains(&(Feature::Clean, m.id.clone()))
                    || r.over_budget.contains(&m.id)
                    || !reading::wants_clean(&thread.ai, &m.id)
                {
                    continue;
                }
                // Its body isn't stored yet: on `BodiesReady`.
                let Some(msg) = reading::message(thread, mix) else {
                    continue;
                };
                let Some(request) = reading::request(&msg) else {
                    r.over_budget.insert(m.id.clone());
                    continue;
                };
                ask(r, Feature::Clean, &m.id, &hash, &|| {
                    Some(reading::job(
                        Feature::Clean,
                        &m.id,
                        thread,
                        hash.clone(),
                        Work::Llm(request.clone()),
                    ))
                });
            }
        }

        // AI-11.
        if on.replies
            && needs_reply
            && !replies_blocked
            && let (Some(key), Some(hash)) =
                (reading::replies_key(thread), reading::replies_hash(thread))
        {
            ask(r, Feature::Replies, &key, &hash, &|| {
                reading::replies_job(thread)
            });
        }

        for job in jobs {
            self.backend.update(cx, |b, cx| {
                b.ai_enqueue(job, cx);
            });
        }
    }

    /// A reply to this conversation is being written already.
    fn has_reply_draft(&self, ix: usize) -> bool {
        let id = &self.data.threads[ix].id;
        self.reply_draft_of(id).is_some()
            || (self.selected == Some(ix) && self.reply_card_composer().is_some())
    }

    // What the reader shows.

    /// AI-6: the label to suggest, by index into `data.labels`.
    pub(crate) fn suggested_label(&self, thread: &Thread) -> Option<usize> {
        let r = &self.ai.reader;
        if !r.on.label || !r.is_open(thread) {
            return None;
        }
        let Some(Some(ask)) = &r.label else {
            return None;
        };
        let row = thread.ai.fresh(Feature::Label, &thread.id, &ask.hash)?;
        let id = reading::suggested(&row.value)?;
        if thread.labels.contains(&id) || !ask.options.iter().any(|(o, _)| *o == id) {
            return None;
        }
        self.data.labels.iter().position(|l| l.id == id)
    }

    /// AI-8 or AI-9's block, if it shows.
    pub(crate) fn summary_view(&self, thread: &Thread, cx: &App) -> Option<SummaryView> {
        let r = &self.ai.reader;
        if !r.is_open(thread) || !r.summary_shown(&thread.id) {
            return None;
        }
        let (feature, hash, catch_up) = match &r.snapshot {
            Some(snapshot) if r.on.catch_up => (
                Feature::CatchUp,
                reading::catch_up_hash(snapshot),
                Some(snapshot.len()),
            ),
            None if r.on.summary => (Feature::Summary, reading::summary_hash(thread)?, None),
            _ => return None,
        };
        let b = self.backend.read(cx);
        let text = thread
            .ai
            .fresh(feature, &thread.id, &hash)
            .map(|row| row.value.clone())
            .or_else(|| b.ai_partial(feature, &thread.id))
            .filter(|t| !t.trim().is_empty());
        let failed =
            text.is_none() && matches!(b.ai_done(feature, &thread.id), Some(Done::Failed(_)));
        Some(SummaryView {
            catch_up,
            text,
            failed,
        })
    }

    /// AI-10: message `ix`'s Clean View. Notes the ones still being made,
    /// so they arrive as a bar rather than replacing what's read.
    pub(crate) fn clean_view(&self, thread: &Thread, ix: usize, cx: &mut App) -> CleanView {
        let r = &self.ai.reader;
        let m = &thread.messages[ix];
        if !r.on.clean
            || !r.is_open(thread)
            || !reading::wants_clean(&thread.ai, &m.id)
            || r.over_budget.contains(&m.id)
        {
            return CleanView::Off;
        }
        let before = r.retrying.get(&m.id).copied();
        let row = thread
            .ai
            .fresh(Feature::Clean, &m.id, &reading::clean_hash(&m.id))
            .filter(|row| before.is_none_or(|b| row.created > b));
        let Some(row) = row else {
            let failed = matches!(
                self.backend.read(cx).ai_done(Feature::Clean, &m.id),
                Some(Done::Failed(_))
            );
            if failed {
                return CleanView::Failed;
            }
            cx.default_global::<ReaderUi>()
                .clean_waiting
                .insert(m.id.clone());
            return CleanView::Waiting;
        };
        let ui = cx.default_global::<ReaderUi>();
        match ui.clean_choice.get(&m.id).copied() {
            Some(false) => return CleanView::Original,
            None if ui.clean_waiting.contains(&m.id) => return CleanView::Ready,
            _ => {}
        }
        let html = match ui.clean_html.get(&m.id) {
            Some((created, html)) if *created == row.created => html.clone(),
            _ => {
                let html: SharedString =
                    reading::to_html(&reading::clean(&row.value, m.html())).into();
                ui.clean_html
                    .insert(m.id.clone(), (row.created, html.clone()));
                html
            }
        };
        CleanView::Clean(html)
    }

    /// AI-11: the reply chips under the last message.
    pub(crate) fn quick_replies(&self, thread: &Thread) -> Vec<String> {
        let r = &self.ai.reader;
        if !r.on.replies || !r.is_open(thread) || !self.ai.needs_reply.contains(&thread.id) {
            return Vec::new();
        }
        let Some(ix) = self.data.thread_ix(&thread.id) else {
            return Vec::new();
        };
        if self.has_reply_draft(ix) {
            return Vec::new();
        }
        let (Some(key), Some(hash)) = (reading::replies_key(thread), reading::replies_hash(thread))
        else {
            return Vec::new();
        };
        thread
            .ai
            .fresh(Feature::Replies, &key, &hash)
            .map(|row| reading::replies(&row.value))
            .unwrap_or_default()
    }

    /// AI-16: the date the bar offers, and the message it's in.
    pub(crate) fn event_offer(&self, thread: &Thread, cx: &App) -> Option<(usize, EventCandidate)> {
        if !self.ai.reader.on.events || !reading::reader_allowed(thread) {
            return None;
        }
        let found = reading::event_candidates(thread);
        if found.is_empty() {
            return None;
        }
        let calendar = self.calendar.read(cx);
        let events = &calendar.data.events;
        let existing: Vec<chrono::DateTime<Utc>> = found
            .iter()
            .flat_map(|(_, c, _)| {
                let local = c.start.with_timezone(&Local).naive_local();
                crate::calendar::recurrence::occurrences(
                    events,
                    |_| true,
                    local,
                    local + chrono::Duration::minutes(1),
                )
                .into_iter()
                .filter(move |o| o.start == local)
                .filter_map(|o| Local.from_local_datetime(&o.start).earliest())
                .map(|d| d.with_timezone(&Utc))
            })
            .collect();
        let candidates: Vec<EventCandidate> = found.iter().map(|(_, c, _)| c.clone()).collect();
        let answers: Vec<Option<f64>> = found.iter().map(|(_, _, yes)| *yes).collect();
        let pick = reading::pick_candidate(&candidates, &answers, Utc::now(), &existing)?;
        Some((found[pick].0, candidates[pick].clone()))
    }

    /// The message a Clean View command acts on: the focused one, else
    /// the latest whose view `fits`.
    fn clean_target(
        &self,
        fits: impl Fn(&CleanView) -> bool,
        cx: &mut App,
    ) -> Option<(usize, usize, CleanView)> {
        let ix = self.selected?;
        let thread = &self.data.threads[ix];
        let order = self
            .focused_message
            .filter(|_| self.reader_focused)
            .into_iter()
            .chain((0..thread.messages.len()).rev());
        for m in order {
            let view = self.clean_view(thread, m, cx);
            if fits(&view) {
                return Some((ix, m, view));
            }
        }
        None
    }

    /// The bar's Show, and ⇧V.
    pub(crate) fn set_clean_choice(
        &mut self,
        message_id: &str,
        clean: bool,
        cx: &mut Context<Self>,
    ) {
        cx.default_global::<ReaderUi>()
            .clean_choice
            .insert(message_id.to_string(), clean);
        cx.notify();
    }

    // Commands.

    pub(crate) fn apply_suggested_label(
        &mut self,
        _: &ApplySuggestedLabel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.selected else { return };
        if let Some(label) = self.suggested_label(&self.data.threads[ix]) {
            self.set_label(label, vec![ix], false, cx);
        }
    }

    fn summarize_conversation(
        &mut self,
        _: &SummarizeConversation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.selected else { return };
        let on = self.ai.reader.on;
        if !(on.summary || on.catch_up) || !self.ai.reader.is_open(&self.data.threads[ix]) {
            return;
        }
        let id = self.data.threads[ix].id.clone();
        let shown = self.ai.reader.summary_shown(&id);
        self.ai.reader.summary.insert(id, !shown);
        self.reader_ai_jobs(cx);
        cx.notify();
    }

    fn toggle_clean_view(&mut self, _: &ToggleCleanView, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((ix, m, view)) = self.clean_target(CleanView::made, cx) {
            let id = self.data.threads[ix].messages[m].id.clone();
            let clean = !matches!(view, CleanView::Clean(_));
            self.set_clean_choice(&id, clean, cx);
        }
    }

    /// A failed or reported Clean View, asked for again.
    pub(crate) fn retry_clean_view(
        &mut self,
        _: &RetryCleanView,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .clean_target(|v| *v == CleanView::Failed, cx)
            .map(|(ix, m, _)| (ix, m))
            .or_else(|| {
                // Reported: its row is what makes it `Off`.
                let ix = self.selected?;
                let thread = &self.data.threads[ix];
                let m = (0..thread.messages.len()).rev().find(|&m| {
                    let id = &thread.messages[m].id;
                    thread.ai.get(Feature::CleanRejected, id).is_some()
                })?;
                Some((ix, m))
            });
        let Some((ix, m)) = target else { return };
        let thread = &mut self.data.threads[ix];
        let thread_id = thread.id.clone();
        let id = thread.messages[m].id.clone();
        let rejected = thread.ai.get(Feature::CleanRejected, &id).is_some();
        if let Some(row) = thread.ai.get(Feature::Clean, &id) {
            self.ai.reader.retrying.insert(id.clone(), row.created);
        }
        if rejected {
            thread
                .ai
                .rows
                .retain(|r| !(r.feature == Feature::CleanRejected && r.key == id));
        }
        self.backend.update(cx, |b, cx| {
            if rejected {
                b.ai_mark(Feature::CleanRejected, &thread_id, &id, false, cx);
            }
            b.ai_retry(Feature::Clean, &id);
        });
        self.ai.reader.asked.remove(&(Feature::Clean, id.clone()));
        self.ai.reader.over_budget.remove(&id);
        let ui = cx.default_global::<ReaderUi>();
        ui.clean_choice.remove(&id);
        ui.clean_waiting.remove(&id);
        self.reader_ai_jobs(cx);
        cx.notify();
    }

    fn report_bad_clean_view(
        &mut self,
        _: &ReportBadCleanView,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((ix, m, _)) = self.clean_target(CleanView::made, cx) else {
            return;
        };
        let thread_id = self.data.threads[ix].id.clone();
        let id = self.data.threads[ix].messages[m].id.clone();
        cx.default_global::<ReaderUi>().clean_choice.remove(&id);
        self.backend.update(cx, |b, cx| {
            b.ai_mark(Feature::CleanRejected, &thread_id, &id, true, cx)
        });
        self.show_toast(ToastKind::Done, "Showing the original", None, cx);
    }

    pub(crate) fn quick_reply(
        &mut self,
        action: &QuickReply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.selected else { return };
        let Some(text) = self
            .quick_replies(&self.data.threads[ix])
            .into_iter()
            .nth(action.0)
        else {
            return;
        };
        self.compose_with_body(ComposeKind::Reply, Some(text), window, cx);
    }

    pub(crate) fn add_to_calendar(
        &mut self,
        _: &AddToCalendar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.selected else { return };
        let Some((_, candidate)) = self.event_offer(&self.data.threads[ix], cx) else {
            return;
        };
        self.set_mode(Mode::Calendar, window, cx);
        self.calendar
            .update(cx, |c, cx| c.add_from_candidate(candidate, window, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_keeps_the_snapshot_until_another_conversation_opens() {
        let mut r = ReaderAi::default();
        // The first open: all unread, no catch-up.
        assert!(r.open("t1", &[("m1", true), ("m2", true)]));
        assert_eq!(r.snapshot, None);
        // Another, with new mail after read mail.
        assert!(r.open("t2", &[("a", false), ("b", true)]));
        assert_eq!(r.snapshot, Some(vec!["b".to_string()]));
        r.asked.insert((Feature::CatchUp, "t2".into()));
        // Opened again (re-click, undo), now read: kept.
        assert!(!r.open("t2", &[("a", false), ("b", false)]));
        assert_eq!(r.snapshot, Some(vec!["b".to_string()]));
        assert!(r.asked.contains(&(Feature::CatchUp, "t2".into())));
        assert!(r.summary_shown("t2"));
        // The next open replaces it.
        assert!(r.open("t3", &[("x", false)]));
        assert_eq!(r.snapshot, None);
        assert!(r.asked.is_empty());
        assert!(!r.summary_shown("t3"));
        r.summary.insert("t3".into(), true);
        assert!(r.summary_shown("t3"));
    }
}

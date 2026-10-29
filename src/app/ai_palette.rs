//! ⌘K's AI (W4): plain words (AI-12), Best Matches in search (AI-13), Ask
//! Pigeon (AI-14) and Do It for Me (AI-15), with their commands. Jobs go
//! through `Backend::ai_enqueue`; answers come back on `AiChanged` and are
//! read with `ai_done` and `ai_partial`. Each request has a serial key, so
//! an answer to older text never shows.

use std::time::Duration;

use gpui::{
    AppContext as _, Context, Div, Entity, EntityId, Focusable as _, Stateful, Subscription, Task,
    Window, prelude::*,
};
use nucleo_matcher::{
    Config, Matcher, Utf32String,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

use super::{Mode, PigeonApp, ToastKind};
use crate::{
    actions::{AiNeed, AskPigeon, COMMANDS, Command, DoItForMe},
    ai::{
        self, Done, Feature, Job, JobGroup, Output, Priority, Source, SourceId, Work, do_it, jev,
        search, text,
    },
    assets::Icon,
    calendar::{quick_add, recurrence},
    data::{Body, Folder},
    search::Query,
    ui::{
        ask_panel::{AskEvent, AskPanel, AskStatus},
        command_palette::{CommandPalette, Leading, PaletteCommand, PaletteEvent, PaletteItem},
    },
};

/// How long ⌘K's text settles before Jev is asked (AI-12).
const PLAIN_DELAY: Duration = Duration::from_millis(250);
const PLAIN_WORDS: usize = 3;
const PLAIN_CONFIDENCE: f64 = 0.6;
/// Jev takes 255 options; "none" is one.
const MAX_OPTIONS: usize = 254;
/// After this, Ask uses the snippet of mail whose body isn't in.
const BODY_WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct PaletteAi {
    serial: u64,
    /// The main palette, which plain words work in.
    main: Option<EntityId>,
    /// Its text as last typed.
    typed: String,
    plain: Option<Plain>,
    plain_task: Option<Task<()>>,
    best: Option<Best>,
    best_task: Option<Task<()>>,
    pub(super) ask_panel: Option<(Entity<AskPanel>, Subscription)>,
    ask: Option<Ask>,
    do_it: Option<DoIt>,
}

impl PaletteAi {
    fn next(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }
}

/// AI-12: a question asked; option `c<n>` is `commands[n - 1]`.
struct Plain {
    key: String,
    commands: Vec<&'static Command>,
}

/// AI-13: a search's rerank.
struct Best {
    /// The query as typed, for `refresh_rows` to match.
    raw: String,
    /// The selection when it started; moved since, the section is dropped.
    selected: Option<String>,
    ids: Vec<String>,
    records: Vec<search::Record>,
    pending: Vec<(usize, String)>,
    scores: Vec<(usize, f64)>,
    /// Thread ids shown under "Best Matches".
    shown: Vec<String>,
}

struct Ask {
    serial: u64,
    question: String,
    stage: AskStage,
    sources: Vec<Source>,
}

enum AskStage {
    Terms(String),
    Rerank {
        ids: Vec<String>,
        records: Vec<search::Record>,
        pending: Vec<(usize, String)>,
        scores: Vec<(usize, f64)>,
        events: Vec<Source>,
    },
    Bodies {
        mail: Vec<String>,
        events: Vec<Source>,
        due: bool,
    },
    Answer(String),
    Done,
}

struct DoIt {
    palette: EntityId,
    account: String,
    stage: DoItStage,
}

enum DoItStage {
    Prompt,
    Working(String),
    Preview { plan: do_it::Plan, ids: Vec<String> },
}

/// AI-12: the options' order, best first, when there are more commands
/// than Jev takes: by fuzzy score of the typed words on each title.
pub fn top_options(text: &str, titles: &[&str], max: usize) -> Vec<usize> {
    if titles.len() <= max {
        return (0..titles.len()).collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let patterns: Vec<Pattern> = text
        .split_whitespace()
        .map(|w| {
            Pattern::new(
                w,
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
            )
        })
        .collect();
    let mut scored: Vec<(usize, u32)> = titles
        .iter()
        .enumerate()
        .map(|(ix, title)| {
            let hay = Utf32String::from(*title);
            let score = patterns
                .iter()
                .filter_map(|p| p.score(hay.slice(..), &mut matcher))
                .sum();
            (ix, score)
        })
        .collect();
    scored.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
    scored.into_iter().take(max).map(|(ix, _)| ix).collect()
}

/// AI-12: the option picked, by index, when it isn't "none" and Jev is
/// sure enough.
pub fn suggestion(answer: Option<&jev::Answer>, options: usize) -> Option<usize> {
    let (id, confidence) = answer?.choice()?;
    let n: usize = id.strip_prefix('c')?.parse().ok()?;
    (confidence >= PLAIN_CONFIDENCE && (1..=options).contains(&n)).then(|| n - 1)
}

/// A delayed ask still goes: nothing typed since, no newer request.
fn still_current(typed: &str, latest: u64, text: &str, serial: u64) -> bool {
    typed == text && latest == serial
}

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

    fn may_ask(&self, feature: Feature, cx: &gpui::App) -> bool {
        self.backend.read(cx).ai_allows(feature)
    }

    fn queue_ask(
        &mut self,
        feature: Feature,
        key: &str,
        group: JobGroup,
        work: Work,
        cx: &mut Context<Self>,
    ) -> bool {
        let job = Job {
            priority: Priority::Interactive,
            group: Some(group),
            ..Job::new(feature, key, work)
        };
        self.backend.update(cx, |b, cx| b.ai_enqueue(job, cx))
    }

    fn cancel_asks(&mut self, group: JobGroup, cx: &mut Context<Self>) {
        self.backend.update(cx, |b, _| b.ai_cancel(group));
    }

    /// `open_palette` made the main palette.
    pub(super) fn palette_opened(&mut self, palette: EntityId) {
        self.ai.palette.main = Some(palette);
        self.ai.palette.typed.clear();
    }

    /// The palette closed: what it asked is moot.
    pub(super) fn palette_closed(&mut self, cx: &mut Context<Self>) {
        let p = &mut self.ai.palette;
        let asked = p.plain.take().is_some() || p.do_it.take().is_some();
        p.plain_task = None;
        p.main = None;
        if asked {
            self.cancel_asks(JobGroup::Palette, cx);
        }
    }

    /// ⌘K's own events; true when handled here.
    pub(super) fn on_ai_palette_event(
        &mut self,
        palette: &Entity<CommandPalette>,
        event: &PaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = palette.entity_id();
        let do_it = self
            .ai
            .palette
            .do_it
            .as_ref()
            .is_some_and(|d| d.palette == id);
        match event {
            PaletteEvent::Query(text) => {
                if do_it {
                    self.do_it_edited(cx);
                } else if self.ai.palette.main == Some(id) {
                    self.plain_query(text, cx);
                }
                true
            }
            PaletteEvent::Text(text) if do_it => {
                self.do_it_submit(text, cx);
                true
            }
            PaletteEvent::Confirm => {
                self.do_it_confirm(window, cx);
                true
            }
            PaletteEvent::Suggested(command) => {
                let typed = self.ai.palette.typed.clone();
                if let PaletteCommand::Action(action) = command
                    && action().name() == gpui::Action::name(&DoItForMe)
                {
                    self.close_overlays_at_once(window, cx);
                    self.open_do_it(&typed, window, cx);
                } else {
                    self.on_palette_event(palette, &PaletteEvent::Run(*command), window, cx);
                }
                true
            }
            _ => false,
        }
    }

    // AI-12: plain words.

    fn plain_query(&mut self, text: &str, cx: &mut Context<Self>) {
        let p = &mut self.ai.palette;
        p.typed = text.to_string();
        p.plain_task = None;
        if p.plain.take().is_some() {
            self.cancel_asks(JobGroup::Palette, cx);
        }
        // Quick Add's text never goes to Jev.
        if text.split_whitespace().count() < PLAIN_WORDS
            || quick_add::command_text(text).is_some()
            || !self.may_ask(Feature::Palette, cx)
        {
            return;
        }
        let serial = self.ai.palette.next();
        let text = text.to_string();
        self.ai.palette.plain_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PLAIN_DELAY).await;
            this.update(cx, |this, cx| this.ask_plain(&text, serial, cx))
                .ok();
        }));
    }

    /// The commands ⌘K offers here, as the palette lists them.
    pub(super) fn available_commands(&self, cx: &gpui::App) -> Vec<&'static Command> {
        let scope = self.scope();
        COMMANDS
            .iter()
            .filter(|c| {
                !c.help_only
                    && (c.scope() == crate::actions::Scope::Global || c.scope() == scope)
                    && self.command_available(c.when, cx)
            })
            .collect()
    }

    fn ask_plain(&mut self, text: &str, serial: u64, cx: &mut Context<Self>) {
        if !still_current(&self.ai.palette.typed, self.ai.palette.serial, text, serial) {
            return;
        }
        let available = self.available_commands(cx);
        let titles: Vec<&str> = available.iter().map(|c| c.name).collect();
        let commands: Vec<&'static Command> = top_options(text, &titles, MAX_OPTIONS)
            .into_iter()
            .map(|ix| available[ix])
            .collect();
        let named: Vec<(&str, &str)> = commands.iter().map(|c| (c.name, c.group.title())).collect();
        let request = search::palette_request(text, &named);
        let key = format!("palette-{serial}");
        if self.queue_ask(
            Feature::Palette,
            &key,
            JobGroup::Palette,
            Work::Jev(request),
            cx,
        ) {
            self.ai.palette.plain = Some(Plain { key, commands });
        }
    }

    fn poll_plain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(plain) = &self.ai.palette.plain else {
            return;
        };
        // A newer text replaced `plain`: an older answer is never read.
        let b = self.backend.read(cx);
        let Some(done) = b.ai_done(Feature::Palette, &plain.key) else {
            return;
        };
        let pick = match done {
            Done::Ready(Output::Answers(answers)) => {
                suggestion(answers.get("command"), plain.commands.len())
            }
            _ => None,
        };
        let plain = self.ai.palette.plain.take().expect("checked");
        let (Some(ix), Some((palette, _))) = (pick, &self.palette) else {
            return;
        };
        if self.ai.palette.main != Some(palette.entity_id()) {
            return;
        }
        let item = self.command_item(plain.commands[ix], window, cx);
        palette
            .clone()
            .update(cx, |p, cx| p.set_suggested(item, cx));
    }

    // AI-13: Best Matches.

    /// The search changed: what was asked for the last one is moot.
    pub(super) fn search_changed(&mut self, cx: &mut Context<Self>) {
        let p = &mut self.ai.palette;
        p.best_task = None;
        if p.best.take().is_some() {
            self.cancel_asks(JobGroup::Search, cx);
        }
        if self.mode != Mode::Mail || !self.is_searching() || !self.may_ask(Feature::Rerank, cx) {
            return;
        }
        let Some(words) = self.search_query.plain_words() else {
            return;
        };
        if words.len() < PLAIN_WORDS {
            return;
        }
        let raw = self.search_input.read(cx).value().to_string();
        let serial = self.ai.palette.next();
        self.ai.palette.best_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::search::SEARCH_DEBOUNCE)
                .await;
            this.update(cx, |this, cx| {
                this.ask_best(raw, words.join(" "), serial, cx)
            })
            .ok();
        }));
    }

    fn ask_best(&mut self, raw: String, query: String, serial: u64, cx: &mut Context<Self>) {
        if self.ai.palette.serial != serial || !self.is_searching() {
            return;
        }
        let ixs = search::candidates(std::slice::from_ref(&query), &self.data, search::CANDIDATES);
        let records = search::records(&ixs, &self.data);
        let ids: Vec<String> = ixs
            .iter()
            .map(|&ix| self.data.threads[ix].id.clone())
            .collect();
        let pending = self.queue_rerank(&query, &records, "search", serial, JobGroup::Search, cx);
        if pending.is_empty() {
            return;
        }
        self.ai.palette.best = Some(Best {
            raw,
            selected: self.selected.map(|ix| self.data.threads[ix].id.clone()),
            ids,
            records,
            pending,
            scores: Vec::new(),
            shown: Vec::new(),
        });
    }

    /// One Jev request per batch of records; (offset, key) of each queued.
    fn queue_rerank(
        &mut self,
        query: &str,
        records: &[search::Record],
        prefix: &str,
        serial: u64,
        group: JobGroup,
        cx: &mut Context<Self>,
    ) -> Vec<(usize, String)> {
        let mut pending = Vec::new();
        for (b, batch) in records.chunks(search::BATCH).enumerate() {
            let key = format!("{prefix}-{serial}-{b}");
            let work = Work::Jev(search::rerank(query, batch));
            if self.queue_ask(Feature::Rerank, &key, group, work, cx) {
                pending.push((b * search::BATCH, key));
            }
        }
        pending
    }

    /// Scores in for the batches answered; true once none is pending.
    fn take_scores(
        &self,
        records: &[search::Record],
        pending: &mut Vec<(usize, String)>,
        scores: &mut Vec<(usize, f64)>,
        cx: &gpui::App,
    ) -> bool {
        let b = self.backend.read(cx);
        pending.retain(|(offset, key)| {
            let Some(done) = b.ai_done(Feature::Rerank, key) else {
                return true;
            };
            if let Done::Ready(Output::Answers(answers)) = done {
                let batch = &records[*offset..(*offset + search::BATCH).min(records.len())];
                scores.extend(
                    search::read_scores(batch, answers)
                        .into_iter()
                        .map(|(pos, s)| (offset + pos, s)),
                );
            }
            false
        });
        pending.is_empty()
    }

    fn poll_best(&mut self, cx: &mut Context<Self>) {
        let Some(mut best) = self.ai.palette.best.take() else {
            return;
        };
        if best.pending.is_empty()
            || !self.take_scores(&best.records, &mut best.pending, &mut best.scores, cx)
        {
            self.ai.palette.best = Some(best);
            return;
        }
        let selected = self.selected.map(|ix| self.data.threads[ix].id.clone());
        let scored = best
            .scores
            .iter()
            .map(|&(pos, s)| (best.ids[pos].clone(), s))
            .collect();
        best.shown = search::best(scored, search::BEST_SHOWN);
        // Moved since the search started: nothing shifts under it.
        if selected != best.selected || best.shown.is_empty() {
            return;
        }
        self.ai.palette.best = Some(best);
        self.refresh_rows(true);
        cx.notify();
    }

    /// The Best Matches of the search showing, by thread index.
    pub(super) fn best_match_rows(&self) -> Vec<usize> {
        match &self.ai.palette.best {
            Some(best)
                if !best.shown.is_empty() && Query::parse(&best.raw) == self.search_query =>
            {
                best.shown
                    .iter()
                    .filter_map(|id| self.data.thread_ix(id))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    // AI-14: Ask Pigeon.

    fn ask_pigeon(&mut self, _: &AskPigeon, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ai_command_available(AiNeed::Ask, cx) {
            return;
        }
        if let Some((panel, _)) = &self.ai.palette.ask_panel {
            return window.focus(&panel.focus_handle(cx), cx);
        }
        self.palette = None;
        self.palette_closed(cx);
        self.help_open = false;
        self.contact_card = None;
        self.later_picker = None;
        self.leaving_overlay = None;
        let panel = cx.new(|cx| AskPanel::new(window, cx));
        let events = cx.subscribe_in(&panel, window, Self::on_ask_event);
        window.focus(&panel.focus_handle(cx), cx);
        self.ai.palette.ask_panel = Some((panel, events));
        cx.notify();
    }

    /// The panel went: so does its question.
    pub(super) fn ask_closed(&mut self, cx: &mut Context<Self>) {
        if self.ai.palette.ask_panel.take().is_some() {
            self.ai.palette.ask = None;
            self.cancel_asks(JobGroup::Ask, cx);
        }
    }

    fn on_ask_event(
        &mut self,
        _: &Entity<AskPanel>,
        event: &AskEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AskEvent::Submit(question) => self.ask_start(question, cx),
            AskEvent::Dismiss => self.close_overlays(window, cx),
            AskEvent::Open(ix) => self.ask_open_source(*ix, window, cx),
        }
    }

    fn ask_show(&mut self, status: AskStatus, answer: String, cx: &mut Context<Self>) {
        let sources = self
            .ai
            .palette
            .ask
            .as_ref()
            .map(|a| a.sources.clone())
            .unwrap_or_default();
        if let Some((panel, _)) = &self.ai.palette.ask_panel {
            panel.update(cx, |p, cx| p.show(status, answer, sources, cx));
        }
    }

    fn ask_failed(&mut self, why: &str, cx: &mut Context<Self>) {
        if let Some(ask) = &mut self.ai.palette.ask {
            ask.stage = AskStage::Done;
        }
        self.ask_show(AskStatus::Failed(why.to_string().into()), String::new(), cx);
    }

    fn ask_start(&mut self, question: &str, cx: &mut Context<Self>) {
        self.cancel_asks(JobGroup::Ask, cx);
        let serial = self.ai.palette.next();
        let key = format!("ask-terms-{serial}");
        let today = chrono::Local::now().date_naive();
        let work = Work::Llm(search::terms_request(question, today));
        self.ai.palette.ask = Some(Ask {
            serial,
            question: question.to_string(),
            stage: AskStage::Terms(key.clone()),
            sources: Vec::new(),
        });
        if self.queue_ask(Feature::AskTerms, &key, JobGroup::Ask, work, cx) {
            self.ask_show(AskStatus::Searching, String::new(), cx);
        } else {
            self.ask_failed("Ask Pigeon isn't available now. Check Settings › AI.", cx);
        }
    }

    fn poll_ask(&mut self, cx: &mut Context<Self>) {
        let Some(mut ask) = self.ai.palette.ask.take() else {
            return;
        };
        let stage = std::mem::replace(&mut ask.stage, AskStage::Done);
        let serial = ask.serial;
        let question = ask.question.clone();
        self.ai.palette.ask = Some(ask);
        match stage {
            AskStage::Terms(key) => {
                let done = self
                    .backend
                    .read(cx)
                    .ai_done(Feature::AskTerms, &key)
                    .cloned();
                match done {
                    None => self.ask_stage(AskStage::Terms(key)),
                    Some(Done::Ready(Output::Text(answer))) => match search::read_terms(&answer) {
                        Some(terms) => self.ask_search(&question, &terms, serial, cx),
                        None => self
                            .ask_failed("Pigeon couldn't search for that. Try rewording it.", cx),
                    },
                    Some(_) => self.ask_failed(
                        "Ask Pigeon couldn't reach the writing model. Try again.",
                        cx,
                    ),
                }
            }
            AskStage::Rerank {
                ids,
                records,
                mut pending,
                mut scores,
                events,
            } => {
                if !self.take_scores(&records, &mut pending, &mut scores, cx) {
                    return self.ask_stage(AskStage::Rerank {
                        ids,
                        records,
                        pending,
                        scores,
                        events,
                    });
                }
                let scored = scores
                    .iter()
                    .map(|&(pos, s)| (ids[pos].clone(), s))
                    .collect();
                let mail = search::best(scored, search::ASK_MAIL);
                self.ask_bodies(mail, events, serial, cx);
            }
            AskStage::Bodies { mail, events, due } => {
                let ready = mail.iter().all(|id| {
                    self.data.thread_ix(id).is_none_or(|ix| {
                        !matches!(
                            self.data.threads[ix].last_message().body,
                            Body::NotLoaded | Body::Loading
                        )
                    })
                });
                if ready || due {
                    self.ask_answer(&question, &mail, events, serial, cx);
                } else {
                    self.ask_stage(AskStage::Bodies { mail, events, due });
                }
            }
            AskStage::Answer(key) => {
                let b = self.backend.read(cx);
                let valid: Vec<usize> = self
                    .ai
                    .palette
                    .ask
                    .as_ref()
                    .map(|a| a.sources.iter().map(|s| s.n).collect())
                    .unwrap_or_default();
                match b.ai_done(Feature::Ask, &key).cloned() {
                    None => {
                        let partial = b.ai_partial(Feature::Ask, &key).unwrap_or_default();
                        self.ask_stage(AskStage::Answer(key));
                        let shown = search::keep_citations(&partial, &valid);
                        self.ask_show(AskStatus::Writing, shown, cx);
                    }
                    Some(Done::Ready(Output::Text(answer))) => {
                        let shown = search::keep_citations(answer.trim(), &valid);
                        self.ask_show(AskStatus::Done, shown, cx);
                    }
                    Some(_) => {
                        self.ask_failed("Ask Pigeon couldn't finish the answer. Try again.", cx)
                    }
                }
            }
            AskStage::Done => {}
        }
    }

    fn ask_stage(&mut self, stage: AskStage) {
        if let Some(ask) = &mut self.ai.palette.ask {
            ask.stage = stage;
        }
    }

    /// Step 2 and 3: mail candidates to rerank, and events in range.
    fn ask_search(
        &mut self,
        question: &str,
        terms: &search::AskTerms,
        serial: u64,
        cx: &mut Context<Self>,
    ) {
        let events = if terms.calendar {
            self.ask_events(terms, cx)
        } else {
            Vec::new()
        };
        let ixs = if terms.terms.is_empty() {
            Vec::new()
        } else {
            search::candidates(&terms.terms, &self.data, search::CANDIDATES)
        };
        let records = search::records(&ixs, &self.data);
        let ids: Vec<String> = ixs
            .iter()
            .map(|&ix| self.data.threads[ix].id.clone())
            .collect();
        let pending = self.queue_rerank(question, &records, "ask", serial, JobGroup::Ask, cx);
        if pending.is_empty() {
            return self.ask_bodies(Vec::new(), events, serial, cx);
        }
        self.ask_stage(AskStage::Rerank {
            ids,
            records,
            pending,
            scores: Vec::new(),
            events,
        });
    }

    fn ask_events(&self, terms: &search::AskTerms, cx: &gpui::App) -> Vec<Source> {
        let calendar = &self.calendar.read(cx).data;
        let (from, to) = terms.range(chrono::Local::now().date_naive());
        let start = from.and_hms_opt(0, 0, 0).expect("midnight");
        let end = (to + chrono::Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight");
        recurrence::occurrences(&calendar.events, |_| true, start, end)
            .into_iter()
            .take(search::ASK_EVENTS)
            .map(|o| {
                let event = &calendar.events[o.event];
                let date = if event.all_day {
                    format!("{} (all day)", o.start.format("%a %b %-d %Y"))
                } else {
                    format!(
                        "{}–{}",
                        o.start.format("%a %b %-d %Y, %-I:%M %p"),
                        o.end.format("%-I:%M %p")
                    )
                };
                let description: String = event.description.chars().take(200).collect();
                let excerpt = [event.location.as_str(), description.as_str()]
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                Source {
                    n: 0,
                    id: SourceId::Event {
                        id: event.id.clone(),
                        occurrence: o.start,
                    },
                    title: event.display_title().to_string(),
                    date,
                    excerpt,
                }
            })
            .collect()
    }

    /// Step 4: the chosen mail's bodies, asked for; after `BODY_WAIT` the
    /// snippet does.
    fn ask_bodies(
        &mut self,
        mail: Vec<String>,
        events: Vec<Source>,
        serial: u64,
        cx: &mut Context<Self>,
    ) {
        for id in &mail {
            let Some(ix) = self.data.thread_ix(id) else {
                continue;
            };
            self.ensure_bodies(ix, cx);
            if matches!(self.data.threads[ix].last_message().body, Body::NotLoaded) {
                self.backend.update(cx, |b, cx| b.fetch_body(id, cx));
            }
        }
        self.ask_stage(AskStage::Bodies {
            mail,
            events,
            due: false,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BODY_WAIT).await;
            this.update(cx, |this, cx| {
                if let Some(ask) = &mut this.ai.palette.ask
                    && ask.serial == serial
                    && let AskStage::Bodies { due, .. } = &mut ask.stage
                {
                    *due = true;
                    this.poll_ask(cx);
                }
            })
            .ok();
        })
        .detach();
        self.poll_ask(cx);
    }

    /// Step 5: the prompt, with exactly what it holds as the sources.
    fn ask_answer(
        &mut self,
        question: &str,
        mail: &[String],
        events: Vec<Source>,
        serial: u64,
        cx: &mut Context<Self>,
    ) {
        let mut sources: Vec<Source> = mail
            .iter()
            .filter_map(|id| {
                let t = &self.data.threads[self.data.thread_ix(id)?];
                let last = t.messages.len() - 1;
                let excerpt = match text::Message::from_thread(t, last) {
                    Some(m) => format!("From: {}\n{}", m.from, m.body),
                    None => format!(
                        "From: {}\n{}",
                        t.last_message().from.display_name(),
                        t.last_message().snippet
                    ),
                };
                Some(Source {
                    n: 0,
                    id: SourceId::Mail(t.id.clone()),
                    title: t.subject.clone(),
                    date: t.last_message().date.format("%b %-d, %Y").to_string(),
                    excerpt,
                })
            })
            .collect();
        sources.extend(events);
        let (request, sent) = search::ask_request(question, sources, ai::INPUT_BUDGET);
        let key = format!("ask-answer-{serial}");
        if let Some(ask) = &mut self.ai.palette.ask {
            ask.sources = sent;
            ask.stage = AskStage::Answer(key.clone());
        }
        if self.queue_ask(Feature::Ask, &key, JobGroup::Ask, Work::Llm(request), cx) {
            self.ask_show(AskStatus::Writing, String::new(), cx);
        } else {
            self.ask_failed("Ask Pigeon isn't available now. Check Settings › AI.", cx);
        }
    }

    /// Step 6: a source opens its conversation, or its event in Calendar.
    fn ask_open_source(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self
            .ai
            .palette
            .ask
            .as_ref()
            .and_then(|a| a.sources.get(ix))
            .cloned()
        else {
            return;
        };
        self.close_overlays_at_once(window, cx);
        match source.id {
            SourceId::Mail(id) => match self.data.thread_ix(&id) {
                Some(ix) => {
                    self.set_mode(Mode::Mail, window, cx);
                    self.reveal_thread(ix, window, cx);
                }
                None => self.show_error_toast("That conversation is no longer on this Mac", cx),
            },
            SourceId::Event { id, occurrence } => {
                self.set_mode(Mode::Calendar, window, cx);
                self.calendar.update(cx, |c, cx| {
                    c.anchor = occurrence.date();
                    c.open_event(&id, occurrence, false, window, cx);
                });
            }
        }
    }

    // AI-15: Do It for Me.

    fn do_it_for_me(&mut self, _: &DoItForMe, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_command_available(AiNeed::DoIt, cx) {
            self.open_do_it("", window, cx);
        }
    }

    /// The prompt, with `text` in it.
    fn open_do_it(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.help_open = false;
        let palette = cx.new(|cx| {
            let mut p = CommandPalette::prompt(
                text,
                "Describe what to do".into(),
                "Preview".into(),
                "Nothing changes until you confirm".into(),
                window,
                cx,
            );
            p.set_empty_text("Say what to do, like “archive newsletters older than a month”");
            p
        });
        self.set_palette(palette.clone(), window, cx);
        self.ai.palette.do_it = Some(DoIt {
            palette: palette.entity_id(),
            account: self.data.account.email.clone(),
            stage: DoItStage::Prompt,
        });
    }

    fn do_it_palette(&self) -> Option<Entity<CommandPalette>> {
        let id = self.ai.palette.do_it.as_ref()?.palette;
        self.palette
            .as_ref()
            .map(|(p, _)| p.clone())
            .filter(|p| p.entity_id() == id)
    }

    fn do_it_hint(&mut self, hint: impl Into<gpui::SharedString>, cx: &mut Context<Self>) {
        if let Some(palette) = self.do_it_palette() {
            let hint = hint.into();
            palette.update(cx, |p, cx| p.set_hint(hint, cx));
        }
    }

    /// The text changed: a plan for the old text is moot.
    fn do_it_edited(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = &mut self.ai.palette.do_it
            && !matches!(d.stage, DoItStage::Prompt)
        {
            d.stage = DoItStage::Prompt;
            self.cancel_asks(JobGroup::Palette, cx);
        }
    }

    fn do_it_submit(&mut self, text: &str, cx: &mut Context<Self>) {
        self.cancel_asks(JobGroup::Palette, cx);
        let serial = self.ai.palette.next();
        let key = format!("do-it-{serial}");
        let today = chrono::Local::now().date_naive();
        let work = Work::Llm(do_it::request(text, &self.data.labels, today));
        if !self.queue_ask(Feature::DoIt, &key, JobGroup::Palette, work, cx) {
            return self.do_it_hint("Do It for Me isn't available now. Check Settings › AI", cx);
        }
        if let Some(d) = &mut self.ai.palette.do_it {
            d.stage = DoItStage::Working(key);
        }
        self.do_it_hint("Working…", cx);
    }

    fn poll_do_it(&mut self, cx: &mut Context<Self>) {
        let Some(DoIt {
            stage: DoItStage::Working(key),
            ..
        }) = &self.ai.palette.do_it
        else {
            return;
        };
        let Some(done) = self.backend.read(cx).ai_done(Feature::DoIt, key).cloned() else {
            return;
        };
        let now = chrono::Utc::now().timestamp_millis();
        let planned = match done {
            Done::Ready(Output::Text(answer)) => do_it::validate(&answer, &self.data.labels)
                .and_then(|plan| Ok((do_it::matches(&plan, &self.data, now)?, plan))),
            _ => Err("Couldn't plan that. Try again".to_string()),
        };
        let Some(d) = &mut self.ai.palette.do_it else {
            return;
        };
        match planned {
            Ok((ixs, _)) if ixs.is_empty() => {
                d.stage = DoItStage::Prompt;
                self.do_it_hint("No conversations on this Mac match. Nothing changed", cx);
            }
            Ok((ixs, plan)) => {
                let header = do_it::header(&plan, ixs.len(), &self.data.labels);
                let ids = ixs
                    .iter()
                    .map(|&ix| self.data.threads[ix].id.clone())
                    .collect();
                d.stage = DoItStage::Preview { plan, ids };
                let items = ixs.iter().map(|&ix| self.thread_item(ix)).collect();
                if let Some(palette) = self.do_it_palette() {
                    palette.update(cx, |p, cx| p.preview(header, items, cx));
                }
            }
            Err(why) => {
                d.stage = DoItStage::Prompt;
                self.do_it_hint(why, cx);
            }
        }
    }

    /// A conversation as a palette row: subject, sender and date.
    pub(super) fn thread_item(&self, ix: usize) -> PaletteItem {
        let thread = &self.data.threads[ix];
        let from = thread
            .messages
            .iter()
            .rev()
            .map(|m| &m.from)
            .find(|c| !self.data.is_me(c))
            .unwrap_or(&thread.last_message().from);
        // Same-subject conversations tell apart by date (ME-078).
        let date = crate::data::list_date(&thread.last_message().date);
        PaletteItem {
            command: PaletteCommand::Thread(ix),
            title: thread.subject.clone().into(),
            subtitle: Some(format!("{} · {date}", from.display_name()).into()),
            leading: Leading::Icon(Icon::Mail),
            binding: None,
            search_only: true,
        }
    }

    /// Enter on the preview: exactly the conversations shown that are
    /// still here, through the explicit set helpers, so Undo works.
    fn do_it_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(DoIt {
            account,
            stage: DoItStage::Preview { plan, ids },
            ..
        }) = self.ai.palette.do_it.take()
        else {
            return;
        };
        self.close_overlays_at_once(window, cx);
        if account != self.data.account.email || !self.may_ask(Feature::DoIt, cx) {
            return self
                .show_error_toast("Nothing changed: the account or AI settings changed", cx);
        }
        let (targets, gone) = do_it::still_present(&ids, &self.data);
        if targets.is_empty() {
            return self.show_error_toast("Nothing changed: those conversations are gone", cx);
        }
        let to =
            |folder: Folder| move |t: &crate::data::Thread| (t.folder != folder).then_some(folder);
        match plan.action {
            do_it::Action::Archive => self.move_threads(targets, to(Folder::Archive), window, cx),
            do_it::Action::MoveToInbox => self.move_threads(targets, to(Folder::Inbox), window, cx),
            do_it::Action::Trash => self.move_threads(targets, to(Folder::Trash), window, cx),
            do_it::Action::MarkRead => self.set_unread(targets, false, cx),
            do_it::Action::MarkUnread => self.set_unread(targets, true, cx),
            do_it::Action::Star => self.set_starred(targets, true, cx),
            do_it::Action::Unstar => self.set_starred(targets, false, cx),
            do_it::Action::AddLabel | do_it::Action::RemoveLabel => {
                let label = plan
                    .label_id
                    .as_ref()
                    .and_then(|id| self.data.labels.iter().position(|l| &l.id == id));
                let Some(label) = label else {
                    return self.show_error_toast("Nothing changed: that label is gone", cx);
                };
                let remove = plan.action == do_it::Action::RemoveLabel;
                self.set_label(label, targets, remove, cx);
            }
        }
        if gone > 0
            && let Some(toast) = &mut self.toast
            && toast.kind != ToastKind::Error
        {
            let skipped = if gone == 1 {
                "1 was gone".to_string()
            } else {
                format!("{gone} were gone")
            };
            toast.message = format!("{} · {skipped}", toast.message).into();
        }
    }

    // Answers arrive.

    /// A backend event: AI answers, or bodies Ask waits for.
    pub(super) fn on_palette_backend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.poll_plain(window, cx);
        self.poll_best(cx);
        self.poll_ask(cx);
        self.poll_do_it(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn choice(id: &str, confidence: f64) -> jev::Answer {
        jev::Answer::Choice {
            choice: Some(id.into()),
            confidence: Some(confidence),
            probabilities: BTreeMap::new(),
        }
    }

    #[test]
    fn a_suggestion_needs_a_sure_known_command() {
        assert_eq!(suggestion(Some(&choice("c3", 0.8)), 5), Some(2));
        assert_eq!(suggestion(Some(&choice("none", 0.99)), 5), None);
        assert_eq!(suggestion(Some(&choice("c3", 0.5)), 5), None);
        assert_eq!(suggestion(Some(&choice("c9", 0.9)), 5), None);
        assert_eq!(suggestion(Some(&choice("c0", 0.9)), 5), None);
        assert_eq!(suggestion(Some(&choice("archive", 0.9)), 5), None);
        assert_eq!(suggestion(None, 5), None);
    }

    #[test]
    fn stale_asks_are_dropped() {
        assert!(still_current("archive old news", 4, "archive old news", 4));
        // Typed on, or a newer request since.
        assert!(!still_current(
            "archive old newsletters",
            4,
            "archive old news",
            4
        ));
        assert!(!still_current("archive old news", 5, "archive old news", 4));
    }

    #[test]
    fn options_keep_the_best_fuzzy_matches_when_too_many() {
        let titles = ["Archive", "Mark as Read", "Go to Inbox", "Star"];
        assert_eq!(top_options("x", &titles, 10), [0, 1, 2, 3]);
        let top = top_options("archive this", &titles, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0], 0);
    }
}

//! AI in Pigeon (docs/AI_SPRINT.md): a writing model (OpenAI-compatible)
//! for text, and Jev (OpenRouter's System One API) for yes/no, pick-one
//! and score decisions. Nothing is sent until Settings › AI is on and its
//! note confirmed (`AiPrefs::consented`).
//!
//! `AiService` lives in `Backend`, which runs its jobs off the main thread
//! (`backend::Backend::ai_*`); this module is the plain part: the clients,
//! the queue, the cache's rows and hashes, and what's sent.

// Parts of the API are for the workstreams that follow (W1b–W5).
#![allow(dead_code)]

pub mod config;
pub mod events;
pub mod jev;
pub mod llm;
pub mod log;
pub mod notice;
pub mod pass;
pub mod queue;
pub mod reading;
pub mod search;
pub mod text;

use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use self::queue::{JobGroup, Priority, Provider};
use self::{config::AiConfig, queue::ErrorKind};
use crate::backend::store::Store;

/// Tokens of input per writing-model call (characters / 4).
pub const INPUT_BUDGET: usize = 12_000;
/// Between bytes of a writing-model answer; no cap on the whole.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Added to every max-output figure: the writing model reasons first.
pub const REASONING_ROOM: u32 = 512;
/// How often a streamed Interactive answer shows its progress.
pub const STREAM_TICK: Duration = Duration::from_millis(100);
/// How often background results reach the UI.
pub const BACKGROUND_TICK: Duration = Duration::from_secs(1);
/// The largest answer read.
const BODY_LIMIT: u64 = 8 * 1024 * 1024;

/// What a job or a stored row is for. Stored by `as_str`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Feature {
    // Jev.
    /// Request A: from sender, subject and snippet (AI-5).
    Important,
    /// Request B, once the body is stored: needs_reply, newsletter,
    /// event_n (AI-4, AI-10, AI-16).
    Signals,
    /// AI-6.
    Label,
    /// AI-12, not stored.
    Palette,
    /// AI-13/AI-14 rerank, not stored.
    Rerank,
    // The writing model.
    /// AI-7.
    Line,
    /// AI-8.
    Summary,
    /// AI-9.
    CatchUp,
    /// AI-11.
    Replies,
    /// AI-10.
    Clean,
    /// AI-14's search terms, not stored.
    AskTerms,
    /// AI-14's answer, not stored.
    Ask,
    /// AI-15, not stored.
    DoIt,
    // Rows that aren't a job's.
    /// AI-16: candidates with their answers.
    Event,
    NeedsReplyDismissed,
    CleanRejected,
    /// Test Connection.
    Test,
}

impl Feature {
    pub const ALL: [Feature; 17] = [
        Feature::Important,
        Feature::Signals,
        Feature::Label,
        Feature::Palette,
        Feature::Rerank,
        Feature::Line,
        Feature::Summary,
        Feature::CatchUp,
        Feature::Replies,
        Feature::Clean,
        Feature::AskTerms,
        Feature::Ask,
        Feature::DoIt,
        Feature::Event,
        Feature::NeedsReplyDismissed,
        Feature::CleanRejected,
        Feature::Test,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Feature::Important => "important",
            Feature::Signals => "signals",
            Feature::Label => "label",
            Feature::Palette => "palette",
            Feature::Rerank => "rerank",
            Feature::Line => "line",
            Feature::Summary => "summary",
            Feature::CatchUp => "catch_up",
            Feature::Replies => "replies",
            Feature::Clean => "clean",
            Feature::AskTerms => "ask_terms",
            Feature::Ask => "ask",
            Feature::DoIt => "do_it",
            Feature::Event => "event",
            Feature::NeedsReplyDismissed => "needs_reply_dismissed",
            Feature::CleanRejected => "clean_rejected",
            Feature::Test => "test",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == s)
    }

    /// Who answers it; `None` for rows that aren't model output, and for
    /// Test, whose job names its provider.
    pub fn provider(self) -> Option<Provider> {
        match self {
            Feature::Important
            | Feature::Signals
            | Feature::Label
            | Feature::Palette
            | Feature::Rerank => Some(Provider::Jev),
            Feature::Line
            | Feature::Summary
            | Feature::CatchUp
            | Feature::Replies
            | Feature::Clean
            | Feature::AskTerms
            | Feature::Ask
            | Feature::DoIt => Some(Provider::Writing),
            Feature::Event
            | Feature::NeedsReplyDismissed
            | Feature::CleanRejected
            | Feature::Test => None,
        }
    }

    /// Its results go to `ai_results`; the others are for the moment
    /// (`Backend::ai_done`).
    pub fn cached(self) -> bool {
        !matches!(
            self,
            Feature::Palette
                | Feature::Rerank
                | Feature::AskTerms
                | Feature::Ask
                | Feature::DoIt
                | Feature::Test
        )
    }
}

/// Settings › AI, saved (in `Saved::ai`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiPrefs {
    pub enabled: bool,
    /// The fingerprint the note was confirmed for (`fingerprint`).
    pub note_ack: Option<String>,
    pub list_summaries: bool,
    pub clean_view: bool,
    /// Also gates quick replies.
    pub needs_reply: bool,
    pub notify_for: NotifyFor,
}

impl Default for AiPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            note_ack: None,
            list_summaries: true,
            clean_view: true,
            needs_reply: true,
            notify_for: NotifyFor::All,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyFor {
    #[default]
    All,
    Important,
}

impl AiPrefs {
    /// On, with the note confirmed for where things go now.
    pub fn consented(&self, fingerprint: &str) -> bool {
        self.enabled && self.note_ack.as_deref() == Some(fingerprint)
    }

    /// The feature's own switch; most follow the main one alone.
    pub fn switched_on(&self, feature: Feature) -> bool {
        match feature {
            Feature::Line => self.list_summaries,
            Feature::Clean => self.clean_view,
            Feature::Replies => self.needs_reply,
            _ => true,
        }
    }
}

/// What the note was confirmed for: the source kind and both hosts. A
/// change shows the note again.
pub fn fingerprint(demo: bool, config: &AiConfig) -> String {
    let host = |e: &Option<config::Endpoint>| e.as_ref().map_or("-".into(), |e| e.host());
    format!(
        "{}|{}|{}",
        if demo { "demo" } else { "gmail" },
        host(&config.llm),
        host(&config.jev)
    )
}

/// Whether `feature` may send now.
pub fn allows(prefs: &AiPrefs, fingerprint: &str, config: &AiConfig, feature: Feature) -> bool {
    let configured = match feature.provider() {
        Some(Provider::Writing) => config.llm.is_some(),
        Some(Provider::Jev) => config.jev.is_some(),
        None => config.llm.is_some() || config.jev.is_some(),
    };
    prefs.consented(fingerprint) && prefs.switched_on(feature) && configured
}

/// Something allowed under `old` isn't under `new`: time to bump.
pub fn turned_off(old: &AiPrefs, new: &AiPrefs, fingerprint: &str, config: &AiConfig) -> bool {
    Feature::ALL
        .into_iter()
        .any(|f| allows(old, fingerprint, config, f) && !allows(new, fingerprint, config, f))
}

/// sha256 over the prompt or question version, the message ids covered,
/// in order, and the feature's extras. The model isn't part of it.
pub fn input_hash(version: &str, message_ids: &[&str], extras: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in [version]
        .iter()
        .chain([""].iter())
        .chain(message_ids)
        .chain([""].iter())
        .chain(extras)
    {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// One `ai_results` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiRow {
    pub feature: Feature,
    pub key: String,
    pub input_hash: String,
    pub model: String,
    pub value: String,
    /// Unix ms.
    pub created: i64,
}

/// A conversation's AI results (`Thread::ai`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadAi {
    pub rows: Vec<AiRow>,
}

impl ThreadAi {
    pub fn get(&self, feature: Feature, key: &str) -> Option<&AiRow> {
        self.rows
            .iter()
            .find(|r| r.feature == feature && r.key == key)
    }

    /// The row only if it was made from the same input.
    pub fn fresh(&self, feature: Feature, key: &str, hash: &str) -> Option<&AiRow> {
        self.get(feature, key).filter(|r| r.input_hash == hash)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// AI-16: a date found in a message, stored with its answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventCandidate {
    /// The subject without Re:/Fwd:.
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub all_day: bool,
    pub zone: Option<String>,
    pub location: Option<String>,
    pub sentence: String,
}

/// AI-14: what an answer may cite, exactly as sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub n: usize,
    pub id: SourceId,
    pub title: String,
    pub date: String,
    pub excerpt: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SourceId {
    Mail(String),
    Event {
        id: String,
        occurrence: NaiveDateTime,
    },
}

/// What a job sends.
#[derive(Clone, Debug)]
pub enum Work {
    Llm(llm::Request),
    Jev(jev::Request),
    /// Test Connection: one small request.
    Test(Provider),
}

/// One request to make. `account` is filled in by `Backend::ai_enqueue`.
#[derive(Clone, Debug)]
pub struct Job {
    pub account: String,
    pub feature: Feature,
    /// A message id (message features) or a conversation id.
    pub key: String,
    /// The conversation it's about; empty for ⌘K, search and Ask.
    pub thread_id: String,
    pub input_hash: String,
    pub priority: Priority,
    pub group: Option<JobGroup>,
    pub work: Work,
}

impl Job {
    pub fn new(feature: Feature, key: impl Into<String>, work: Work) -> Self {
        Self {
            account: String::new(),
            feature,
            key: key.into(),
            thread_id: String::new(),
            input_hash: String::new(),
            priority: Priority::Background,
            group: None,
            work,
        }
    }

    pub fn ident(&self) -> queue::Ident {
        queue::Ident {
            account: self.account.clone(),
            feature: self.feature,
            key: self.key.clone(),
        }
    }

    pub fn provider(&self) -> Provider {
        match &self.work {
            Work::Llm(_) => Provider::Writing,
            Work::Jev(_) => Provider::Jev,
            Work::Test(p) => *p,
        }
    }
}

/// A job's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    Text(String),
    Answers(BTreeMap<String, jev::Answer>),
}

impl Output {
    /// As stored in `ai_results.value`.
    pub fn value(&self) -> String {
        match self {
            Output::Text(text) => text.clone(),
            Output::Answers(answers) => serde_json::to_string(answers).unwrap_or_default(),
        }
    }
}

/// The last result of an uncached job, or any job's failure, for the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    Ready(Output),
    Failed(String),
}

/// How a call failed. Shown in the activity log: never carries a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    Status(u16),
    Timeout,
    Network(String),
    Cancelled,
    /// `finish_reason: "length"`.
    CutOff,
    /// Not set up, refused before sending, or an answer that doesn't read.
    Invalid(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Status(code) => write!(f, "HTTP {code}"),
            CallError::Timeout => write!(f, "Timed out"),
            CallError::Network(_) => write!(f, "Network error"),
            CallError::Cancelled => write!(f, "Canceled"),
            CallError::CutOff => write!(f, "Answer cut off"),
            CallError::Invalid(why) => write!(f, "Invalid: {why}"),
        }
    }
}

impl CallError {
    pub fn from_io(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => CallError::Timeout,
            kind => CallError::Network(kind.to_string()),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        match self {
            CallError::Status(401 | 403 | 404) => ErrorKind::Stop,
            CallError::Status(429) | CallError::Status(500..) => ErrorKind::Backoff,
            CallError::Status(_) | CallError::CutOff | CallError::Invalid(_) => ErrorKind::Job,
            CallError::Timeout | CallError::Network(_) => ErrorKind::Backoff,
            CallError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

/// A POST of JSON with a bearer key. No `Debug`: it holds the key.
pub struct HttpRequest<'a> {
    pub url: String,
    pub key: &'a str,
    pub body: Vec<u8>,
    /// A cap on the whole call (Jev); `None` streams with `IDLE_TIMEOUT`
    /// between reads.
    pub total: Option<Duration>,
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Box<dyn Read>,
}

/// The network, swapped for `FakeTransport` in tests. Called off the main
/// thread; blocks.
pub trait Transport: Send + Sync {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, CallError>;
}

pub struct UreqTransport {
    streaming: ureq::Agent,
    capped: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqTransport {
    pub fn new() -> Self {
        let agent = |total: Option<Duration>, per_read: Option<Duration>| {
            ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .timeout_connect(Some(CONNECT_TIMEOUT))
                    .timeout_global(total)
                    .timeout_per_read(per_read)
                    .user_agent("Pigeon")
                    .build(),
            )
        };
        Self {
            streaming: agent(None, Some(IDLE_TIMEOUT)),
            capped: agent(Some(jev::TIMEOUT), None),
        }
    }
}

impl Transport for UreqTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, CallError> {
        let agent = if request.total.is_some() {
            &self.capped
        } else {
            &self.streaming
        };
        let response = agent
            .post(&request.url)
            .header("Authorization", &format!("Bearer {}", request.key))
            .header("Content-Type", "application/json")
            .send(&request.body[..])
            .map_err(|e| match e {
                ureq::Error::Timeout(_) => CallError::Timeout,
                ureq::Error::Io(e) => CallError::from_io(e),
                // Never the request's headers.
                e => CallError::Network(e.to_string()),
            })?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .into_with_config()
            .limit(BODY_LIMIT)
            .reader();
        Ok(HttpResponse {
            status,
            body: Box::new(body),
        })
    }
}

/// Canned answers in order, and what was sent (URL, body, cap; never the
/// key).
#[cfg(test)]
type Answer = Result<(u16, Vec<u8>), CallError>;

#[cfg(test)]
pub struct FakeTransport {
    answers: Mutex<std::collections::VecDeque<Answer>>,
    sent: Mutex<Vec<(String, String, Option<Duration>)>>,
}

#[cfg(test)]
impl FakeTransport {
    pub fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub fn sent(&self) -> Vec<(String, String)> {
        let sent = self.sent.lock().unwrap();
        sent.iter()
            .map(|(u, b, _)| (u.clone(), b.clone()))
            .collect()
    }

    pub fn totals(&self) -> Vec<Option<Duration>> {
        let sent = self.sent.lock().unwrap();
        sent.iter().map(|(_, _, t)| *t).collect()
    }
}

#[cfg(test)]
impl Transport for FakeTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, CallError> {
        self.sent.lock().unwrap().push((
            request.url.clone(),
            String::from_utf8_lossy(&request.body).into_owned(),
            request.total,
        ));
        let (status, body) = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(CallError::Network("no answer".into())))?;
        Ok(HttpResponse {
            status,
            body: Box::new(std::io::Cursor::new(body)),
        })
    }
}

/// What a job needs to run, off the main thread.
pub struct Run<'a> {
    pub transport: &'a dyn Transport,
    pub config: &'a AiConfig,
    /// Off after a streamed request was refused, until restart.
    pub streaming: &'a AtomicBool,
    pub generation: &'a AtomicU64,
    pub cancel: &'a AtomicBool,
    /// An Interactive answer's text so far.
    pub partial: Option<&'a Mutex<String>>,
    /// Where cached results go.
    pub store: Option<&'a Store>,
}

/// What became of a job.
#[derive(Clone, Debug)]
pub struct Ran {
    pub result: Result<Output, CallError>,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub ms: u64,
    /// Written to `ai_results` (its conversation still stored).
    pub stored: bool,
}

/// Run `job`, queued at `generation`: checked before sending, per chunk,
/// and before its result is stored.
pub fn execute(job: &Job, generation: u64, run: &Run) -> Ran {
    let started = Instant::now();
    let current = || run.generation.load(Ordering::SeqCst) == generation;
    let stop = || run.cancel.load(Ordering::SeqCst) || !current();
    let endpoint = match job.provider() {
        Provider::Writing => run.config.llm.as_ref(),
        Provider::Jev => run.config.jev.as_ref(),
    };
    // The writing model's name carries the feature's effort.
    let model = endpoint.map(|e| match job.provider() {
        Provider::Writing => llm::model_for(&e.model, job.feature),
        Provider::Jev => e.model.clone(),
    });
    let mut ran = Ran {
        result: Err(CallError::Invalid("no key in .env".into())),
        model: model.clone().unwrap_or_default(),
        tokens_in: 0,
        tokens_out: 0,
        ms: 0,
        stored: false,
    };
    let Some(endpoint) = endpoint else {
        return ran;
    };
    if stop() {
        ran.result = Err(CallError::Cancelled);
        return ran;
    }
    let ran_model = ran.model.clone();
    let mut on_text = |text: &str| {
        if let Some(partial) = run.partial {
            partial
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_str(text);
        }
    };
    let llm = |request: &llm::Request, on_text: &mut dyn FnMut(&str)| {
        llm::run(
            run.transport,
            endpoint,
            &ran_model,
            request,
            run.streaming,
            &stop,
            on_text,
        )
        .map(|r| (Output::Text(r.text), r.tokens_in, r.tokens_out))
    };
    let jev = |request: &jev::Request| {
        jev::run(run.transport, endpoint, request)
            .map(|r| (Output::Answers(r.answers), r.tokens_in, r.tokens_out))
    };
    let result = match &job.work {
        Work::Llm(request) => llm(request, &mut on_text),
        Work::Jev(request) => jev(request),
        Work::Test(Provider::Writing) => llm(
            &llm::Request::new("Answer with the single word OK.", "Test", 5),
            &mut on_text,
        ),
        Work::Test(Provider::Jev) => jev(&jev::Request::new("A connection test from Pigeon.")
            .ask("ok", jev::Question::noul("This text is a connection test."))),
    };
    ran.ms = started.elapsed().as_millis() as u64;
    ran.result = match result {
        Ok((output, tokens_in, tokens_out)) => {
            (ran.tokens_in, ran.tokens_out) = (tokens_in, tokens_out);
            // A late answer, from before a bump or a cancel, isn't kept.
            if stop() {
                Err(CallError::Cancelled)
            } else {
                Ok(output)
            }
        }
        Err(e) => Err(e),
    };
    if let (Ok(output), Some(store), true) = (&ran.result, run.store, job.feature.cached()) {
        let row = AiRow {
            feature: job.feature,
            key: job.key.clone(),
            input_hash: job.input_hash.clone(),
            model: ran.model.clone(),
            value: output.value(),
            created: Utc::now().timestamp_millis(),
        };
        match store.put_ai_result(&job.account, &job.thread_id, &row) {
            Ok(stored) => ran.stored = stored,
            Err(e) => eprintln!("Pigeon: couldn't store an AI result: {e:#}"),
        }
    }
    ran
}

/// A provider as Settings shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderInfo {
    pub host: Option<String>,
    pub model: Option<String>,
    /// `None` when its key, URL or model isn't in `.env`.
    pub health: Option<queue::Health>,
    /// The last Test Connection's result, since launch.
    pub last_test: Option<Result<(), String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Providers {
    pub writing: ProviderInfo,
    pub jev: ProviderInfo,
}

/// The AI's state in `Backend`. It survives `Backend::switch`, so its
/// generation, log and provider state outlast sign-out, demo and sign-in.
pub struct AiService {
    pub config: AiConfig,
    pub transport: Arc<dyn Transport>,
    pub queue: queue::Queue,
    pub log: log::Log,
    pub streaming: Arc<AtomicBool>,
    pub prefs: AiPrefs,
    /// Streamed text so far, by feature and key.
    pub partial: HashMap<(Feature, String), Arc<Mutex<String>>>,
    pub done: HashMap<(Feature, String), Done>,
    /// Conversations with background results not yet announced.
    pub changed: Vec<String>,
    pub flush_pending: bool,
    pub wake_pending: bool,
    pub tests: HashMap<Provider, Result<(), String>>,
}

impl AiService {
    pub fn new(config: AiConfig, transport: Arc<dyn Transport>) -> Self {
        Self {
            config,
            transport,
            queue: queue::Queue::new(),
            log: log::Log::default(),
            streaming: Arc::new(AtomicBool::new(true)),
            prefs: AiPrefs::default(),
            partial: HashMap::new(),
            done: HashMap::new(),
            changed: Vec::new(),
            flush_pending: false,
            wake_pending: false,
            tests: HashMap::new(),
        }
    }

    /// Drop everything queued, close what runs, forget what's in memory.
    pub fn bump(&mut self) {
        self.queue.bump();
        self.partial.clear();
        self.done.clear();
        self.changed.clear();
    }

    pub fn providers(&self, now: Instant) -> Providers {
        let info = |provider: Provider, endpoint: &Option<config::Endpoint>| ProviderInfo {
            host: endpoint.as_ref().map(|e| e.host()),
            model: endpoint.as_ref().map(|e| e.model.clone()),
            health: endpoint.as_ref().map(|_| self.queue.health(provider, now)),
            last_test: self.tests.get(&provider).cloned(),
        };
        Providers {
            writing: info(Provider::Writing, &self.config.llm),
            jev: info(Provider::Jev, &self.config.jev),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Body, Contact, Folder, Message, Thread};

    fn config() -> AiConfig {
        let endpoint = |base: &str, model: &str| config::Endpoint {
            base_url: base.into(),
            key: "fake".into(),
            model: model.into(),
        };
        AiConfig {
            llm: Some(endpoint("https://llm.example.ts.net/v1", "test-model")),
            jev: Some(endpoint("https://openrouter.ai/api", "typesafe/jev-1.14")),
        }
    }

    #[test]
    fn consent_follows_the_fingerprint() {
        let fp = fingerprint(false, &config());
        assert_eq!(fp, "gmail|llm.example.ts.net|openrouter.ai");
        let mut prefs = AiPrefs {
            enabled: true,
            note_ack: Some(fp.clone()),
            ..Default::default()
        };
        assert!(allows(&prefs, &fp, &config(), Feature::Line));
        // Demo has its own; another host shows the note again.
        assert!(!allows(
            &prefs,
            &fingerprint(true, &config()),
            &config(),
            Feature::Line
        ));
        let no_llm = AiConfig {
            llm: None,
            ..config()
        };
        assert!(!allows(&prefs, &fp, &no_llm, Feature::Line));
        prefs.list_summaries = false;
        assert!(!allows(&prefs, &fp, &config(), Feature::Line));
        assert!(allows(&prefs, &fp, &config(), Feature::Signals));
        let on = AiPrefs {
            list_summaries: true,
            ..prefs.clone()
        };
        assert!(turned_off(&on, &prefs, &fp, &config()));
        assert!(!turned_off(&prefs, &on, &fp, &config()));
        let off = AiPrefs {
            enabled: false,
            ..on.clone()
        };
        assert!(turned_off(&on, &off, &fp, &config()));
    }

    #[test]
    fn prefs_default_from_partial_json() {
        let prefs: AiPrefs = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
        assert!(prefs.enabled && prefs.list_summaries && prefs.note_ack.is_none());
        assert_eq!(prefs.notify_for, NotifyFor::All);
        let json = serde_json::to_string(&AiPrefs {
            notify_for: NotifyFor::Important,
            ..prefs
        })
        .unwrap();
        assert!(json.contains("\"important\""));
    }

    #[test]
    fn features_round_trip_by_name() {
        for f in Feature::ALL {
            assert_eq!(Feature::parse(f.as_str()), Some(f));
        }
    }

    fn head_and_full() -> (Thread, Thread) {
        let make = |body: Body| {
            let messages = ["m1", "m2"]
                .iter()
                .map(|id| {
                    let mut m = Message::new(Contact::default());
                    m.id = id.to_string();
                    m.body = body.clone();
                    m
                })
                .collect();
            Thread::local("t".into(), "S".into(), Folder::Inbox, messages, None)
        };
        (
            make(Body::NotLoaded),
            make(Body::Loaded {
                html: "<p>x</p>".into(),
            }),
        )
    }

    fn hash_of(thread: &Thread) -> String {
        let ids: Vec<&str> = thread.messages.iter().map(|m| m.id.as_str()).collect();
        input_hash("line-v1", &ids, &[])
    }

    #[test]
    fn hashes_need_only_heads_and_change_with_new_mail() {
        let (head, full) = head_and_full();
        assert_eq!(hash_of(&head), hash_of(&full));
        assert_eq!(hash_of(&head).len(), 64);
        let mut newer = head;
        let mut m = Message::new(Contact::default());
        m.id = "m3".into();
        newer.messages.push(m);
        assert_ne!(hash_of(&newer), hash_of(&full));
        assert_ne!(input_hash("line-v2", &["m1", "m2"], &[]), hash_of(&full));
        // Ids and extras can't run together.
        assert_ne!(
            input_hash("v", &["a", "b"], &[]),
            input_hash("v", &["a"], &["b"])
        );
        let mut ai = ThreadAi::default();
        ai.rows.push(AiRow {
            feature: Feature::Line,
            key: "t".into(),
            input_hash: hash_of(&full),
            model: "m".into(),
            value: "Line".into(),
            created: 0,
        });
        assert!(ai.fresh(Feature::Line, "t", &hash_of(&full)).is_some());
        assert!(ai.fresh(Feature::Line, "t", &hash_of(&newer)).is_none());
    }

    #[test]
    fn errors_map_to_what_the_queue_does() {
        assert_eq!(CallError::Status(401).kind(), ErrorKind::Stop);
        assert_eq!(CallError::Status(404).kind(), ErrorKind::Stop);
        assert_eq!(CallError::Status(400).kind(), ErrorKind::Job);
        assert_eq!(CallError::Status(429).kind(), ErrorKind::Backoff);
        assert_eq!(CallError::Status(503).kind(), ErrorKind::Backoff);
        assert_eq!(CallError::Timeout.kind(), ErrorKind::Backoff);
        assert_eq!(CallError::CutOff.kind(), ErrorKind::Job);
        assert_eq!(CallError::Network("x".into()).to_string(), "Network error");
    }

    fn signals_job(account: &str, thread: &str) -> Job {
        Job {
            account: account.into(),
            thread_id: thread.into(),
            input_hash: "h".into(),
            ..Job::new(
                Feature::Signals,
                "m1",
                Work::Jev(jev::Request::new("s").ask("needs_reply", jev::Question::noul("q"))),
            )
        }
    }

    #[test]
    fn execute_stores_a_result_and_drops_a_late_one() {
        let store = Store::open_in_memory().unwrap();
        let mut m = Message::new(Contact::default());
        m.id = "m1".into();
        let thread = Thread::local("t1".into(), "S".into(), Folder::Inbox, vec![m], None);
        store.put_threads("me", &[thread]).unwrap();
        let noul = include_bytes!("testdata/jev_noul.json").to_vec();
        let fake = FakeTransport::new(vec![Ok((200, noul.clone())), Ok((200, noul))]);
        let config = config();
        let streaming = AtomicBool::new(true);
        let generation = AtomicU64::new(3);
        let cancel = AtomicBool::new(false);
        let run = Run {
            transport: &fake,
            config: &config,
            streaming: &streaming,
            generation: &generation,
            cancel: &cancel,
            partial: None,
            store: Some(&store),
        };
        let ran = execute(&signals_job("me", "t1"), 3, &run);
        assert!(ran.stored, "{:?}", ran.result);
        assert_eq!(ran.model, "typesafe/jev-1.14");
        let rows = store.ai_results("me", None).unwrap();
        let answers = jev::answers_from(&rows["t1"].get(Feature::Signals, "m1").unwrap().value);
        assert_eq!(answers["needs_reply"].yes(), Some(0.98));
        // Queued a generation ago: never sent.
        let ran = execute(&signals_job("me", "t1"), 2, &run);
        assert_eq!(ran.result.unwrap_err(), CallError::Cancelled);
        assert_eq!(fake.sent().len(), 1);
        // The account went: nothing stored.
        store.delete_account("me").unwrap();
        let ran = execute(&signals_job("me", "t1"), 3, &run);
        assert!(ran.result.is_ok() && !ran.stored);
        assert!(store.ai_results("me", None).unwrap().is_empty());
    }
}

#[cfg(test)]
mod check;

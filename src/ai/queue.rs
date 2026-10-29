//! Which AI job goes next: priorities, slots per provider, groups that
//! cancel together, the generation that drops everything at once, and
//! what failed. Pure: the clock comes in as `now`, and nothing here sends.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use super::{Feature, Job};

/// At most this many jobs wait, in all.
pub const CAP: usize = 200;
/// Failed attempts before a key is left alone until restart, Retry or
/// Test Connection.
pub const MAX_ATTEMPTS: u32 = 3;
const RETRY_AFTER: Duration = Duration::from_secs(30);
const BACKOFF_FIRST: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    Writing,
    Jev,
}

impl Provider {
    /// (in all, of which background may use).
    fn slots(self) -> (usize, usize) {
        match self {
            Provider::Writing => (2, 1),
            Provider::Jev => (6, 2),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// The pass and the backfill.
    Background,
    /// You asked, opened a conversation, or a notification waits.
    Interactive,
}

/// Jobs a new request makes moot: another conversation opened, new ⌘K
/// text, a new search, a new Ask question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JobGroup {
    Open,
    Palette,
    Search,
    Ask,
}

/// Same account, feature and key: the same job.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ident {
    pub account: String,
    pub feature: Feature,
    pub key: String,
}

/// How a job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// 401, 403, 404: the provider stops until Test Connection or restart.
    Stop,
    /// 400, a cut-off or invalid answer: this job failed.
    Job,
    /// Timeouts, network errors, 429, 5xx: the provider backs off.
    Backoff,
    /// Canceled, or a generation ago: nothing to learn from it.
    Cancelled,
}

/// A provider's health, for Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Ready,
    /// "Check the key in .env".
    Stopped,
    /// Backing off for this long still.
    Waiting(Duration),
}

/// A job taken to run.
pub struct Started {
    pub id: u64,
    pub job: Job,
    /// The generation it was queued at, to check before sending and before
    /// publishing.
    pub generation: u64,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    Queued,
    /// Already waiting: raised to this priority instead.
    Promoted,
    /// Already running.
    Running,
    /// Failed before and not due again, or failed too often.
    Failed,
    Full,
}

struct Waiting {
    job: Job,
    generation: u64,
}

struct Running {
    id: u64,
    ident: Ident,
    provider: Provider,
    priority: Priority,
    group: Option<JobGroup>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct ProviderState {
    stopped: bool,
    backoff: Duration,
    until: Option<Instant>,
}

struct Failure {
    attempts: u32,
    retry_at: Instant,
}

pub struct Queue {
    waiting: Vec<Waiting>,
    running: Vec<Running>,
    failures: HashMap<Ident, Failure>,
    providers: HashMap<Provider, ProviderState>,
    generation: Arc<AtomicU64>,
    next_id: u64,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

impl Queue {
    pub fn new() -> Self {
        Self {
            waiting: Vec::new(),
            running: Vec::new(),
            failures: HashMap::new(),
            providers: HashMap::new(),
            generation: Arc::new(AtomicU64::new(0)),
            next_id: 0,
        }
    }

    pub fn generation(&self) -> Arc<AtomicU64> {
        self.generation.clone()
    }

    pub fn current(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Queue `job`; the same job waiting is promoted instead, and takes
    /// the newer input.
    pub fn push(&mut self, job: Job, now: Instant) -> Pushed {
        let ident = job.ident();
        if self.running.iter().any(|r| r.ident == ident) {
            return Pushed::Running;
        }
        if self.blocked(&ident, job.priority, now) {
            return Pushed::Failed;
        }
        let generation = self.current();
        if let Some(w) = self.waiting.iter_mut().find(|w| w.job.ident() == ident) {
            let priority = w.job.priority.max(job.priority);
            w.job = Job { priority, ..job };
            w.generation = generation;
            return Pushed::Promoted;
        }
        if self.waiting.len() >= CAP {
            // An Interactive job makes room by dropping the newest
            // background one; the pass queues it again later.
            let last_background = self
                .waiting
                .iter()
                .rposition(|w| w.job.priority == Priority::Background);
            match (job.priority, last_background) {
                (Priority::Interactive, Some(ix)) => {
                    self.waiting.remove(ix);
                }
                _ => return Pushed::Full,
            }
        }
        self.waiting.push(Waiting { job, generation });
        Pushed::Queued
    }

    /// The next job that may start now: Interactive first, then in the
    /// order queued, each within its provider's slots.
    pub fn next(&mut self, now: Instant) -> Option<Started> {
        let ix = [Priority::Interactive, Priority::Background]
            .into_iter()
            .find_map(|priority| {
                self.waiting.iter().position(|w| {
                    w.job.priority == priority && self.has_slot(w.job.provider(), priority, now)
                })
            })?;
        let Waiting { job, generation } = self.waiting.remove(ix);
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.running.push(Running {
            id: self.next_id,
            ident: job.ident(),
            provider: job.provider(),
            priority: job.priority,
            group: job.group,
            cancel: cancel.clone(),
        });
        Some(Started {
            id: self.next_id,
            job,
            generation,
            cancel,
        })
    }

    fn has_slot(&self, provider: Provider, priority: Priority, now: Instant) -> bool {
        if self.health(provider, now) != Health::Ready {
            return false;
        }
        let (all, background) = provider.slots();
        let running = self.running.iter().filter(|r| r.provider == provider);
        let busy = running.clone().count();
        let busy_background = running
            .filter(|r| r.priority == Priority::Background)
            .count();
        busy < all && (priority == Priority::Interactive || busy_background < background)
    }

    /// A job ended; `Ok` or how it failed.
    pub fn finish(&mut self, id: u64, result: Result<(), ErrorKind>, now: Instant) {
        let Some(ix) = self.running.iter().position(|r| r.id == id) else {
            return;
        };
        let Running {
            ident, provider, ..
        } = self.running.remove(ix);
        let state = self.providers.entry(provider).or_default();
        match result {
            Ok(()) => {
                state.backoff = Duration::ZERO;
                state.until = None;
                self.failures.remove(&ident);
            }
            Err(ErrorKind::Cancelled) => {}
            Err(ErrorKind::Stop) => state.stopped = true,
            Err(kind) => {
                if kind == ErrorKind::Backoff {
                    state.backoff = if state.backoff.is_zero() {
                        BACKOFF_FIRST
                    } else {
                        (state.backoff * 2).min(BACKOFF_MAX)
                    };
                    state.until = Some(now + state.backoff);
                }
                let failure = self.failures.entry(ident).or_insert(Failure {
                    attempts: 0,
                    retry_at: now,
                });
                failure.attempts += 1;
                failure.retry_at = now + RETRY_AFTER * 2u32.pow(failure.attempts - 1);
            }
        }
    }

    /// Drop the group's waiting jobs and close its running ones.
    pub fn cancel(&mut self, group: JobGroup) {
        self.waiting.retain(|w| w.job.group != Some(group));
        for r in self.running.iter().filter(|r| r.group == Some(group)) {
            r.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// AI turned off, a feature switched off, or the account changed:
    /// nothing queued goes, nothing running is published.
    pub fn bump(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.waiting.clear();
        for r in &self.running {
            r.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Waiting or running.
    pub fn holds(&self, ident: &Ident) -> bool {
        self.running.iter().any(|r| &r.ident == ident)
            || self.waiting.iter().any(|w| &w.job.ident() == ident)
    }

    /// Failed too often, or (for background work) not due again yet.
    pub fn blocked(&self, ident: &Ident, priority: Priority, now: Instant) -> bool {
        self.failures.get(ident).is_some_and(|f| {
            f.attempts >= MAX_ATTEMPTS || (priority == Priority::Background && now < f.retry_at)
        })
    }

    pub fn retry(&mut self, ident: &Ident) {
        self.failures.remove(ident);
    }

    pub fn clear_failures(&mut self) {
        self.failures.clear();
    }

    /// Test Connection: the provider may go again at once.
    pub fn reset(&mut self, provider: Provider) {
        self.providers.remove(&provider);
    }

    pub fn health(&self, provider: Provider, now: Instant) -> Health {
        match self.providers.get(&provider) {
            Some(s) if s.stopped => Health::Stopped,
            Some(ProviderState {
                until: Some(until), ..
            }) if *until > now => Health::Waiting(*until - now),
            _ => Health::Ready,
        }
    }

    /// How long until a backoff ends that holds up waiting jobs.
    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        self.waiting
            .iter()
            .filter_map(|w| match self.health(w.job.provider(), now) {
                Health::Waiting(d) => Some(d),
                _ => None,
            })
            .min()
    }

    pub fn waiting_len(&self) -> usize {
        self.waiting.len()
    }

    pub fn running_len(&self) -> usize {
        self.running.len()
    }

    /// Whether a job taken at `generation` is still current.
    pub fn is_current(&self, generation: u64) -> bool {
        self.current() == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{Work, jev, llm};

    fn job(feature: Feature, key: &str, priority: Priority) -> Job {
        let work = match feature.provider().unwrap() {
            Provider::Writing => Work::Llm(llm::Request::new("s", "u", 60)),
            Provider::Jev => Work::Jev(jev::Request::new("s").ask("q", jev::Question::noul("q"))),
        };
        Job {
            account: "me@example.com".into(),
            feature,
            key: key.into(),
            thread_id: key.into(),
            input_hash: "h".into(),
            priority,
            group: None,
            work,
        }
    }

    fn bg(key: &str) -> Job {
        job(Feature::Line, key, Priority::Background)
    }

    fn ident(feature: Feature, key: &str) -> Ident {
        job(feature, key, Priority::Background).ident()
    }

    #[test]
    fn interactive_starts_while_a_background_writing_job_runs() {
        let now = Instant::now();
        let mut q = Queue::new();
        q.push(bg("a"), now);
        q.push(bg("b"), now);
        let first = q.next(now).unwrap();
        assert_eq!(first.job.key, "a");
        // Background may use one writing slot only.
        assert!(q.next(now).is_none());
        q.push(job(Feature::Summary, "c", Priority::Interactive), now);
        assert_eq!(q.next(now).unwrap().job.key, "c");
        assert!(q.next(now).is_none());
        q.finish(first.id, Ok(()), now);
        assert_eq!(q.next(now).unwrap().job.key, "b");
    }

    #[test]
    fn jev_has_six_slots_two_for_background() {
        let now = Instant::now();
        let mut q = Queue::new();
        for i in 0..4 {
            q.push(
                job(Feature::Signals, &format!("b{i}"), Priority::Background),
                now,
            );
        }
        for i in 0..6 {
            q.push(
                job(Feature::Label, &format!("i{i}"), Priority::Interactive),
                now,
            );
        }
        let started: Vec<String> = std::iter::from_fn(|| q.next(now).map(|s| s.job.key)).collect();
        assert_eq!(started, ["i0", "i1", "i2", "i3", "i4", "i5"]);
        let mut q = Queue::new();
        for i in 0..4 {
            q.push(
                job(Feature::Signals, &format!("b{i}"), Priority::Background),
                now,
            );
        }
        assert_eq!(std::iter::from_fn(|| q.next(now)).count(), 2);
    }

    #[test]
    fn the_same_job_is_promoted_not_duplicated() {
        let now = Instant::now();
        let mut q = Queue::new();
        q.push(bg("x"), now);
        q.push(bg("y"), now);
        let mut again = bg("y");
        again.priority = Priority::Interactive;
        again.input_hash = "newer".into();
        assert_eq!(q.push(again, now), Pushed::Promoted);
        assert_eq!(q.waiting_len(), 2);
        let next = q.next(now).unwrap();
        assert_eq!(
            (next.job.key.as_str(), next.job.input_hash.as_str()),
            ("y", "newer")
        );
        assert_eq!(q.push(bg("y"), now), Pushed::Running);
    }

    #[test]
    fn groups_cancel_together() {
        let now = Instant::now();
        let mut q = Queue::new();
        let open = |key: &str| Job {
            group: Some(JobGroup::Open),
            ..job(Feature::Summary, key, Priority::Interactive)
        };
        q.push(open("a"), now);
        q.push(open("b"), now);
        q.push(open("c"), now);
        q.push(bg("keep"), now);
        let running = q.next(now).unwrap();
        q.cancel(JobGroup::Open);
        assert!(running.cancel.load(Ordering::SeqCst));
        q.finish(running.id, Err(ErrorKind::Cancelled), now);
        assert_eq!(q.next(now).unwrap().job.key, "keep");
        assert!(q.next(now).is_none());
        // A cancel isn't a failure.
        assert!(!q.blocked(&ident(Feature::Summary, "a"), Priority::Background, now));
    }

    #[test]
    fn a_bump_drops_queued_jobs_and_outdates_running_ones() {
        let now = Instant::now();
        let mut q = Queue::new();
        q.push(bg("a"), now);
        q.push(bg("b"), now);
        let running = q.next(now).unwrap();
        let generation = q.generation();
        q.bump();
        assert!(running.cancel.load(Ordering::SeqCst));
        // Its late result is discarded: the generation moved on.
        assert_ne!(generation.load(Ordering::SeqCst), running.generation);
        assert!(!q.is_current(running.generation));
        assert_eq!(q.waiting_len(), 0);
        q.finish(running.id, Ok(()), now);
        assert_eq!(q.running_len(), 0);
    }

    #[test]
    fn three_failed_attempts_skip_the_key() {
        let t0 = Instant::now();
        let mut q = Queue::new();
        let mut now = t0;
        for attempt in 1..=3 {
            assert_eq!(q.push(bg("x"), now), Pushed::Queued, "attempt {attempt}");
            let s = q.next(now).unwrap();
            q.finish(s.id, Err(ErrorKind::Job), now);
            // Not due again for 30 s, doubling.
            assert_eq!(q.push(bg("x"), now), Pushed::Failed);
            now += RETRY_AFTER * 2u32.pow(attempt - 1);
        }
        now += Duration::from_secs(3600);
        assert_eq!(q.push(bg("x"), now), Pushed::Failed);
        let mut asked = bg("x");
        asked.priority = Priority::Interactive;
        assert_eq!(q.push(asked, now), Pushed::Failed);
        q.retry(&ident(Feature::Line, "x"));
        assert_eq!(q.push(bg("x"), now), Pushed::Queued);
    }

    #[test]
    fn failing_first_jobs_dont_stop_the_rest() {
        let now = Instant::now();
        let mut q = Queue::new();
        for i in 0..120 {
            q.push(
                job(Feature::Signals, &format!("k{i}"), Priority::Background),
                now,
            );
        }
        let mut done = 0;
        while let Some(s) = q.next(now) {
            let failed = s.job.key[1..].parse::<usize>().unwrap() < 10;
            q.finish(s.id, if failed { Err(ErrorKind::Job) } else { Ok(()) }, now);
            done += usize::from(!failed);
        }
        assert_eq!(done, 110);
    }

    #[test]
    fn the_queue_holds_two_hundred() {
        let now = Instant::now();
        let mut q = Queue::new();
        for i in 0..CAP {
            assert_eq!(q.push(bg(&i.to_string()), now), Pushed::Queued);
        }
        assert_eq!(q.push(bg("over"), now), Pushed::Full);
        let mut asked = bg("asked");
        asked.priority = Priority::Interactive;
        assert_eq!(q.push(asked, now), Pushed::Queued);
        assert_eq!(q.waiting_len(), CAP);
        assert!(!q.holds(&ident(Feature::Line, &(CAP - 1).to_string())));
    }

    #[test]
    fn providers_stop_and_back_off() {
        let t0 = Instant::now();
        let mut q = Queue::new();
        q.push(bg("a"), t0);
        let s = q.next(t0).unwrap();
        q.finish(s.id, Err(ErrorKind::Backoff), t0);
        assert_eq!(
            q.health(Provider::Writing, t0),
            Health::Waiting(BACKOFF_FIRST)
        );
        q.push(bg("b"), t0);
        assert!(q.next(t0).is_none());
        assert_eq!(q.next_wake(t0), Some(BACKOFF_FIRST));
        // Jev isn't held up by the writing model.
        q.push(job(Feature::Signals, "j", Priority::Background), t0);
        assert_eq!(q.next(t0).unwrap().job.key, "j");
        let t1 = t0 + BACKOFF_FIRST;
        let s = q.next(t1).unwrap();
        q.finish(s.id, Err(ErrorKind::Backoff), t1);
        assert_eq!(
            q.health(Provider::Writing, t1),
            Health::Waiting(BACKOFF_FIRST * 2)
        );
        q.reset(Provider::Writing);
        q.push(bg("c"), t1);
        let s = q.next(t1).unwrap();
        q.finish(s.id, Err(ErrorKind::Stop), t1);
        assert_eq!(
            q.health(Provider::Writing, t1 + BACKOFF_MAX),
            Health::Stopped
        );
        q.reset(Provider::Writing);
        assert_eq!(q.health(Provider::Writing, t1), Health::Ready);
    }
}

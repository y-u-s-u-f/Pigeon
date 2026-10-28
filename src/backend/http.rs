//! HTTP: the `Transport` the backend sends requests through, a fake one for
//! tests, and Google's HTTP batch format (up to 50 calls in one request).
//!
//! - `UreqTransport`: the network, logging each call as
//!   `METHOD host/path status ms` and nothing else.
//! - `GoogleTransport`: on top of it, the access token, one refresh and
//!   retry on 401, backoff on network errors and rate limits, the Gmail
//!   quota limiter, and at most 4 calls in flight. Sync code calls it
//!   through `Transport`, so tests swap in `FakeTransport`.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: &'static str,
    /// Absolute, with any query string.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Quota units the call costs (2.6).
    pub cost: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// No connection, DNS, TLS or a timeout: worth retrying.
    Network(String),
    /// The refresh token stopped working (`invalid_grant`): nothing goes
    /// until the user signs in again (STA-4).
    AuthExpired,
    /// The answer was bigger than `BODY_LIMIT`: an answer all the same,
    /// so never a sign of being offline (MC-012).
    TooLarge,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // `{:#}` adds the raw cause, for the log.
            TransportError::Network(e) if f.alternate() => {
                write!(f, "Couldn't reach Google. Check your connection. ({e})")
            }
            TransportError::Network(_) => {
                write!(f, "Couldn't reach Google. Check your connection.")
            }
            TransportError::AuthExpired => write!(f, "Sign in again to keep syncing."),
            TransportError::TooLarge => write!(f, "It's too large to download."),
        }
    }
}

impl std::error::Error for TransportError {}

impl TransportError {
    /// No network or no sign-in: trying again later may work. A too large
    /// answer stays too large.
    pub fn goes_later(&self) -> bool {
        !matches!(self, TransportError::TooLarge)
    }
}

/// Sends one request and blocks for its response. Called off the main
/// thread.
pub trait Transport: Send + Sync {
    fn send(&self, request: &Request) -> Result<Response, TransportError>;

    /// `send` with `Authorization: Bearer {token}`. The network adds the
    /// header as it goes, so a large upload isn't copied for it (MC-014).
    fn send_authorized(&self, request: &Request, token: &str) -> Result<Response, TransportError> {
        let mut signed = request.clone();
        signed
            .headers
            .push(("Authorization".into(), format!("Bearer {token}")));
        self.send(&signed)
    }
}

impl Request {
    pub fn get(url: impl Into<String>, cost: u32) -> Self {
        Self {
            method: "GET",
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            cost,
        }
    }

    pub fn json(
        method: &'static str,
        url: impl Into<String>,
        body: &serde_json::Value,
        cost: u32,
    ) -> Self {
        Self {
            method,
            url: url.into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.to_string().into_bytes(),
            cost,
        }
    }

    /// The URL's path, without host or query.
    pub fn path(&self) -> &str {
        let rest = self
            .url
            .split_once("://")
            .map_or(self.url.as_str(), |(_, rest)| rest);
        let path = rest.find('/').map_or("/", |ix| &rest[ix..]);
        path.split('?').next().unwrap_or(path)
    }

    /// The query's pairs, undecoded.
    pub fn query(&self) -> Vec<(&str, &str)> {
        self.url
            .split_once('?')
            .map(|(_, q)| q)
            .unwrap_or_default()
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').unwrap_or((p, "")))
            .collect()
    }

    /// The path and query, as a batch part's request line wants them.
    fn path_and_query(&self) -> &str {
        let rest = self
            .url
            .split_once("://")
            .map_or(self.url.as_str(), |(_, rest)| rest);
        rest.find('/').map_or("/", |ix| &rest[ix..])
    }
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> serde_json::Result<T> {
        serde_json::from_slice(&self.body)
    }

    /// Google's `error.message`, when the body carries one.
    pub fn error_message(&self) -> Option<String> {
        self.json::<serde_json::Value>()
            .ok()?
            .pointer("/error/message")?
            .as_str()
            .map(str::to_string)
    }
}

/// Percent-encodes a path segment or query value: all but RFC 3986's
/// unreserved characters.
pub fn encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[usize::from(b >> 4)] as char);
                out.push(HEX[usize::from(b & 0xf)] as char);
            }
        }
    }
    out
}

/// `%XX` escapes as UTF-8; a broken escape stays as written.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Never the query, the headers' secrets or the body: they hold tokens,
/// codes and mail.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                let secret = name.eq_ignore_ascii_case("authorization")
                    || name.eq_ignore_ascii_case("cookie");
                (name.as_str(), if secret { "***" } else { value.as_str() })
            })
            .collect();
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &redact_url(&self.url))
            .field("headers", &headers)
            .field("body", &format_args!("{} bytes", self.body.len()))
            .field("cost", &self.cost)
            .finish()
    }
}

/// `host/path`, without the scheme or the query.
pub fn redact_url(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    if url.contains('?') {
        format!("{rest}?…")
    } else {
        rest.to_string()
    }
}

/// The one line a call leaves in the log.
pub fn log_line(method: &str, url: &str, status: Option<u16>, elapsed: Duration) -> String {
    let status = status.map_or("ERR".to_string(), |s| s.to_string());
    format!(
        "{method} {} {status} {}ms",
        redact_url(url),
        elapsed.as_millis()
    )
}

// The network.

/// Biggest body read: a raw message with its attachments, in base64.
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

pub struct UreqTransport {
    agent: ureq::Agent,
    /// `BODY_LIMIT`; smaller in tests.
    body_limit: u64,
    /// The dev offline switch's file, with `PIGEON_DEV=1` (`offline_switch`).
    offline_switch: Option<PathBuf>,
    /// W6: the folder the failure drills' files go in, with `PIGEON_DEV=1`.
    drills: Option<PathBuf>,
}

/// W11, dev only: with `PIGEON_DEV=1`, every request fails as a network
/// error while this file is in the data folder, so offline states and the
/// queue can be tested without touching Wi-Fi.
pub const OFFLINE_SWITCH_FILE: &str = "simulate-offline";

/// The dev offline switch's path; `None` in normal runs.
pub fn offline_switch() -> Option<PathBuf> {
    std::env::var("PIGEON_DEV")
        .is_ok_and(|v| v == "1")
        .then(|| super::data_dir().join(OFFLINE_SWITCH_FILE))
}

/// Whether the dev offline switch is on now.
pub fn simulating_offline() -> bool {
    offline_switch().is_some_and(|f| f.exists())
}

/// Sprint 2 W6, dev only: with `PIGEON_DEV=1`, the next Google write (a
/// POST, PUT, PATCH or DELETE, or a batch holding one; never a read or the
/// sign-in's token calls) fails once while one of these files is in the
/// data folder, which is then deleted. Refusal answers 400 as Gmail does;
/// conflict answers 412, as for a stale ETag.
pub const REFUSAL_DRILL_FILE: &str = "simulate-refusal";
pub const CONFLICT_DRILL_FILE: &str = "simulate-conflict";

pub const REFUSAL_BODY: &str = r#"{"error":{"code":400,"message":"Simulated refusal (drill)","errors":[{"message":"Simulated refusal (drill)","domain":"global","reason":"invalidArgument"}],"status":"INVALID_ARGUMENT"}}"#;
pub const CONFLICT_BODY: &str = r#"{"error":{"code":412,"message":"Precondition Failed","errors":[{"message":"Precondition Failed","domain":"global","reason":"conditionNotMet","location":"If-Match","locationType":"header"}]}}"#;

/// The drills' folder: the data folder with `PIGEON_DEV=1`, else none.
pub fn drill_dir() -> Option<PathBuf> {
    std::env::var("PIGEON_DEV")
        .is_ok_and(|v| v == "1")
        .then(super::data_dir)
}

/// A write to a Google API: what a drill may fail.
fn is_google_write(request: &Request) -> bool {
    const WRITES: [&str; 4] = ["POST", "PUT", "PATCH", "DELETE"];
    let rest = request
        .url
        .split_once("://")
        .map_or(request.url.as_str(), |(_, rest)| rest);
    let host = rest.split(['/', '?']).next().unwrap_or_default();
    if !WRITES.contains(&request.method)
        || !host.ends_with("googleapis.com")
        || host == "oauth2.googleapis.com"
    {
        return false;
    }
    // A batch is a POST either way: it counts when a call in it writes.
    if request.path().starts_with("/batch") {
        return String::from_utf8_lossy(&request.body)
            .lines()
            .any(|line| WRITES.iter().any(|w| line.starts_with(&format!("{w} /"))));
    }
    true
}

/// The drill's answer to `request`, taking the drill's file, if one is set
/// and `request` is a write.
fn drill(dir: &std::path::Path, request: &Request) -> Option<Response> {
    if !is_google_write(request) {
        return None;
    }
    let (file, status, body) = [
        (REFUSAL_DRILL_FILE, 400, REFUSAL_BODY),
        (CONFLICT_DRILL_FILE, 412, CONFLICT_BODY),
    ]
    .into_iter()
    .find(|(file, ..)| dir.join(file).exists())?;
    // Taken once: a drill whose file is already gone fails nothing.
    std::fs::remove_file(dir.join(file)).ok()?;
    crate::logging::info(&format!(
        "drill: {file}: {}",
        log_line(request.method, &request.url, Some(status), Duration::ZERO)
    ));
    Some(Response {
        status,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: body.as_bytes().to_vec(),
    })
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqTransport {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(120)))
            // Google gzips only for a User-Agent saying so (MC-011).
            .user_agent("Pigeon (gzip)")
            // As many as `GoogleTransport` sends at once (MC-022).
            .max_idle_connections_per_host(MAX_IN_FLIGHT)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            body_limit: BODY_LIMIT,
            offline_switch: offline_switch(),
            drills: drill_dir(),
        }
    }

    /// Fails the next write once while a drill's file is in `dir`, as
    /// `PIGEON_DEV=1` does with the data folder.
    pub fn with_drills(dir: PathBuf) -> Self {
        Self {
            drills: Some(dir),
            ..Self::new()
        }
    }

    /// Fails every request while `file` exists, as `PIGEON_DEV=1` does with
    /// the data folder's `simulate-offline`.
    pub fn with_offline_switch(file: PathBuf) -> Self {
        Self {
            offline_switch: Some(file),
            ..Self::new()
        }
    }

    fn run(&self, request: &Request, token: Option<&str>) -> Result<Response, ureq::Error> {
        run(&self.agent, request, token, self.body_limit)
    }
}

/// `request` on `agent`, with `token` as its bearer, reading at most
/// `limit` bytes of the body.
pub fn run(
    agent: &ureq::Agent,
    request: &Request,
    token: Option<&str>,
    limit: u64,
) -> Result<Response, ureq::Error> {
    let mut builder = ureq::http::Request::builder()
        .method(request.method)
        .uri(&request.url);
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let response = if request.body.is_empty() {
        agent.run(builder.body(())?)?
    } else {
        agent.run(builder.body(&request.body[..])?)?
    };
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or_default().to_string()))
        .collect();
    let body = response
        .into_body()
        .with_config()
        .limit(limit)
        .read_to_vec()?;
    Ok(Response {
        status,
        headers,
        body,
    })
}

impl Transport for UreqTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.send_as(request, None)
    }

    fn send_authorized(&self, request: &Request, token: &str) -> Result<Response, TransportError> {
        self.send_as(request, Some(token))
    }
}

impl UreqTransport {
    fn send_as(&self, request: &Request, token: Option<&str>) -> Result<Response, TransportError> {
        let started = Instant::now();
        if self.offline_switch.as_ref().is_some_and(|f| f.exists()) {
            eprintln!(
                "Pigeon: {} (simulated offline)",
                log_line(request.method, &request.url, None, started.elapsed())
            );
            return Err(TransportError::Network("simulated offline".into()));
        }
        if let Some(response) = self.drills.as_deref().and_then(|dir| drill(dir, request)) {
            return Ok(response);
        }
        let result = self.run(request, token);
        let status = result.as_ref().ok().map(|r| r.status);
        eprintln!(
            "Pigeon: {}",
            log_line(request.method, &request.url, status, started.elapsed())
        );
        // ureq's errors name the host at most; never the query or headers.
        result.map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => TransportError::TooLarge,
            e => TransportError::Network(e.to_string()),
        })
    }
}

// Time, so tests can run backoff and the limiter without waiting.

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
    /// Up to 1 s of jitter added to each backoff.
    fn jitter(&self) -> Duration;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn jitter(&self) -> Duration {
        // Random enough for spreading retries out.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        Duration::from_millis(u64::from(nanos % 1000))
    }
}

// Retries (docs/BACKEND_SPRINT_1.md, 2.7).

/// Tries per request, the first included, before it fails.
pub const MAX_ATTEMPTS: u32 = 8;

/// The wait before retry number `attempt + 1`: 1, 2, 4 … 64 s, plus jitter.
pub fn backoff(attempt: u32, jitter: Duration) -> Duration {
    Duration::from_secs(1u64 << attempt.min(6)) + jitter
}

/// 429, 5xx, and 403 for a rate limit (not for a permission).
pub fn is_retryable(response: &Response) -> bool {
    match response.status {
        429 | 500..=599 => true,
        403 => {
            let body = String::from_utf8_lossy(&response.body);
            body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded")
        }
        _ => false,
    }
}

// The Gmail quota (2.6): 6,000 units a minute per user.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Sync: may use all but the reserve.
    Background,
    /// Something the user did: goes first, into the reserve too.
    User,
}

/// A token bucket in quota units.
pub struct Limiter {
    capacity: f64,
    /// Units only user actions may spend.
    reserve: f64,
    per_sec: f64,
    state: Mutex<(f64, Instant)>,
}

impl Limiter {
    pub fn new(capacity: u32, reserve: u32, per_minute: u32, now: Instant) -> Self {
        Self {
            capacity: capacity.into(),
            reserve: reserve.into(),
            per_sec: f64::from(per_minute) / 60.,
            state: Mutex::new((capacity.into(), now)),
        }
    }

    /// Gmail's: 6,000 a minute, 1,000 of them kept for user actions.
    pub fn gmail(now: Instant) -> Self {
        Self::new(6000, 1000, 6000, now)
    }

    /// Takes `cost` and answers zero, or answers how long until it can.
    /// A call costing more than the bucket goes once it's full.
    pub fn try_take(&self, cost: u32, priority: Priority, now: Instant) -> Duration {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let (tokens, last) = &mut *state;
        let elapsed = now.saturating_duration_since(*last).as_secs_f64();
        *tokens = (*tokens + elapsed * self.per_sec).min(self.capacity);
        *last = now;
        let floor = match priority {
            Priority::Background => self.reserve,
            Priority::User => 0.,
        };
        let need = f64::from(cost).min(self.capacity - floor);
        let available = *tokens - floor;
        if available >= need {
            *tokens -= f64::from(cost);
            Duration::ZERO
        } else {
            Duration::from_secs_f64((need - available) / self.per_sec)
        }
    }

    pub fn acquire(&self, cost: u32, priority: Priority, clock: &dyn Clock) {
        loop {
            let wait = self.try_take(cost, priority, clock.now());
            if wait.is_zero() {
                return;
            }
            clock.sleep(wait);
        }
    }

    /// Units left now, for tests.
    pub fn tokens(&self, now: Instant) -> f64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let elapsed = now.saturating_duration_since(state.1).as_secs_f64();
        (state.0 + elapsed * self.per_sec).min(self.capacity)
    }
}

/// At most `max` at once: `ureq` blocks a background thread per call.
struct InFlight {
    count: Mutex<usize>,
    freed: Condvar,
    max: usize,
}

struct Permit<'a>(&'a InFlight);

impl InFlight {
    fn new(max: usize) -> Self {
        Self {
            count: Mutex::new(0),
            freed: Condvar::new(),
            max,
        }
    }

    fn acquire(&self) -> Permit<'_> {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count >= self.max {
            count = self.freed.wait(count).unwrap_or_else(|e| e.into_inner());
        }
        *count += 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.count.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        self.0.freed.notify_one();
    }
}

pub const MAX_IN_FLIGHT: usize = 4;

/// Hands out access tokens (`auth::Auth`).
pub trait Authorizer: Send + Sync {
    /// A token good for at least the next minute, refreshing if needed.
    fn access_token(&self) -> Result<String, TransportError>;
    /// A request using `token` got 401: the next call refreshes.
    fn invalidate(&self, token: &str);
}

/// Google's APIs, signed in.
pub struct GoogleTransport {
    inner: Arc<dyn Transport>,
    auth: Arc<dyn Authorizer>,
    clock: Arc<dyn Clock>,
    limiter: Limiter,
    in_flight: InFlight,
    /// Network failures in a row; two mean offline (2.9).
    failures: AtomicU32,
    /// Requests sleeping through a long rate-limit or server-error backoff.
    backing_off: AtomicU32,
}

/// A backoff at least this long shows as "Waiting for Gmail…".
pub const LONG_BACKOFF: Duration = Duration::from_secs(4);

impl GoogleTransport {
    pub fn new(
        inner: Arc<dyn Transport>,
        auth: Arc<dyn Authorizer>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let now = clock.now();
        Self {
            inner,
            auth,
            clock,
            limiter: Limiter::gmail(now),
            in_flight: InFlight::new(MAX_IN_FLIGHT),
            failures: AtomicU32::new(0),
            backing_off: AtomicU32::new(0),
        }
    }

    /// A request is waiting out a long backoff (2.9): "Waiting for Gmail…".
    pub fn waiting_for_gmail(&self) -> bool {
        self.backing_off.load(Ordering::Relaxed) > 0
    }

    /// Two network failures in a row, and no answer since.
    pub fn is_offline(&self) -> bool {
        self.failures.load(Ordering::Relaxed) >= 2
    }

    /// For something the user did: ahead of sync, into the quota's reserve.
    pub fn send_user(&self, request: &Request) -> Result<Response, TransportError> {
        self.send_with(request, Priority::User)
    }

    /// Sends with the access token, waiting for quota first. A 401 refreshes
    /// the token and retries once. Network errors, 429, 5xx and rate-limit
    /// 403s back off and retry, up to `MAX_ATTEMPTS` tries; then the last
    /// response or error is the answer. Once offline, a network error is
    /// the answer at once: polls retry, backing off (2.9).
    pub fn send_with(
        &self,
        request: &Request,
        priority: Priority,
    ) -> Result<Response, TransportError> {
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            let mut long_wait = false;
            self.limiter.acquire(request.cost, priority, &*self.clock);
            let result = self.auth.access_token().and_then(|token| {
                let _permit = self.in_flight.acquire();
                self.inner
                    .send_authorized(request, &token)
                    .map(|r| (r, token))
            });
            match result {
                Ok((response, token)) => {
                    self.failures.store(0, Ordering::Relaxed);
                    if response.status == 401 && !refreshed {
                        refreshed = true;
                        self.auth.invalidate(&token);
                        continue;
                    }
                    if !is_retryable(&response) || attempt + 1 >= MAX_ATTEMPTS {
                        return Ok(response);
                    }
                    long_wait = true;
                }
                Err(TransportError::AuthExpired) => return Err(TransportError::AuthExpired),
                // It answered: the network is there, and asking again
                // gets the same.
                Err(TransportError::TooLarge) => {
                    self.failures.store(0, Ordering::Relaxed);
                    return Err(TransportError::TooLarge);
                }
                Err(e @ TransportError::Network(_)) => {
                    self.failures.fetch_add(1, Ordering::Relaxed);
                    if self.is_offline() || attempt + 1 >= MAX_ATTEMPTS {
                        return Err(e);
                    }
                }
            }
            let wait = backoff(attempt, self.clock.jitter());
            let long_wait = long_wait && wait >= LONG_BACKOFF;
            if long_wait {
                self.backing_off.fetch_add(1, Ordering::Relaxed);
            }
            self.clock.sleep(wait);
            if long_wait {
                self.backing_off.fetch_sub(1, Ordering::Relaxed);
            }
            attempt += 1;
        }
    }
}

impl Transport for GoogleTransport {
    /// Sync's priority; see `send_user`.
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.send_with(request, Priority::Background)
    }
}

// Tests' clock and transport.

/// Time that moves only when slept through; jitter is always 250 ms.
#[cfg(test)]
pub struct FakeClock {
    start: Instant,
    offset: Mutex<Duration>,
    sleeps: Mutex<Vec<Duration>>,
}

#[cfg(test)]
impl FakeClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            offset: Mutex::new(Duration::ZERO),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    pub fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.start + *self.offset.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) {
        *self.offset.lock().unwrap() += duration;
        self.sleeps.lock().unwrap().push(duration);
    }

    fn jitter(&self) -> Duration {
        Duration::from_millis(250)
    }
}

/// Answers requests from a script: each route matches a method, a path and
/// some query pairs, and replies with its responses in turn, the last one
/// again once the others are used. Records every request.
#[derive(Default)]
pub struct FakeTransport {
    routes: Mutex<Vec<Route>>,
    log: Mutex<Vec<Request>>,
}

struct Route {
    method: &'static str,
    path: String,
    query: Vec<(String, String)>,
    responses: VecDeque<(u16, String)>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reply to `method path` (with every pair in `query`) with
    /// `responses`, as (status, body), in order.
    pub fn on(
        self,
        method: &'static str,
        path: &str,
        query: &[(&str, &str)],
        responses: &[(u16, &str)],
    ) -> Self {
        self.routes.lock().unwrap().push(Route {
            method,
            path: path.into(),
            query: query
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            responses: responses
                .iter()
                .map(|&(status, body)| (status, body.to_string()))
                .collect(),
        });
        self
    }

    /// Every request sent so far.
    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }
}

impl Transport for FakeTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.log.lock().unwrap().push(request.clone());
        let mut routes = self.routes.lock().unwrap();
        let query = request.query();
        let route = routes.iter_mut().find(|r| {
            r.method == request.method
                && r.path == request.path()
                && r.query
                    .iter()
                    .all(|(k, v)| query.contains(&(k.as_str(), v.as_str())))
        });
        // A Calendar batch no route answers whole: each call by its own
        // route. (Gmail tests script their batches whole.)
        if route.is_none() && request.path() == "/batch/calendar/v3" {
            drop(routes);
            return self.answer_batch(request);
        }
        let Some(route) = route else {
            return Ok(Response {
                status: 404,
                headers: Vec::new(),
                body: b"{\"error\":{\"code\":404,\"message\":\"no fake route\"}}".to_vec(),
            });
        };
        let (status, body) = if route.responses.len() > 1 {
            route.responses.pop_front().expect("checked")
        } else {
            route
                .responses
                .front()
                .cloned()
                .unwrap_or((200, String::new()))
        };
        if status == 0 {
            return Err(TransportError::Network("scripted failure".into()));
        }
        // A scripted batch answer uses the boundary `batch_body` writes.
        let content_type = if request.path().starts_with("/batch") {
            "multipart/mixed; boundary=b"
        } else {
            "application/json"
        };
        Ok(Response {
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.into_bytes(),
        })
    }
}

/// A batch answer as Google writes it, boundary `b`: each part's (status,
/// body), in order.
#[cfg(test)]
pub fn batch_body(parts: &[(u16, &str)]) -> String {
    let mut out = String::new();
    for (ix, (status, body)) in parts.iter().enumerate() {
        out.push_str(&format!(
            "--b\r\nContent-Type: application/http\r\nContent-ID: <response-item-{}>\r\n\r\n\
             HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\r\n{body}\r\n",
            ix + 1
        ));
    }
    out.push_str("--b--\r\n");
    out
}

impl FakeTransport {
    /// Answers each call of a batch (as `encode_batch` writes them) by its
    /// route, as Google's batch answer.
    fn answer_batch(&self, batch: &Request) -> Result<Response, TransportError> {
        let rest = batch.url.split_once("://").map_or("", |(_, r)| r);
        let host = rest.split('/').next().unwrap_or_default();
        let text = String::from_utf8_lossy(&batch.body).into_owned();
        let mut body = String::new();
        for (ix, part) in text.split("--batch_pigeon").skip(1).enumerate() {
            if part.starts_with("--") {
                break;
            }
            let Some((_, http)) = part.split_once("\r\n\r\n") else {
                continue;
            };
            let (head, call_body) = http.split_once("\r\n\r\n").unwrap_or((http, ""));
            let mut words = head.lines().next().unwrap_or_default().split_whitespace();
            let method = match words.next() {
                Some("POST") => "POST",
                Some("PATCH") => "PATCH",
                Some("PUT") => "PUT",
                Some("DELETE") => "DELETE",
                _ => "GET",
            };
            let path = words.next().unwrap_or("/");
            let call = Request {
                method,
                url: format!("https://{host}{path}"),
                headers: Vec::new(),
                body: call_body.trim_end_matches("\r\n").as_bytes().to_vec(),
                cost: 0,
            };
            let answer = self.send(&call)?;
            body.push_str(&format!(
                "--fake_batch\r\nContent-Type: application/http\r\n\
                 Content-ID: <response-item-{}>\r\n\r\nHTTP/1.1 {} X\r\n\
                 Content-Type: application/json\r\n\r\n{}\r\n",
                ix + 1,
                answer.status,
                String::from_utf8_lossy(&answer.body)
            ));
        }
        body.push_str("--fake_batch--\r\n");
        Ok(Response {
            status: 200,
            headers: vec![(
                "Content-Type".into(),
                "multipart/mixed; boundary=fake_batch".into(),
            )],
            body: body.into_bytes(),
        })
    }
}

// Google's HTTP batch: https://developers.google.com/gmail/api/guides/batch

/// Most calls Google takes in one batch is 100; it advises 50.
pub const BATCH_LIMIT: usize = 50;

/// One request carrying `requests` to `endpoint` (Gmail's is
/// `https://gmail.googleapis.com/batch/gmail/v1`). Parts are numbered from
/// 1 in order; costs add up.
pub fn encode_batch<'a>(
    endpoint: &str,
    requests: impl IntoIterator<Item = &'a Request>,
    boundary: &str,
) -> Request {
    let mut body = String::new();
    let mut cost = 0;
    for (ix, request) in requests.into_iter().enumerate() {
        cost += request.cost;
        body.push_str(&format!(
            "--{boundary}\r\nContent-Type: application/http\r\nContent-ID: <item-{}>\r\n\r\n",
            ix + 1
        ));
        body.push_str(&format!(
            "{} {} HTTP/1.1\r\n",
            request.method,
            request.path_and_query()
        ));
        for (name, value) in &request.headers {
            body.push_str(&format!("{name}: {value}\r\n"));
        }
        if !request.body.is_empty() {
            body.push_str(&format!("Content-Length: {}\r\n", request.body.len()));
        }
        body.push_str("\r\n");
        body.push_str(&String::from_utf8_lossy(&request.body));
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{boundary}--\r\n"));
    Request {
        method: "POST",
        url: endpoint.into(),
        headers: vec![(
            "Content-Type".into(),
            format!("multipart/mixed; boundary={boundary}"),
        )],
        body: body.into_bytes(),
        cost,
    }
}

/// Why a call sent in a batch has no answer of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchError {
    /// The batch didn't go, or no answer came.
    Transport(TransportError),
    /// Google refused the whole batch with this answer (the transport
    /// already retried it).
    Refused(Response),
    /// The batch's answer, or the call's part of it, didn't parse.
    Malformed(String),
}

/// Each request's answer, in order, sent to `endpoint` in HTTP batches of
/// at most `size`, one after another. Calls answered with a rate limit
/// or a server error (429, 5xx, rate-limit 403) go again after a backoff,
/// up to `MAX_ATTEMPTS` tries; the others are answered as they came.
pub fn batch(
    transport: &dyn Transport,
    clock: &dyn Clock,
    endpoint: &str,
    size: usize,
    requests: &[Request],
) -> Vec<Result<Response, BatchError>> {
    let mut out: Vec<Option<Result<Response, BatchError>>> = vec![None; requests.len()];
    let mut pending: Vec<usize> = (0..requests.len()).collect();
    let mut attempt = 0;
    while !pending.is_empty() {
        let last = attempt + 1 >= MAX_ATTEMPTS;
        let mut retry = Vec::new();
        for chunk in pending.chunks(size) {
            let calls: Vec<&Request> = chunk.iter().map(|&ix| &requests[ix]).collect();
            for (&ix, part) in chunk.iter().zip(send_batch(transport, endpoint, &calls)) {
                match part {
                    Ok(r) if !last && is_retryable(&r) => retry.push(ix),
                    part => out[ix] = Some(part),
                }
            }
        }
        if !retry.is_empty() {
            let wait = backoff(attempt, clock.jitter());
            eprintln!(
                "Pigeon: batch {}: {} of {} calls rate-limited; retrying in {:.1}s",
                redact_url(endpoint),
                retry.len(),
                pending.len(),
                wait.as_secs_f32()
            );
            clock.sleep(wait);
        }
        pending = retry;
        attempt += 1;
    }
    out.into_iter()
        .map(|r| r.expect("every call answered"))
        .collect()
}

/// One HTTP batch: each call's response, or why there's none.
fn send_batch(
    transport: &dyn Transport,
    endpoint: &str,
    calls: &[&Request],
) -> Vec<Result<Response, BatchError>> {
    let whole = |e: BatchError| calls.iter().map(|_| Err(e.clone())).collect();
    let response = match transport.send(&encode_batch(
        endpoint,
        calls.iter().copied(),
        "batch_pigeon",
    )) {
        Ok(r) if r.is_success() => r,
        Ok(r) => return whole(BatchError::Refused(r)),
        Err(e) => return whole(BatchError::Transport(e)),
    };
    match decode_batch(&response, calls.len()) {
        Ok(parts) => parts
            .into_iter()
            .map(|part| part.ok_or_else(|| BatchError::Malformed("missing from the batch".into())))
            .collect(),
        Err(e) => whole(BatchError::Malformed(e)),
    }
}

/// The parts of a batch response, put back in the order of the requests
/// (by their `Content-ID: <response-item-N>`). A part missing from the
/// response, or one that doesn't parse, is `None`. Parts are split on the
/// boundary's bytes, CRLF or LF, and each body copied once (MC-015).
pub fn decode_batch(response: &Response, count: usize) -> Result<Vec<Option<Response>>, String> {
    let content_type = response.header("Content-Type").unwrap_or_default();
    let boundary = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))
        .map(|b| b.trim_matches('"'))
        .ok_or("batch response without a boundary")?;
    let delimiter = format!("--{boundary}");
    let mut out: Vec<Option<Response>> = vec![None; count];
    for part in split_bytes(&response.body, delimiter.as_bytes()).skip(1) {
        if part.starts_with(b"--") {
            break;
        }
        let part = trim_start_newlines(part);
        let Some((part_headers, http)) = split_head(part) else {
            continue;
        };
        let index = String::from_utf8_lossy(part_headers)
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if !name.trim().eq_ignore_ascii_case("content-id") {
                    return None;
                }
                let id = value.trim().trim_start_matches('<').trim_end_matches('>');
                id.rsplit('-').next()?.parse::<usize>().ok()
            });
        let Some(slot) = index.and_then(|n| n.checked_sub(1)).filter(|&n| n < count) else {
            continue;
        };
        out[slot] = parse_http(http);
    }
    Ok(out)
}

/// The pieces of `bytes` between occurrences of `delimiter`.
fn split_bytes<'a>(bytes: &'a [u8], delimiter: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
    let mut rest = Some(bytes);
    std::iter::from_fn(move || {
        let current = rest?;
        match find(current, delimiter) {
            Some(ix) => {
                rest = Some(&current[ix + delimiter.len()..]);
                Some(&current[..ix])
            }
            None => {
                rest = None;
                Some(current)
            }
        }
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn trim_start_newlines(mut bytes: &[u8]) -> &[u8] {
    while let [b'\r' | b'\n', rest @ ..] = bytes {
        bytes = rest;
    }
    bytes
}

fn trim_end_newlines(mut bytes: &[u8]) -> &[u8] {
    while let [rest @ .., b'\r' | b'\n'] = bytes {
        bytes = rest;
    }
    bytes
}

/// The head before the first blank line (CRLF or LF), and what follows.
fn split_head(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let crlf = find(bytes, b"\r\n\r\n").map(|ix| (ix, 4));
    let lf = find(bytes, b"\n\n").map(|ix| (ix, 2));
    let (ix, len) = match (crlf, lf) {
        (Some(a), Some(b)) => a.min(b),
        (a, b) => a.or(b)?,
    };
    Some((&bytes[..ix], &bytes[ix + len..]))
}

/// "HTTP/1.1 200 OK", headers, a blank line, the body.
fn parse_http(bytes: &[u8]) -> Option<Response> {
    let (head, body) = split_head(bytes).unwrap_or((bytes, b""));
    let head = String::from_utf8_lossy(head);
    let mut lines = head.lines();
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    Some(Response {
        status,
        headers,
        body: trim_end_newlines(body).to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GMAIL_BATCH: &str = "https://gmail.googleapis.com/batch/gmail/v1";

    #[test]
    fn batch_request_encodes_each_call() {
        let requests = [
            Request::get(
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/t1?format=metadata",
                40,
            ),
            Request::json(
                "POST",
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/t2/modify",
                &serde_json::json!({ "removeLabelIds": ["INBOX"] }),
                10,
            ),
        ];
        let batch = encode_batch(GMAIL_BATCH, &requests, "batch_pigeon");
        assert_eq!(batch.method, "POST");
        assert_eq!(batch.url, GMAIL_BATCH);
        assert_eq!(batch.cost, 50);
        assert_eq!(
            batch.headers,
            [(
                "Content-Type".to_string(),
                "multipart/mixed; boundary=batch_pigeon".to_string()
            )]
        );
        let body = String::from_utf8(batch.body).unwrap();
        assert_eq!(
            body,
            "--batch_pigeon\r\n\
             Content-Type: application/http\r\n\
             Content-ID: <item-1>\r\n\
             \r\n\
             GET /gmail/v1/users/me/threads/t1?format=metadata HTTP/1.1\r\n\
             \r\n\
             \r\n\
             --batch_pigeon\r\n\
             Content-Type: application/http\r\n\
             Content-ID: <item-2>\r\n\
             \r\n\
             POST /gmail/v1/users/me/threads/t2/modify HTTP/1.1\r\n\
             Content-Type: application/json\r\n\
             Content-Length: 28\r\n\
             \r\n\
             {\"removeLabelIds\":[\"INBOX\"]}\r\n\
             --batch_pigeon--\r\n"
        );
    }

    #[test]
    fn batch_response_decodes_in_request_order() {
        let fixture = include_str!("../../tests/fixtures/google/gmail/batch_response.txt");
        let response = Response {
            status: 200,
            headers: vec![(
                "Content-Type".into(),
                "multipart/mixed; boundary=batch_fixture".into(),
            )],
            body: fixture.as_bytes().to_vec(),
        };
        let parts = decode_batch(&response, 3).unwrap();
        // The fixture answers item 2 first.
        let first = parts[0].as_ref().unwrap();
        assert_eq!(first.status, 200);
        assert_eq!(
            first.header("content-type"),
            Some("application/json; charset=UTF-8")
        );
        let thread: serde_json::Value = first.json().unwrap();
        assert_eq!(thread["id"], "thread-a1");
        let second = parts[1].as_ref().unwrap();
        assert_eq!(second.status, 404);
        // Item 3 is missing from the response.
        assert!(parts[2].is_none());
    }

    #[test]
    fn batch_response_with_bare_newlines_decodes() {
        let body = "--b\nContent-Type: application/http\nContent-ID: <response-item-2>\n\n\
                    HTTP/1.1 200 OK\nContent-Type: application/json\n\n{\"id\":\"x\"}\n\
                    --b\r\nContent-ID: <response-item-1>\r\n\r\nHTTP/1.1 404 X\r\n\r\n{}\r\n--b--\n";
        let response = Response {
            status: 200,
            headers: vec![("Content-Type".into(), "multipart/mixed; boundary=b".into())],
            body: body.as_bytes().to_vec(),
        };
        let parts = decode_batch(&response, 2).unwrap();
        assert_eq!(parts[0].as_ref().unwrap().status, 404);
        assert_eq!(parts[0].as_ref().unwrap().body, b"{}");
        let second = parts[1].as_ref().unwrap();
        assert_eq!(second.header("content-type"), Some("application/json"));
        assert_eq!(second.body, br#"{"id":"x"}"#);
    }

    /// MC-012: an answer over the read limit is `TooLarge`, not a network
    /// error, served by a one-shot local server.
    #[test]
    fn an_answer_over_the_limit_is_too_large() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buf = [0; 1024];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
            }
            let body = vec![b'x'; 256];
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 256\r\n\r\n").unwrap();
            stream.write_all(&body).unwrap();
        });
        let transport = UreqTransport {
            body_limit: 64,
            offline_switch: None,
            drills: None,
            ..UreqTransport::new()
        };
        let request = Request::get(format!("http://127.0.0.1:{port}/big"), 1);
        assert_eq!(transport.send(&request), Err(TransportError::TooLarge));
        server.join().unwrap();
    }

    /// MC-012: a too large answer fails at once and never counts toward
    /// offline.
    #[test]
    fn too_large_is_never_offline() {
        struct Big(Mutex<usize>);
        impl Transport for Big {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                *self.0.lock().unwrap() += 1;
                Err(TransportError::TooLarge)
            }
        }
        let big = Arc::new(Big(Mutex::new(0)));
        let clock = Arc::new(FakeClock::new());
        let transport = GoogleTransport::new(big.clone(), Arc::new(FakeAuth::default()), clock);
        for _ in 0..3 {
            assert_eq!(
                transport.send(&Request::get(PROFILE, 1)),
                Err(TransportError::TooLarge)
            );
        }
        assert_eq!(*big.0.lock().unwrap(), 3, "no retries");
        assert!(!transport.is_offline());
        assert!(!TransportError::TooLarge.goes_later());
    }

    /// Hands out `token-1`, then `token-2` after each invalidation.
    #[derive(Default)]
    struct FakeAuth {
        generation: Mutex<u32>,
    }

    impl Authorizer for FakeAuth {
        fn access_token(&self) -> Result<String, TransportError> {
            Ok(format!("token-{}", *self.generation.lock().unwrap() + 1))
        }

        fn invalidate(&self, token: &str) {
            let mut generation = self.generation.lock().unwrap();
            if token == format!("token-{}", *generation + 1) {
                *generation += 1;
            }
        }
    }

    const PROFILE: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";

    fn google(fake: Arc<FakeTransport>) -> (GoogleTransport, Arc<FakeClock>) {
        let clock = Arc::new(FakeClock::new());
        (
            GoogleTransport::new(fake, Arc::new(FakeAuth::default()), clock.clone()),
            clock,
        )
    }

    fn bearer(request: &Request) -> &str {
        request
            .headers
            .iter()
            .find(|(n, _)| n == "Authorization")
            .map(|(_, v)| v.as_str())
            .unwrap()
    }

    #[test]
    fn unauthorized_refreshes_and_retries_once() {
        let fake = Arc::new(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/profile",
            &[],
            &[(401, ""), (200, "{}")],
        ));
        let (transport, clock) = google(fake.clone());
        let response = transport.send(&Request::get(PROFILE, 1)).unwrap();
        assert_eq!(response.status, 200);
        let requests = fake.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(bearer(&requests[0]), "Bearer token-1");
        assert_eq!(bearer(&requests[1]), "Bearer token-2");
        // No backoff for a refresh.
        assert!(clock.sleeps().is_empty());

        // A second 401 in a row is the answer.
        let fake = Arc::new(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/profile",
            &[],
            &[(401, "")],
        ));
        let (transport, _) = google(fake.clone());
        assert_eq!(
            transport.send(&Request::get(PROFILE, 1)).unwrap().status,
            401
        );
        assert_eq!(fake.requests().len(), 2);
    }

    #[test]
    fn rate_limits_and_server_errors_back_off() {
        let fake = Arc::new(FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/profile",
            &[],
            &[(429, ""), (503, ""), (200, "{}")],
        ));
        let (transport, clock) = google(fake.clone());
        assert_eq!(
            transport.send(&Request::get(PROFILE, 1)).unwrap().status,
            200
        );
        let j = Duration::from_millis(250);
        assert_eq!(
            clock.sleeps(),
            [Duration::from_secs(1) + j, Duration::from_secs(2) + j]
        );
        assert_eq!(fake.requests().len(), 3);
    }

    #[test]
    fn network_errors_fail_fast_once_offline() {
        let fake =
            Arc::new(FakeTransport::new().on("GET", "/gmail/v1/users/me/profile", &[], &[(0, "")]));
        let (transport, clock) = google(fake.clone());
        assert!(matches!(
            transport.send(&Request::get(PROFILE, 1)),
            Err(TransportError::Network(_))
        ));
        // One retry; the second failure in a row is offline, and polls
        // take over the retrying.
        assert_eq!(fake.requests().len(), 2);
        let seconds: Vec<u64> = clock.sleeps().iter().map(|d| d.as_secs()).collect();
        assert_eq!(seconds, [1]);
        assert!(transport.is_offline());
        // Offline already: no retry at all.
        assert!(transport.send(&Request::get(PROFILE, 1)).is_err());
        assert_eq!(fake.requests().len(), 3);
        assert_eq!(backoff(12, Duration::ZERO), Duration::from_secs(64));
    }

    #[test]
    fn offline_switch_fails_every_request() {
        let dir = std::env::temp_dir().join(format!("pigeon-offline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(OFFLINE_SWITCH_FILE);
        std::fs::write(&file, "").unwrap();
        let transport = UreqTransport::with_offline_switch(file.clone());
        // Never reaches the network: the address doesn't resolve anyway.
        let request = Request::get("https://pigeon.invalid/gmail/v1/users/me/profile", 1);
        assert_eq!(
            transport.send(&request),
            Err(TransportError::Network("simulated offline".into()))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn drills_fail_the_next_write_once() {
        let dir = std::env::temp_dir().join(format!("pigeon-drills-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let modify = Request::json(
            "POST",
            "https://gmail.googleapis.com/gmail/v1/users/me/threads/t1/modify",
            &serde_json::json!({"removeLabelIds": ["INBOX"]}),
            10,
        );
        let read = Request::get(
            "https://gmail.googleapis.com/gmail/v1/users/me/threads/t1",
            10,
        );
        let token = Request {
            method: "POST",
            ..Request::get("https://oauth2.googleapis.com/token", 0)
        };
        let reads = encode_batch(
            "https://gmail.googleapis.com/batch/gmail/v1",
            &[read.clone(), read.clone()],
            "b",
        );
        let writes = encode_batch(
            "https://gmail.googleapis.com/batch/gmail/v1",
            &[modify.clone(), modify.clone()],
            "b",
        );
        let event = Request::json(
            "PATCH",
            "https://www.googleapis.com/calendar/v3/calendars/c/events/e",
            &serde_json::json!({"summary": "x"}),
            1,
        );

        // Nothing set: nothing fails.
        assert_eq!(drill(&dir, &modify), None);

        // Reads, read-only batches and the sign-in leave the drill waiting.
        std::fs::write(dir.join(REFUSAL_DRILL_FILE), "").unwrap();
        for request in [&read, &reads, &token] {
            assert_eq!(drill(&dir, request), None, "{request:?}");
        }
        let refused = drill(&dir, &writes).unwrap();
        assert_eq!(refused.status, 400);
        assert!(
            String::from_utf8_lossy(&refused.body).contains("Simulated refusal"),
            "a Gmail-like error body"
        );
        // Once.
        assert!(!dir.join(REFUSAL_DRILL_FILE).exists());
        assert_eq!(drill(&dir, &modify), None);

        std::fs::write(dir.join(CONFLICT_DRILL_FILE), "").unwrap();
        assert_eq!(drill(&dir, &event).unwrap().status, 412);
        assert_eq!(drill(&dir, &event), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn drills_are_inert_without_dev_mode() {
        // The test runner doesn't set PIGEON_DEV=1; read only, never set.
        if std::env::var("PIGEON_DEV").is_ok_and(|v| v == "1") {
            return;
        }
        assert_eq!(drill_dir(), None);
        assert_eq!(UreqTransport::new().drills, None);
    }

    #[test]
    fn only_rate_limit_forbiddens_retry() {
        let response = |status, body: &str| Response {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        };
        let limited = r#"{"error":{"errors":[{"reason":"userRateLimitExceeded"}],"code":403}}"#;
        assert!(is_retryable(&response(403, limited)));
        assert!(!is_retryable(&response(
            403,
            r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#
        )));
        assert!(is_retryable(&response(500, "")));
        assert!(!is_retryable(&response(404, "")));
        assert!(!is_retryable(&response(412, "")));
    }

    #[test]
    fn limiter_keeps_a_reserve_for_the_user() {
        let start = Instant::now();
        let limiter = Limiter::gmail(start);
        // Sync spends down to the reserve, then waits.
        assert_eq!(
            limiter.try_take(5000, Priority::Background, start),
            Duration::ZERO
        );
        let wait = limiter.try_take(40, Priority::Background, start);
        assert_eq!(wait, Duration::from_millis(400));
        // The user still gets through.
        assert_eq!(limiter.try_take(100, Priority::User, start), Duration::ZERO);
        assert_eq!(limiter.tokens(start), 900.);
        // 100 units a second come back.
        let later = start + Duration::from_secs(2);
        assert_eq!(limiter.tokens(later), 1100.);
        assert_eq!(
            limiter.try_take(100, Priority::Background, later),
            Duration::ZERO
        );
        // A call bigger than the bucket goes once it's full.
        let full = later + Duration::from_secs(60);
        assert_eq!(
            limiter.try_take(9000, Priority::Background, full),
            Duration::ZERO
        );
        assert!(limiter.tokens(full) < 0.);
    }

    #[test]
    fn limiter_waits_on_the_clock() {
        let clock = FakeClock::new();
        let limiter = Limiter::new(100, 0, 60, clock.now());
        limiter.acquire(100, Priority::Background, &clock);
        limiter.acquire(10, Priority::Background, &clock);
        assert_eq!(clock.sleeps(), [Duration::from_secs(10)]);
    }

    #[test]
    fn logs_and_debug_leave_secrets_out() {
        let mut request = Request::get(
            "https://oauth2.googleapis.com/token?code=4/secret-code&state=s",
            0,
        );
        request
            .headers
            .push(("Authorization".into(), "Bearer secret-token".into()));
        request.body = b"refresh_token=secret-refresh".to_vec();
        let printed = format!("{request:?}");
        for secret in ["secret-code", "secret-token", "secret-refresh"] {
            assert!(!printed.contains(secret), "{secret} in {printed}");
        }
        assert_eq!(
            log_line("POST", &request.url, Some(200), Duration::from_millis(42)),
            "POST oauth2.googleapis.com/token?… 200 42ms"
        );
        assert_eq!(
            log_line("GET", PROFILE, None, Duration::from_millis(7)),
            "GET gmail.googleapis.com/gmail/v1/users/me/profile ERR 7ms"
        );
    }

    #[test]
    fn at_most_four_in_flight() {
        let in_flight = Arc::new(InFlight::new(MAX_IN_FLIGHT));
        let peak = Arc::new(Mutex::new((0usize, 0usize)));
        let threads: Vec<_> = (0..12)
            .map(|_| {
                let (in_flight, peak) = (in_flight.clone(), peak.clone());
                std::thread::spawn(move || {
                    let _permit = in_flight.acquire();
                    {
                        let mut p = peak.lock().unwrap();
                        p.0 += 1;
                        p.1 = p.1.max(p.0);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                    peak.lock().unwrap().0 -= 1;
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let max = peak.lock().unwrap().1;
        assert!((1..=MAX_IN_FLIGHT).contains(&max));
    }

    #[test]
    fn fake_transport_scripts_and_records() {
        let fake = FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/history",
            &[("startHistoryId", "7")],
            &[(503, ""), (200, "{\"historyId\":\"9\"}")],
        );
        let request = Request::get(
            "https://gmail.googleapis.com/gmail/v1/users/me/history?startHistoryId=7",
            2,
        );
        assert_eq!(fake.send(&request).unwrap().status, 503);
        assert_eq!(fake.send(&request).unwrap().status, 200);
        assert_eq!(fake.send(&request).unwrap().status, 200);
        let other = Request::get("https://gmail.googleapis.com/gmail/v1/users/me/history", 2);
        assert_eq!(fake.send(&other).unwrap().status, 404);
        assert_eq!(fake.requests().len(), 4);
        assert_eq!(request.path(), "/gmail/v1/users/me/history");
    }
}

//! Gmail: the API types and calls (W3), and how Gmail's labels map onto
//! Pigeon's model (DAT-2).

use std::sync::{Arc, Mutex};

use chrono::{DateTime, FixedOffset, Local, TimeZone as _};
use serde::Deserialize;

use super::http::{self, Clock, Request, Response, SystemClock, Transport, TransportError};
use crate::data::{Body, Contact, DraftOrigin, Folder, Label, Loaded, Message, Thread};

pub const INBOX: &str = "INBOX";
pub const SPAM: &str = "SPAM";
pub const TRASH: &str = "TRASH";
pub const SENT: &str = "SENT";
pub const DRAFT: &str = "DRAFT";
pub const UNREAD: &str = "UNREAD";
pub const STARRED: &str = "STARRED";
pub const IMPORTANT: &str = "IMPORTANT";

/// Labels that decide a thread's folder.
pub const FOLDER_LABELS: [&str; 4] = [INBOX, SPAM, TRASH, SENT];

/// Gmail's own labels, which Pigeon shows as folders and flags rather than
/// as labels.
pub fn is_system_label(id: &str) -> bool {
    matches!(
        id,
        "INBOX" | "SPAM" | "TRASH" | "SENT" | "DRAFT" | "UNREAD" | "STARRED" | "IMPORTANT" | "CHAT"
    ) || id.starts_with("CATEGORY_")
        || id.ends_with("_STAR")
}

/// A thread's folder from its messages' labels (drafts left out), first
/// match wins: any TRASH, any SPAM, any INBOX, only SENT messages, else
/// Archive.
pub fn derive_folder(message_labels: &[Vec<String>]) -> Folder {
    let any = |label: &str| message_labels.iter().any(|m| m.iter().any(|l| l == label));
    if any(TRASH) {
        Folder::Trash
    } else if any(SPAM) {
        Folder::Spam
    } else if any(INBOX) {
        Folder::Inbox
    } else if !message_labels.is_empty()
        && message_labels.iter().all(|m| m.iter().any(|l| l == SENT))
    {
        Folder::Sent
    } else {
        Folder::Archive
    }
}

/// A thread's labels: the union over its messages, in first-seen order.
pub fn union_labels<'a>(message_labels: impl IntoIterator<Item = &'a Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for label in message_labels.into_iter().flatten() {
        if !out.contains(label) {
            out.push(label.clone());
        }
    }
    out
}

// Reads (W3): https://developers.google.com/gmail/api/reference/rest

const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const BATCH_ENDPOINT: &str = "https://gmail.googleapis.com/batch/gmail/v1";

/// The headers a list row and the reader need (2.6, SYN-1 step 4).
const METADATA_HEADERS: [&str; 10] = [
    "From",
    "To",
    "Cc",
    "Bcc",
    "Reply-To",
    "Subject",
    "Date",
    "Message-ID",
    "In-Reply-To",
    "References",
];

/// Quota units per call (2.6).
pub mod cost {
    pub const PROFILE: u32 = 1;
    pub const LABELS_LIST: u32 = 1;
    pub const LABELS_GET: u32 = 1;
    pub const SEND_AS_LIST: u32 = 1;
    pub const THREADS_LIST: u32 = 10;
    pub const THREADS_GET: u32 = 40;
    pub const MESSAGES_GET: u32 = 20;
    pub const MESSAGES_GET_RAW: u32 = 60;
    pub const HISTORY_LIST: u32 = 2;
    pub const DRAFTS_LIST: u32 = 5;
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub email_address: String,
    #[serde(deserialize_with = "number")]
    pub history_id: u64,
}

#[derive(Debug, Default, Deserialize)]
pub struct LabelList {
    #[serde(default)]
    pub labels: Vec<GLabel>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GLabel {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label_list_visibility: Option<String>,
    #[serde(default)]
    pub threads_total: Option<u64>,
    #[serde(default)]
    pub threads_unread: Option<u64>,
    #[serde(default)]
    pub color: Option<LabelColor>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelColor {
    #[serde(default)]
    pub background_color: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadList {
    #[serde(default)]
    pub threads: Vec<ThreadRef>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRef {
    pub id: String,
    #[serde(default, deserialize_with = "opt_number")]
    pub history_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GThread {
    pub id: String,
    #[serde(default, deserialize_with = "opt_number")]
    pub history_id: Option<u64>,
    #[serde(default)]
    pub messages: Vec<GMessage>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GMessage {
    pub id: String,
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub snippet: String,
    #[serde(default)]
    pub payload: Option<Payload>,
    #[serde(default)]
    pub size_estimate: u64,
    #[serde(default, deserialize_with = "opt_number")]
    pub internal_date: Option<u64>,
    /// `format=raw` only: the whole message, base64url.
    #[serde(default)]
    pub raw: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub headers: Vec<Header>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryList {
    #[serde(default)]
    pub history: Vec<History>,
    #[serde(default)]
    pub next_page_token: Option<String>,
    #[serde(default, deserialize_with = "opt_number")]
    pub history_id: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    #[serde(default)]
    pub messages: Vec<GMessage>,
    #[serde(default)]
    pub messages_added: Vec<HistoryMessage>,
    #[serde(default)]
    pub messages_deleted: Vec<HistoryMessage>,
    #[serde(default)]
    pub labels_added: Vec<HistoryMessage>,
    #[serde(default)]
    pub labels_removed: Vec<HistoryMessage>,
}

#[derive(Debug, Default, Deserialize)]
pub struct HistoryMessage {
    pub message: GMessage,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftList {
    #[serde(default)]
    pub drafts: Vec<DraftRef>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DraftRef {
    pub id: String,
    pub message: GMessage,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendAsList {
    #[serde(default)]
    pub send_as: Vec<SendAs>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendAs {
    pub send_as_email: String,
}

/// Google sends 64-bit numbers as strings.
fn number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    opt_number(d)?.ok_or_else(|| serde::de::Error::custom("missing number"))
}

fn opt_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum N {
        S(String),
        N(u64),
    }
    Ok(match Option::<N>::deserialize(d)? {
        Some(N::S(s)) => Some(s.parse().map_err(serde::de::Error::custom)?),
        Some(N::N(n)) => Some(n),
        None => None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GmailError {
    /// No connection: worth retrying later.
    Network(String),
    /// Gmail answered with an error status.
    Status { status: u16, message: String },
    /// An answer that didn't parse.
    Parse(String),
}

impl GmailError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, GmailError::Status { status: 404, .. })
    }

    pub fn is_network(&self) -> bool {
        matches!(self, GmailError::Network(_))
    }
}

impl std::fmt::Display for GmailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GmailError::Network(_) => write!(f, "Couldn't reach Gmail"),
            GmailError::Status { status, message } if message.is_empty() => {
                write!(f, "Gmail answered {status}")
            }
            GmailError::Status { message, .. } => write!(f, "Gmail: {message}"),
            GmailError::Parse(_) => write!(f, "Gmail's answer didn't make sense"),
        }
    }
}

impl std::error::Error for GmailError {}

impl From<TransportError> for GmailError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::AuthExpired => GmailError::Status {
                status: 401,
                message: String::new(),
            },
            TransportError::Network(_) => GmailError::Network(e.to_string()),
        }
    }
}

/// The response's JSON, or the error it carries.
fn parse<T: serde::de::DeserializeOwned>(response: &Response) -> Result<T, GmailError> {
    if !response.is_success() {
        let message = response
            .json::<serde_json::Value>()
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or_default();
        return Err(GmailError::Status {
            status: response.status,
            message,
        });
    }
    response
        .json()
        .map_err(|e| GmailError::Parse(e.to_string()))
}

/// Percent-encodes a query value.
fn enc(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// The requests, each with its quota cost.

pub fn profile_request() -> Request {
    Request::get(format!("{API}/profile"), cost::PROFILE)
}

pub fn labels_list_request() -> Request {
    Request::get(format!("{API}/labels"), cost::LABELS_LIST)
}

pub fn label_get_request(id: &str) -> Request {
    Request::get(format!("{API}/labels/{}", enc(id)), cost::LABELS_GET)
}

pub fn send_as_request() -> Request {
    Request::get(format!("{API}/settings/sendAs"), cost::SEND_AS_LIST)
}

/// A page of threads: in `label` (or all but spam and trash), matching `q`.
pub fn threads_list_request(
    label: Option<&str>,
    q: Option<&str>,
    max: usize,
    page: Option<&str>,
) -> Request {
    let mut url = format!("{API}/threads?maxResults={max}");
    if let Some(label) = label {
        url.push_str(&format!("&labelIds={}", enc(label)));
    }
    if let Some(q) = q {
        url.push_str(&format!("&q={}", enc(q)));
    }
    if let Some(page) = page {
        url.push_str(&format!("&pageToken={}", enc(page)));
    }
    Request::get(url, cost::THREADS_LIST)
}

fn metadata_query() -> String {
    let mut q = "format=metadata".to_string();
    for h in METADATA_HEADERS {
        q.push_str(&format!("&metadataHeaders={h}"));
    }
    q
}

pub fn thread_get_request(id: &str) -> Request {
    Request::get(
        format!("{API}/threads/{}?{}", enc(id), metadata_query()),
        cost::THREADS_GET,
    )
}

pub fn message_metadata_request(id: &str) -> Request {
    Request::get(
        format!("{API}/messages/{}?{}", enc(id), metadata_query()),
        cost::MESSAGES_GET,
    )
}

pub fn message_raw_request(id: &str) -> Request {
    Request::get(
        format!("{API}/messages/{}?format=raw", enc(id)),
        cost::MESSAGES_GET_RAW,
    )
}

pub fn history_request(start: u64, page: Option<&str>) -> Request {
    let mut url = format!("{API}/history?startHistoryId={start}&maxResults=500");
    if let Some(page) = page {
        url.push_str(&format!("&pageToken={}", enc(page)));
    }
    Request::get(url, cost::HISTORY_LIST)
}

pub fn drafts_list_request(page: Option<&str>) -> Request {
    let mut url = format!("{API}/drafts?maxResults=100");
    if let Some(page) = page {
        url.push_str(&format!("&pageToken={}", enc(page)));
    }
    Request::get(url, cost::DRAFTS_LIST)
}

/// Calls per HTTP batch. Gmail runs a batch's calls in parallel and caps
/// how many run at once per user: at 50 `threads.get` (Google's suggested
/// ceiling) it answers some with 429 "Too many concurrent requests for
/// user". 20 stays under it, and costs no speed: 20 `threads.get` are 800
/// units, so sync is bound by the quota (5,000 a minute) anyway.
pub const BATCH_SIZE: usize = 20;

/// Gmail's calls over a transport. Blocking; called off the main thread.
#[derive(Clone)]
pub struct Gmail {
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    /// One batch in flight at a time, shared by every clone (sync and
    /// bodies), so their calls don't add up to too many at once.
    batching: Arc<Mutex<()>>,
}

impl Gmail {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self::with_clock(transport, Arc::new(SystemClock))
    }

    pub fn with_clock(transport: Arc<dyn Transport>, clock: Arc<dyn Clock>) -> Self {
        Self {
            transport,
            clock,
            batching: Arc::default(),
        }
    }

    pub fn call<T: serde::de::DeserializeOwned>(&self, request: &Request) -> Result<T, GmailError> {
        parse(&self.transport.send(request)?)
    }

    /// Each request's answer, in order, sent in HTTP batches of at most
    /// `BATCH_SIZE`, one after another. Calls answered with a rate limit or
    /// a server error (429, 5xx, rate-limit 403) go again after the
    /// transport's backoff, up to `MAX_ATTEMPTS` tries; the others are
    /// answered as they came.
    pub fn batch<T: serde::de::DeserializeOwned>(
        &self,
        requests: &[Request],
    ) -> Vec<Result<T, GmailError>> {
        let mut out: Vec<Option<Result<T, GmailError>>> = std::iter::repeat_with(|| None)
            .take(requests.len())
            .collect();
        let mut pending: Vec<usize> = (0..requests.len()).collect();
        let mut attempt = 0;
        while !pending.is_empty() {
            let last = attempt + 1 >= http::MAX_ATTEMPTS;
            let mut retry = Vec::new();
            for chunk in pending.chunks(BATCH_SIZE) {
                let calls: Vec<Request> = chunk.iter().map(|&ix| requests[ix].clone()).collect();
                for (&ix, part) in chunk.iter().zip(self.send_batch(&calls)) {
                    match part {
                        Ok(r) if !last && http::is_retryable(&r) => retry.push(ix),
                        Ok(r) => out[ix] = Some(parse(&r)),
                        Err(e) => out[ix] = Some(Err(e)),
                    }
                }
            }
            if !retry.is_empty() {
                let wait = http::backoff(attempt, self.clock.jitter());
                eprintln!(
                    "Pigeon: Gmail batch: {} of {} calls rate-limited; retrying in {:.1}s",
                    retry.len(),
                    pending.len(),
                    wait.as_secs_f32()
                );
                self.clock.sleep(wait);
            }
            pending = retry;
            attempt += 1;
        }
        out.into_iter()
            .map(|r| r.expect("every call answered"))
            .collect()
    }

    /// One HTTP batch: each call's response, or why there's none.
    fn send_batch(&self, calls: &[Request]) -> Vec<Result<Response, GmailError>> {
        let batch = http::encode_batch(BATCH_ENDPOINT, calls, "batch_pigeon");
        let sent = {
            let _one = self.batching.lock().unwrap_or_else(|e| e.into_inner());
            self.transport.send(&batch)
        };
        let response = match sent {
            Ok(r) if r.is_success() => r,
            Ok(r) => {
                let e = parse::<serde_json::Value>(&r)
                    .err()
                    .unwrap_or(GmailError::Status {
                        status: r.status,
                        message: String::new(),
                    });
                return calls.iter().map(|_| Err(e.clone())).collect();
            }
            Err(e) => {
                let e = GmailError::from(e);
                return calls.iter().map(|_| Err(e.clone())).collect();
            }
        };
        match http::decode_batch(&response, calls.len()) {
            Ok(parts) => parts
                .into_iter()
                .map(|part| part.ok_or_else(|| GmailError::Parse("missing from the batch".into())))
                .collect(),
            Err(e) => calls
                .iter()
                .map(|_| Err(GmailError::Parse(e.clone())))
                .collect(),
        }
    }
}

// Gmail → model.

/// A label for the sidebar, from Gmail's. Labels without a colour get
/// Gmail's own grey.
pub fn label_from_gmail(label: &GLabel) -> Label {
    let color = label
        .color
        .as_ref()
        .and_then(|c| c.background_color.as_deref())
        .and_then(crate::data::from_hex)
        .or_else(|| crate::data::from_hex("#a4a9ad"))
        .expect("a valid colour");
    Label {
        id: label.id.clone(),
        name: label.name.clone(),
        color,
    }
}

/// Whether the sidebar lists it: user labels the user hasn't hidden.
pub fn shows_label(label: &GLabel) -> bool {
    label.kind == "user" && label.label_list_visibility.as_deref() != Some("labelHide")
}

/// The headers a message's metadata carries, parsed like a real header
/// block, so encoded words and address lists come out right.
struct Headers {
    from: Contact,
    to: Vec<Contact>,
    cc: Vec<Contact>,
    bcc: Vec<Contact>,
    reply_to: Vec<Contact>,
    subject: String,
    date: Option<DateTime<FixedOffset>>,
    message_id: String,
    in_reply_to: String,
    references: Vec<String>,
}

fn contacts(list: Option<mail_parser::AddressList<'_>>) -> Vec<Contact> {
    list.map(|l| {
        l.mailboxes()
            .filter_map(|m| {
                Some(Contact {
                    name: m.name().unwrap_or_default().trim_matches('"').to_string(),
                    email: m.address()?.to_string(),
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

fn headers(payload: Option<&Payload>) -> Headers {
    let mut block = String::new();
    for h in payload.map(|p| p.headers.as_slice()).unwrap_or_default() {
        // One line each; Gmail unfolds them already.
        let value = h.value.replace(['\r', '\n'], " ");
        block.push_str(&format!("{}: {}\r\n", h.name, value));
    }
    block.push_str("\r\n");
    let parsed = mail_parser::MessageParser::default().parse(block.as_bytes());
    let message = parsed.unwrap_or_default();
    Headers {
        from: contacts(message.from())
            .into_iter()
            .next()
            .unwrap_or_default(),
        to: contacts(message.to()),
        cc: contacts(message.cc()),
        bcc: contacts(message.bcc()),
        reply_to: contacts(message.reply_to()),
        subject: message.subject().unwrap_or_default().trim().to_string(),
        date: message
            .date()
            .filter(|d| d.is_valid())
            .and_then(|d| DateTime::parse_from_rfc3339(&d.to_rfc3339()).ok()),
        message_id: message.message_id().unwrap_or_default().to_string(),
        in_reply_to: message
            .in_reply_to()
            .and_then(|l| l.first().map(String::from))
            .unwrap_or_default(),
        references: message
            .references()
            .map(|l| l.iter().map(String::from).collect())
            .unwrap_or_default(),
    }
}

/// Gmail's snippets come HTML-escaped ("it&#39;s").
fn unescape(snippet: &str) -> String {
    if !snippet.contains('&') {
        return snippet.to_string();
    }
    crate::html::plain_text(&crate::html::parse(snippet))
        .trim()
        .to_string()
}

/// A message from its metadata; the body comes later (RDR-6).
pub fn message_from_gmail(m: &GMessage) -> (Message, String) {
    let h = headers(m.payload.as_ref());
    let internal = m.internal_date.unwrap_or_default() as i64;
    let date = h.date.unwrap_or_else(|| {
        Local
            .timestamp_millis_opt(internal)
            .single()
            .unwrap_or_else(Local::now)
            .fixed_offset()
    });
    let message = Message {
        id: m.id.clone(),
        label_ids: m.label_ids.clone(),
        from: h.from,
        to: h.to,
        cc: h.cc,
        bcc: h.bcc,
        date,
        internal_date: if internal > 0 {
            internal
        } else {
            date.timestamp_millis()
        },
        snippet: unescape(&m.snippet),
        body: Body::NotLoaded,
        attachments: Vec::new(),
        invite: None,
        message_id: h.message_id,
        in_reply_to: h.in_reply_to,
        references: h.references,
        reply_to: h.reply_to,
        size_estimate: m.size_estimate,
    };
    (message, h.subject)
}

/// A conversation from `threads.get format=metadata`, drafts left out
/// (they're their own Drafts threads). `None` when only drafts are left.
pub fn thread_from_gmail(t: &GThread) -> Option<Thread> {
    let mut parsed: Vec<(Message, String)> = t
        .messages
        .iter()
        .filter(|m| !m.label_ids.iter().any(|l| l == DRAFT))
        .map(message_from_gmail)
        .collect();
    if parsed.is_empty() {
        return None;
    }
    parsed.sort_by_key(|(m, _)| m.internal_date);
    // The first message's subject, like Gmail.
    let subject = parsed
        .iter()
        .map(|(_, s)| s.clone())
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    let messages: Vec<Message> = parsed.into_iter().map(|(m, _)| m).collect();
    let message_labels: Vec<Vec<String>> = messages.iter().map(|m| m.label_ids.clone()).collect();
    let label_ids = union_labels(&message_labels);
    let has = |label: &str| label_ids.iter().any(|l| l == label);
    let mut thread = Thread {
        id: t.id.clone(),
        subject,
        folder: derive_folder(&message_labels),
        labels: label_ids
            .iter()
            .filter(|l| !is_system_label(l))
            .cloned()
            .collect(),
        starred: has(STARRED),
        unread: has(UNREAD),
        messages,
        history_id: t.history_id,
        loaded: Loaded::Metadata,
        search: Default::default(),
        origin: None,
        label_ids,
    };
    thread.reindex();
    Some(thread)
}

/// The Drafts thread for a Gmail draft (DRF-2). Its id is `draft-<draft
/// id>`; one written here keeps its `draft-local-*` id.
pub fn draft_thread(draft_id: &str, m: &GMessage) -> Thread {
    let (message, subject) = message_from_gmail(m);
    // A reply draft sits in the thread it answers, whose id isn't its own.
    let origin = (!m.thread_id.is_empty() && m.thread_id != m.id).then(|| DraftOrigin {
        kind: crate::app::ComposeKind::Reply,
        markdown: String::new(),
        reply_to: Some(m.thread_id.clone()),
        quoted: None,
    });
    let mut thread = Thread::local(
        draft_thread_id(draft_id),
        subject,
        Folder::Drafts,
        vec![message],
        origin,
    );
    thread.label_ids = m.label_ids.clone();
    thread.loaded = Loaded::Metadata;
    thread
}

pub fn draft_thread_id(draft_id: &str) -> String {
    format!("draft-{draft_id}")
}

/// Whether the Drafts thread id came from a Gmail draft, not from here.
pub fn is_server_draft(thread_id: &str) -> bool {
    thread_id.starts_with("draft-") && !thread_id.starts_with("draft-local-")
}

/// The threads a page of history touched, once each, in order.
pub fn touched_threads(history: &[History]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in history {
        let ids = h
            .messages
            .iter()
            .chain(h.messages_added.iter().map(|m| &m.message))
            .chain(h.messages_deleted.iter().map(|m| &m.message))
            .chain(h.labels_added.iter().map(|m| &m.message))
            .chain(h.labels_removed.iter().map(|m| &m.message))
            .map(|m| &m.thread_id);
        for id in ids {
            if !id.is_empty() && !out.contains(id) {
                out.push(id.clone());
            }
        }
    }
    out
}

/// Whether the history touched a draft, so the Drafts list is re-read.
pub fn touches_drafts(history: &[History]) -> bool {
    history.iter().any(|h| {
        h.messages_added
            .iter()
            .chain(&h.messages_deleted)
            .chain(&h.labels_added)
            .chain(&h.labels_removed)
            .any(|m| m.message.label_ids.iter().any(|l| l == DRAFT))
            || !h.messages_deleted.is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::http::FakeTransport;

    fn labels(messages: &[&[&str]]) -> Vec<Vec<String>> {
        messages
            .iter()
            .map(|m| m.iter().map(|l| l.to_string()).collect())
            .collect()
    }

    #[test]
    fn derive_folder_table() {
        let cases: &[(&[&[&str]], Folder)] = &[
            (&[&["INBOX", "TRASH"]], Folder::Trash),
            (&[&["SPAM"], &["INBOX"]], Folder::Spam),
            (&[&["TRASH"], &["SPAM"]], Folder::Trash),
            (&[&["SENT"], &["INBOX", "UNREAD"]], Folder::Inbox),
            (&[&["SENT"], &["SENT", "STARRED"]], Folder::Sent),
            (&[&["SENT"], &["work"]], Folder::Archive),
            (&[&["work", "UNREAD"]], Folder::Archive),
            (&[], Folder::Archive),
        ];
        for (messages, folder) in cases {
            assert_eq!(derive_folder(&labels(messages)), *folder, "{messages:?}");
        }
    }

    #[test]
    fn system_labels() {
        assert!(is_system_label("INBOX"));
        assert!(is_system_label("CATEGORY_PROMOTIONS"));
        assert!(is_system_label("YELLOW_STAR"));
        assert!(!is_system_label("Label_12"));
        assert!(!is_system_label("work"));
    }

    #[test]
    fn metadata_thread_maps_to_thread() {
        let t: GThread = serde_json::from_str(include_str!(
            "../../tests/fixtures/google/gmail/thread_metadata.json"
        ))
        .unwrap();
        let thread = thread_from_gmail(&t).unwrap();
        assert_eq!(thread.id, "thread-a1");
        assert_eq!(thread.history_id, Some(1052));
        assert_eq!(thread.subject, "Lunch on Friday?");
        // The draft reply stays out of the conversation.
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.folder, Folder::Inbox);
        assert!(thread.unread && thread.starred);
        assert_eq!(thread.labels, ["Label_7"]);
        assert_eq!(thread.loaded, Loaded::Metadata);
        let first = &thread.messages[0];
        assert_eq!(first.from.name, "Ana Lima");
        assert_eq!(first.from.email, "ana@example.com");
        assert_eq!(first.to.len(), 2);
        assert_eq!(first.to[1].name, "Bo Chen");
        assert_eq!(first.snippet, "Are you free on Friday? It's my treat");
        assert_eq!(first.message_id, "m1@example.com");
        assert_eq!(first.internal_date, 1793000000000);
        assert_eq!(first.date.to_rfc3339(), "2026-10-26T09:13:20-07:00");
        assert!(matches!(first.body, Body::NotLoaded));
        let second = &thread.messages[1];
        assert_eq!(second.from.email, "me@example.com");
        assert_eq!(second.in_reply_to, "m1@example.com");
        assert_eq!(second.references, ["m1@example.com"]);
        assert_eq!(second.cc[0].email, "cc@example.com");
        // The encoded word in the subject header is decoded.
        assert!(thread.search.all.contains("lunch on friday?"));
    }

    #[test]
    fn only_drafts_is_no_conversation() {
        let t = GThread {
            id: "t".into(),
            history_id: None,
            messages: vec![GMessage {
                id: "m".into(),
                label_ids: vec![DRAFT.into()],
                ..Default::default()
            }],
        };
        assert!(thread_from_gmail(&t).is_none());
    }

    #[test]
    fn history_gives_touched_threads() {
        let h: HistoryList = serde_json::from_str(include_str!(
            "../../tests/fixtures/google/gmail/history.json"
        ))
        .unwrap();
        assert_eq!(h.history_id, Some(2000));
        assert_eq!(
            touched_threads(&h.history),
            ["thread-a1", "thread-b2", "thread-c3", "thread-d4"]
        );
        assert!(touches_drafts(&h.history));
    }

    #[test]
    fn batch_returns_each_answer_in_order() {
        let fixture = include_str!("../../tests/fixtures/google/gmail/batch_response.txt");
        let fake = FakeTransport::new().on("POST", "/batch/gmail/v1", &[], &[(200, fixture)]);
        // The fixture's boundary.
        struct Boundary(FakeTransport);
        impl Transport for Boundary {
            fn send(&self, r: &Request) -> Result<Response, TransportError> {
                let mut response = self.0.send(r)?;
                response.headers = vec![(
                    "Content-Type".into(),
                    "multipart/mixed; boundary=batch_fixture".into(),
                )];
                Ok(response)
            }
        }
        let gmail = Gmail::new(Arc::new(Boundary(fake)));
        let requests: Vec<Request> = ["thread-a1", "gone", "missing"]
            .iter()
            .map(|id| thread_get_request(id))
            .collect();
        let results = gmail.batch::<serde_json::Value>(&requests);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].as_ref().unwrap()["id"], "thread-a1");
        assert!(results[1].as_ref().unwrap_err().is_not_found());
        assert!(results[2].is_err());
    }

    /// Answers batches with boundary `b`.
    struct Batches(Arc<FakeTransport>);
    impl Transport for Batches {
        fn send(&self, r: &Request) -> Result<Response, TransportError> {
            let mut response = self.0.send(r)?;
            response.headers = vec![("Content-Type".into(), "multipart/mixed; boundary=b".into())];
            Ok(response)
        }
    }

    fn batch_body(parts: &[(u16, &str)]) -> String {
        let mut out = String::new();
        for (ix, (status, body)) in parts.iter().enumerate() {
            out.push_str(&format!(
                "--b\r\nContent-ID: <response-item-{}>\r\n\r\nHTTP/1.1 {status} X\r\n\r\n{body}\r\n",
                ix + 1
            ));
        }
        out + "--b--\r\n"
    }

    /// The paths a batch request carries, in order.
    fn batch_paths(request: &Request) -> Vec<String> {
        String::from_utf8_lossy(&request.body)
            .lines()
            .filter_map(|l| l.strip_prefix("GET "))
            .map(|l| l.split(['?', ' ']).next().unwrap().to_string())
            .collect()
    }

    #[test]
    fn rate_limited_calls_in_a_batch_retry_alone() {
        let busy = r#"{"error":{"code":429,"message":"Too many concurrent requests for user."}}"#;
        let limited = r#"{"error":{"code":403,"errors":[{"reason":"rateLimitExceeded"}]}}"#;
        let first = batch_body(&[
            (200, r#"{"id":"a"}"#),
            (429, busy),
            (404, r#"{"error":{"code":404}}"#),
            (403, limited),
        ]);
        let second = batch_body(&[(200, r#"{"id":"b"}"#), (503, "{}")]);
        let third = batch_body(&[(200, r#"{"id":"d"}"#)]);
        let fake = Arc::new(FakeTransport::new().on(
            "POST",
            "/batch/gmail/v1",
            &[],
            &[(200, &first), (200, &second), (200, &third)],
        ));
        let clock = Arc::new(http::FakeClock::new());
        let gmail = Gmail::with_clock(Arc::new(Batches(fake.clone())), clock.clone());
        let requests: Vec<Request> = ["a", "b", "c", "d"]
            .iter()
            .map(|id| thread_get_request(id))
            .collect();
        let results = gmail.batch::<serde_json::Value>(&requests);
        assert_eq!(results[0].as_ref().unwrap()["id"], "a");
        assert_eq!(results[1].as_ref().unwrap()["id"], "b");
        assert!(results[2].as_ref().unwrap_err().is_not_found());
        assert_eq!(results[3].as_ref().unwrap()["id"], "d");
        let sent: Vec<Vec<String>> = fake.requests().iter().map(batch_paths).collect();
        let path = |id: &str| format!("/gmail/v1/users/me/threads/{id}");
        assert_eq!(
            sent,
            [
                vec![path("a"), path("b"), path("c"), path("d")],
                vec![path("b"), path("d")],
                vec![path("d")],
            ]
        );
        let j = std::time::Duration::from_millis(250);
        assert_eq!(
            clock.sleeps(),
            [
                std::time::Duration::from_secs(1) + j,
                std::time::Duration::from_secs(2) + j
            ]
        );
    }

    #[test]
    fn batches_hold_at_most_batch_size_calls() {
        let fake = Arc::new(FakeTransport::new().on("POST", "/batch/gmail/v1", &[], &[(200, "")]));
        let gmail = Gmail::with_clock(
            Arc::new(Batches(fake.clone())),
            Arc::new(http::FakeClock::new()),
        );
        let requests: Vec<Request> = (0..45)
            .map(|i| thread_get_request(&format!("t{i}")))
            .collect();
        let results = gmail.batch::<serde_json::Value>(&requests);
        assert_eq!(results.len(), 45);
        let sizes: Vec<usize> = fake
            .requests()
            .iter()
            .map(|r| batch_paths(r).len())
            .collect();
        assert_eq!(sizes, [BATCH_SIZE, BATCH_SIZE, 5]);
    }

    #[test]
    fn requests_declare_their_cost() {
        assert_eq!(thread_get_request("t").cost, 40);
        assert_eq!(message_raw_request("m").cost, 60);
        assert_eq!(history_request(5, None).cost, 2);
        let list = threads_list_request(Some("INBOX"), Some("newer_than:90d"), 100, None);
        assert_eq!(list.cost, 10);
        assert!(list.url.contains("labelIds=INBOX"));
        assert!(list.url.contains("q=newer_than%3A90d"));
        assert!(
            thread_get_request("t")
                .url
                .contains("metadataHeaders=Message-ID")
        );
    }
}

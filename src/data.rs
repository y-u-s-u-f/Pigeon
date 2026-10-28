//! Mailbox model. The backend builds it from the store (`backend::store`);
//! demo mode seeds the store from the bundled placeholder data, which these
//! types also deserialize from.

use chrono::{DateTime, Datelike, FixedOffset, Local};
use gpui::{Hsla, Rgba};
use serde::{Deserialize, Deserializer, Serialize};

use crate::app::ComposeKind;

#[derive(Debug, Deserialize)]
pub struct MailData {
    pub account: Account,
    pub labels: Vec<Label>,
    pub threads: Vec<Thread>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Account {
    pub name: String,
    pub email: String,
    /// Other addresses the user sends as (Gmail's `sendAs`), for `is_me`.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// W3 (MST-1): every address the user sends as, the main one too, as
    /// `sendAs.list` gives them. Empty until the first sync.
    #[serde(default)]
    pub send_as: Vec<SendAs>,
}

/// W3 (MST-1, MST-2): an address the user sends as.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendAs {
    pub email: String,
    /// Gmail's `displayName`; empty means the account's name.
    pub name: String,
    pub reply_to: String,
    /// HTML, as Gmail keeps it; edited in Gmail's settings.
    pub signature: String,
    pub is_default: bool,
    pub is_primary: bool,
    /// Not verified yet, so it can't send.
    pub pending: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Label {
    pub id: String,
    /// The whole name; `/` nests it under the label named before it
    /// (FOL-9).
    pub name: String,
    #[serde(deserialize_with = "hex_color")]
    pub color: Hsla,
    /// W4b (FOL-10): Gmail's `labelListVisibility`, for the sidebar.
    #[serde(default)]
    pub list_visibility: ListVisibility,
    /// Gmail's `messageListVisibility`: whether rows show it.
    #[serde(default = "yes")]
    pub in_message_list: bool,
}

fn yes() -> bool {
    true
}

impl Label {
    pub fn new(id: impl Into<String>, name: impl Into<String>, color: Hsla) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            color,
            list_visibility: ListVisibility::Show,
            in_message_list: true,
        }
    }
}

/// Where the sidebar lists a label: Gmail's `labelListVisibility`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListVisibility {
    #[default]
    Show,
    ShowIfUnread,
    Hide,
}

impl ListVisibility {
    pub const ALL: [ListVisibility; 3] = [
        ListVisibility::Show,
        ListVisibility::ShowIfUnread,
        ListVisibility::Hide,
    ];

    pub fn gmail(self) -> &'static str {
        match self {
            ListVisibility::Show => "labelShow",
            ListVisibility::ShowIfUnread => "labelShowIfUnread",
            ListVisibility::Hide => "labelHide",
        }
    }

    pub fn from_gmail(value: Option<&str>) -> Self {
        match value {
            Some("labelShowIfUnread") => ListVisibility::ShowIfUnread,
            Some("labelHide") => ListVisibility::Hide,
            _ => ListVisibility::Show,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ListVisibility::Show => "Show",
            ListVisibility::ShowIfUnread => "Show if unread",
            ListVisibility::Hide => "Hide",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Folder {
    Inbox,
    Archive,
    Sent,
    Drafts,
    Spam,
    Trash,
}

impl Folder {
    /// Every folder, in sidebar order.
    pub const ALL: [Folder; 6] = [
        Folder::Inbox,
        Folder::Archive,
        Folder::Sent,
        Folder::Drafts,
        Folder::Spam,
        Folder::Trash,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Folder::Inbox => "Inbox",
            Folder::Archive => "Archive",
            Folder::Sent => "Sent",
            Folder::Drafts => "Drafts",
            Folder::Spam => "Spam",
            Folder::Trash => "Trash",
        }
    }

    /// The folder's name in the store.
    pub fn key(self) -> &'static str {
        match self {
            Folder::Inbox => "inbox",
            Folder::Archive => "archive",
            Folder::Sent => "sent",
            Folder::Drafts => "drafts",
            Folder::Spam => "spam",
            Folder::Trash => "trash",
        }
    }

    pub fn from_key(key: &str) -> Option<Folder> {
        Folder::ALL.into_iter().find(|f| f.key() == key)
    }

    /// Where archive moves a thread in this folder, or `None` when archive
    /// does nothing there.
    pub fn archive_target(self) -> Option<Folder> {
        match self {
            Folder::Inbox => Some(Folder::Archive),
            Folder::Archive | Folder::Spam | Folder::Trash => Some(Folder::Inbox),
            Folder::Sent | Folder::Drafts => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Thread {
    /// Gmail's `threadId`; `draft-local-<uuid>` or `sent-local-<uuid>` until
    /// the server answers with one (`BackendEvent::IdReplaced`).
    pub id: String,
    pub subject: String,
    pub folder: Folder,
    /// User labels, by id.
    pub labels: Vec<String>,
    pub starred: bool,
    pub unread: bool,
    /// W4b (FOL-12): Gmail's IMPORTANT label.
    #[serde(default)]
    pub important: bool,
    /// Oldest first.
    pub messages: Vec<Message>,
    /// Every Gmail label on the thread as last stored, system ones too (the
    /// union over its messages). `folder`, `unread`, `starred` and `labels`
    /// come from it when the thread is loaded; the UI changes those directly
    /// and leaves this as it was.
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub history_id: Option<u64>,
    #[serde(skip)]
    pub loaded: Loaded,
    #[serde(skip)]
    pub search: SearchIndex,
    /// For a draft: how it was started, so reopening restores it.
    #[serde(skip)]
    pub origin: Option<DraftOrigin>,
}

/// How much of a thread the store has: headers only, or bodies too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Loaded {
    Metadata,
    #[default]
    Full,
}

/// How a draft was started. Kept apart from its body, which holds only what
/// the user wrote.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DraftOrigin {
    pub kind: ComposeKind,
    /// The body as typed, in markdown; the message holds it as HTML.
    pub markdown: String,
    /// The thread a reply answers, by id.
    pub reply_to: Option<String>,
    pub quoted: Option<Quote>,
    /// W3 (MST-2): it carries the From alias's signature, kept apart from
    /// the markdown.
    #[serde(default)]
    pub signature: bool,
}

/// A forwarded message: its header lines and HTML body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub header: Vec<(String, String)>,
    pub html: String,
}

/// Lowercased text of a thread, built once per change rather than per keystroke.
#[derive(Debug, Default)]
pub struct SearchIndex {
    /// Subject, people, snippets and bodies.
    pub all: String,
    pub from: String,
    /// To, cc and bcc.
    pub to: String,
    pub subject: String,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    /// Gmail's message id; `local-<uuid>` for one written here.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
    pub from: Contact,
    pub to: Vec<Contact>,
    #[serde(default)]
    pub cc: Vec<Contact>,
    #[serde(default)]
    pub bcc: Vec<Contact>,
    /// The Date header, for showing.
    pub date: DateTime<FixedOffset>,
    /// Gmail's `internalDate` in Unix milliseconds: the sort key.
    #[serde(default)]
    pub internal_date: i64,
    pub snippet: String,
    #[serde(rename = "html", deserialize_with = "loaded_body")]
    pub body: Body,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    // Threading headers (DAT-4).
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub in_reply_to: String,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub reply_to: Vec<Contact>,
    #[serde(default)]
    pub size_estimate: u64,
}

/// A message's body: fetched on demand (RDR-6).
#[allow(dead_code)] // Loading and Failed come with W3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    NotLoaded,
    Loading,
    Loaded { html: String },
    Failed(String),
}

fn loaded_body<'de, D: Deserializer<'de>>(d: D) -> Result<Body, D::Error> {
    Ok(Body::Loaded {
        html: String::deserialize(d)?,
    })
}

impl Body {
    /// The HTML, or nothing until it's loaded.
    pub fn html(&self) -> &str {
        match self {
            Body::Loaded { html } => html,
            _ => "",
        }
    }
}

impl Message {
    /// An empty message from `from`, dated now, with a local id.
    pub fn new(from: Contact) -> Self {
        let now = Local::now();
        Self {
            id: format!("local-{}", uuid::Uuid::new_v4().simple()),
            label_ids: Vec::new(),
            from,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            date: now.fixed_offset(),
            internal_date: now.timestamp_millis(),
            snippet: String::new(),
            body: Body::Loaded {
                html: String::new(),
            },
            attachments: Vec::new(),
            message_id: String::new(),
            in_reply_to: String::new(),
            references: Vec::new(),
            reply_to: Vec::new(),
            size_estimate: 0,
        }
    }

    pub fn html(&self) -> &str {
        self.body.html()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub name: String,
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub size_bytes: u64,
    pub mime: String,
    /// Gmail's ids for fetching it; empty for a file picked here.
    #[serde(default)]
    pub attachment_id: String,
    #[serde(default)]
    pub part_id: String,
    /// The file on disk, for one picked here: read when it's sent (ATT-6).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// The Gmail message whose `part_id` it is, when that isn't the message
    /// showing it: a forward's copy of the original's file (ATT-7).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message_id: String,
}

/// What an attachment is, for its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    Image,
    Archive,
    Document,
    Spreadsheet,
    Other,
}

impl Attachment {
    pub fn kind(&self) -> FileKind {
        let mime = self.mime.to_ascii_lowercase();
        let ext = self
            .name
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
            .unwrap_or_default();
        if mime == "application/pdf" || ext == "pdf" {
            FileKind::Pdf
        } else if mime.starts_with("image/") {
            FileKind::Image
        } else if mime.contains("zip")
            || mime.contains("compressed")
            || matches!(ext.as_str(), "zip" | "gz" | "tgz" | "rar" | "7z" | "tar")
        {
            FileKind::Archive
        } else if mime.contains("spreadsheet")
            || mime.contains("excel")
            || matches!(ext.as_str(), "xlsx" | "xls" | "csv" | "numbers")
        {
            FileKind::Spreadsheet
        } else if mime.contains("word")
            || mime.contains("document")
            || mime.starts_with("text/")
            || matches!(
                ext.as_str(),
                "doc" | "docx" | "txt" | "md" | "rtf" | "pages"
            )
        {
            FileKind::Document
        } else {
            FileKind::Other
        }
    }

    /// An attachment for a file the user picked, guessing its type from
    /// the extension.
    pub fn from_file(name: String, size_bytes: u64) -> Self {
        let ext = name
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
            .unwrap_or_default();
        let mime = match ext.as_str() {
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "heic" => "image/heic",
            "webp" => "image/webp",
            "zip" => "application/zip",
            "txt" | "md" => "text/plain",
            "csv" => "text/csv",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            _ => "application/octet-stream",
        };
        Self {
            name,
            size_bytes,
            mime: mime.into(),
            attachment_id: String::new(),
            part_id: String::new(),
            path: String::new(),
            message_id: String::new(),
        }
    }
}

pub(crate) fn hex_color<'de, D: Deserializer<'de>>(d: D) -> Result<Hsla, D::Error> {
    let s = String::deserialize(d)?;
    from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("not a color: {s}")))
}

/// `#rrggbb`, for the store.
pub fn to_hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

pub fn from_hex(s: &str) -> Option<Hsla> {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).ok()?;
    Some(
        Rgba {
            r: ((v >> 16) & 0xff) as f32 / 255.,
            g: ((v >> 8) & 0xff) as f32 / 255.,
            b: (v & 0xff) as f32 / 255.,
            a: 1.,
        }
        .into(),
    )
}

impl MailData {
    /// Newest first; every view is a filter over this order.
    pub fn sort_threads(&mut self) {
        self.threads
            .sort_by_key(|t| std::cmp::Reverse(t.last_message().internal_date));
    }

    /// Fresh copies of threads: each replaces its thread, or joins the
    /// list if it's new; newest first again.
    pub fn upsert_threads(&mut self, threads: Vec<Thread>) {
        for thread in threads {
            match self.thread_ix(&thread.id) {
                Some(ix) => self.threads[ix] = thread,
                None => self.threads.push(thread),
            }
        }
        self.sort_threads();
    }

    pub fn label(&self, id: &str) -> Option<&Label> {
        self.labels.iter().find(|l| l.id == id)
    }

    pub fn is_me(&self, contact: &Contact) -> bool {
        self.account.is_me(&contact.email)
    }

    pub fn me(&self) -> Contact {
        Contact {
            name: self.account.name.clone(),
            email: self.account.email.clone(),
        }
    }

    pub fn thread_ix(&self, id: &str) -> Option<usize> {
        self.threads.iter().position(|t| t.id == id)
    }

    /// Everyone in the mailbox but me and spam senders, most frequent
    /// first: what the store's `contacts` gives, for its test.
    #[cfg(test)]
    pub fn contacts(&self) -> Vec<Contact> {
        // Spam senders aren't people to write to.
        let messages = self
            .threads
            .iter()
            .filter(|t| t.folder != Folder::Spam)
            .flat_map(|t| &t.messages);
        rank_contacts(
            messages.flat_map(|m| [&m.from].into_iter().chain(&m.to).chain(&m.cc)),
            |c| self.is_me(c),
        )
    }
}

/// The contacts seen, once each, most frequent first, leaving out `me`.
pub fn rank_contacts<'a>(
    seen_in_mail: impl IntoIterator<Item = &'a Contact>,
    me: impl Fn(&Contact) -> bool,
) -> Vec<Contact> {
    let mut seen: Vec<(Contact, usize)> = Vec::new();
    for contact in seen_in_mail {
        if me(contact) {
            continue;
        }
        match seen
            .iter_mut()
            .find(|(c, _)| c.email.eq_ignore_ascii_case(&contact.email))
        {
            Some((known, count)) => {
                *count += 1;
                if known.name.is_empty() {
                    known.name = contact.name.clone();
                }
            }
            None => seen.push((contact.clone(), 1)),
        }
    }
    seen.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    seen.into_iter().map(|(c, _)| c).collect()
}

impl Account {
    /// The address is the user's own or one of their aliases (ACC-2).
    pub fn is_me(&self, email: &str) -> bool {
        email.eq_ignore_ascii_case(&self.email)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(email))
            || self
                .send_as
                .iter()
                .any(|s| s.email.eq_ignore_ascii_case(email))
    }

    /// W3 (MST-1): the addresses a message can go from: the verified
    /// send-as addresses, or the main one alone before the first sync.
    pub fn identities(&self) -> Vec<SendAs> {
        let mut out: Vec<SendAs> = self
            .send_as
            .iter()
            .filter(|s| !s.pending && !s.email.is_empty())
            .cloned()
            .collect();
        if !out
            .iter()
            .any(|s| s.email.eq_ignore_ascii_case(&self.email))
        {
            out.insert(
                0,
                SendAs {
                    email: self.email.clone(),
                    is_primary: true,
                    ..Default::default()
                },
            );
        }
        if !out.iter().any(|s| s.is_default) {
            out[0].is_default = true;
        }
        out
    }

    /// The identity `email` names (ignoring case), else the default one.
    pub fn identity(&self, email: &str) -> SendAs {
        let all = self.identities();
        all.iter()
            .find(|s| s.email.eq_ignore_ascii_case(email))
            .or_else(|| all.iter().find(|s| s.is_default))
            .cloned()
            .unwrap_or_default()
    }

    /// Who a message from `identity` is from: its name, or the account's.
    pub fn sender(&self, identity: &SendAs) -> Contact {
        Contact {
            name: if identity.name.is_empty() {
                self.name.clone()
            } else {
                identity.name.clone()
            },
            email: identity.email.clone(),
        }
    }
}

impl Thread {
    /// A thread written here: a draft, or a message sent in a new thread.
    pub fn local(
        id: String,
        subject: String,
        folder: Folder,
        messages: Vec<Message>,
        origin: Option<DraftOrigin>,
    ) -> Self {
        let mut thread = Thread {
            id,
            subject,
            folder,
            labels: Vec::new(),
            starred: false,
            unread: false,
            important: false,
            messages,
            label_ids: Vec::new(),
            history_id: None,
            loaded: Loaded::Full,
            search: SearchIndex::default(),
            origin,
        };
        thread.reindex();
        thread
    }

    pub fn last_message(&self) -> &Message {
        self.messages.last().expect("threads always have messages")
    }

    pub fn has_attachments(&self) -> bool {
        self.messages.iter().any(|m| !m.attachments.is_empty())
    }

    /// Rebuild `search`; call after changing the thread's messages.
    pub fn reindex(&mut self) {
        let all = self.search_text();
        self.set_search(all);
    }

    /// Everything a search looks through, lowercased: what the store keeps
    /// as `threads.search_text`, so loading doesn't parse HTML (DAT-6).
    pub fn search_text(&self) -> String {
        let (from, to, subject) = self.people_index();
        let mut all = format!("{subject}\n{from}{to}");
        for message in &self.messages {
            all.push_str(&message.snippet.to_lowercase());
            all.push('\n');
            all.push_str(
                &crate::html::plain_text(&crate::html::parse(message.html())).to_lowercase(),
            );
            all.push('\n');
            for attachment in &message.attachments {
                all.push_str(&attachment.name.to_lowercase());
                all.push('\n');
            }
        }
        all
    }

    /// `search` from a stored `search_text`.
    pub fn set_search(&mut self, all: String) {
        let (from, to, subject) = self.people_index();
        self.search = SearchIndex {
            all,
            from,
            to,
            subject,
        };
    }

    /// The lowercased senders, recipients and subject.
    fn people_index(&self) -> (String, String, String) {
        let people = |contacts: &mut dyn Iterator<Item = &Contact>| {
            let mut out = String::new();
            for c in contacts {
                out.push_str(&c.name);
                out.push(' ');
                out.push_str(&c.email);
                out.push('\n');
            }
            out.to_lowercase()
        };
        let from = people(&mut self.messages.iter().map(|m| &m.from));
        let to = people(
            &mut self
                .messages
                .iter()
                .flat_map(|m| m.to.iter().chain(&m.cc).chain(&m.bcc)),
        );
        (from, to, self.subject.to_lowercase())
    }

    /// Spam and trash stay out of Starred and label views, like Gmail.
    pub fn is_hidden(&self) -> bool {
        matches!(self.folder, Folder::Spam | Folder::Trash)
    }

    /// In the Sent view: every conversation holding a message the user
    /// sent, like Gmail, so a thread can show in Sent and Archive both.
    pub fn in_sent(&self) -> bool {
        self.folder == Folder::Sent
            || (!self.is_hidden()
                && self.folder != Folder::Drafts
                && self
                    .messages
                    .iter()
                    .any(|m| m.label_ids.iter().any(|l| l == crate::backend::gmail::SENT)))
    }
}

impl Contact {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.email
        } else {
            &self.name
        }
    }

    pub fn initials(&self) -> String {
        let mut words = self.display_name().split_whitespace();
        let first = words.next().and_then(|w| w.chars().next());
        let last = words.last().and_then(|w| w.chars().next());
        first
            .into_iter()
            .chain(last)
            .flat_map(char::to_uppercase)
            .collect()
    }
}

/// The snippet without the heading HTML mail often opens with, usually the
/// sender's brand ("Skyline Air", "Stripe payout", a bold "Harbor Bank
/// Alert"), which the row already shows. Inline tags also leave a space
/// before punctuation ("the doc ."); that goes too.
pub(crate) fn clean_snippet(message: &Message) -> String {
    use crate::html::{Block, TextKind};
    let mut snippet = message.snippet.trim();
    let doc = crate::html::parse(message.html());
    if let Some(Block::Text { text, kind, .. }) = doc.blocks.first() {
        let bold = text.spans.len() == 1
            && text.spans[0].0 == (0..text.text.len())
            && text.spans[0].1.bold;
        if matches!(kind, TextKind::Heading(_)) || bold {
            snippet = strip_word_prefix(snippet, text.text.trim());
        }
    }
    let mut out = String::with_capacity(snippet.len());
    for c in snippet.chars() {
        if matches!(c, '.' | ',' | ';' | ':' | '!' | '?') && out.ends_with(' ') {
            out.pop();
        }
        out.push(c);
    }
    out
}

/// `text` without a leading `prefix` that ends at a word boundary.
fn strip_word_prefix<'a>(text: &'a str, prefix: &str) -> &'a str {
    match text.get(..prefix.len()) {
        Some(head)
            if !prefix.is_empty()
                && head.eq_ignore_ascii_case(prefix)
                && text[prefix.len()..].starts_with(char::is_whitespace) =>
        {
            text[prefix.len()..].trim_start()
        }
        _ => text,
    }
}

/// "9:12 AM" (or "09:12") today, "Yesterday", "Tue" this week, "Oct 3" this
/// year, else "Oct 3, 2025".
pub fn list_date(date: &DateTime<FixedOffset>) -> String {
    let date = date.with_timezone(&Local);
    let now = Local::now();
    let days = (now.date_naive() - date.date_naive()).num_days();
    match days {
        0 => crate::calendar::time::time_label(date.time()),
        1 => "Yesterday".into(),
        2..=6 => date.format("%a").to_string(),
        _ if date.year() == now.year() => date.format("%b %-d").to_string(),
        _ => date.format("%b %-d, %Y").to_string(),
    }
}

/// "Sun, Oct 25, 9:12 AM", with the year only when it isn't this year.
pub fn full_date(date: &DateTime<FixedOffset>) -> String {
    let date = date.with_timezone(&Local);
    crate::calendar::time::date_time_label(
        date.date_naive(),
        date.time(),
        Local::now().date_naive(),
    )
}

pub fn file_size(bytes: u64) -> String {
    const KB: f64 = 1024.;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W3 (MST-1, ACC-2).
    #[test]
    fn identities_and_me() {
        let mut account = Account {
            name: "Me".into(),
            email: "me@x.com".into(),
            ..Default::default()
        };
        // Before the first sync: the main address alone, as the default.
        let ids = account.identities();
        assert_eq!(ids.len(), 1);
        assert!(ids[0].is_default);
        account.send_as = vec![
            SendAs {
                email: "me@x.com".into(),
                is_primary: true,
                ..Default::default()
            },
            SendAs {
                email: "work@x.com".into(),
                name: "Work Me".into(),
                is_default: true,
                ..Default::default()
            },
            SendAs {
                email: "new@x.com".into(),
                pending: true,
                ..Default::default()
            },
        ];
        let ids = account.identities();
        assert_eq!(ids.len(), 2, "unverified addresses can't send");
        assert_eq!(account.identity("").email, "work@x.com");
        assert_eq!(account.identity("ME@x.com").email, "me@x.com");
        assert_eq!(account.sender(&account.identity("")).name, "Work Me");
        assert_eq!(account.sender(&account.identity("me@x.com")).name, "Me");
        assert!(account.is_me("WORK@x.com") && !account.is_me("ann@x.com"));
    }

    #[test]
    fn archive_target_per_folder() {
        assert_eq!(Folder::Inbox.archive_target(), Some(Folder::Archive));
        assert_eq!(Folder::Archive.archive_target(), Some(Folder::Inbox));
        assert_eq!(Folder::Spam.archive_target(), Some(Folder::Inbox));
        assert_eq!(Folder::Trash.archive_target(), Some(Folder::Inbox));
        assert_eq!(Folder::Sent.archive_target(), None);
        assert_eq!(Folder::Drafts.archive_target(), None);
    }

    #[test]
    fn upserted_threads_replace_or_join_newest_first() {
        let thread = |id: &str, folder: Folder, at: i64| {
            let message = Message {
                internal_date: at,
                ..Message::new(Contact::default())
            };
            Thread::local(id.into(), id.into(), folder, vec![message], None)
        };
        let mut data = MailData {
            account: Account::default(),
            labels: Vec::new(),
            threads: vec![
                thread("b", Folder::Inbox, 20),
                thread("a", Folder::Inbox, 10),
            ],
        };
        data.upsert_threads(vec![
            thread("new", Folder::Inbox, 30),
            thread("a", Folder::Archive, 10),
            thread("mid", Folder::Inbox, 15),
        ]);
        let ids: Vec<&str> = data.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["new", "b", "mid", "a"]);
        assert_eq!(data.threads[3].folder, Folder::Archive);
    }

    #[test]
    fn snippets_skip_the_heading_and_sender() {
        let message = |name: &str, html: &str, snippet: &str| Message {
            from: Contact {
                name: name.into(),
                email: "x@example.com".into(),
            },
            snippet: snippet.into(),
            body: Body::Loaded { html: html.into() },
            ..Message::new(Contact::default())
        };
        let m = message(
            "Skyline Air",
            "<h2>Skyline Air</h2><p>Hi Alex</p>",
            "Skyline Air Hi Alex",
        );
        assert_eq!(clean_snippet(&m), "Hi Alex");
        let m = message(
            "Stripe",
            "<h2>Stripe payout</h2><p>Your</p>",
            "Stripe payout Your",
        );
        assert_eq!(clean_snippet(&m), "Your");
        let m = message("Sarah", "<p>See the doc .</p>", "See the doc . Thanks");
        assert_eq!(clean_snippet(&m), "See the doc. Thanks");
        // Only whole words.
        let m = message("Sam", "<h1>Sam</h1><p>Samples</p>", "Samples ready");
        assert_eq!(clean_snippet(&m), "Samples ready");
        let m = message(
            "Bank",
            "<p><b>Bank Alert</b></p><p>A charge</p>",
            "Bank Alert A charge",
        );
        assert_eq!(clean_snippet(&m), "A charge");
        // A sentence that starts with the sender's name stays whole.
        let m = message(
            "Hannah",
            "<p><b>Hannah</b> invited you</p>",
            "Hannah invited you",
        );
        assert_eq!(clean_snippet(&m), "Hannah invited you");
    }

    #[test]
    fn attachment_kinds() {
        let a = |name: &str, mime: &str| Attachment {
            mime: mime.into(),
            ..Attachment::from_file(name.into(), 1)
        };
        assert_eq!(a("x.pdf", "application/pdf").kind(), FileKind::Pdf);
        assert_eq!(a("x.png", "image/png").kind(), FileKind::Image);
        assert_eq!(a("x.zip", "application/zip").kind(), FileKind::Archive);
        assert_eq!(
            Attachment::from_file("Budget.xlsx".into(), 10).kind(),
            FileKind::Spreadsheet
        );
        assert_eq!(
            a(
                "notes.docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            )
            .kind(),
            FileKind::Document
        );
        assert_eq!(
            a("x.bin", "application/octet-stream").kind(),
            FileKind::Other
        );
    }
}

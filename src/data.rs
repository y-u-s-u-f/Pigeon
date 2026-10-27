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
}

#[derive(Debug, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "hex_color")]
    pub color: Hsla,
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
    /// A calendar invitation (or its cancellation) this message carries.
    #[serde(default)]
    pub invite: Option<Invite>,
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
            invite: None,
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

/// An invitation email's link to its event in the calendar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invite {
    /// `Event::id`, resolved from `ical_uid`.
    pub event_id: String,
    pub method: InviteMethod,
    /// The `text/calendar` part's UID.
    #[serde(default)]
    pub ical_uid: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InviteMethod {
    /// Asks the user to come.
    Request,
    /// The organizer called it off.
    Cancel,
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

/// "9:12 AM" today, "Yesterday", "Tue" this week, "Oct 3" this year, else "Oct 3, 2025".
pub fn list_date(date: &DateTime<FixedOffset>) -> String {
    let date = date.with_timezone(&Local);
    let now = Local::now();
    let days = (now.date_naive() - date.date_naive()).num_days();
    match days {
        0 => date.format("%-I:%M %p").to_string(),
        1 => "Yesterday".into(),
        2..=6 => date.format("%a").to_string(),
        _ if date.year() == now.year() => date.format("%b %-d").to_string(),
        _ => date.format("%b %-d, %Y").to_string(),
    }
}

/// "Sun, Oct 25, 9:12 AM", with the year only when it isn't this year.
pub fn full_date(date: &DateTime<FixedOffset>) -> String {
    let date = date.with_timezone(&Local);
    if date.year() == Local::now().year() {
        date.format("%a, %b %-d, %-I:%M %p").to_string()
    } else {
        date.format("%a, %b %-d %Y, %-I:%M %p").to_string()
    }
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

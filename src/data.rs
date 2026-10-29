//! Mailbox model. The backend builds it from the store (`backend::store`);
//! demo mode seeds the store from the bundled placeholder data, which these
//! types also deserialize from.

use chrono::{DateTime, Datelike, FixedOffset, Local};
use gpui::{Hsla, Rgba};
use serde::{Deserialize, Deserializer, Serialize};

use crate::app::ComposeKind;
use crate::backend::categories::Category;

/// Gmail's label for Primary, which `Category` leaves implicit.
const CATEGORY_PERSONAL: &str = "CATEGORY_PERSONAL";

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
            ListVisibility::ShowIfUnread => "Show If Unread",
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

    /// What archive is called here (ME-027): "Not Spam" in Spam, "Move to
    /// Inbox" from Archive and Trash.
    pub fn archive_title(self) -> Option<&'static str> {
        Some(match (self, self.archive_target()?) {
            (Folder::Spam, _) => "Not Spam",
            (_, Folder::Inbox) => "Move to Inbox",
            _ => "Archive",
        })
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
    /// Snooze and Send Later (ACT-11, CMP-17), kept here, not in Gmail.
    #[serde(skip)]
    pub later: Later,
    /// AI sprint: the conversation's `ai_results` rows (AI-2).
    #[serde(skip)]
    pub ai: crate::ai::ThreadAi,
}

/// When a snoozed conversation comes back, when it last did, and when a
/// draft's scheduled send goes; all Unix ms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Later {
    pub snoozed_until: Option<i64>,
    /// It came back then, so it sorts as if new then.
    pub woke_at: Option<i64>,
    pub send_at: Option<i64>,
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
    /// ME-010: the message in it the reply answers, by id; empty for the
    /// thread's last.
    #[serde(default)]
    pub answers: String,
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
    Video,
    Calendar,
    Other,
}

impl Attachment {
    pub fn kind(&self) -> FileKind {
        let mime = self.mime.to_ascii_lowercase();
        let ext = extension(&self.name);
        if mime == "application/pdf" || ext == "pdf" {
            FileKind::Pdf
        } else if mime.starts_with("image/") || matches!(ext.as_str(), "heic" | "heif") {
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
        } else if mime.starts_with("video/")
            || matches!(ext.as_str(), "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm")
        {
            FileKind::Video
        } else if mime == "text/calendar" || mime == "application/ics" || ext == "ics" {
            FileKind::Calendar
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
        let ext = extension(&name);
        let mime = match ext.as_str() {
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "heic" => "image/heic",
            "heif" => "image/heif",
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

/// A file name's extension, lower case; empty when it has none.
fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
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
    Some(gpui::rgb(v).into())
}

impl MailData {
    /// Newest first; every view is a filter over this order.
    pub fn sort_threads(&mut self) {
        self.threads
            .sort_by_key(|t| std::cmp::Reverse(t.sort_time()));
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
    let mut index: std::collections::HashMap<String, usize> = Default::default();
    for contact in seen_in_mail {
        if me(contact) {
            continue;
        }
        match index.entry(contact.email.to_ascii_lowercase()) {
            std::collections::hash_map::Entry::Occupied(ix) => {
                let (known, count) = &mut seen[*ix.get()];
                *count += 1;
                if known.name.is_empty() {
                    known.name = contact.name.clone();
                }
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(seen.len());
                seen.push((contact.clone(), 1));
            }
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

    /// More than one address to send from (MC-067): `identities().len() > 1`
    /// without building them.
    pub fn has_several_identities(&self) -> bool {
        let (mut usable, mut main) = (0, 0);
        for s in self
            .send_as
            .iter()
            .filter(|s| !s.pending && !s.email.is_empty())
        {
            usable += 1;
            main += usize::from(s.email.eq_ignore_ascii_case(&self.email));
        }
        // The main address joins the list when no send-as entry is it.
        usable + usize::from(main == 0) > 1
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
            later: Later::default(),
            ai: Default::default(),
        };
        thread.reindex();
        thread
    }

    /// Where it sorts: its newest message, or its return from a snooze.
    pub fn sort_time(&self) -> i64 {
        let last = self.last_message().internal_date;
        self.later.woke_at.map_or(last, |w| w.max(last))
    }

    pub fn last_message(&self) -> &Message {
        self.messages.last().expect("threads always have messages")
    }

    /// Its labels and flags from its messages' labels, as Gmail derives
    /// them: the union of the label ids, the user labels among them, and
    /// unread, starred and important.
    pub fn derive_from_labels(&mut self) {
        use crate::backend::gmail;
        self.label_ids = gmail::union_labels(self.messages.iter().map(|m| &m.label_ids));
        let has = |label: &str| self.label_ids.iter().any(|l| l == label);
        self.unread = has(gmail::UNREAD);
        self.starred = has(gmail::STARRED);
        self.important = has(gmail::IMPORTANT);
        self.labels = self
            .label_ids
            .iter()
            .filter(|l| !gmail::is_system_label(l))
            .cloned()
            .collect();
    }

    pub fn has_attachments(&self) -> bool {
        self.messages.iter().any(|m| !m.attachments.is_empty())
    }

    /// The subject, or "(no subject)" for none (ME-021): views show the
    /// placeholder faint and italic, where `has_subject` is false.
    pub fn subject_or_placeholder(&self) -> &str {
        if self.has_subject() {
            &self.subject
        } else {
            "(no subject)"
        }
    }

    pub fn has_subject(&self) -> bool {
        !self.subject.trim().is_empty()
    }

    /// Who wrote, Gmail style (ME-011): "Ana, me, Bob"; first names past
    /// two people, and "Ana … Cy, Dan" past three.
    pub fn senders_line(&self, is_me: impl Fn(&Contact) -> bool) -> String {
        let mut people: Vec<&Contact> = Vec::new();
        for message in &self.messages {
            let from = &message.from;
            if !people
                .iter()
                .any(|p| p.email.eq_ignore_ascii_case(&from.email))
            {
                people.push(from);
            }
        }
        let short = people.len() > 2;
        let name = |c: &Contact| -> String {
            if is_me(c) {
                "me".into()
            } else if short {
                c.first_name().into()
            } else {
                c.display_name().into()
            }
        };
        match people.as_slice() {
            [] => String::new(),
            [first, .., a, b] if people.len() > 3 => {
                format!("{} … {}, {}", name(first), name(a), name(b))
            }
            _ => people
                .iter()
                .map(|c| name(c))
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    /// Who the last message went to, for Sent and Drafts (ME-011): "To:
    /// Ana, Bob +3".
    pub fn recipients_line(&self) -> String {
        let last = self.last_message();
        let mut people: Vec<&Contact> = Vec::new();
        for c in last.to.iter().chain(&last.cc).chain(&last.bcc) {
            if !people
                .iter()
                .any(|p| p.email.eq_ignore_ascii_case(&c.email))
            {
                people.push(c);
            }
        }
        let shown: Vec<&str> = people
            .iter()
            .take(2)
            .map(|c| {
                if people.len() > 1 {
                    c.first_name()
                } else {
                    c.display_name()
                }
            })
            .collect();
        match people.len() {
            0 => "(no recipients)".into(),
            1 | 2 => format!("To: {}", shown.join(", ")),
            n => format!("To: {} +{}", shown.join(", "), n - 2),
        }
    }

    /// The conversation in Gmail on the web, in the account `email` is
    /// (MB-061); none for one not in Gmail yet.
    pub fn gmail_link(&self, email: &str) -> Option<String> {
        if self.id.contains("-local-") || self.id.is_empty() {
            return None;
        }
        Some(format!(
            "https://mail.google.com/mail/?authuser={}#all/{}",
            url_escape(email),
            self.id
        ))
    }

    /// Move the thread's Inbox messages to a Gmail category (MB-062): the
    /// labels Gmail is asked to add and take off, applied here too.
    pub fn set_category(&mut self, category: Category) -> (Vec<String>, Vec<String>) {
        let target = category.label_id().unwrap_or(CATEGORY_PERSONAL);
        let remove: Vec<String> = Category::ALL
            .iter()
            .filter_map(|c| c.label_id())
            .chain([CATEGORY_PERSONAL])
            .filter(|l| *l != target)
            .map(String::from)
            .collect();
        for message in &mut self.messages {
            message.label_ids.retain(|l| !remove.contains(l));
            if !message.label_ids.iter().any(|l| l == target) {
                message.label_ids.push(target.into());
            }
        }
        (vec![target.into()], remove)
    }

    /// Rebuild `search`; call after changing the thread's messages.
    pub fn reindex(&mut self) {
        // The people index once, for both (MC-026).
        let (from, to, subject) = self.people_index();
        let all = self.search_text(&from, &to, &subject);
        self.search = SearchIndex {
            all,
            from,
            to,
            subject,
        };
    }

    /// Everything a search looks through, lowercased: what the store keeps
    /// as `threads.search_text`, so loading doesn't parse HTML (DAT-6).
    fn search_text(&self, from: &str, to: &str, subject: &str) -> String {
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

    /// "Ana" for Ana Lima; an address's part before the @.
    pub fn first_name(&self) -> &str {
        if self.name.trim().is_empty() {
            self.email.split('@').next().unwrap_or(&self.email)
        } else {
            self.name.split_whitespace().next().unwrap_or(&self.name)
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
/// year, else "Oct 3, 2025"; "3 Oct" and "3 Oct 2025" where the region
/// puts the day first (MA-043).
pub fn list_date(date: &DateTime<FixedOffset>) -> String {
    list_date_on(date, Local::now())
}

/// `list_date` against a `now` read once for many rows (MC-043).
pub fn list_date_on(date: &DateTime<FixedOffset>, now: DateTime<Local>) -> String {
    let date = date.with_timezone(&Local);
    let days = (now.date_naive() - date.date_naive()).num_days();
    let day_first = day_first();
    match days {
        0 => crate::calendar::time::time_label(date.time()),
        1 => "Yesterday".into(),
        2..=6 => date.format("%a").to_string(),
        _ if date.year() == now.year() && day_first => date.format("%-d %b").to_string(),
        _ if date.year() == now.year() => date.format("%b %-d").to_string(),
        _ if day_first => date.format("%-d %b %Y").to_string(),
        _ => date.format("%b %-d, %Y").to_string(),
    }
}

/// Whether the region writes the day before the month ("3 Oct"), from
/// the system's own short date pattern; read once (MA-043).
pub fn day_first() -> bool {
    static DAY_FIRST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DAY_FIRST.get_or_init(|| {
        use objc2_foundation::{NSDateFormatter, NSLocale, NSString};
        let locale = NSLocale::autoupdatingCurrentLocale();
        NSDateFormatter::dateFormatFromTemplate_options_locale(
            &NSString::from_str("MMMd"),
            0,
            Some(&locale),
        )
        .is_some_and(|pattern| day_first_pattern(&pattern.to_string()))
    })
}

/// A date pattern ("d MMM", "MMM d") puts the day first. Quoted text is
/// literal.
fn day_first_pattern(pattern: &str) -> bool {
    let unquoted: String = pattern.split('\'').step_by(2).collect();
    match (unquoted.find('d'), unquoted.find(['M', 'L'])) {
        (Some(day), Some(month)) => day < month,
        _ => false,
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

/// A query value for a URL: everything but unreserved characters and `@`
/// percent-encoded.
fn url_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Sizes as Finder shows them: 1,000-based (MA-043).
pub fn file_size(bytes: u64) -> String {
    const K: f64 = 1000.;
    let b = bytes as f64;
    if bytes < 1000 {
        format!("{bytes} bytes")
    } else if b < K * K {
        format!("{:.0} KB", b / K)
    } else if b < K * K * K {
        format!("{:.1} MB", b / (K * K))
    } else {
        format!("{:.2} GB", b / (K * K * K))
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

    /// MC-067: the same answer as building the identities.
    #[test]
    fn several_identities_without_building_them() {
        let send_as = |email: &str, pending: bool| SendAs {
            email: email.into(),
            pending,
            ..Default::default()
        };
        let cases = [
            vec![],
            vec![send_as("me@x.com", false)],
            vec![send_as("ME@x.com", false), send_as("new@x.com", true)],
            vec![send_as("work@x.com", false)],
            vec![send_as("me@x.com", false), send_as("work@x.com", false)],
            vec![send_as("", false), send_as("me@x.com", false)],
        ];
        for send_as in cases {
            let account = Account {
                email: "me@x.com".into(),
                send_as,
                ..Default::default()
            };
            assert_eq!(
                account.has_several_identities(),
                account.identities().len() > 1,
                "{:?}",
                account.send_as
            );
        }
    }

    fn contact(name: &str, email: &str) -> Contact {
        Contact {
            name: name.into(),
            email: email.into(),
        }
    }

    fn thread_from(people: &[Contact]) -> Thread {
        let messages = people
            .iter()
            .map(|from| Message::new(from.clone()))
            .collect();
        Thread::local("t".into(), "s".into(), Folder::Inbox, messages, None)
    }

    /// ME-011.
    #[test]
    fn sender_lines_read_like_gmail() {
        let ana = contact("Ana Lima", "ana@x.com");
        let me = contact("Me", "me@x.com");
        let bob = contact("Bob Hart", "bob@x.com");
        let cy = contact("", "cy@x.com");
        let dan = contact("Dan Ode", "dan@x.com");
        let is_me = |c: &Contact| c.email == "me@x.com";
        let line = |people: &[Contact]| thread_from(people).senders_line(is_me);
        assert_eq!(line(std::slice::from_ref(&ana)), "Ana Lima");
        assert_eq!(
            line(&[ana.clone(), me.clone(), ana.clone()]),
            "Ana Lima, me"
        );
        assert_eq!(
            line(&[ana.clone(), me.clone(), bob.clone()]),
            "Ana, me, Bob"
        );
        assert_eq!(
            line(&[
                ana.clone(),
                me.clone(),
                bob.clone(),
                cy.clone(),
                dan.clone()
            ]),
            "Ana … cy, Dan"
        );

        let mut thread = thread_from(std::slice::from_ref(&me));
        assert_eq!(thread.recipients_line(), "(no recipients)");
        thread.messages[0].to = vec![ana.clone()];
        assert_eq!(thread.recipients_line(), "To: Ana Lima");
        thread.messages[0].cc = vec![bob, cy, dan, ana];
        assert_eq!(thread.recipients_line(), "To: Ana, Bob +2");
    }

    #[test]
    fn no_subject_placeholder() {
        let mut thread = thread_from(&[contact("", "a@x.com")]);
        thread.subject = "  ".into();
        assert!(!thread.has_subject());
        assert_eq!(thread.subject_or_placeholder(), "(no subject)");
        thread.subject = "Hi".into();
        assert_eq!(thread.subject_or_placeholder(), "Hi");
    }

    /// MB-061.
    #[test]
    fn gmail_links_carry_the_account() {
        let mut thread = thread_from(&[contact("", "a@x.com")]);
        thread.id = "18c2f".into();
        assert_eq!(
            thread.gmail_link("me+work@x.com").as_deref(),
            Some("https://mail.google.com/mail/?authuser=me%2Bwork@x.com#all/18c2f")
        );
        thread.id = "sent-local-abc".into();
        assert_eq!(thread.gmail_link("me@x.com"), None);
    }

    /// MB-062: one category label, the others off.
    #[test]
    fn moving_to_a_category_swaps_its_label() {
        use crate::backend::categories::{category_of, in_category};
        let mut thread = thread_from(&[contact("", "a@x.com"), contact("", "b@x.com")]);
        thread.messages[0].label_ids = vec!["INBOX".into(), "CATEGORY_PROMOTIONS".into()];
        thread.messages[1].label_ids = vec!["INBOX".into()];
        let (add, remove) = thread.set_category(Category::Social);
        assert_eq!(add, vec!["CATEGORY_SOCIAL".to_string()]);
        assert!(remove.contains(&"CATEGORY_PROMOTIONS".to_string()));
        assert!(remove.contains(&"CATEGORY_PERSONAL".to_string()));
        assert!(in_category(&thread, Category::Social));
        assert!(!in_category(&thread, Category::Promotions));
        thread.set_category(Category::Primary);
        assert!(
            thread
                .messages
                .iter()
                .all(|m| category_of(&m.label_ids) == Category::Primary)
        );
    }

    /// MA-043.
    #[test]
    fn day_order_and_finder_sizes() {
        assert!(day_first_pattern("d MMM"));
        assert!(day_first_pattern("d. MMM"));
        assert!(!day_first_pattern("MMM d"));
        assert!(!day_first_pattern("'d'MMM d"));
        assert_eq!(file_size(999), "999 bytes");
        assert_eq!(file_size(1_500), "2 KB");
        assert_eq!(file_size(1_500_000), "1.5 MB");
        assert_eq!(file_size(2_340_000_000), "2.34 GB");
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
        assert_eq!(a("clip.mov", "video/quicktime").kind(), FileKind::Video);
        assert_eq!(a("invite.ics", "text/calendar").kind(), FileKind::Calendar);
        assert_eq!(
            a("x.bin", "application/octet-stream").kind(),
            FileKind::Other
        );
    }
}

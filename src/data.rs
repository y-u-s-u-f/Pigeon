//! Mailbox model, loaded from the bundled placeholder data.

use chrono::{DateTime, Datelike, FixedOffset, Local};
use gpui::{Hsla, Rgba};
use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
pub struct MailData {
    pub account: Account,
    pub labels: Vec<Label>,
    pub threads: Vec<Thread>,
}

#[derive(Debug, Deserialize)]
pub struct Account {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "hex_color")]
    pub color: Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Folder {
    Inbox,
    Archive,
    Sent,
    Drafts,
    Spam,
    Trash,
}

#[derive(Debug, Deserialize)]
pub struct Thread {
    pub subject: String,
    pub folder: Folder,
    pub labels: Vec<String>,
    pub starred: bool,
    pub unread: bool,
    /// Oldest first.
    pub messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub from: Contact,
    pub to: Vec<Contact>,
    #[serde(default)]
    pub cc: Vec<Contact>,
    pub date: DateTime<FixedOffset>,
    pub snippet: String,
    pub html: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Deserialize)]
pub struct Contact {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub size_bytes: u64,
    #[allow(dead_code)] // Used once attachments can be opened.
    pub mime: String,
}

fn hex_color<'de, D: Deserializer<'de>>(d: D) -> Result<Hsla, D::Error> {
    let s = String::deserialize(d)?;
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).map_err(serde::de::Error::custom)?;
    Ok(Rgba {
        r: ((v >> 16) & 0xff) as f32 / 255.,
        g: ((v >> 8) & 0xff) as f32 / 255.,
        b: (v & 0xff) as f32 / 255.,
        a: 1.,
    }
    .into())
}

impl MailData {
    pub fn load() -> anyhow::Result<Self> {
        let mut data: MailData = serde_json::from_str(include_str!("../assets/mailbox.json"))?;
        for thread in &mut data.threads {
            thread.messages.sort_by_key(|m| m.date);
        }
        data.threads.retain(|t| !t.messages.is_empty());
        // Newest first; every view is a filter over this order.
        data.threads
            .sort_by_key(|t| std::cmp::Reverse(t.last_message().date));
        Ok(data)
    }

    pub fn label(&self, id: &str) -> Option<&Label> {
        self.labels.iter().find(|l| l.id == id)
    }

    pub fn is_me(&self, contact: &Contact) -> bool {
        contact.email.eq_ignore_ascii_case(&self.account.email)
    }
}

impl Thread {
    pub fn last_message(&self) -> &Message {
        self.messages.last().expect("threads always have messages")
    }

    pub fn has_attachments(&self) -> bool {
        self.messages.iter().any(|m| !m.attachments.is_empty())
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

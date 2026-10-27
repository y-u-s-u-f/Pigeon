//! Gmail: the API types and calls (W3), and how Gmail's labels map onto
//! Pigeon's model (DAT-2).

use crate::data::Folder;

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!is_system_label("Label_12"));
        assert!(!is_system_label("work"));
    }
}

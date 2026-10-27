//! Demo mode's data: the bundled placeholder JSON, made to look like what a
//! Gmail sync stores (message ids, label ids, internal dates), and seeded
//! into an in-memory store. Changes then apply to that store only.

use super::{gmail, store::Store};
use crate::{
    calendar::CalendarData,
    data::{Account, Folder, MailData, Message, SendAs, Thread, clean_snippet},
};

/// The placeholder mailbox, ready to store: newest first, snippets
/// cleaned, indexed for search.
pub fn mail_data() -> MailData {
    let mut data: MailData = serde_json::from_str(include_str!("../../assets/mailbox.json"))
        .expect("bundled mailbox.json should parse");
    data.threads.retain(|t| !t.messages.is_empty());
    let account = data.account.clone();
    for thread in &mut data.threads {
        thread.messages.sort_by_key(|m| m.date);
        let last = thread.messages.len() - 1;
        for ix in 0..thread.messages.len() {
            let labels = demo_labels(thread, &thread.messages[ix], ix == last, &account);
            let message = &mut thread.messages[ix];
            message.id = format!("{}-m{ix}", thread.id);
            message.internal_date = message.date.timestamp_millis();
            message.label_ids = labels;
            message.snippet = clean_snippet(message);
        }
        thread.label_ids = gmail::union_labels(thread.messages.iter().map(|m| &m.label_ids));
        thread.reindex();
    }
    data.sort_threads();
    data
}

/// The labels Gmail would have on `message`, from what the placeholder
/// data says about its thread.
fn demo_labels(thread: &Thread, message: &Message, last: bool, me: &Account) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    let folder = match thread.folder {
        Folder::Inbox => Some(gmail::INBOX),
        Folder::Spam => Some(gmail::SPAM),
        Folder::Trash => Some(gmail::TRASH),
        Folder::Drafts => Some(gmail::DRAFT),
        Folder::Archive | Folder::Sent => None,
    };
    labels.extend(folder.map(String::from));
    if me.is_me(&message.from.email) && thread.folder != Folder::Drafts {
        labels.push(gmail::SENT.into());
    }
    if last && thread.unread {
        labels.push(gmail::UNREAD.into());
    }
    if last && thread.starred {
        labels.push(gmail::STARRED.into());
    }
    labels.extend(thread.labels.iter().cloned());
    // W4a: the placeholder data has no categories; Gmail would sort these.
    if !me.is_me(&message.from.email)
        && let Some(category) = demo_category(&message.from.email)
    {
        labels.push(category.into());
    }
    labels
}

/// Gmail's category for mail from these senders; the rest is Primary.
fn demo_category(from: &str) -> Option<&'static str> {
    const PROMOTIONS: &[&str] = &[
        "hello@makersmail.com",
        "news@sketchwave.app",
        "orders@ferroandfinch.com",
    ];
    const UPDATES: &[&str] = &[
        "updates@parcelway.com",
        "receipts@stripe.com",
        "bookings@skylineair.com",
        "no-reply@aws.amazon.com",
        "noreply@uber.com",
        "shipment-tracking@amazon.com",
        "renewals@harbormutual.com",
        "billing@cwu-city.gov",
        "drive-shares-noreply@google.com",
        "tickets@eastsideconcerts.org",
        "reservations@riversidehotel.pt",
    ];
    const FORUMS: &[&str] = &["noreply@github.com", "newsletter@rustacean-weekly.dev"];
    let from = from.to_ascii_lowercase();
    let is = |list: &[&str]| list.contains(&from.as_str());
    if is(PROMOTIONS) {
        Some("CATEGORY_PROMOTIONS")
    } else if is(UPDATES) {
        Some("CATEGORY_UPDATES")
    } else if is(FORUMS) {
        Some("CATEGORY_FORUMS")
    } else {
        None
    }
}

/// The placeholder calendars and events.
pub fn calendar_data() -> CalendarData {
    let mut data: CalendarData = serde_json::from_str(include_str!("../../assets/calendar.json"))
        .expect("bundled calendar.json should parse");
    let me = data.account.email.clone();
    for event in &mut data.events {
        event.organizer_self = event.organizer.email.eq_ignore_ascii_case(&me);
    }
    data
}

/// The demo account's addresses: its own, and a studio alias.
fn demo_send_as(account: &Account) -> Vec<SendAs> {
    vec![
        SendAs {
            email: account.email.clone(),
            signature: format!("{}<br>Sent with Pigeon", account.name),
            is_default: true,
            is_primary: true,
            ..Default::default()
        },
        SendAs {
            email: "alex@riverastudio.example".into(),
            name: "Alex Rivera · Rivera Studio".into(),
            reply_to: "hello@riverastudio.example".into(),
            signature: "<b>Alex Rivera</b><br>Rivera Studio · \
                <a href=\"https://riverastudio.example\">riverastudio.example</a>"
                .into(),
            ..Default::default()
        },
    ]
}

/// Fill `store` with the placeholder data; returns its account.
pub fn seed(store: &Store) -> anyhow::Result<Account> {
    let mail = mail_data();
    let mut account = mail.account.clone();
    // W3 (MST-1, MST-2): an alias with a signature, so the From picker
    // and the signature show.
    account.send_as = demo_send_as(&account);
    account.aliases = account.send_as[1..]
        .iter()
        .map(|s| s.email.clone())
        .collect();
    store.set_sync_state(
        &account.email,
        super::store::ALIASES_KEY,
        &serde_json::to_string(&account.aliases)?,
    )?;
    store.set_sync_state(
        &account.email,
        super::store::SEND_AS_KEY,
        &serde_json::to_string(&account.send_as)?,
    )?;
    store.put_labels(&account.email, &mail.labels)?;
    store.put_threads(&account.email, &mail.threads)?;
    let calendar = calendar_data();
    store.put_calendars(&account.email, &calendar.calendars)?;
    store.put_events(&account.email, &calendar.events)?;
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_counts() {
        let store = Store::open_in_memory().unwrap();
        let account = seed(&store).unwrap();
        assert_eq!(account.email, "alex.rivera@example.com");
        let threads = store.load_all_threads(&account.email).unwrap();
        assert_eq!(threads.len(), 132);
        assert_eq!(
            store.load_labels(&account.email).unwrap().len(),
            mail_data().labels.len()
        );
        assert_eq!(store.load_calendars(&account.email).unwrap().len(), 8);
        assert_eq!(store.load_all_events(&account.email).unwrap().len(), 167);
        // Every folder has something to show.
        for folder in Folder::ALL {
            assert!(threads.iter().any(|t| t.folder == folder), "{folder:?}");
        }
    }

    #[test]
    fn demo_labels_follow_the_thread() {
        let data = mail_data();
        for thread in &data.threads {
            assert_eq!(
                thread.unread,
                thread.label_ids.iter().any(|l| l == gmail::UNREAD)
            );
            if matches!(thread.folder, Folder::Inbox | Folder::Spam | Folder::Trash) {
                assert_eq!(
                    gmail::derive_folder(&thread.messages, &data.account),
                    thread.folder,
                    "{}",
                    thread.id
                );
            }
        }
    }
}

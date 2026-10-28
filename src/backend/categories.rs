//! Gmail's Inbox categories (FOL-11, W4a): which category a message and a
//! thread are in, and the unread count of each in the Inbox.
//!
//! - Per message: Primary unless it carries one of the four category
//!   labels. A thread is in a category when any of its Inbox messages is,
//!   as Gmail's `category:` search matches threads.
//! - Counts: Google mode asks Gmail (`threads.list labelIds=INBOX,UNREAD
//!   q=category:<x>`, ids only, up to `COUNT_CAP`), at launch and after a
//!   sync pass whose history touched the Inbox; the last ones are kept in
//!   `sync_state`. Demo mode counts the store.

use gpui::{Context, Task};
use serde::{Deserialize, Serialize};

use super::{
    Backend, BackendEvent, Source,
    gmail::{self, Gmail, GmailError, History, ThreadList},
};
use crate::data::{Folder, Message, Thread};

const COUNTS_KEY: &str = "category_counts";

/// Counts stop here: "500+".
pub const COUNT_CAP: u32 = 500;
/// Pages read at most per category, in case Gmail answers with short ones.
const MAX_PAGES: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    Primary,
    Social,
    Promotions,
    Updates,
    Forums,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Primary,
        Category::Social,
        Category::Promotions,
        Category::Updates,
        Category::Forums,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Primary => "Primary",
            Category::Social => "Social",
            Category::Promotions => "Promotions",
            Category::Updates => "Updates",
            Category::Forums => "Forums",
        }
    }

    /// Gmail's label for it; Primary is having none of the others.
    pub fn label_id(self) -> Option<&'static str> {
        match self {
            Category::Primary => None,
            Category::Social => Some("CATEGORY_SOCIAL"),
            Category::Promotions => Some("CATEGORY_PROMOTIONS"),
            Category::Updates => Some("CATEGORY_UPDATES"),
            Category::Forums => Some("CATEGORY_FORUMS"),
        }
    }

    /// `category:<x>`, as Gmail's search takes it.
    pub fn search_term(self) -> &'static str {
        match self {
            Category::Primary => "category:primary",
            Category::Social => "category:social",
            Category::Promotions => "category:promotions",
            Category::Updates => "category:updates",
            Category::Forums => "category:forums",
        }
    }

    /// The view's threads on the server, for its first page and "Load
    /// older" (LST-7).
    pub fn view_query(self) -> &'static str {
        match self {
            Category::Primary => "in:inbox category:primary",
            Category::Social => "in:inbox category:social",
            Category::Promotions => "in:inbox category:promotions",
            Category::Updates => "in:inbox category:updates",
            Category::Forums => "in:inbox category:forums",
        }
    }

    fn ix(self) -> usize {
        self as usize
    }
}

/// One message's category, from its labels.
pub fn category_of(label_ids: &[String]) -> Category {
    Category::ALL[1..]
        .iter()
        .copied()
        .find(|c| label_ids.iter().any(|l| Some(l.as_str()) == c.label_id()))
        .unwrap_or(Category::Primary)
}

/// Whether any of the thread's Inbox messages is in `category`. A thread
/// none of whose messages carries INBOX yet (moved there here, not synced
/// back) is judged on all of them.
pub fn in_category(thread: &Thread, category: Category) -> bool {
    let in_inbox = |m: &&Message| m.label_ids.iter().any(|l| l == gmail::INBOX);
    let any_inbox = thread.messages.iter().any(|m| in_inbox(&m));
    thread
        .messages
        .iter()
        .filter(|m| !any_inbox || in_inbox(m))
        .any(|m| category_of(&m.label_ids) == category)
}

/// Unread Inbox threads per category, capped at `COUNT_CAP`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryCounts {
    unread: [u32; 5],
    /// More than `COUNT_CAP`.
    capped: [bool; 5],
}

impl CategoryCounts {
    pub fn unread(&self, category: Category) -> u32 {
        self.unread[category.ix()]
    }

    /// Past the cap: show "500+".
    #[cfg(test)]
    pub fn capped(&self, category: Category) -> bool {
        self.capped[category.ix()]
    }

    /// For a badge: "12", "500+", or none at zero.
    #[cfg(test)]
    pub fn text(&self, category: Category) -> Option<String> {
        match self.unread(category) {
            0 => None,
            n if self.capped(category) => Some(format!("{n}+")),
            n => Some(n.to_string()),
        }
    }

    fn set(&mut self, category: Category, count: usize, more: bool) {
        let cap = COUNT_CAP as usize;
        self.unread[category.ix()] = count.min(cap) as u32;
        self.capped[category.ix()] = count > cap || (count == cap && more);
    }
}

/// The counts from threads in memory: demo mode.
pub fn local_counts(threads: &[Thread]) -> CategoryCounts {
    let mut counts = CategoryCounts::default();
    for category in Category::ALL {
        let n = threads
            .iter()
            .filter(|t| t.folder == Folder::Inbox && t.unread && in_category(t, category))
            .count();
        counts.set(category, n, false);
    }
    counts
}

/// The counts from Gmail: one `threads.list` per category (10 units
/// each), more only past a short page. Blocking.
pub fn fetch_counts(gmail: &Gmail) -> Result<CategoryCounts, GmailError> {
    let mut counts = CategoryCounts::default();
    for category in Category::ALL {
        let (mut n, mut page) = (0usize, None::<String>);
        for _ in 0..MAX_PAGES {
            let request = gmail::unread_inbox_ids_request(
                category.search_term(),
                COUNT_CAP as usize - n.min(COUNT_CAP as usize),
                page.as_deref(),
            );
            let list: ThreadList = gmail.call(&request)?;
            n += list.threads.len();
            page = list.next_page_token;
            if page.is_none() || n >= COUNT_CAP as usize {
                break;
            }
        }
        counts.set(category, n, page.is_some());
    }
    Ok(counts)
}

/// Whether a page of history changed anything in the Inbox: a message
/// added to, removed from, or changed while in it.
pub fn touches_inbox(history: &[History]) -> bool {
    let inbox = |labels: &[String]| labels.iter().any(|l| l == gmail::INBOX);
    history.iter().any(|h| {
        h.messages_added
            .iter()
            .chain(&h.messages_deleted)
            .chain(&h.labels_added)
            .chain(&h.labels_removed)
            .any(|m| inbox(&m.message.label_ids) || inbox(&m.label_ids))
    })
}

/// The backend's counts and their refresh.
#[derive(Default)]
pub struct CategoryState {
    counts: CategoryCounts,
    /// Gmail's counts are out of date: at launch, and once the Inbox changed.
    due: bool,
    running: bool,
    _refresh: Option<Task<()>>,
}

impl Backend {
    /// Unread Inbox threads per category (FOL-11, NTF-2). Changes emit
    /// `CountsChanged`.
    pub fn category_counts(&self) -> &CategoryCounts {
        &self.categories.counts
    }

    /// At construction: the last known counts at once.
    pub(super) fn start_categories(&mut self) {
        let email = self.email().to_string();
        match self.source {
            Source::Demo => {
                // The Inbox's heads, without bodies (MC-035).
                let threads = self.store.load_inbox_heads(&email, &[]).unwrap_or_default();
                self.categories.counts = local_counts(&threads);
            }
            Source::Google => {
                if let Ok(Some(json)) = self.store.sync_state(&email, COUNTS_KEY) {
                    self.categories.counts = serde_json::from_str(&json).unwrap_or_default();
                }
                self.categories.due = true;
            }
            Source::SignedOut => {}
        }
    }

    /// Sync saw the Inbox change: count again after the pass.
    pub(super) fn inbox_touched(&mut self) {
        self.categories.due = true;
    }

    /// Count again if due: demo at once from the store, Google in the
    /// background. A failed count stays due for the next pass.
    pub(super) fn refresh_category_counts(&mut self, cx: &mut Context<Self>) {
        let email = self.email().to_string();
        if self.source == Source::Demo {
            let threads = self.store.load_inbox_heads(&email, &[]).unwrap_or_default();
            self.set_category_counts(local_counts(&threads), cx);
            return;
        }
        if !self.categories.due || self.categories.running {
            return;
        }
        let Some(transport) = self.transport() else {
            return;
        };
        self.categories.due = false;
        self.categories.running = true;
        let store = self.store.clone();
        self.categories._refresh = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let counts = fetch_counts(&Gmail::new(transport))?;
                    if let Ok(json) = serde_json::to_string(&counts) {
                        store.set_sync_state(&email, COUNTS_KEY, &json).ok();
                    }
                    Ok::<_, GmailError>(counts)
                })
                .await;
            this.update(cx, |this, cx| {
                this.categories.running = false;
                match result {
                    Ok(counts) => this.set_category_counts(counts, cx),
                    Err(e) => {
                        this.categories.due = true;
                        eprintln!("Pigeon: couldn't count the Inbox categories: {e}");
                    }
                }
            })
            .ok();
        }));
    }

    fn set_category_counts(&mut self, counts: CategoryCounts, cx: &mut Context<Self>) {
        if self.categories.counts != counts {
            self.categories.counts = counts;
            cx.emit(BackendEvent::CountsChanged);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{backend::http::FakeTransport, data::Contact};

    fn message(labels: &[&str]) -> Message {
        Message {
            label_ids: labels.iter().map(|l| l.to_string()).collect(),
            ..Message::new(Contact::default())
        }
    }

    fn thread(messages: Vec<Message>) -> Thread {
        Thread::local("t".into(), "S".into(), Folder::Inbox, messages, None)
    }

    #[test]
    fn a_message_without_a_category_is_primary() {
        assert_eq!(category_of(&[]), Category::Primary);
        let labels = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            category_of(&labels(&["INBOX", "UNREAD", "CATEGORY_PERSONAL"])),
            Category::Primary
        );
        assert_eq!(
            category_of(&labels(&["INBOX", "CATEGORY_PROMOTIONS"])),
            Category::Promotions
        );
        assert_eq!(category_of(&labels(&["CATEGORY_FORUMS"])), Category::Forums);
    }

    #[test]
    fn a_mixed_thread_is_in_each_of_its_inbox_messages_categories() {
        let t = thread(vec![
            message(&["INBOX", "CATEGORY_UPDATES"]),
            message(&["INBOX", "UNREAD"]),
            // Archived: no longer counts.
            message(&["CATEGORY_SOCIAL"]),
        ]);
        assert!(in_category(&t, Category::Updates));
        assert!(in_category(&t, Category::Primary));
        assert!(!in_category(&t, Category::Social));
        assert!(!in_category(&t, Category::Promotions));
    }

    #[test]
    fn a_thread_without_inbox_labels_is_judged_on_all_its_messages() {
        let t = thread(vec![message(&["CATEGORY_SOCIAL"]), message(&["SENT"])]);
        assert!(in_category(&t, Category::Social));
        assert!(in_category(&t, Category::Primary));
        let none = thread(vec![message(&["SENT"])]);
        assert!(in_category(&none, Category::Primary));
        assert!(!in_category(&none, Category::Forums));
    }

    #[test]
    fn local_counts_take_unread_inbox_threads() {
        let unread = |labels: &[&str]| {
            let mut t = thread(vec![message(labels)]);
            t.unread = true;
            t
        };
        let promo = unread(&["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"]);
        let primary = unread(&["INBOX", "UNREAD"]);
        let mut read = unread(&["INBOX", "CATEGORY_PROMOTIONS"]);
        read.unread = false;
        let mut archived = unread(&["UNREAD", "CATEGORY_PROMOTIONS"]);
        archived.folder = Folder::Archive;
        let counts = local_counts(&[promo, primary, read, archived]);
        assert_eq!(counts.unread(Category::Promotions), 1);
        assert_eq!(counts.unread(Category::Primary), 1);
        assert_eq!(counts.unread(Category::Social), 0);
        assert_eq!(counts.text(Category::Primary).as_deref(), Some("1"));
        assert_eq!(counts.text(Category::Social), None);
    }

    #[test]
    fn demo_counts_come_from_the_seed() {
        let store = crate::backend::store::Store::open_in_memory().unwrap();
        let account = crate::backend::demo::seed(&store).unwrap();
        let threads = store.load_all_threads(&account.email).unwrap();
        let counts = local_counts(&threads);
        let unread_inbox = threads
            .iter()
            .filter(|t| t.folder == Folder::Inbox && t.unread)
            .count() as u32;
        // Every unread Inbox thread is in some category; the demo has a few
        // of each but Social.
        let sum: u32 = Category::ALL.iter().map(|&c| counts.unread(c)).sum();
        assert!(sum >= unread_inbox);
        assert!(counts.unread(Category::Primary) > 0);
        assert!(counts.unread(Category::Primary) < unread_inbox);
        assert!(counts.unread(Category::Updates) > 0);
        assert!(counts.unread(Category::Forums) > 0);
        assert_eq!(counts.unread(Category::Social), 0);
    }

    fn ids(n: usize, next: Option<&str>) -> String {
        let threads: Vec<String> = (0..n).map(|i| format!(r#"{{"id":"t{i}"}}"#)).collect();
        match next {
            Some(token) => format!(
                r#"{{"threads":[{}],"nextPageToken":"{token}"}}"#,
                threads.join(",")
            ),
            None => format!(r#"{{"threads":[{}]}}"#, threads.join(",")),
        }
    }

    #[test]
    fn counts_page_up_to_the_cap() {
        const PATH: &str = "/gmail/v1/users/me/threads";
        fn q(c: &'static str) -> (&'static str, &'static str) {
            ("q", c)
        }
        let fake = Arc::new(
            FakeTransport::new()
                // Primary: two short pages, then the cap with more to come.
                .on(
                    "GET",
                    PATH,
                    &[q("category%3Aprimary"), ("pageToken", "p2")],
                    &[(200, &ids(200, Some("p3")))],
                )
                .on(
                    "GET",
                    PATH,
                    &[q("category%3Aprimary"), ("pageToken", "p3")],
                    &[(200, &ids(100, Some("p4")))],
                )
                .on(
                    "GET",
                    PATH,
                    &[q("category%3Aprimary")],
                    &[(200, &ids(200, Some("p2")))],
                )
                // Promotions: exactly 500, nothing more.
                .on(
                    "GET",
                    PATH,
                    &[q("category%3Apromotions")],
                    &[(200, &ids(500, None))],
                )
                .on(
                    "GET",
                    PATH,
                    &[q("category%3Aupdates")],
                    &[(200, &ids(3, None))],
                )
                .on("GET", PATH, &[], &[(200, "{}")]),
        );
        let counts = fetch_counts(&Gmail::new(fake.clone())).unwrap();
        assert_eq!(counts.unread(Category::Primary), 500);
        assert!(counts.capped(Category::Primary));
        assert_eq!(counts.text(Category::Primary).as_deref(), Some("500+"));
        assert_eq!(counts.unread(Category::Promotions), 500);
        assert!(!counts.capped(Category::Promotions));
        assert_eq!(counts.unread(Category::Updates), 3);
        assert_eq!(counts.unread(Category::Social), 0);
        let requests = fake.requests();
        // Primary's three pages, one each for the rest.
        assert_eq!(requests.len(), 7);
        assert_eq!(requests.iter().map(|r| r.cost).sum::<u32>(), 70);
        for r in &requests {
            let query = r.query();
            assert!(query.contains(&("labelIds", "INBOX")), "{}", r.url);
            assert!(query.contains(&("labelIds", "UNREAD")), "{}", r.url);
            assert!(
                query.contains(&("fields", "threads%2Fid%2CnextPageToken")),
                "{}",
                r.url
            );
        }
        // Each page asks only for what's left under the cap.
        assert!(requests[2].query().contains(&("maxResults", "100")));
    }

    #[test]
    fn an_empty_answer_counts_as_none() {
        const PATH: &str = "/gmail/v1/users/me/threads";
        let fake = FakeTransport::new()
            .on("GET", PATH, &[("q", "category%3Aforums")], &[(204, "")])
            .on("GET", PATH, &[("q", "category%3Asocial")], &[(200, "")])
            .on(
                "GET",
                PATH,
                &[("q", "category%3Aupdates")],
                &[(200, &ids(4, None))],
            )
            .on("GET", PATH, &[], &[(200, &ids(2, None))]);
        let counts = fetch_counts(&Gmail::new(Arc::new(fake))).unwrap();
        assert_eq!(counts.unread(Category::Forums), 0);
        assert_eq!(counts.unread(Category::Social), 0);
        assert_eq!(counts.unread(Category::Updates), 4);
        assert_eq!(counts.unread(Category::Primary), 2);
        assert_eq!(counts.unread(Category::Promotions), 2);
    }

    #[test]
    fn a_failed_count_is_an_error() {
        let fake = FakeTransport::new().on("GET", "/gmail/v1/users/me/threads", &[], &[(0, "")]);
        assert!(
            fetch_counts(&Gmail::new(Arc::new(fake)))
                .unwrap_err()
                .is_network()
        );
    }

    #[test]
    fn only_inbox_history_asks_for_new_counts() {
        let history: gmail::HistoryList = serde_json::from_str(include_str!(
            "../../tests/fixtures/google/gmail/history.json"
        ))
        .unwrap();
        assert!(touches_inbox(&history.history));
        // The draft deleted and the star: not the Inbox.
        assert!(!touches_inbox(&history.history[2..]));
        // Archiving: INBOX removed, so no longer on the message.
        let archived: Vec<History> = serde_json::from_str(
            r#"[{"labelsRemoved":[{"message":{"id":"m","threadId":"t","labelIds":["UNREAD"]},
                "labelIds":["INBOX"]}]}]"#,
        )
        .unwrap();
        assert!(touches_inbox(&archived));
        assert!(!touches_inbox(&[]));
    }

    #[test]
    fn the_category_views_ask_gmail_by_category() {
        use crate::{backend::ViewKey, search::GmailQuery};
        let query = GmailQuery::view(&ViewKey::Category(Category::Primary)).unwrap();
        assert_eq!(query.q, "in:inbox category:primary");
        assert_eq!(query.label, None);
        let request = gmail::search_request(&query, 50, None);
        assert!(
            request
                .query()
                .contains(&("q", "in%3Ainbox%20category%3Aprimary"))
        );
    }
}

//! Everyone the user could write to, and the suggestions recipient fields
//! show while typing. The app shares one book (`SharedContactBook`) with
//! every composer and the event panel, and updates it in place when the
//! contacts change (CON-1).

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use nucleo_matcher::{
    Config, Matcher, Utf32String,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

use crate::data::Contact;

const MAX_SUGGESTIONS: usize = 6;

/// Queries shorter than this match only the starts of words: one or two
/// letters inside words ("ra" in "insurance") match nearly everyone.
const MIN_SUBSTRING_QUERY: usize = 3;
const MIN_FUZZY_QUERY: usize = 3;

/// The one book the app, its composers and the event panel share.
pub type SharedContactBook = Rc<RefCell<ContactBook>>;

/// Everyone the user could write to, with what suggestions match against.
/// Shared by the composer's recipient fields and the event panel's guests.
///
/// A contact keeps its index for the book's life, so suggestion lists made
/// before an update still point at the right people.
pub struct ContactBook {
    pub(crate) contacts: Vec<Contact>,
    keys: Vec<ContactKey>,
    haystacks: Vec<Utf32String>,
    /// Each contact's place in the ranking, best first; `None` once it has
    /// left the book.
    rank: Vec<Option<usize>>,
    matcher: RefCell<Matcher>,
}

/// A contact's lowercased name and address, and the words in both.
struct ContactKey {
    name: String,
    email: String,
    words: Vec<String>,
}

impl ContactKey {
    fn new(contact: &Contact) -> Self {
        let name = contact.name.to_lowercase();
        let email = contact.email.to_lowercase();
        let words = name
            .split(|c: char| !c.is_alphanumeric())
            .chain(email.split(|c: char| !c.is_alphanumeric()))
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        Self { name, email, words }
    }

    /// How well `query` (lowercased, trimmed) matches: 0 when the name or
    /// address starts with it, 1 when every word of it starts a word of
    /// theirs, 2 when it's anywhere in them.
    fn tier(&self, query: &str) -> Option<u8> {
        if self.name.starts_with(query) || self.email.starts_with(query) {
            Some(0)
        } else if query
            .split_whitespace()
            .all(|term| self.words.iter().any(|w| w.starts_with(term)))
        {
            Some(1)
        } else if query.chars().count() >= MIN_SUBSTRING_QUERY
            && (self.name.contains(query) || self.email.contains(query))
        {
            Some(2)
        } else {
            None
        }
    }
}

/// Addresses that send but aren't read: no-reply senders, notifications,
/// newsletters and the like. Never suggested.
pub fn is_automated(email: &str) -> bool {
    const PARTS: [&str; 9] = [
        "noreply",
        "tracking",
        "no-reply",
        "no_reply",
        "donotreply",
        "do-not-reply",
        "notification",
        "newsletter",
        "mailer-daemon",
    ];
    const NAMES: [&str; 9] = [
        "news",
        "digest",
        "alerts",
        "updates",
        "receipts",
        "offers",
        "marketing",
        "bounce",
        "notifications",
    ];
    let local = email.split('@').next().unwrap_or_default().to_lowercase();
    PARTS.iter().any(|p| local.contains(p)) || NAMES.contains(&local.as_str())
}

fn haystack(contact: &Contact) -> Utf32String {
    format!("{} {}", contact.name, contact.email).into()
}

impl ContactBook {
    /// `contacts` best first.
    pub fn new(contacts: Vec<Contact>) -> Self {
        let mut book = Self {
            contacts: Vec::new(),
            keys: Vec::new(),
            haystacks: Vec::new(),
            rank: Vec::new(),
            matcher: RefCell::new(Matcher::new(Config::DEFAULT)),
        };
        book.update(contacts);
        book
    }

    pub fn shared(contacts: Vec<Contact>) -> SharedContactBook {
        Rc::new(RefCell::new(Self::new(contacts)))
    }

    /// Take `contacts` (best first) as the book: known addresses keep their
    /// index and take the new name and rank, new ones join, and those not
    /// listed are no longer suggested.
    pub fn update(&mut self, contacts: Vec<Contact>) {
        let mut ix_of: HashMap<String, usize> = self
            .contacts
            .iter()
            .enumerate()
            .map(|(ix, c)| (c.email.to_lowercase(), ix))
            .collect();
        let mut rank = vec![None; self.contacts.len()];
        let mut next = 0;
        for contact in contacts {
            if is_automated(&contact.email) {
                continue;
            }
            let ix = match ix_of.get(&contact.email.to_lowercase()) {
                Some(&ix) => {
                    if rank[ix].is_some() {
                        continue;
                    }
                    if self.contacts[ix] != contact {
                        self.keys[ix] = ContactKey::new(&contact);
                        self.haystacks[ix] = haystack(&contact);
                        self.contacts[ix] = contact;
                    }
                    ix
                }
                None => {
                    ix_of.insert(contact.email.to_lowercase(), self.contacts.len());
                    self.keys.push(ContactKey::new(&contact));
                    self.haystacks.push(haystack(&contact));
                    self.contacts.push(contact);
                    rank.push(None);
                    self.contacts.len() - 1
                }
            };
            rank[ix] = Some(next);
            next += 1;
        }
        self.rank = rank;
    }

    pub fn contact(&self, ix: usize) -> &Contact {
        &self.contacts[ix]
    }

    pub fn find(&self, email: &str) -> Option<&Contact> {
        self.contacts
            .iter()
            .zip(&self.rank)
            .find(|(c, rank)| rank.is_some() && c.email.eq_ignore_ascii_case(email))
            .map(|(c, _)| c)
    }

    /// The best matches for `query` that aren't `taken`, best first. Names
    /// and addresses that start with the query come first, then those with
    /// a word that does, then those that contain it anywhere; only when
    /// none do is it matched fuzzily. Ties keep the book's order, most
    /// frequent first.
    pub fn suggest(&self, query: &str, taken: impl Fn(&Contact) -> bool) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(u8, usize)> = self
            .keys
            .iter()
            .enumerate()
            .filter(|&(ix, _)| self.rank[ix].is_some())
            .filter_map(|(ix, key)| Some((key.tier(&query)?, ix)))
            .filter(|&(_, ix)| !taken(&self.contacts[ix]))
            .collect();
        found.sort_by_key(|&(tier, ix)| (tier, self.rank[ix]));
        if found.is_empty() && query.chars().count() >= MIN_FUZZY_QUERY {
            found = self.fuzzy(&query, &taken);
        }
        found
            .into_iter()
            .take(MAX_SUGGESTIONS)
            .map(|(_, ix)| ix)
            .collect()
    }

    fn fuzzy(&self, query: &str, taken: &impl Fn(&Contact) -> bool) -> Vec<(u8, usize)> {
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let matcher = &mut *self.matcher.borrow_mut();
        let mut scored: Vec<(u32, usize)> = self
            .haystacks
            .iter()
            .enumerate()
            .filter(|&(ix, _)| self.rank[ix].is_some())
            .filter_map(|(ix, h)| Some((pattern.score(h.slice(..), matcher)?, ix)))
            .filter(|&(_, ix)| !taken(&self.contacts[ix]))
            .collect();
        scored.sort_by_key(|&(score, ix)| (std::cmp::Reverse(score), self.rank[ix]));
        scored.into_iter().map(|(_, ix)| (3, ix)).collect()
    }

    /// The contact a typed name most likely means.
    pub fn best(&self, query: &str) -> Option<&Contact> {
        self.suggest(query, |_| false)
            .first()
            .map(|&ix| &self.contacts[ix])
    }
}

/// What the contacts sync keeps beyond names and addresses (CON-5), from
/// the People API's `phoneNumbers` and `organizations`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContactDetails {
    #[serde(default)]
    pub phones: Vec<Phone>,
    #[serde(default)]
    pub organization: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Phone {
    pub number: String,
    /// "mobile", "work"…; may be empty.
    #[serde(default)]
    pub label: String,
}

/// A conversation with someone, for their card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardThread {
    pub id: String,
    pub subject: String,
    /// The newest message's internal date, ms.
    pub last_date: i64,
}

/// The contact card (CON-5): what the local cache knows about someone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactCard {
    pub name: String,
    /// The address it was opened for first.
    pub emails: Vec<String>,
    pub details: ContactDetails,
    /// One of the user's contacts (not only an address seen in mail).
    pub is_contact: bool,
    /// Newest first.
    pub threads: Vec<CardThread>,
}

impl ContactCard {
    pub fn contact(&self) -> Contact {
        Contact {
            name: self.name.clone(),
            email: self.emails.first().cloned().unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> ContactBook {
        let c = |name: &str, email: &str| Contact {
            name: name.into(),
            email: email.into(),
        };
        ContactBook::new(vec![
            c("Amazon Web Services", "no-reply@aws.amazon.com"),
            c("Rustacean Weekly", "newsletter@rustacean-weekly.dev"),
            c("Harbor Mutual Insurance", "renewals@harbormutual.com"),
            c("Sarah Chen", "sarah.chen@acme.io"),
            c("Rahul Mehta", "rahul@mehta.dev"),
            c("Mateo Ruiz", "mateo@ruizpadel.es"),
        ])
    }

    fn names(book: &ContactBook, query: &str) -> Vec<String> {
        book.suggest(query, |_| false)
            .into_iter()
            .map(|ix| book.contact(ix).name.clone())
            .collect()
    }

    #[test]
    fn prefixes_rank_before_substrings_and_automated_senders_are_left_out() {
        let book = book();
        assert_eq!(names(&book, "ra"), ["Rahul Mehta"]);
        assert_eq!(names(&book, "rah"), ["Rahul Mehta", "Sarah Chen"]);
        assert_eq!(names(&book, "ru"), ["Mateo Ruiz"]);
        assert_eq!(names(&book, "chen"), ["Sarah Chen"]);
        assert_eq!(names(&book, "sarah c"), ["Sarah Chen"]);
        assert!(names(&book, "amazon").is_empty());
    }

    #[test]
    fn fuzzy_only_as_a_last_resort() {
        let book = book();
        assert_eq!(names(&book, "rhl")[0], "Rahul Mehta");
        assert!(names(&book, "zq").is_empty());
    }

    #[test]
    fn updates_rerank_in_place_and_keep_indices() {
        let mut book = book();
        let sarah = book.suggest("sarah", |_| false)[0];
        let c = |name: &str, email: &str| Contact {
            name: name.into(),
            email: email.into(),
        };
        // Rahul now ranks first, Mateo left, Sara Lee is new.
        book.update(vec![
            c("Rahul Mehta", "rahul@mehta.dev"),
            c("Sarah Chen-Li", "SARAH.CHEN@acme.io"),
            c("Sara Lee", "sara@lee.dev"),
        ]);
        assert_eq!(book.contact(sarah).name, "Sarah Chen-Li");
        assert_eq!(names(&book, "sar"), ["Sarah Chen-Li", "Sara Lee"]);
        assert_eq!(names(&book, "r"), ["Rahul Mehta"]);
        assert!(names(&book, "mateo").is_empty());
        assert!(book.find("mateo@ruizpadel.es").is_none());
        // Ties go by the new rank.
        book.update(vec![
            c("Sara Lee", "sara@lee.dev"),
            c("Sarah Chen", "sarah.chen@acme.io"),
        ]);
        assert_eq!(names(&book, "sar"), ["Sara Lee", "Sarah Chen"]);
    }

    #[test]
    fn aliases_are_me() {
        let account = crate::data::Account {
            name: "Me".into(),
            email: "me@example.com".into(),
            aliases: vec!["me.work@example.com".into()],
            send_as: Vec::new(),
        };
        assert!(account.is_me("ME@example.com"));
        assert!(account.is_me("Me.Work@Example.com"));
        assert!(!account.is_me("sarah.chen@acme.io"));
    }

    #[test]
    fn automated_addresses() {
        assert!(is_automated("noreply@github.com"));
        assert!(is_automated("drive-shares-noreply@google.com"));
        assert!(is_automated("calendar-notification@google.com"));
        assert!(is_automated("news@sketchwave.app"));
        assert!(!is_automated("sarah.chen@acme.io"));
        assert!(!is_automated("newsom@example.com"));
    }
}

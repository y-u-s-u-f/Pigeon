//! Everyone the user could write to, and the suggestions recipient fields
//! show while typing. Moved out of `ui/composer.rs` as is.

use std::cell::RefCell;

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

/// Everyone the user could write to, with what suggestions match against.
/// Shared by the composer's recipient fields and the event panel's guests.
pub struct ContactBook {
    pub(crate) contacts: Vec<Contact>,
    keys: Vec<ContactKey>,
    haystacks: Vec<Utf32String>,
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

impl ContactBook {
    pub fn new(contacts: Vec<Contact>) -> Self {
        let contacts: Vec<Contact> = contacts
            .into_iter()
            .filter(|c| !is_automated(&c.email))
            .collect();
        let keys = contacts.iter().map(ContactKey::new).collect();
        let haystacks = contacts
            .iter()
            .map(|c| format!("{} {}", c.name, c.email).into())
            .collect();
        Self {
            contacts,
            keys,
            haystacks,
            matcher: RefCell::new(Matcher::new(Config::DEFAULT)),
        }
    }

    pub fn contact(&self, ix: usize) -> &Contact {
        &self.contacts[ix]
    }

    pub fn find(&self, email: &str) -> Option<&Contact> {
        self.contacts
            .iter()
            .find(|c| c.email.eq_ignore_ascii_case(email))
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
            .filter_map(|(ix, key)| Some((key.tier(&query)?, ix)))
            .filter(|&(_, ix)| !taken(&self.contacts[ix]))
            .collect();
        found.sort_by_key(|&(tier, _)| tier);
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
            .filter_map(|(ix, h)| Some((pattern.score(h.slice(..), matcher)?, ix)))
            .filter(|&(_, ix)| !taken(&self.contacts[ix]))
            .collect();
        scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
        scored.into_iter().map(|(_, ix)| (3, ix)).collect()
    }

    /// The contact a typed name most likely means.
    pub fn best(&self, query: &str) -> Option<&Contact> {
        self.suggest(query, |_| false)
            .first()
            .map(|&ix| &self.contacts[ix])
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
    fn automated_addresses() {
        assert!(is_automated("noreply@github.com"));
        assert!(is_automated("drive-shares-noreply@google.com"));
        assert!(is_automated("calendar-notification@google.com"));
        assert!(is_automated("news@sketchwave.app"));
        assert!(!is_automated("sarah.chen@acme.io"));
        assert!(!is_automated("newsom@example.com"));
    }
}

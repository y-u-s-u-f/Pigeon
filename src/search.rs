//! Search queries: free text plus a few Gmail-style operators, matched against
//! each thread's precomputed `SearchIndex`. Terms AND together. Signed in,
//! Gmail is asked too (`to_gmail_q`, `ServerSearch`), for mail outside the
//! cache.

use std::{collections::HashSet, time::Duration};

use gpui::{Task, actions};

use crate::{
    backend::{
        SearchResult, ViewKey,
        gmail::{self, GmailError},
    },
    data::{Folder, Label, MailData, Thread},
};

actions!(search, [FocusResults, NextSuggestion, PrevSuggestion]);

#[derive(Debug, PartialEq)]
enum Term {
    Text(String),
    From(String),
    To(String),
    Subject(String),
    /// As typed, and `normalize`d for matching.
    Label {
        raw: String,
        norm: String,
    },
    Unread,
    Starred,
    Attachment,
}

/// Where a query looks, when it says so with `in:`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Folder(Folder),
    /// Everything but spam and trash.
    All,
    /// Everything.
    Anywhere,
}

#[derive(Debug, Default, PartialEq)]
pub struct Query {
    terms: Vec<Term>,
    /// Set by `in:`; otherwise the current view is searched.
    pub scope: Option<Scope>,
    /// W4b (SRC-9): the query as Gmail takes it, when it has operators only
    /// Gmail can match (`gmail_only`): as typed, without the `in:` that
    /// set `scope`.
    gmail_q: Option<String>,
    /// Their chips.
    gmail_chips: Vec<String>,
}

/// Operators only Gmail can match (SRC-9); `is:important` too.
const GMAIL_KEYS: [&str; 11] = [
    "after",
    "before",
    "older_than",
    "newer_than",
    "larger",
    "smaller",
    "filename",
    "cc",
    "bcc",
    "list",
    "category",
];

/// A token only Gmail understands: one of `GMAIL_KEYS`, `is:important`,
/// a `-` negation, `OR`, or grouping with parentheses or braces.
fn gmail_only(token: &str) -> bool {
    if token == "OR" || token.contains(['(', ')', '{', '}']) {
        return true;
    }
    if token.len() > 1 && token.starts_with('-') {
        return true;
    }
    match token.split_once(':') {
        Some((key, value)) if !value.is_empty() => {
            let key = key.to_lowercase();
            GMAIL_KEYS.contains(&key.as_str())
                || (key == "is" && value.eq_ignore_ascii_case("important"))
        }
        _ => false,
    }
}

/// How a Gmail-only token reads as a chip: "not from:bob", "OR".
fn gmail_chip(token: &str) -> Option<String> {
    let bare = token.trim_matches(['(', ')', '{', '}']);
    if bare.is_empty() {
        return None;
    }
    Some(match bare.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => format!("not {rest}"),
        _ => bare.to_string(),
    })
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let mut query = Query::default();
        let raw = tokenize(input, true);
        if raw.iter().any(|t| gmail_only(t)) {
            let operator = |t: &str| {
                gmail_only(t)
                    || (!sets_scope(t)
                        && t.split_once(':')
                            .is_some_and(|(k, v)| !k.is_empty() && !v.is_empty()))
            };
            let mut chips: Vec<String> = raw
                .iter()
                .filter(|t| operator(t))
                .filter_map(|t| gmail_chip(t))
                .collect();
            if raw.iter().any(|t| t.contains(['(', ')', '{', '}'])) {
                chips.push("( … )".into());
            }
            query.gmail_chips = chips;
            query.gmail_q = Some(
                raw.iter()
                    .filter(|t| !sets_scope(t))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        for token in tokenize(input, false) {
            let (key, value) = match token.split_once(':') {
                // An operator still waiting for its value, as it's typed.
                Some((key, ""))
                    if ["from", "to", "subject", "label", "is", "has", "in"]
                        .contains(&key.to_lowercase().as_str()) =>
                {
                    continue;
                }
                Some((key, value)) if !value.is_empty() => (key.to_lowercase(), value.to_string()),
                _ => (String::new(), token.clone()),
            };
            let value = value.to_lowercase();
            let term = match (key.as_str(), value.as_str()) {
                ("from", _) => Term::From(value),
                ("to", _) => Term::To(value),
                ("subject", _) => Term::Subject(value),
                ("label", _) => Term::label(value),
                ("is", "unread") => Term::Unread,
                ("is", "starred") => Term::Starred,
                ("has", "attachment" | "attachments") => Term::Attachment,
                ("in", scope) => {
                    query.scope = Some(match (folder_scope(scope), scope) {
                        (Some(scope), _) => scope,
                        (None, "starred") => {
                            query.terms.push(Term::Starred);
                            Scope::All
                        }
                        // Like Gmail, `in:` also takes a label.
                        (None, label) => {
                            query.terms.push(Term::label(label.to_string()));
                            Scope::All
                        }
                    });
                    continue;
                }
                _ => Term::Text(token.to_lowercase()),
            };
            query.terms.push(term);
        }
        query
    }

    /// The operators the query recognized, as they read: "from: sarah",
    /// "is:unread", "in: Archive". Free text is left out.
    pub fn operators(&self) -> Vec<String> {
        let mut out: Vec<String> = self.gmail_chips.clone();
        if !self.gmail_only() {
            out.extend(self.terms.iter().filter_map(|term| match term {
                Term::Text(_) => None,
                Term::From(v) => Some(format!("from: {v}")),
                Term::To(v) => Some(format!("to: {v}")),
                Term::Subject(v) => Some(format!("subject: {v}")),
                Term::Label { raw, .. } => Some(format!("label: {raw}")),
                Term::Unread => Some("is:unread".into()),
                Term::Starred => Some("is:starred".into()),
                Term::Attachment => Some("has:attachment".into()),
            }));
        }
        if let Some(scope) = self.scope {
            out.push(match scope {
                Scope::Folder(folder) => format!("in: {}", folder.title()),
                Scope::All => "in: all mail".into(),
                Scope::Anywhere => "in: anywhere".into(),
            });
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.scope.is_none()
    }

    /// W4b (SRC-9): only Gmail can answer it, so only Gmail's results show.
    pub fn gmail_only(&self) -> bool {
        self.gmail_q.is_some()
    }

    /// The thread is where `scope` looks, the way Gmail's half of the search
    /// (`GmailQuery::folder`) finds it: Sent by any message the user sent,
    /// Archive by being in no folder, so a sent-only conversation counts.
    /// Labels and stars are terms, matched on the thread's own.
    pub fn in_scope(scope: Scope, thread: &Thread) -> bool {
        match scope {
            Scope::Folder(Folder::Sent) => thread.in_sent(),
            Scope::Folder(Folder::Archive) => {
                matches!(thread.folder, Folder::Archive | Folder::Sent)
            }
            Scope::Folder(folder) => thread.folder == folder,
            Scope::All => !thread.is_hidden(),
            Scope::Anywhere => true,
        }
    }

    /// The query as Gmail's `threads.list` takes it (SRC-2/3/4). Without an
    /// `in:`, it looks in `view` (`None`: all mail). `labels` turn `label:`
    /// into Gmail's label names. `None` where Gmail has nothing to add: the
    /// Drafts, which sync whole.
    pub fn to_gmail_q(&self, labels: &[Label], view: Option<&ViewKey>) -> Option<GmailQuery> {
        let mut out = match (self.scope, view) {
            (Some(Scope::Anywhere), _) => GmailQuery {
                include_spam_trash: true,
                ..Default::default()
            },
            (Some(Scope::All), _) | (None, None) => GmailQuery::default(),
            // The Inbox's search covers its archived threads too
            // (`refresh_rows`); a Gmail-only one, the Inbox alone, since
            // only Gmail's results show.
            (None, Some(ViewKey::Folder(Folder::Inbox))) if self.gmail_q.is_none() => {
                GmailQuery::default()
            }
            (Some(Scope::Folder(folder)), _) => GmailQuery::folder(folder)?,
            (None, Some(view)) => GmailQuery::view(view)?,
        };
        // SRC-9: as typed, grouped so the view's own terms apply to all of it.
        if let Some(q) = &self.gmail_q {
            out.q = match out.q.as_str() {
                "" => q.clone(),
                scope => format!("({q}) {scope}"),
            };
            return Some(out);
        }
        let mut parts: Vec<String> = self
            .terms
            .iter()
            .map(|term| match term {
                Term::Text(text) => quote(text),
                Term::From(v) => format!("from:{}", quote(v)),
                Term::To(v) => format!("to:{}", quote(v)),
                Term::Subject(v) => format!("subject:{}", quote(v)),
                Term::Label { raw, .. } => gmail_label(raw, labels),
                Term::Unread => "is:unread".into(),
                Term::Starred => "is:starred".into(),
                Term::Attachment => "has:attachment".into(),
            })
            .collect();
        if !out.q.is_empty() {
            parts.push(std::mem::take(&mut out.q));
        }
        out.q = parts.join(" ");
        Some(out)
    }

    pub fn matches(&self, thread: &Thread, data: &MailData) -> bool {
        let index = &thread.search;
        self.terms.iter().all(|term| match term {
            Term::Text(text) => index.all.contains(text.as_str()),
            Term::From(text) => index.from.contains(text.as_str()),
            Term::To(text) => index.to.contains(text.as_str()),
            Term::Subject(text) => index.subject.contains(text.as_str()),
            Term::Label { norm, .. } => thread.labels.iter().any(|id| {
                normalized_starts_with(id, norm)
                    || data
                        .label(id)
                        .is_some_and(|l| normalized_starts_with(&l.name, norm))
            }),
            Term::Unread => thread.unread,
            Term::Starred => thread.starred,
            Term::Attachment => thread.has_attachments(),
        })
    }
}

impl Term {
    fn label(raw: String) -> Self {
        let norm = normalize(&raw);
        Term::Label { raw, norm }
    }
}

/// "Side Projects", "side-projects" and "sideprojects" all compare equal.
fn normalize(text: &str) -> String {
    normalized_chars(text).collect()
}

fn normalized_chars(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
}

/// `normalize(text).starts_with(prefix)`, without building the string.
fn normalized_starts_with(text: &str, prefix: &str) -> bool {
    let mut chars = normalized_chars(text);
    prefix.chars().all(|p| chars.next() == Some(p))
}

/// Gmail's archive: in no folder (SRC-4).
const ARCHIVE_Q: &str = "-in:inbox -in:spam -in:trash -in:drafts";

/// A query in `threads.list`'s terms: `q`, `labelIds` and
/// `includeSpamTrash`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GmailQuery {
    pub q: String,
    pub label: Option<String>,
    pub include_spam_trash: bool,
}

impl GmailQuery {
    fn label(id: &str, include_spam_trash: bool) -> Self {
        Self {
            label: Some(id.into()),
            include_spam_trash,
            ..Default::default()
        }
    }

    /// The folder's threads. `None` for Drafts, which sync whole.
    pub fn folder(folder: Folder) -> Option<Self> {
        Some(match folder {
            Folder::Inbox => Self::label(gmail::INBOX, false),
            Folder::Sent => Self::label(gmail::SENT, false),
            Folder::Spam => Self::label(gmail::SPAM, true),
            Folder::Trash => Self::label(gmail::TRASH, true),
            Folder::Archive => Self {
                q: ARCHIVE_Q.into(),
                ..Default::default()
            },
            Folder::Drafts => return None,
        })
    }

    /// A view's threads (LST-7).
    pub fn view(view: &ViewKey) -> Option<Self> {
        match view {
            ViewKey::Folder(folder) => Self::folder(*folder),
            ViewKey::Starred => Some(Self::label(gmail::STARRED, false)),
            ViewKey::Label(id) => Some(Self::label(id, false)),
            ViewKey::Category(category) => Some(Self {
                q: category.view_query().into(),
                ..Default::default()
            }),
        }
    }
}

/// A value as Gmail reads it: phrases in quotes.
fn quote(text: &str) -> String {
    if text.contains(char::is_whitespace) {
        format!("\"{text}\"")
    } else {
        text.to_string()
    }
}

/// `label:` with the Gmail name of the label it means, quoted; the way
/// `matches` finds it: by name, else by the start of the name or id.
fn gmail_label(wanted: &str, labels: &[Label]) -> String {
    let wanted_n = normalize(wanted);
    let mut found: Vec<&Label> = labels
        .iter()
        .filter(|l| normalize(&l.name) == wanted_n)
        .collect();
    if found.is_empty() {
        found = labels
            .iter()
            .filter(|l| {
                normalize(&l.name).starts_with(&wanted_n) || normalize(&l.id).starts_with(&wanted_n)
            })
            .collect();
    }
    let term = |name: &str| format!("label:\"{name}\"");
    match found[..] {
        [] => term(wanted),
        [one] => term(&one.name),
        // Any of them, as Gmail writes OR.
        _ => {
            let any: Vec<String> = found.iter().map(|l| term(&l.name)).collect();
            format!("{{{}}}", any.join(" "))
        }
    }
}

/// How long typing settles before Gmail is asked (SRC-8).
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Where Gmail's half of the open search is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ServerState {
    /// Nothing asked: no search, or demo data.
    #[default]
    Idle,
    /// "Searching Gmail…" (SRC-7).
    Waiting,
    Done,
    /// Only the downloaded mail was searched.
    Offline,
    Failed,
}

/// Gmail's side of the open search (SRC-5…8): the threads it found, which
/// `merge` adds to the local matches.
#[derive(Default)]
pub struct ServerSearch {
    /// Signed in to Google: there's a Gmail to ask, and views page (LST-7).
    pub available: bool,
    pub state: ServerState,
    /// The query asked; asking for the same one again does nothing.
    query: Option<GmailQuery>,
    /// Counts the queries asked; an answer to an older one is dropped.
    serial: u64,
    found: HashSet<String>,
    /// Gmail's estimate of the matches in all (SRC-6).
    pub estimate: Option<usize>,
    next_page: Option<String>,
    /// The debounce, then the request. A newer query drops it.
    pub task: Option<Task<()>>,
}

impl ServerSearch {
    /// A new query (`None`: no search). Forgets the last one's results and
    /// gives the serial to answer with, when Gmail is to be asked.
    pub fn ask(&mut self, query: Option<GmailQuery>) -> Option<u64> {
        if query == self.query {
            return None;
        }
        self.serial += 1;
        self.found.clear();
        self.estimate = None;
        self.next_page = None;
        self.task = None;
        self.state = if query.is_some() {
            ServerState::Waiting
        } else {
            ServerState::Idle
        };
        self.query = query;
        self.query.as_ref().map(|_| self.serial)
    }

    /// The next page of results, if Gmail has one and none is on its way.
    pub fn more(&mut self) -> Option<(u64, GmailQuery, String)> {
        if matches!(self.state, ServerState::Idle | ServerState::Waiting) {
            return None;
        }
        let page = self.next_page.clone()?;
        self.state = ServerState::Waiting;
        Some((self.serial, self.query.clone()?, page))
    }

    /// Gmail has another page, or it's on its way.
    pub fn has_more(&self) -> bool {
        self.next_page.is_some()
    }

    /// Gmail's answer to the query `serial` asked. False, and dropped, when
    /// a newer query has been asked since (SRC-8).
    pub fn answer(&mut self, serial: u64, result: Result<SearchResult, GmailError>) -> bool {
        if serial != self.serial || self.query.is_none() {
            return false;
        }
        match result {
            Ok(result) => {
                self.found.extend(result.thread_ids);
                self.estimate = result.estimate;
                self.next_page = result.next_page;
                self.state = ServerState::Done;
            }
            Err(e) if e.is_network() => self.state = ServerState::Offline,
            Err(_) => self.state = ServerState::Failed,
        }
        true
    }

    /// The threads a search shows, by index, newest first as `threads` is:
    /// those in scope that match here or that Gmail found, each once.
    pub fn merge(
        &self,
        threads: &[Thread],
        in_scope: impl Fn(&Thread) -> bool,
        matches: impl Fn(&Thread) -> bool,
    ) -> Vec<usize> {
        (0..threads.len())
            .filter(|&ix| {
                let t = &threads[ix];
                in_scope(t) && (self.found.contains(&t.id) || matches(t))
            })
            .collect()
    }
}

/// The scope an `in:` value sets: a folder, `all` or `anywhere`.
/// `in:starred` and `in:<label>` are terms instead.
fn folder_scope(value: &str) -> Option<Scope> {
    Some(match value {
        "inbox" => Scope::Folder(Folder::Inbox),
        "archive" | "archived" => Scope::Folder(Folder::Archive),
        "sent" => Scope::Folder(Folder::Sent),
        "drafts" | "draft" => Scope::Folder(Folder::Drafts),
        "spam" => Scope::Folder(Folder::Spam),
        "trash" => Scope::Folder(Folder::Trash),
        "all" => Scope::All,
        "anywhere" => Scope::Anywhere,
        _ => return None,
    })
}

/// An `in:` that `parse` takes as the scope (`folder_scope`). `in:<label>`
/// stays in the query.
fn sets_scope(token: &str) -> bool {
    token.split_once(':').is_some_and(|(key, value)| {
        key.eq_ignore_ascii_case("in") && folder_scope(&value.to_lowercase()).is_some()
    })
}

/// Splits on whitespace, keeping "quoted phrases" (also after `key:`)
/// whole; with `keep_quotes`, the quotes stay in, for Gmail.
fn tokenize(input: &str, keep_quotes: bool) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                if keep_quotes {
                    current.push(c);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators_and_phrases() {
        let q = Query::parse(r#"from:"Sarah Chen" is:unread budget in:all"#);
        assert_eq!(
            q.terms,
            [
                Term::From("sarah chen".into()),
                Term::Unread,
                Term::Text("budget".into())
            ]
        );
        assert_eq!(q.scope, Some(Scope::All));
        assert_eq!(Query::parse("  ").terms, []);
        assert_eq!(Query::parse("re:").terms, [Term::Text("re:".into())]);
        assert_eq!(
            Query::parse("budget from:").terms,
            [Term::Text("budget".into())]
        );
        assert_eq!(
            q.operators(),
            ["from: sarah chen", "is:unread", "in: all mail"]
        );
    }

    fn label(id: &str, name: &str) -> Label {
        Label::new(id, name, gpui::black())
    }

    #[test]
    fn gmail_only_operators_are_detected() {
        for input in [
            "after:2026/01/01",
            "before:2026/01/01 budget",
            "older_than:1y",
            "newer_than:2d",
            "larger:5M",
            "smaller:100K",
            "filename:pdf",
            "cc:ana",
            "bcc:ana",
            "list:rust",
            "category:promotions",
            "is:important",
            "-from:bob",
            "-meeting",
            "a OR b",
            "(a b)",
            "{a b}",
        ] {
            assert!(Query::parse(input).gmail_only(), "{input}");
        }
        for input in [
            "budget",
            "from:bob",
            "is:unread",
            "e-mail",
            "or",
            "a - b",
            "after:",
        ] {
            assert!(!Query::parse(input).gmail_only(), "{input}");
        }
        assert_eq!(
            Query::parse("(from:ana OR from:bo) -is:unread larger:5M").operators(),
            [
                "from:ana",
                "OR",
                "from:bo",
                "not is:unread",
                "larger:5M",
                "( … )"
            ]
        );
    }

    #[test]
    fn gmail_only_queries_pass_through_with_the_view_scope() {
        let labels = [label("Label_2", "Work")];
        let q = |input: &str, view: Option<&ViewKey>| {
            Query::parse(input).to_gmail_q(&labels, view).unwrap()
        };
        let just = |text: &str| GmailQuery {
            q: text.into(),
            ..Default::default()
        };
        let typed = r#"(from:ana OR from:"Bo Li") -is:unread"#;
        // As typed, quotes and case kept.
        assert_eq!(q(typed, None), just(typed));
        // The Inbox: the Inbox alone, every category.
        assert_eq!(
            q(typed, Some(&ViewKey::Folder(Folder::Inbox))),
            GmailQuery {
                q: typed.into(),
                label: Some("INBOX".into()),
                include_spam_trash: false,
            }
        );
        // A label view: by label id, the query untouched.
        assert_eq!(
            q(typed, Some(&ViewKey::Label("Label_2".into()))),
            GmailQuery {
                q: typed.into(),
                label: Some("Label_2".into()),
                include_spam_trash: false,
            }
        );
        // A view with its own terms: the typed query grouped before them.
        assert_eq!(
            q("a OR b", Some(&ViewKey::Folder(Folder::Archive))),
            just("(a OR b) -in:inbox -in:spam -in:trash -in:drafts")
        );
        assert_eq!(
            q(
                "-from:shop",
                Some(&ViewKey::Category(
                    crate::backend::categories::Category::Promotions
                ))
            ),
            just("(-from:shop) in:inbox category:promotions")
        );
        // `in:` sets the scope and leaves the query; `in:<label>` stays.
        assert_eq!(
            q("after:2026/01/01 in:sent", Some(&ViewKey::Starred)),
            GmailQuery {
                q: "after:2026/01/01".into(),
                label: Some("SENT".into()),
                include_spam_trash: false,
            }
        );
        assert_eq!(
            q("in:inbox category:social", None),
            GmailQuery {
                q: "category:social".into(),
                label: Some("INBOX".into()),
                include_spam_trash: false,
            }
        );
        assert_eq!(q("in:work -x", None), just("in:work -x"));
    }

    #[test]
    fn gmail_only_q_per_scope() {
        use crate::backend::categories::Category;
        let labels = [label("Label_2", "Work")];
        let typed = r#"after:2026/10/01 subject:"Pigeon test""#;
        let sent = |view: Option<ViewKey>| {
            Query::parse(typed)
                .to_gmail_q(&labels, view.as_ref())
                .map(|g| (g.q, g.label, g.include_spam_trash))
        };
        let with = |q: &str, label: Option<&str>, st: bool| {
            Some((q.to_string(), label.map(String::from), st))
        };
        let cases = [
            (
                Some(ViewKey::Folder(Folder::Inbox)),
                with(typed, Some("INBOX"), false),
            ),
            (
                Some(ViewKey::Category(Category::Social)),
                with(&format!("({typed}) in:inbox category:social"), None, false),
            ),
            (
                Some(ViewKey::Label("Label_2".into())),
                with(typed, Some("Label_2"), false),
            ),
            (Some(ViewKey::Starred), with(typed, Some("STARRED"), false)),
            (
                Some(ViewKey::Folder(Folder::Sent)),
                with(typed, Some("SENT"), false),
            ),
            (
                Some(ViewKey::Folder(Folder::Archive)),
                with(&format!("({typed}) {ARCHIVE_Q}"), None, false),
            ),
            (
                Some(ViewKey::Folder(Folder::Spam)),
                with(typed, Some("SPAM"), true),
            ),
            (
                Some(ViewKey::Folder(Folder::Trash)),
                with(typed, Some("TRASH"), true),
            ),
            (Some(ViewKey::Folder(Folder::Drafts)), None),
            // All mail: no terms of its own.
            (None, with(typed, None, false)),
        ];
        for (view, want) in cases {
            assert_eq!(sent(view.clone()), want, "{view:?}");
        }
    }

    #[test]
    fn gmail_only_results_skip_local_matches() {
        let thread = |id: &str| {
            Thread::local(
                id.into(),
                id.into(),
                Folder::Inbox,
                vec![crate::data::Message::new(Default::default())],
                None,
            )
        };
        let threads = [thread("a"), thread("b")];
        let query = Query::parse("a OR b");
        let mut search = ServerSearch::default();
        let s = search.ask(Some(just_q("a OR b"))).unwrap();
        // Nothing yet: local matches would be false ones.
        let rows = |search: &ServerSearch| {
            search.merge(&threads, |_| true, |t| !query.gmail_only() && t.id == "a")
        };
        assert!(rows(&search).is_empty());
        assert!(search.answer(s, found(&["b"], 1, None)));
        assert_eq!(rows(&search), [1]);
        // Offline: said so; nothing local stands in.
        let s = search.ask(Some(just_q("x OR y"))).unwrap();
        assert!(search.answer(s, Err(GmailError::Network("down".into()))));
        assert_eq!(search.state, ServerState::Offline);
        assert!(rows(&search).is_empty());
    }

    #[test]
    fn gmail_q_table() {
        let labels = [
            label("Label_1", "Pigeon Test"),
            label("Label_2", "Work"),
            label("Label_3", "Work/Clients"),
        ];
        let inbox = ViewKey::Folder(Folder::Inbox);
        let work = ViewKey::Label("Label_2".into());
        let q = |input: &str, view: Option<&ViewKey>| {
            Query::parse(input).to_gmail_q(&labels, view).unwrap()
        };
        let just = |text: &str| GmailQuery {
            q: text.into(),
            ..Default::default()
        };
        let labelled = |text: &str, id: &str, spam_trash: bool| GmailQuery {
            q: text.into(),
            label: Some(id.into()),
            include_spam_trash: spam_trash,
        };
        let cases: Vec<(&str, Option<&ViewKey>, GmailQuery)> = vec![
            // Every operator.
            ("budget", None, just("budget")),
            (r#""quarterly budget""#, None, just(r#""quarterly budget""#)),
            ("from:sarah", None, just("from:sarah")),
            (r#"from:"Sarah Chen""#, None, just(r#"from:"sarah chen""#)),
            ("to:me@example.com", None, just("to:me@example.com")),
            ("subject:lunch", None, just("subject:lunch")),
            ("is:unread", None, just("is:unread")),
            ("is:starred", None, just("is:starred")),
            ("has:attachment", None, just("has:attachment")),
            ("has:attachments", None, just("has:attachment")),
            (
                "from:ana is:unread report",
                None,
                just("from:ana is:unread report"),
            ),
            // `label:`: Gmail's name, quoted.
            (
                r#"label:"pigeon test""#,
                None,
                just(r#"label:"Pigeon Test""#),
            ),
            ("label:pigeon-test", None, just(r#"label:"Pigeon Test""#)),
            ("label:work", None, just(r#"label:"Work""#)),
            (
                "label:wo",
                None,
                just(r#"{label:"Work" label:"Work/Clients"}"#),
            ),
            ("label:nope", None, just(r#"label:"nope""#)),
            ("in:work x", None, just(r#"label:"Work" x"#)),
            // Scopes.
            ("x in:inbox", None, labelled("x", "INBOX", false)),
            (
                "x in:archive",
                None,
                just("x -in:inbox -in:spam -in:trash -in:drafts"),
            ),
            ("x in:sent", None, labelled("x", "SENT", false)),
            ("x in:spam", None, labelled("x", "SPAM", true)),
            ("x in:trash", None, labelled("x", "TRASH", true)),
            ("x in:all", Some(&inbox), just("x")),
            (
                "x in:anywhere",
                Some(&inbox),
                GmailQuery {
                    q: "x".into(),
                    include_spam_trash: true,
                    ..Default::default()
                },
            ),
            ("x in:starred", None, just("x is:starred")),
            // The current view, without `in:`.
            ("x", Some(&inbox), just("x")),
            (
                "x",
                Some(&ViewKey::Folder(Folder::Archive)),
                just("x -in:inbox -in:spam -in:trash -in:drafts"),
            ),
            (
                "x",
                Some(&ViewKey::Folder(Folder::Sent)),
                labelled("x", "SENT", false),
            ),
            (
                "x",
                Some(&ViewKey::Starred),
                labelled("x", "STARRED", false),
            ),
            ("x", Some(&work), labelled("x", "Label_2", false)),
            // `in:` wins over the view.
            (
                "x in:sent",
                Some(&ViewKey::Starred),
                labelled("x", "SENT", false),
            ),
        ];
        for (input, view, want) in cases {
            assert_eq!(q(input, view), want, "{input} in {view:?}");
        }
        // Drafts sync whole: nothing to ask.
        assert!(
            Query::parse("x in:drafts")
                .to_gmail_q(&labels, None)
                .is_none()
        );
        assert!(
            Query::parse("x")
                .to_gmail_q(&labels, Some(&ViewKey::Folder(Folder::Drafts)))
                .is_none()
        );
    }

    fn found(
        ids: &[&str],
        estimate: usize,
        next: Option<&str>,
    ) -> Result<SearchResult, GmailError> {
        Ok(SearchResult {
            thread_ids: ids.iter().map(|s| s.to_string()).collect(),
            estimate: Some(estimate),
            next_page: next.map(String::from),
        })
    }

    #[test]
    fn stale_answers_are_dropped() {
        let mut search = ServerSearch::default();
        let a = search.ask(Some(just_q("a"))).unwrap();
        // The same query again (a refresh, the scope chip back) asks nothing.
        assert_eq!(search.ask(Some(just_q("a"))), None);
        let b = search.ask(Some(just_q("ab"))).unwrap();
        assert_eq!(search.state, ServerState::Waiting);
        assert!(!search.answer(a, found(&["old"], 1, None)));
        assert_eq!(search.state, ServerState::Waiting);
        assert!(search.answer(b, found(&["new"], 1, None)));
        assert_eq!(search.state, ServerState::Done);
        assert!(search.found.contains("new") && !search.found.contains("old"));
        // Closing the search drops what's on its way.
        assert_eq!(search.ask(None), None);
        assert!(!search.answer(b, found(&["late"], 1, None)));
        assert!(search.found.is_empty());
        assert_eq!(search.state, ServerState::Idle);
        // Offline: the local results, with a note.
        let c = search.ask(Some(just_q("c"))).unwrap();
        assert!(search.answer(c, Err(GmailError::Network("down".into()))));
        assert_eq!(search.state, ServerState::Offline);
    }

    fn just_q(q: &str) -> GmailQuery {
        GmailQuery {
            q: q.into(),
            ..Default::default()
        }
    }

    #[test]
    fn results_merge_newest_first_once_each() {
        let thread = |id: &str, folder| {
            Thread::local(
                id.into(),
                id.into(),
                folder,
                vec![crate::data::Message::new(Default::default())],
                None,
            )
        };
        // Newest first, as `MailData` keeps them.
        let threads = [
            thread("a", Folder::Inbox),
            thread("b", Folder::Inbox),
            thread("c", Folder::Archive),
            thread("d", Folder::Archive),
            thread("spam", Folder::Spam),
        ];
        let mut search = ServerSearch::default();
        let s = search.ask(Some(just_q("q"))).unwrap();
        assert!(search.answer(s, found(&["d", "a", "spam"], 40, Some("p2"))));
        assert!(search.has_more());
        // The next page repeats one and adds one.
        let (serial, query, page) = search.more().unwrap();
        assert_eq!((serial, query, page.as_str()), (s, just_q("q"), "p2"));
        assert_eq!(search.more(), None, "one page at a time");
        assert!(search.answer(s, found(&["a", "b"], 41, None)));
        assert!(!search.has_more());
        assert_eq!(search.estimate, Some(41));
        let rows = search.merge(&threads, |t| !t.is_hidden(), |t| t.id == "c");
        assert_eq!(rows, [0, 1, 2, 3]);
        // Nothing found by Gmail: the local matches alone.
        search.ask(None);
        assert_eq!(search.merge(&threads, |_| true, |t| t.id == "c"), [2]);
    }

    /// Gmail found every thread; each scope keeps the ones it holds by their
    /// labels and messages, not only by their one folder.
    #[test]
    fn gmail_results_kept_by_scope() {
        use crate::data::Message;
        let msg = |labels: &[&str]| {
            let mut m = Message::new(Default::default());
            m.label_ids = labels.iter().map(|l| l.to_string()).collect();
            m
        };
        let thread = |id: &str, folder, messages| {
            Thread::local(id.into(), id.into(), folder, messages, None)
        };
        let mut starred = thread("starred", Folder::Inbox, vec![msg(&["INBOX", "STARRED"])]);
        starred.starred = true;
        let mut labelled = thread("labelled", Folder::Archive, vec![msg(&["Label_1"])]);
        labelled.labels = vec!["Label_1".into()];
        let threads = [
            // Sent to myself: SENT and INBOX on one message.
            thread("self", Folder::Inbox, vec![msg(&["SENT", "INBOX"])]),
            // A reply I sent, archived.
            thread("reply", Folder::Archive, vec![msg(&[]), msg(&["SENT"])]),
            // Sent only to others.
            thread("sent", Folder::Sent, vec![msg(&["SENT"])]),
            thread("inbox", Folder::Inbox, vec![msg(&["INBOX"])]),
            starred,
            labelled,
            thread("trash", Folder::Trash, vec![msg(&["SENT", "TRASH"])]),
        ];
        let ids: Vec<&str> = threads.iter().map(|t| t.id.as_str()).collect();
        let mut search = ServerSearch::default();
        let s = search.ask(Some(just_q("q"))).unwrap();
        assert!(search.answer(s, found(&ids, ids.len(), None)));
        let data = MailData {
            account: Default::default(),
            labels: vec![label("Label_1", "Pigeon Test")],
            threads: Vec::new(),
        };
        let shown = |search: &ServerSearch, input: &str| -> Vec<&str> {
            let query = Query::parse(input);
            let scope = query.scope.unwrap();
            search
                .merge(
                    &threads,
                    |t| Query::in_scope(scope, t),
                    |t| query.matches(t, &data),
                )
                .into_iter()
                .map(|ix| threads[ix].id.as_str())
                .collect()
        };
        assert_eq!(shown(&search, "in:sent"), ["self", "reply", "sent"]);
        assert_eq!(shown(&search, "in:inbox"), ["self", "inbox", "starred"]);
        assert_eq!(shown(&search, "in:archive"), ["reply", "sent", "labelled"]);
        assert_eq!(shown(&search, "in:trash"), ["trash"]);
        // Labels and stars are terms; Gmail's word on them stands, outside
        // spam and trash.
        assert_eq!(shown(&search, "in:all is:starred").len(), 6);
        assert_eq!(shown(&search, "label:x in:all").len(), 6);
        // Locally, without Gmail's answer.
        search.ask(None);
        assert_eq!(shown(&search, "is:starred in:all"), ["starred"]);
        assert_eq!(shown(&search, "in:label_1"), ["labelled"]);
        assert_eq!(shown(&search, "in:sent"), ["self", "reply", "sent"]);
        assert_eq!(shown(&search, "in:archive"), ["reply", "sent", "labelled"]);
    }
}

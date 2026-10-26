//! Search queries: free text plus a few Gmail-style operators, matched against
//! each thread's precomputed `SearchIndex`. Terms AND together.

use gpui::actions;

use crate::data::{Folder, MailData, Thread};

actions!(search, [FocusResults]);

#[derive(Debug, PartialEq)]
enum Term {
    Text(String),
    From(String),
    To(String),
    Subject(String),
    Label(String),
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
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let mut query = Query::default();
        for token in tokenize(input) {
            let (key, value) = match token.split_once(':') {
                Some((key, value)) if !value.is_empty() => (key.to_lowercase(), value.to_string()),
                _ => (String::new(), token.clone()),
            };
            let value = value.to_lowercase();
            let term = match (key.as_str(), value.as_str()) {
                ("from", _) => Term::From(value),
                ("to", _) => Term::To(value),
                ("subject", _) => Term::Subject(value),
                ("label", _) => Term::Label(value),
                ("is", "unread") => Term::Unread,
                ("is", "starred") => Term::Starred,
                ("has", "attachment" | "attachments") => Term::Attachment,
                ("in", scope) => {
                    query.scope = Some(match scope {
                        "inbox" => Scope::Folder(Folder::Inbox),
                        "archive" | "archived" => Scope::Folder(Folder::Archive),
                        "sent" => Scope::Folder(Folder::Sent),
                        "drafts" | "draft" => Scope::Folder(Folder::Drafts),
                        "spam" => Scope::Folder(Folder::Spam),
                        "trash" => Scope::Folder(Folder::Trash),
                        "anywhere" => Scope::Anywhere,
                        "starred" => {
                            query.terms.push(Term::Starred);
                            Scope::All
                        }
                        "all" => Scope::All,
                        // Like Gmail, `in:` also takes a label.
                        label => {
                            query.terms.push(Term::Label(label.to_string()));
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

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.scope.is_none()
    }

    pub fn in_scope(scope: Scope, thread: &Thread) -> bool {
        match scope {
            Scope::Folder(folder) => thread.folder == folder,
            Scope::All => !thread.is_hidden(),
            Scope::Anywhere => true,
        }
    }

    pub fn matches(&self, thread: &Thread, data: &MailData) -> bool {
        let index = &thread.search;
        self.terms.iter().all(|term| match term {
            Term::Text(text) => index.all.contains(text.as_str()),
            Term::From(text) => index.from.contains(text.as_str()),
            Term::To(text) => index.to.contains(text.as_str()),
            Term::Subject(text) => index.subject.contains(text.as_str()),
            Term::Label(text) => {
                let wanted = normalize(text);
                thread.labels.iter().any(|id| {
                    normalize(id).starts_with(&wanted)
                        || data
                            .label(id)
                            .is_some_and(|l| normalize(&l.name).starts_with(&wanted))
                })
            }
            Term::Unread => thread.unread,
            Term::Starred => thread.starred,
            Term::Attachment => thread.has_attachments(),
        })
    }
}

/// "Side Projects", "side-projects" and "sideprojects" all compare equal.
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Splits on whitespace, keeping "quoted phrases" (also after `key:`) whole.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => quoted = !quoted,
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
    }
}

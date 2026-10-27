//! Who the user writes to (SYN-6): their contacts first, then other
//! contacts (people they've written to, and recipients sent to from here),
//! each by how often the address appears in the cached mail; then everyone
//! else in the mail, most frequent first.

use std::collections::HashMap;

use rusqlite::params;

use super::{Store, from_json};
use crate::{
    backend::people::{Changes, Person, Source},
    data::{Account, Contact, rank_contacts},
};

/// Recipients of a message sent here, until Google lists them as other
/// contacts. Never replaced by a contacts sync.
const SENT: &str = "sent";

impl Store {
    pub fn contacts(&self, account: &Account) -> anyhow::Result<Vec<Contact>> {
        let conn = self.conn();
        let mut synced: Vec<(Contact, bool)> = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT name, email, source FROM contacts WHERE account = ?1
             ORDER BY source != 'connection', rowid",
        )?;
        for row in stmt.query_map([&account.email], |r| {
            Ok((
                Contact {
                    name: r.get(0)?,
                    email: r.get(1)?,
                },
                r.get::<_, String>(2)? == Source::Connection.as_str(),
            ))
        })? {
            synced.push(row?);
        }

        // Spam senders aren't people to write to.
        let mut seen: Vec<Contact> = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT m.\"from\", m.\"to\", m.cc FROM messages m
             JOIN threads t ON t.account = m.account AND t.id = m.thread_id
             WHERE m.account = ?1 AND t.folder != 'spam'
             ORDER BY t.rowid, m.internal_date, m.rowid",
        )?;
        let mut rows = stmt.query(params![account.email])?;
        while let Some(r) = rows.next()? {
            seen.push(from_json(&r.get::<_, String>(0)?));
            seen.extend(from_json::<Vec<Contact>>(&r.get::<_, String>(1)?));
            seen.extend(from_json::<Vec<Contact>>(&r.get::<_, String>(2)?));
        }
        Ok(rank(synced, &seen, account))
    }

    /// Store one list's `changes` and its next sync token, together.
    pub(crate) fn put_people(
        &self,
        account: &str,
        source: Source,
        token_key: &str,
        changes: &Changes,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (people, token) = match changes {
            Changes::Full(people, token) => {
                tx.execute(
                    "DELETE FROM contacts WHERE account = ?1 AND source = ?2",
                    params![account, source.as_str()],
                )?;
                (people, token)
            }
            Changes::Delta(people, token) => (people, token),
        };
        for person in people {
            tx.execute(
                "DELETE FROM contacts WHERE account = ?1 AND resource_name = ?2",
                params![account, person.resource_name],
            )?;
            if !person.deleted() {
                insert_person(&tx, account, source, person)?;
            }
        }
        match token {
            Some(token) => tx.execute(
                "INSERT INTO sync_state (account, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT (account, key) DO UPDATE SET value = excluded.value",
                params![account, token_key, token],
            )?,
            None => tx.execute(
                "DELETE FROM sync_state WHERE account = ?1 AND key = ?2",
                params![account, token_key],
            )?,
        };
        tx.commit()?;
        Ok(())
    }

    /// People just written to join the book at once (W8).
    pub fn add_sent_recipients(&self, account: &str, recipients: &[Contact]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for contact in recipients {
            tx.execute(
                "INSERT OR IGNORE INTO contacts (account, resource_name, email, source, name)
                 VALUES (?1, ?2, ?3, ?2, ?4)",
                params![account, SENT, contact.email.to_lowercase(), contact.name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn insert_person(
    tx: &rusqlite::Transaction<'_>,
    account: &str,
    source: Source,
    person: &Person,
) -> rusqlite::Result<()> {
    for email in person.emails() {
        tx.execute(
            "INSERT OR IGNORE INTO contacts (account, resource_name, email, source, name, etag)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                account,
                person.resource_name,
                email,
                source.as_str(),
                person.name(),
                person.etag
            ],
        )?;
    }
    Ok(())
}

/// `synced` (contact, is a connection) in stored order, contacts first;
/// `seen` every address in the mail. Synced people come first, contacts
/// before other contacts, each by how often the mail has them; then the
/// rest of the mail, most frequent first. Once each, never me.
fn rank(synced: Vec<(Contact, bool)>, seen: &[Contact], account: &Account) -> Vec<Contact> {
    let me = |c: &Contact| account.is_me(&c.email);
    let in_mail = rank_contacts(seen, me);
    let mut count: HashMap<String, usize> = HashMap::new();
    for c in seen {
        *count.entry(c.email.to_lowercase()).or_default() += 1;
    }
    let mut people: Vec<(Contact, bool)> = Vec::new();
    for (mut contact, connection) in synced {
        if me(&contact)
            || people
                .iter()
                .any(|(p, _)| p.email.eq_ignore_ascii_case(&contact.email))
        {
            continue;
        }
        if contact.name.is_empty()
            && let Some(known) = in_mail
                .iter()
                .find(|c| c.email.eq_ignore_ascii_case(&contact.email))
        {
            contact.name = known.name.clone();
        }
        people.push((contact, connection));
    }
    people.sort_by_key(|(c, connection)| {
        let n = count.get(&c.email.to_lowercase()).copied().unwrap_or(0);
        (!connection, std::cmp::Reverse(n))
    });
    let mut out: Vec<Contact> = people.into_iter().map(|(c, _)| c).collect();
    let rest: Vec<Contact> = in_mail
        .into_iter()
        .filter(|c| !out.iter().any(|s| s.email.eq_ignore_ascii_case(&c.email)))
        .collect();
    out.extend(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{demo, store::Store};

    #[test]
    fn contacts_match_the_mailbox() {
        let store = Store::open_in_memory().unwrap();
        let account = demo::seed(&store).unwrap();
        let from_store: Vec<String> = store
            .contacts(&account)
            .unwrap()
            .into_iter()
            .map(|c| c.email)
            .collect();
        let from_model: Vec<String> = demo::mail_data()
            .contacts()
            .into_iter()
            .map(|c| c.email)
            .collect();
        assert_eq!(from_store, from_model);
    }

    fn c(name: &str, email: &str) -> Contact {
        Contact {
            name: name.into(),
            email: email.into(),
        }
    }

    fn account() -> Account {
        Account {
            name: "Me".into(),
            email: "me@example.com".into(),
            aliases: vec!["me.work@example.com".into()],
        }
    }

    #[test]
    fn contacts_then_other_contacts_by_frequency_then_the_mail() {
        let synced = vec![
            (c("Ana", "ana@example.com"), true),
            (c("Bo", "bo@example.com"), true),
            (c("", "cy@example.com"), false),
            (c("Di", "di@example.com"), false),
            // Also a contact: ranks as one.
            (c("Bo again", "BO@example.com"), false),
            (c("Me at work", "me.work@example.com"), false),
        ];
        let seen = vec![
            c("Me", "me@example.com"),
            c("Bo B", "bo@example.com"),
            c("Cy C", "cy@example.com"),
            c("Ed", "ed@example.com"),
            c("Ed", "ed@example.com"),
            c("Cy C", "cy@example.com"),
            c("Fay", "fay@example.com"),
            c("Me", "ME.WORK@example.com"),
        ];
        let ranked = rank(synced, &seen, &account());
        let emails: Vec<&str> = ranked.iter().map(|c| c.email.as_str()).collect();
        assert_eq!(
            emails,
            [
                "bo@example.com",
                "ana@example.com",
                "cy@example.com",
                "di@example.com",
                "ed@example.com",
                "fay@example.com",
            ]
        );
        // A contact's own name wins; a nameless one takes the mail's.
        assert_eq!(ranked[0].name, "Bo");
        assert_eq!(ranked[2].name, "Cy C");
    }

    #[test]
    fn sent_recipients_join_and_survive_a_full_sync() {
        let store = Store::open_in_memory().unwrap();
        let account = account();
        store
            .add_sent_recipients(&account.email, &[c("Gil", "Gil@example.com")])
            .unwrap();
        store
            .put_people(
                &account.email,
                Source::Other,
                "other_contacts_sync_token",
                &Changes::Full(Vec::new(), Some("t".into())),
            )
            .unwrap();
        let contacts = store.contacts(&account).unwrap();
        assert_eq!(contacts, [c("Gil", "gil@example.com")]);
    }
}

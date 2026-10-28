//! Who the user writes to (SYN-6): their contacts first, then other
//! contacts (people they've written to, and recipients sent to from here),
//! each by how often the address appears in the cached mail; then everyone
//! else in the mail, most frequent first.

use std::collections::{HashMap, HashSet};

use rusqlite::params;

use super::{Store, from_json, json};
use crate::{
    backend::people::{Changes, Person, Source},
    contacts::{CardThread, ContactCard, ContactDetails},
    data::{Account, Contact, rank_contacts},
};

/// Conversations a contact card lists.
pub const CARD_THREADS: usize = 8;

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
            tx.prepare_cached("DELETE FROM contacts WHERE account = ?1 AND resource_name = ?2")?
                .execute(params![account, person.resource_name])?;
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

    /// The contact card for `email` (CON-5), from the cache only: the
    /// contact's name, addresses, phones and organisation when they're a
    /// contact, else `name` (or the name the mail gives) and the address;
    /// and the newest conversations with them, Spam, Trash and Drafts left
    /// out.
    pub fn contact_card(
        &self,
        account: &Account,
        email: &str,
        name: &str,
    ) -> anyhow::Result<ContactCard> {
        let conn = self.conn();
        let found: Option<(String, String, String, Option<String>)> = conn
            .query_row(
                "SELECT resource_name, source, name, details FROM contacts
                 WHERE account = ?1 AND email = ?2 COLLATE NOCASE
                 ORDER BY source != 'connection', rowid LIMIT 1",
                params![account.email, email],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let mut card = ContactCard {
            name: name.trim().to_string(),
            emails: vec![email.to_string()],
            ..ContactCard::default()
        };
        if let Some((resource, source, stored_name, details)) = found {
            card.is_contact = source == Source::Connection.as_str();
            if !stored_name.is_empty() {
                card.name = stored_name;
            }
            card.details = details.as_deref().map(from_json).unwrap_or_default();
            if card.is_contact {
                let mut stmt = conn.prepare(
                    "SELECT email FROM contacts
                     WHERE account = ?1 AND resource_name = ?2 ORDER BY rowid",
                )?;
                for other in stmt.query_map(params![account.email, resource], |r| r.get(0))? {
                    let other: String = other?;
                    if !card.emails.iter().any(|e| e.eq_ignore_ascii_case(&other)) {
                        card.emails.push(other);
                    }
                }
            }
        }

        // A cheap text match first, then the parsed addresses.
        let mut stmt = conn.prepare(
            "SELECT t.id, t.subject, t.last_date, m.\"from\", m.\"to\", m.cc FROM messages m
             JOIN threads t ON t.account = m.account AND t.id = m.thread_id
             WHERE m.account = ?1 AND t.folder NOT IN ('spam', 'trash', 'drafts')
               AND (instr(lower(m.\"from\"), ?2) OR instr(lower(m.\"to\"), ?2)
                    OR instr(lower(m.cc), ?2))
             ORDER BY t.last_date DESC, t.rowid",
        )?;
        let mut mail_name = String::new();
        for address in card.emails.clone() {
            let needle = address.to_lowercase();
            let mut rows = stmt.query(params![account.email, needle])?;
            while let Some(r) = rows.next()? {
                let from: Contact = from_json(&r.get::<_, String>(3)?);
                let mut people = from_json::<Vec<Contact>>(&r.get::<_, String>(4)?);
                people.extend(from_json::<Vec<Contact>>(&r.get::<_, String>(5)?));
                let is_them = |c: &Contact| c.email.eq_ignore_ascii_case(&address);
                if is_them(&from) && mail_name.is_empty() {
                    mail_name = from.name.trim().to_string();
                }
                if !is_them(&from) && !people.iter().any(is_them) {
                    continue;
                }
                let id: String = r.get(0)?;
                if !card.threads.iter().any(|t| t.id == id) {
                    card.threads.push(CardThread {
                        id,
                        subject: r.get(1)?,
                        last_date: r.get(2)?,
                    });
                }
            }
        }
        card.threads.sort_by_key(|t| std::cmp::Reverse(t.last_date));
        card.threads.truncate(CARD_THREADS);
        if card.name.is_empty() {
            card.name = mail_name;
        }
        Ok(card)
    }

    /// People just written to join the book at once (W8).
    pub fn add_sent_recipients(&self, account: &str, recipients: &[Contact]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for contact in recipients {
            tx.prepare_cached(
                "INSERT OR IGNORE INTO contacts (account, resource_name, email, source, name)
                 VALUES (?1, ?2, ?3, ?2, ?4)",
            )?
            .execute(params![
                account,
                SENT,
                contact.email.to_lowercase(),
                contact.name
            ])?;
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
    let details = Some(person.details())
        .filter(|d| *d != ContactDetails::default())
        .map(|d| json(&d));
    for email in person.emails() {
        tx.prepare_cached(
            "INSERT OR IGNORE INTO contacts
                 (account, resource_name, email, source, name, etag, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![
            account,
            person.resource_name,
            email,
            source.as_str(),
            person.name(),
            person.etag,
            details
        ])?;
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
    // First match wins, as `find` would.
    let mut names: HashMap<String, &str> = HashMap::new();
    for c in &in_mail {
        names
            .entry(c.email.to_ascii_lowercase())
            .or_insert(c.name.as_str());
    }
    let mut taken: HashSet<String> = HashSet::new();
    let mut people: Vec<(Contact, bool)> = Vec::new();
    for (mut contact, connection) in synced {
        if me(&contact) || !taken.insert(contact.email.to_ascii_lowercase()) {
            continue;
        }
        if contact.name.is_empty()
            && let Some(known) = names.get(&contact.email.to_ascii_lowercase())
        {
            contact.name = known.to_string();
        }
        people.push((contact, connection));
    }
    people.sort_by_key(|(c, connection)| {
        let n = count.get(&c.email.to_lowercase()).copied().unwrap_or(0);
        (!connection, std::cmp::Reverse(n))
    });
    let mut out: Vec<Contact> = people.into_iter().map(|(c, _)| c).collect();
    let rest: Vec<Contact> = in_mail
        .iter()
        .filter(|c| !taken.contains(&c.email.to_ascii_lowercase()))
        .cloned()
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
            send_as: Vec::new(),
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

    // W6: the contact card (CON-5).

    fn person(json: &str) -> Person {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_contacts_card_has_their_details_and_threads() {
        let store = Store::open_in_memory().unwrap();
        let account = demo::seed(&store).unwrap();
        // Someone the placeholder mail has, made a contact with two
        // addresses, phones and an organisation.
        let card = store
            .contact_card(&account, "sarah.chen@acme.io", "")
            .unwrap();
        assert!(!card.is_contact);
        assert_eq!(card.name, "Sarah Chen");
        let threads = card.threads.clone();
        assert!(!threads.is_empty());
        assert!(threads.len() <= CARD_THREADS);
        assert!(threads.windows(2).all(|w| w[0].last_date >= w[1].last_date));

        let sarah = person(
            r#"{"resourceName":"people/c1","names":[{"displayName":"Sarah Chen-Li"}],
            "emailAddresses":[{"value":"sarah.chen@acme.io"},{"value":"sarah@home.example.com"}],
            "phoneNumbers":[{"value":"+1 415 555 0134","formattedType":"Mobile"},
                            {"value":"+1 415 555 0134","formattedType":"Work"}],
            "organizations":[{"name":"Acme","title":"Head of Design"}]}"#,
        );
        store
            .put_people(
                &account.email,
                Source::Connection,
                "people_sync_token",
                &Changes::Full(vec![sarah], None),
            )
            .unwrap();
        // Opened from the second address: it comes first.
        let card = store
            .contact_card(&account, "SARAH@home.example.com", "Sarah")
            .unwrap();
        assert!(card.is_contact);
        assert_eq!(card.name, "Sarah Chen-Li");
        assert_eq!(
            card.emails,
            ["SARAH@home.example.com", "sarah.chen@acme.io"]
        );
        assert_eq!(
            card.details.phones,
            [crate::contacts::Phone {
                number: "+1 415 555 0134".into(),
                label: "mobile".into(),
            }]
        );
        assert_eq!(card.details.organization, "Acme");
        assert_eq!(card.details.title, "Head of Design");
        // The threads of every address.
        assert_eq!(card.threads, threads);
    }

    #[test]
    fn a_stranger_has_a_name_and_an_address_only() {
        let store = Store::open_in_memory().unwrap();
        let account = demo::seed(&store).unwrap();
        let card = store
            .contact_card(&account, "nobody@example.net", "Nobody")
            .unwrap();
        assert_eq!(
            card,
            ContactCard {
                name: "Nobody".into(),
                emails: vec!["nobody@example.net".into()],
                ..ContactCard::default()
            }
        );
        // Spam isn't a conversation with someone.
        let spam = demo::mail_data()
            .threads
            .into_iter()
            .find(|t| t.folder == crate::data::Folder::Spam)
            .unwrap();
        let sender = &spam.messages[0].from;
        let card = store.contact_card(&account, &sender.email, "").unwrap();
        assert!(card.threads.iter().all(|t| t.id != spam.id));
    }

    #[test]
    fn upgrading_drops_the_connections_token_once() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        // A cache from before 170, with both tokens.
        for (version, sql) in super::super::migrations() {
            if version >= 170 {
                break;
            }
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", version).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO sync_state VALUES ('me@example.com', 'people_sync_token', 'old');
             INSERT INTO sync_state VALUES ('me@example.com', 'other_contacts_sync_token', 'keep');",
        )
        .unwrap();
        super::super::migrate(&mut conn).unwrap();
        let token = |conn: &rusqlite::Connection, key: &str| -> Option<String> {
            conn.query_row("SELECT value FROM sync_state WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .ok()
        };
        assert_eq!(token(&conn, "people_sync_token"), None);
        assert_eq!(
            token(&conn, "other_contacts_sync_token").as_deref(),
            Some("keep")
        );
        // Once: a token stored after the upgrade stays.
        conn.execute(
            "INSERT INTO sync_state VALUES ('me@example.com', 'people_sync_token', 'new')",
            [],
        )
        .unwrap();
        super::super::migrate(&mut conn).unwrap();
        assert_eq!(token(&conn, "people_sync_token").as_deref(), Some("new"));
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

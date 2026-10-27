//! Who the user writes to: synced contacts first (W8 fills the table),
//! then everyone in the cached mail by how often they appear.

use rusqlite::params;

use super::{Store, from_json};
use crate::data::{Account, Contact, rank_contacts};

impl Store {
    pub fn contacts(&self, account: &Account) -> anyhow::Result<Vec<Contact>> {
        let conn = self.conn();
        let mut synced: Vec<Contact> = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT name, email FROM contacts WHERE account = ?1
             ORDER BY source = 'other', rowid",
        )?;
        for row in stmt.query_map([&account.email], |r| {
            Ok(Contact {
                name: r.get(0)?,
                email: r.get(1)?,
            })
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
        let mut ranked = rank_contacts(&seen, |c| account.is_me(&c.email));
        ranked.retain(|c| {
            !synced
                .iter()
                .any(|s| s.email.eq_ignore_ascii_case(&c.email))
        });
        synced.retain(|c| !account.is_me(&c.email));
        synced.extend(ranked);
        Ok(synced)
    }
}

#[cfg(test)]
mod tests {
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
}

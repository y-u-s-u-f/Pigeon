//! Contacts sync (SYN-6, 6.4 item 4): the user's contacts
//! (`people.connections.list`) and other contacts (`otherContacts.list`,
//! people they've written to) into the store, then searched locally. Both
//! go by sync token after the first full list; an expired token lists
//! everything again. Read-only. Contacts also bring their phone numbers and
//! organisation, for the contact card (CON-5); other contacts can't.

use anyhow::{Context as _, anyhow};
use serde::Deserialize;

use super::{
    http::{Request, Response, Transport, TransportError, encode as enc},
    store::Store,
};
use crate::contacts::{ContactDetails, Phone};

/// `otherContacts.list`'s scope; contacts need `auth::CONTACTS_SCOPE`.
pub const OTHER_CONTACTS_SCOPE: &str = "https://www.googleapis.com/auth/contacts.other.readonly";

const API: &str = "https://people.googleapis.com/v1";
const COST: u32 = 1;
const PAGE_SIZE: u32 = 1000;
/// The connections' field mask. Other contacts keep their own,
/// `names,emailAddresses`.
const CONNECTION_FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations";

/// Which list a contact comes from. Contacts rank before other contacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Connection,
    Other,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Connection => "connection",
            Source::Other => "other",
        }
    }

    /// The `sync_state` key its token is kept under. People reuses a token
    /// only with the field mask it was made with: a change to `FIELDS`
    /// needs a migration that drops it (as 170 did).
    fn token_key(self) -> &'static str {
        match self {
            Source::Connection => "people_sync_token",
            Source::Other => "other_contacts_sync_token",
        }
    }

    fn url(self, sync_token: Option<&str>, page: Option<&str>) -> String {
        let mut url = match self {
            Source::Connection => format!(
                "{API}/people/me/connections?personFields={CONNECTION_FIELDS}\
                 &requestSyncToken=true&pageSize={PAGE_SIZE}"
            ),
            Source::Other => format!(
                "{API}/otherContacts?readMask=names,emailAddresses\
                 &requestSyncToken=true&pageSize={PAGE_SIZE}"
            ),
        };
        if let Some(token) = sync_token {
            url.push_str(&format!("&syncToken={}", enc(token)));
        }
        if let Some(page) = page {
            url.push_str(&format!("&pageToken={}", enc(page)));
        }
        url
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default, alias = "otherContacts")]
    connections: Vec<Person>,
    next_page_token: Option<String>,
    next_sync_token: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub resource_name: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    metadata: PersonMetadata,
    #[serde(default)]
    names: Vec<Name>,
    #[serde(default)]
    email_addresses: Vec<Email>,
    #[serde(default)]
    phone_numbers: Vec<PhoneNumber>,
    #[serde(default)]
    organizations: Vec<Organization>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PhoneNumber {
    #[serde(default)]
    value: String,
    #[serde(default)]
    formatted_type: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct Organization {
    #[serde(default)]
    name: String,
    #[serde(default)]
    title: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct PersonMetadata {
    #[serde(default)]
    deleted: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Name {
    #[serde(default)]
    display_name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct Email {
    #[serde(default)]
    value: String,
}

impl Person {
    pub fn deleted(&self) -> bool {
        self.metadata.deleted
    }

    pub fn name(&self) -> &str {
        self.names.first().map_or("", |n| n.display_name.trim())
    }

    /// Its addresses, once each.
    pub fn emails(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.email_addresses {
            let email = e.value.trim();
            if !email.contains('@') || out.iter().any(|o| o.eq_ignore_ascii_case(email)) {
                continue;
            }
            out.push(email.to_string());
        }
        out
    }

    /// Phone numbers (once each) and the first organisation, for the card.
    pub fn details(&self) -> ContactDetails {
        let mut phones: Vec<Phone> = Vec::new();
        for p in &self.phone_numbers {
            let number = p.value.trim();
            if number.is_empty() || phones.iter().any(|o| o.number == number) {
                continue;
            }
            phones.push(Phone {
                number: number.to_string(),
                label: p.formatted_type.trim().to_lowercase(),
            });
        }
        let org = self
            .organizations
            .iter()
            .find(|o| !o.name.trim().is_empty() || !o.title.trim().is_empty());
        ContactDetails {
            phones,
            organization: org.map(|o| o.name.trim().to_string()).unwrap_or_default(),
            title: org.map(|o| o.title.trim().to_string()).unwrap_or_default(),
        }
    }
}

/// One list's changes since the stored token.
#[derive(Debug, PartialEq)]
pub enum Changes {
    /// Everything: replaces what's stored.
    Full(Vec<Person>, Option<String>),
    /// Changed and deleted people only.
    Delta(Vec<Person>, Option<String>),
}

/// Brings `account`'s contacts from `sources` (those whose scope was
/// granted) in the store up to date. Blocking: run it off the main thread.
/// True when anything was stored.
pub fn sync_pass(
    transport: &dyn Transport,
    store: &Store,
    account: &str,
    sources: &[Source],
) -> anyhow::Result<bool> {
    let mut changed = false;
    for &source in sources {
        let token = store.sync_state(account, source.token_key())?;
        let changes = list(transport, source, token.as_deref())?;
        changed |= match &changes {
            Changes::Full(..) => true,
            Changes::Delta(people, _) => !people.is_empty(),
        };
        store.put_people(account, source, source.token_key(), &changes)?;
    }
    Ok(changed)
}

/// Every page of `source` since `token`, or all of it when there's none or
/// it expired.
fn list(transport: &dyn Transport, source: Source, token: Option<&str>) -> anyhow::Result<Changes> {
    let mut people = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let url = source.url(token, page.as_deref());
        let response = transport.send(&Request::get(url, COST))?;
        if token.is_some() && expired(&response) {
            return list(transport, source, None);
        }
        if !response.is_success() {
            return Err(api_error(source, &response));
        }
        let body: Page = response
            .json()
            .with_context(|| format!("{}: unexpected response", source.as_str()))?;
        people.extend(body.connections);
        match body.next_page_token {
            Some(next) => page = Some(next),
            None => {
                return Ok(match token {
                    Some(_) => Changes::Delta(people, body.next_sync_token),
                    None => Changes::Full(people, body.next_sync_token),
                });
            }
        }
    }
}

/// The sync token is too old: 410, or 400 with `EXPIRED_SYNC_TOKEN`.
fn expired(response: &Response) -> bool {
    response.status == 410
        || (response.status == 400
            && String::from_utf8_lossy(&response.body).contains("EXPIRED_SYNC_TOKEN"))
}

fn api_error(source: Source, response: &Response) -> anyhow::Error {
    let message = response
        .error_message()
        .unwrap_or_else(|| format!("HTTP {}", response.status));
    anyhow!("contacts ({}): {message}", source.as_str())
}

pub fn is_network(e: &anyhow::Error) -> bool {
    e.downcast_ref::<TransportError>()
        .is_some_and(TransportError::goes_later)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::http::FakeTransport, data::Account};

    const ME: &str = "me@example.com";
    const BOTH: &[Source] = &[Source::Connection, Source::Other];
    const CONNECTIONS: &str = include_str!("../../tests/fixtures/google/people/connections.json");
    const CONNECTIONS_2: &str =
        include_str!("../../tests/fixtures/google/people/connections_page2.json");
    const OTHER: &str = include_str!("../../tests/fixtures/google/people/other_contacts.json");
    const DELTA: &str = include_str!("../../tests/fixtures/google/people/connections_delta.json");
    const PATH: &str = "/v1/people/me/connections";
    const OTHER_PATH: &str = "/v1/otherContacts";
    const EXPIRED: &str = r#"{"error":{"code":400,"message":"Sync token is expired. Clear local cache and retry call without the sync token.","status":"FAILED_PRECONDITION","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"EXPIRED_SYNC_TOKEN"}]}}"#;

    fn account() -> Account {
        Account {
            name: "Me".into(),
            email: ME.into(),
            aliases: vec!["alias@example.com".into()],
            send_as: Vec::new(),
        }
    }

    fn emails(store: &Store) -> Vec<String> {
        store
            .contacts(&account())
            .unwrap()
            .into_iter()
            .map(|c| c.email)
            .collect()
    }

    fn full() -> FakeTransport {
        FakeTransport::new()
            .on("GET", PATH, &[("pageToken", "p2")], &[(200, CONNECTIONS_2)])
            .on("GET", PATH, &[], &[(200, CONNECTIONS)])
            .on("GET", OTHER_PATH, &[], &[(200, OTHER)])
    }

    #[test]
    fn connections_and_other_contacts_become_contacts() {
        let store = Store::open_in_memory().unwrap();
        let fake = full();
        assert!(sync_pass(&fake, &store, ME, BOTH).unwrap());
        // Contacts first, then other contacts; one row per address, no
        // duplicates, never me or an alias.
        assert_eq!(
            emails(&store),
            [
                "sarah.chen@example.com",
                "sarah@home.example.com",
                "rahul@example.com",
                "mateo@example.com",
                "lena@example.com",
            ]
        );
        let contacts = store.contacts(&account()).unwrap();
        assert_eq!(contacts[0].name, "Sarah Chen");
        assert_eq!(contacts[4].name, "");
        // W6: phones (once each) and the organisation, for the card.
        let card = store
            .contact_card(&account(), "sarah.chen@example.com", "")
            .unwrap();
        assert!(card.is_contact);
        let phones: Vec<(&str, &str)> = card
            .details
            .phones
            .iter()
            .map(|p| (p.number.as_str(), p.label.as_str()))
            .collect();
        assert_eq!(
            phones,
            [("+1 415 555 0134", "mobile"), ("+1 415 555 0199", "work")]
        );
        assert_eq!(
            (
                card.details.organization.as_str(),
                card.details.title.as_str()
            ),
            ("Acme", "Head of Design")
        );
        assert_eq!(
            card.emails,
            ["sarah.chen@example.com", "sarah@home.example.com"]
        );
        assert_eq!(
            store
                .sync_state(ME, "people_sync_token")
                .unwrap()
                .as_deref(),
            Some("conn-token-1")
        );
        assert_eq!(
            store
                .sync_state(ME, "other_contacts_sync_token")
                .unwrap()
                .as_deref(),
            Some("other-token-1")
        );
        let requests = fake.requests();
        assert!(
            requests[0]
                .url
                .contains("personFields=names,emailAddresses,phoneNumbers,organizations")
        );
        assert!(requests[0].url.contains("requestSyncToken=true"));
        assert!(
            requests
                .iter()
                .any(|r| r.url.contains("readMask=names,emailAddresses"))
        );
    }

    #[test]
    fn later_passes_go_by_token() {
        let store = Store::open_in_memory().unwrap();
        sync_pass(&full(), &store, ME, BOTH).unwrap();
        let fake = FakeTransport::new()
            .on(
                "GET",
                PATH,
                &[("syncToken", "conn-token-1")],
                &[(200, DELTA)],
            )
            .on(
                "GET",
                OTHER_PATH,
                &[("syncToken", "other-token-1")],
                &[(200, r#"{"nextSyncToken":"other-token-2"}"#)],
            );
        assert!(sync_pass(&fake, &store, ME, BOTH).unwrap());
        // Sarah's home address gone, Ana added; Rahul left the contacts and
        // is now only an other contact.
        assert_eq!(
            emails(&store),
            [
                "sarah.chen@example.com",
                "ana@example.com",
                "mateo@example.com",
                "lena@example.com",
                "Rahul@example.com",
            ]
        );
        assert_eq!(
            store
                .sync_state(ME, "people_sync_token")
                .unwrap()
                .as_deref(),
            Some("conn-token-2")
        );
        // Nothing new: nothing to report.
        let quiet = FakeTransport::new()
            .on("GET", PATH, &[], &[(200, r#"{"nextSyncToken":"c3"}"#)])
            .on(
                "GET",
                OTHER_PATH,
                &[],
                &[(200, r#"{"nextSyncToken":"o3"}"#)],
            );
        assert!(!sync_pass(&quiet, &store, ME, BOTH).unwrap());
    }

    #[test]
    fn an_expired_token_lists_everything_again() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_sync_state(ME, "people_sync_token", "old")
            .unwrap();
        store
            .set_sync_state(ME, "other_contacts_sync_token", "old")
            .unwrap();
        // Left over from before: a full list replaces it.
        let stale = Changes::Full(
            serde_json::from_str::<Page>(DELTA).unwrap().connections,
            None,
        );
        store
            .put_people(ME, Source::Connection, "people_sync_token", &stale)
            .unwrap();
        store
            .set_sync_state(ME, "people_sync_token", "old")
            .unwrap();
        let fake = FakeTransport::new()
            .on("GET", PATH, &[("syncToken", "old")], &[(400, EXPIRED)])
            .on("GET", OTHER_PATH, &[("syncToken", "old")], &[(410, "{}")])
            .on("GET", PATH, &[("pageToken", "p2")], &[(200, CONNECTIONS_2)])
            .on("GET", PATH, &[], &[(200, CONNECTIONS)])
            .on("GET", OTHER_PATH, &[], &[(200, OTHER)]);
        assert!(sync_pass(&fake, &store, ME, BOTH).unwrap());
        assert!(!emails(&store).contains(&"ana@example.com".to_string()));
        assert_eq!(emails(&store).len(), 5);
        assert_eq!(
            store
                .sync_state(ME, "people_sync_token")
                .unwrap()
                .as_deref(),
            Some("conn-token-1")
        );
    }

    #[test]
    fn other_failures_stop_the_pass() {
        let store = Store::open_in_memory().unwrap();
        let denied = r#"{"error":{"code":403,"message":"The caller does not have permission"}}"#;
        let fake = FakeTransport::new().on("GET", PATH, &[], &[(403, denied)]);
        let e = sync_pass(&fake, &store, ME, BOTH).unwrap_err();
        assert!(!is_network(&e));
        assert!(format!("{e}").contains("permission"));
        let offline = FakeTransport::new().on("GET", PATH, &[], &[(0, "")]);
        assert!(is_network(
            &sync_pass(&offline, &store, ME, BOTH).unwrap_err()
        ));
    }
}

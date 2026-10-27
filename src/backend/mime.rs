//! MIME. Parsing raw messages from `messages.get format=raw` (W3, RDR-6);
//! building outgoing ones and reading drafts back (W6).

use base64::Engine as _;
use chrono::NaiveDateTime;
use mail_builder::{
    MessageBuilder,
    headers::{address::Address, message_id::MessageId, raw::Raw},
};
use mail_parser::MessageParser;

use super::Outgoing;
use crate::{
    calendar::time::LocalZone,
    data::{Attachment, Contact, InviteDetails, InviteMethod, Message},
};

// Parse (W3).

/// What the reader needs from a raw message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedBody {
    pub html: String,
    pub plain: Option<String>,
    pub attachments: Vec<Attachment>,
    /// A `text/calendar` part's UID and METHOD.
    pub invite: Option<(String, InviteMethod)>,
    /// That part's event, as it describes it (W10, CAL-20).
    pub invite_details: Option<InviteDetails>,
}

/// Gmail's `raw`: base64url, with or without padding.
pub fn decode_raw(raw: &str) -> Option<Vec<u8>> {
    use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
    const URL_SAFE_ANY: GeneralPurpose = GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    URL_SAFE_ANY.decode(raw.trim()).ok()
}

/// The body as HTML (plain text converted), its attachments and any
/// invitation. Charsets, transfer encodings and RFC 2047 names are decoded.
pub fn parse_message(raw: &[u8]) -> Option<ParsedBody> {
    let message = MessageParser::default().parse(raw)?;
    let html: String = message.html_bodies().collect::<Vec<_>>().join("\n");
    let plain: String = message.text_bodies().collect::<Vec<_>>().join("\n");

    let mut invite = None;
    let mut invite_details = None;
    for part in message.parts() {
        if part.is_content_type("text", "calendar")
            && invite.is_none()
            && let Some(text) = part.text()
        {
            invite = parse_ics(&text);
            invite_details = invite
                .is_some()
                .then(|| parse_ics_details(&text, &LocalZone::system()))
                .flatten();
        }
    }

    let attachments = message
        .attachments()
        .filter(|part| {
            !part.is_content_type("text", "calendar")
                && !part.is_content_type("application", "ics")
                // Images the HTML shows inline (cid:) aren't files to list.
                && !(part.is_inline() && part.content_id().is_some())
        })
        .map(|part| {
            let mime = part
                .content_type()
                .map(|ct| match ct.subtype() {
                    Some(sub) => format!("{}/{}", ct.ctype(), sub),
                    None => ct.ctype().to_string(),
                })
                .unwrap_or_else(|| "application/octet-stream".into())
                .to_ascii_lowercase();
            Attachment {
                name: part.attachment_name().unwrap_or("Untitled").to_string(),
                size_bytes: part.decoded_len() as u64,
                mime,
                attachment_id: String::new(),
                part_id: part.id().to_string(),
                path: String::new(),
            }
        })
        .collect();

    Some(ParsedBody {
        html,
        plain: (!plain.trim().is_empty()).then_some(plain),
        attachments,
        invite,
        invite_details,
    })
}

/// An iCalendar object's event UID and METHOD (REQUEST or CANCEL; others
/// don't ask anything of the user).
pub fn parse_ics(text: &str) -> Option<(String, InviteMethod)> {
    // Unfold: a line starting with a space or tab continues the last.
    let unfolded = text
        .replace("\r\n", "\n")
        .replace("\n ", "")
        .replace("\n\t", "");
    let mut method = None;
    let mut uid = None;
    let mut in_event = false;
    for line in unfolded.lines() {
        let (name, value) = match line.split_once(':') {
            Some((name, value)) => (name.split(';').next().unwrap_or(name), value.trim()),
            None => continue,
        };
        match name.to_ascii_uppercase().as_str() {
            "METHOD" => method = Some(value.to_ascii_uppercase()),
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") => in_event = true,
            "END" if value.eq_ignore_ascii_case("VEVENT") => in_event = false,
            "UID" if in_event && uid.is_none() => uid = Some(value.to_string()),
            _ => {}
        }
    }
    let method = match method.as_deref() {
        Some("CANCEL") => InviteMethod::Cancel,
        Some("REQUEST") => InviteMethod::Request,
        _ => return None,
    };
    Some((uid?, method))
}

/// An iCalendar line split into its name (upper case), parameters and
/// value, after unfolding.
fn ics_lines(text: &str) -> Vec<(String, String, String)> {
    let unfolded = text
        .replace("\r\n", "\n")
        .replace("\n ", "")
        .replace("\n\t", "");
    unfolded
        .lines()
        .filter_map(|line| {
            // The value starts after the first colon outside a quoted
            // parameter.
            let mut quoted = false;
            let split = line.char_indices().find(|&(_, c)| {
                if c == '"' {
                    quoted = !quoted;
                }
                c == ':' && !quoted
            })?;
            let (head, value) = (&line[..split.0], &line[split.0 + 1..]);
            let (name, params) = head.split_once(';').unwrap_or((head, ""));
            Some((
                name.to_ascii_uppercase(),
                params.to_string(),
                value.trim().to_string(),
            ))
        })
        .collect()
}

/// TEXT values escape `\\`, `\;`, `\,` and newlines.
fn ics_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

/// A DTSTART or DTEND as a local wall time, and whether it's a date. UTC
/// times (`…Z`) convert to `zone`; times with a TZID are taken as wall
/// times, as elsewhere (6.4 item 8).
fn ics_time(params: &str, value: &str, zone: &LocalZone) -> Option<(NaiveDateTime, bool)> {
    let params = params.to_ascii_uppercase();
    let date_param = params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME");
    if date_param || value.len() == 8 {
        let date = chrono::NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((date.and_hms_opt(0, 0, 0)?, true));
    }
    let (wall, utc) = match value.strip_suffix(['Z', 'z']) {
        Some(v) => (v, true),
        None => (value, false),
    };
    let t = NaiveDateTime::parse_from_str(wall, "%Y%m%dT%H%M%S").ok()?;
    Some((if utc { (zone.to_local)(t.and_utc()) } else { t }, false))
}

/// The first VEVENT's title, times, place and organizer.
pub fn parse_ics_details(text: &str, zone: &LocalZone) -> Option<InviteDetails> {
    let mut details = InviteDetails::default();
    let mut in_event = false;
    let mut seen = false;
    for (name, params, value) in ics_lines(text) {
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") && !seen => in_event = true,
            "END" if value.eq_ignore_ascii_case("VEVENT") && in_event => {
                in_event = false;
                seen = true;
            }
            _ if !in_event => {}
            "SUMMARY" => details.title = ics_text(&value),
            "LOCATION" => details.location = ics_text(&value),
            "DTSTART" => {
                if let Some((t, all_day)) = ics_time(&params, &value, zone) {
                    details.start = Some(t);
                    details.all_day = all_day;
                }
            }
            "DTEND" => details.end = ics_time(&params, &value, zone).map(|(t, _)| t),
            "ORGANIZER" => {
                let email = value
                    .strip_prefix("mailto:")
                    .or_else(|| value.strip_prefix("MAILTO:"))
                    .unwrap_or(&value);
                let name = params
                    .split(';')
                    .find_map(|p| {
                        let (k, v) = p.split_once('=')?;
                        k.eq_ignore_ascii_case("CN").then(|| v.trim_matches('"'))
                    })
                    .unwrap_or_default();
                details.organizer = Contact {
                    name: ics_text(name),
                    email: email.to_string(),
                };
            }
            _ => {}
        }
    }
    seen.then_some(details)
}

// Build (W6).

/// Marks a draft written here: its `text/plain` part is the markdown as
/// typed, so it reopens as written (DRF-3).
pub const MARKDOWN_HEADER: &str = "X-Pigeon-Markdown";

/// What a reply carries to thread in other clients (DAT-4): ids without
/// angle brackets, as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Threading {
    pub in_reply_to: String,
    pub references: Vec<String>,
}

/// Answering `parent`: its id, after the chain it was part of.
pub fn reply_threading(parent: &Message) -> Option<Threading> {
    if parent.message_id.is_empty() {
        return None;
    }
    let mut references: Vec<String> = parent.references.clone();
    if references.is_empty() && !parent.in_reply_to.is_empty() {
        references.push(parent.in_reply_to.clone());
    }
    references.retain(|r| !r.is_empty() && *r != parent.message_id);
    references.push(parent.message_id.clone());
    Some(Threading {
        in_reply_to: parent.message_id.clone(),
        references,
    })
}

/// A new Message-ID, without brackets, on the sender's domain.
pub fn new_message_id(from: &str) -> String {
    let domain = from
        .rsplit_once('@')
        .map(|(_, d)| d)
        .filter(|d| !d.is_empty())
        .unwrap_or("pigeon.local");
    format!("{}@{domain}", uuid::Uuid::new_v4().simple())
}

/// How to build one message.
#[derive(Clone, Debug, Default)]
pub struct Build {
    pub message_id: String,
    /// Unix seconds.
    pub date: i64,
    pub threading: Option<Threading>,
    /// A draft carries `X-Pigeon-Markdown`; a send doesn't.
    pub draft: bool,
    /// The files, read: (attachment, bytes).
    pub files: Vec<(Attachment, Vec<u8>)>,
}

fn address(c: &Contact) -> Address<'_> {
    let name = (!c.name.is_empty()).then_some(c.name.as_str());
    Address::new_address(name, c.email.as_str())
}

fn addresses(list: &[Contact]) -> Address<'_> {
    Address::new_list(list.iter().map(address).collect())
}

/// The message as RFC 5322 bytes: `multipart/alternative` (the markdown as
/// typed, and `out.html`), inside `multipart/mixed` when files go with it.
pub fn build_message(out: &Outgoing, build: &Build) -> Vec<u8> {
    let mut m = MessageBuilder::new().from(address(&out.from));
    if !out.to.is_empty() {
        m = m.to(addresses(&out.to));
    }
    if !out.cc.is_empty() {
        m = m.cc(addresses(&out.cc));
    }
    if !out.bcc.is_empty() {
        m = m.bcc(addresses(&out.bcc));
    }
    m = m
        .subject(out.subject.as_str())
        .date(build.date)
        .message_id(build.message_id.as_str());
    if let Some(t) = &build.threading {
        m = m
            .in_reply_to(t.in_reply_to.as_str())
            .references(MessageId::new_list(t.references.iter().map(String::as_str)));
    }
    if build.draft {
        m = m.header(MARKDOWN_HEADER, Raw::new("1"));
    }
    let html = if out.html.is_empty() {
        crate::markdown::to_html(&out.markdown)
    } else {
        out.html.clone()
    };
    m = m.text_body(out.markdown.clone()).html_body(html);
    for (attachment, bytes) in &build.files {
        m = m.attachment(
            attachment.mime.clone(),
            attachment.name.clone(),
            bytes.as_slice(),
        );
    }
    m.write_to_vec().unwrap_or_default()
}

/// A draft read back from `drafts.get format=raw`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedDraft {
    /// Written here (`X-Pigeon-Markdown: 1`).
    pub ours: bool,
    /// As typed when written here; otherwise the text.
    pub markdown: String,
    pub html: String,
    pub attachments: Vec<Attachment>,
}

pub fn parse_draft(raw: &[u8]) -> Option<ParsedDraft> {
    let message = MessageParser::default().parse(raw)?;
    let ours = message
        .headers()
        .get(MARKDOWN_HEADER)
        .is_some_and(|h| String::from_utf8_lossy(h.raw_value()).trim() == "1");
    let plain = message
        .text_body()
        .find(|p| p.is_content_type("text", "plain"))
        .and_then(|p| p.text().map(|t| t.into_owned()));
    let body = parse_message(raw)?;
    // Ours: the plain part is the markdown. Others: their plain part, or
    // the HTML's text.
    let markdown =
        plain.unwrap_or_else(|| crate::html::plain_text(&crate::html::parse(&body.html)));
    Some(ParsedDraft {
        ours,
        // Lines go out as CRLF; the editor's are LF.
        markdown: markdown.replace("\r\n", "\n").trim_end().to_string(),
        html: body.html,
        attachments: body.attachments,
    })
}

/// A file a raw message carries, by name: to send a draft's files again.
pub fn attachment_bytes(raw: &[u8], name: &str) -> Option<Vec<u8>> {
    let message = MessageParser::default().parse(raw)?;
    message
        .attachments()
        .find(|p| p.attachment_name() == Some(name))
        .map(|p| p.decoded().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(eml: &str) -> ParsedBody {
        parse_message(eml.as_bytes()).expect("parses")
    }

    #[test]
    fn latin1_body_is_decoded() {
        let body = parse(include_str!("../../tests/fixtures/google/gmail/latin1.eml"));
        assert!(body.plain.as_deref().unwrap().contains("Café à côté"));
        assert!(body.html.contains("Café à côté"));
        assert!(body.attachments.is_empty());
    }

    #[test]
    fn rfc2047_subject_and_alternative_prefers_html() {
        let raw = include_str!("../../tests/fixtures/google/gmail/alternative.eml");
        let body = parse(raw);
        assert!(body.html.contains("<b>bold</b>"), "{}", body.html);
        assert!(!body.html.contains("*bold*"));
        assert!(body.plain.as_deref().unwrap().contains("*bold*"));
        let message = MessageParser::default().parse(raw.as_bytes()).unwrap();
        assert_eq!(message.subject(), Some("Grüße aus München ☕"));
    }

    #[test]
    fn attachments_are_listed() {
        let body = parse(include_str!(
            "../../tests/fixtures/google/gmail/attachments.eml"
        ));
        assert!(body.html.contains("See attached"));
        let names: Vec<_> = body.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Q3 report.pdf", "résumé.txt"]);
        assert_eq!(body.attachments[0].mime, "application/pdf");
        assert_eq!(body.attachments[0].size_bytes, 11);
        assert_eq!(body.attachments[1].mime, "text/plain");
    }

    #[test]
    fn calendar_request_and_cancel() {
        let body = parse(include_str!(
            "../../tests/fixtures/google/gmail/invite_request.eml"
        ));
        assert_eq!(
            body.invite,
            Some(("abc123@example.com".into(), InviteMethod::Request))
        );
        // The .ics is the invite card, not a file.
        assert!(body.attachments.is_empty());
        let body = parse(include_str!(
            "../../tests/fixtures/google/gmail/invite_cancel.eml"
        ));
        assert_eq!(
            body.invite,
            Some(("folded-uid-0042@example.com".into(), InviteMethod::Cancel))
        );
    }

    #[test]
    fn ics_details_for_the_read_only_card() {
        let zone = LocalZone {
            name: "America/Los_Angeles",
            to_local: |t| t.naive_utc() - chrono::Duration::hours(7),
            to_utc: |t| (t + chrono::Duration::hours(7)).and_utc(),
        };
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\n\
            DTSTART:20261028T170000Z\r\nDTEND:20261028T173000Z\r\n\
            SUMMARY:Design review\\, round 2\r\nLOCATION:Room 4\r\n\
            ORGANIZER;CN=\"Ada: Lovelace\":mailto:ada@example.com\r\n\
            UID:r2@example.com\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Other\r\n\
            END:VEVENT\r\nEND:VCALENDAR\r\n";
        assert_eq!(
            parse_ics(ics),
            Some(("r2@example.com".into(), InviteMethod::Request))
        );
        let d = parse_ics_details(ics, &zone).unwrap();
        assert_eq!(d.title, "Design review, round 2");
        assert_eq!(d.location, "Room 4");
        assert_eq!(
            d.start.unwrap().to_string(),
            "2026-10-28 10:00:00",
            "UTC to the local wall time"
        );
        assert_eq!(d.end.unwrap().to_string(), "2026-10-28 10:30:00");
        assert!(!d.all_day);
        assert_eq!(d.organizer.name, "Ada: Lovelace");
        assert_eq!(d.organizer.email, "ada@example.com");

        let all_day = "BEGIN:VEVENT\nDTSTART;VALUE=DATE:20261102\nSUMMARY:Offsite\nEND:VEVENT";
        let d = parse_ics_details(all_day, &zone).unwrap();
        assert!(d.all_day);
        assert_eq!(d.start.unwrap().to_string(), "2026-11-02 00:00:00");

        let body = parse(include_str!(
            "../../tests/fixtures/google/gmail/invite_request.eml"
        ));
        let d = body.invite_details.unwrap();
        assert_eq!(d.title, "Planning");
        // A TZID time is a wall time.
        assert_eq!(d.start.unwrap().to_string(), "2026-11-02 10:00:00");
    }

    // Build (W6).

    use crate::data::{Contact, Message};

    fn contact(name: &str, email: &str) -> Contact {
        Contact {
            name: name.into(),
            email: email.into(),
        }
    }

    fn outgoing() -> Outgoing {
        Outgoing {
            from: contact("Me Myself", "me@example.com"),
            to: vec![
                contact("Ana Lima", "ana@example.com"),
                contact("", "bo@example.com"),
            ],
            cc: vec![contact("", "cc@example.com")],
            bcc: vec![contact("", "hidden@example.com")],
            subject: "Lunch on Friday?".into(),
            markdown: "Are you **free**?\n\n- yes\n- no".into(),
            html: String::new(),
            ..Outgoing::default()
        }
    }

    fn build(
        draft: bool,
        threading: Option<Threading>,
        files: Vec<(Attachment, Vec<u8>)>,
    ) -> Build {
        Build {
            message_id: "abc123@example.com".into(),
            // 2026-10-27 12:00:00 UTC.
            date: 1_793_102_400,
            threading,
            draft,
            files,
        }
    }

    /// The header block, and the body with its random boundaries named.
    fn snapshot(raw: &[u8]) -> (String, String) {
        let mut text = String::from_utf8(raw.to_vec()).unwrap();
        let boundaries: Vec<String> = text
            .split("boundary=\"")
            .skip(1)
            .map(|rest| rest.split('"').next().unwrap().to_string())
            .collect();
        for (n, b) in boundaries.iter().enumerate() {
            text = text.replace(b, &format!("B{}", n + 1));
        }
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        (head.to_string(), body.to_string())
    }

    #[test]
    fn plain_and_html_alternative_with_every_header() {
        let raw = build_message(&outgoing(), &build(false, None, Vec::new()));
        let (head, body) = snapshot(&raw);
        assert_eq!(
            head,
            "From: \"Me Myself\" <me@example.com>\r\n\
             To: \"Ana Lima\" <ana@example.com>, <bo@example.com>\r\n\
             Cc: <cc@example.com>\r\n\
             Bcc: <hidden@example.com>\r\n\
             Subject: Lunch on Friday?\r\n\
             Date: Tue, 27 Oct 2026 12:00:00 +0000\r\n\
             Message-ID: <abc123@example.com>\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/alternative;\r\n boundary=\"B1\""
        );
        assert!(!head.contains(MARKDOWN_HEADER));
        let message = MessageParser::default().parse(&raw).unwrap();
        assert_eq!(
            message.body_text(0).unwrap(),
            "Are you **free**?\r\n\r\n- yes\r\n- no"
        );
        let html = message.body_html(0).unwrap();
        assert!(html.contains("<strong>free</strong>"), "{html}");
        assert!(html.contains("<li>yes</li>"));
        assert!(
            body.contains("Content-Type: text/plain; charset=\"utf-8\""),
            "{body}"
        );
        assert!(body.contains("Content-Type: text/html; charset=\"utf-8\""));
    }

    #[test]
    fn attachments_make_it_mixed() {
        let file = Attachment::from_file("report.pdf".into(), 5);
        let raw = build_message(
            &outgoing(),
            &build(false, None, vec![(file, b"%PDF-".to_vec())]),
        );
        let (head, body) = snapshot(&raw);
        assert!(
            head.ends_with("Content-Type: multipart/mixed;\r\n boundary=\"B1\""),
            "{head}"
        );
        assert!(body.contains("Content-Type: multipart/alternative;\r\n boundary=\"B2\""));
        assert!(body.contains("Content-Type: application/pdf"));
        assert!(body.contains("Content-Disposition: attachment; filename=\"report.pdf\""));
        let parsed = parse_message(&raw).unwrap();
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].name, "report.pdf");
        assert_eq!(attachment_bytes(&raw, "report.pdf").unwrap(), b"%PDF-");
    }

    #[test]
    fn non_ascii_subject_is_encoded() {
        let out = Outgoing {
            subject: "Grüße aus München ☕".into(),
            ..outgoing()
        };
        let raw = build_message(&out, &build(false, None, Vec::new()));
        let (head, _) = snapshot(&raw);
        let subject = head.lines().find(|l| l.starts_with("Subject:")).unwrap();
        assert!(subject.contains("=?utf-8?"), "{subject}");
        assert!(head.is_ascii());
        let message = MessageParser::default().parse(&raw).unwrap();
        assert_eq!(message.subject(), Some("Grüße aus München ☕"));
    }

    #[test]
    fn reply_headers_and_the_references_chain() {
        let mut parent = Message::new(contact("Ana", "ana@example.com"));
        parent.message_id = "m3@example.com".into();
        parent.references = vec!["m1@example.com".into(), "m2@example.com".into()];
        let t = reply_threading(&parent).unwrap();
        assert_eq!(t.in_reply_to, "m3@example.com");
        assert_eq!(
            t.references,
            ["m1@example.com", "m2@example.com", "m3@example.com"]
        );
        // Only In-Reply-To known: it starts the chain.
        parent.references.clear();
        parent.in_reply_to = "m2@example.com".into();
        assert_eq!(
            reply_threading(&parent).unwrap().references,
            ["m2@example.com", "m3@example.com"]
        );
        parent.message_id.clear();
        assert!(reply_threading(&parent).is_none());

        let raw = build_message(&outgoing(), &build(false, Some(t), Vec::new()));
        let (head, _) = snapshot(&raw);
        assert!(
            head.contains(
                "In-Reply-To: <m3@example.com>\r\n\
             References: <m1@example.com> <m2@example.com> <m3@example.com>\r\n"
            ),
            "{head}"
        );
    }

    #[test]
    fn drafts_round_trip_their_markdown() {
        let raw = build_message(&outgoing(), &build(true, None, Vec::new()));
        assert!(snapshot(&raw).0.contains("X-Pigeon-Markdown: 1\r\n"));
        let draft = parse_draft(&raw).unwrap();
        assert!(draft.ours);
        assert_eq!(draft.markdown, "Are you **free**?\n\n- yes\n- no");
        assert!(draft.html.contains("<strong>free</strong>"));

        // One written in Gmail opens as its text.
        let web = "From: me@example.com\r\nSubject: Hi\r\nContent-Type: text/html\r\n\r\n<p>Hello <b>there</b></p>";
        let draft = parse_draft(web.as_bytes()).unwrap();
        assert!(!draft.ours);
        assert_eq!(draft.markdown, "Hello there");
    }

    #[test]
    fn message_ids_use_the_senders_domain() {
        let id = new_message_id("me@example.com");
        assert!(id.ends_with("@example.com") && id.len() > 20);
        assert!(new_message_id("").ends_with("@pigeon.local"));
    }

    #[test]
    fn raw_decodes_with_or_without_padding() {
        assert_eq!(decode_raw("aGk_").unwrap(), b"hi?");
        assert_eq!(decode_raw("aGk").unwrap(), b"hi");
        assert_eq!(decode_raw("aGk=").unwrap(), b"hi");
    }
}

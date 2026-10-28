//! MIME. Parsing raw messages from `messages.get format=raw` (W3, RDR-6);
//! building outgoing ones and reading drafts back (W6).

use base64::Engine as _;
use mail_builder::{
    MessageBuilder,
    headers::{address::Address, message_id::MessageId, raw::Raw},
};
use mail_parser::MessageParser;

use super::Outgoing;
use crate::data::{Attachment, Contact, Message};

// Parse (W3).

/// What the reader needs from a raw message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedBody {
    pub html: String,
    pub plain: Option<String>,
    pub attachments: Vec<Attachment>,
    /// Images the HTML shows by `cid:` (ATT-2), under the address `html`
    /// now uses for them.
    pub inline_images: Vec<InlineImage>,
}

/// An image a message carries for its HTML (ATT-2).
#[derive(Clone, Debug, PartialEq)]
pub struct InlineImage {
    /// `cid:pigeon-<hash of the bytes>`: unique, so it can share the image
    /// cache with remote images.
    pub url: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// The `cid:` address an inline image is shown and cached under.
pub fn inline_url(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(bytes);
    let hex: String = hash[..12].iter().map(|b| format!("{b:02x}")).collect();
    format!("cid:pigeon-{hex}")
}

/// Whether `src` is an inline image's address.
pub fn is_cid(src: &str) -> bool {
    src.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("cid:"))
}

/// The message's images with a Content-ID, by that id (lower case).
fn inline_parts(message: &mail_parser::Message) -> Vec<(String, InlineImage)> {
    let mut out: Vec<(String, InlineImage)> = Vec::new();
    for part in message.parts() {
        let Some(id) = part.content_id() else {
            continue;
        };
        if !part
            .content_type()
            .is_some_and(|ct| ct.ctype().eq_ignore_ascii_case("image"))
        {
            continue;
        }
        let bytes = part.decoded();
        if bytes.len() > super::images::IMAGE_LIMIT {
            continue;
        }
        let Some((mime, _, _)) = super::images::sniff(&bytes) else {
            continue;
        };
        let id = id.trim().trim_matches(['<', '>']).to_lowercase();
        if out.iter().any(|(seen, _)| *seen == id) {
            continue;
        }
        out.push((
            id,
            InlineImage {
                url: inline_url(&bytes),
                mime,
                bytes: bytes.into_owned(),
            },
        ));
    }
    out
}

/// `%XX` escapes decoded, as `cid:` URLs may carry them (RFC 2392).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Every `cid:<id>` in `html` whose id is in `ids` (lower case) pointed at
/// its image's own address.
fn rewrite_cids(html: &str, ids: &[(String, InlineImage)]) -> String {
    if ids.is_empty() {
        return html.to_string();
    }
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut last = 0;
    let mut from = 0;
    while let Some(at) = lower[from..].find("cid:").map(|i| i + from) {
        let start = at + 4;
        let end = html[start..]
            .find(|c: char| matches!(c, '"' | '\'' | '>' | ')' | '<') || c.is_whitespace())
            .map_or(html.len(), |i| i + start);
        let id = percent_decode(&html[start..end]).to_lowercase();
        if let Some((_, image)) = ids.iter().find(|(seen, _)| *seen == id) {
            out.push_str(&html[last..at]);
            out.push_str(&image.url);
            last = end;
        }
        from = end.max(start);
    }
    out.push_str(&html[last..]);
    out
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

/// The body as HTML (plain text converted) and its attachments. Charsets, transfer encodings and RFC 2047 names are decoded.
pub fn parse_message(raw: &[u8]) -> Option<ParsedBody> {
    let message = MessageParser::default().parse(raw)?;
    let html: String = message.html_bodies().collect::<Vec<_>>().join("\n");
    let plain: String = message.text_bodies().collect::<Vec<_>>().join("\n");

    let attachments = message
        .attachments()
        .filter(|part| {
            // An invitation's unnamed iCalendar alternative is part of
            // the body; a named .ics is a file like any other.
            !(part.is_content_type("text", "calendar") && part.attachment_name().is_none())
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
                message_id: String::new(),
            }
        })
        .collect();

    // ATT-2: the images go to the cache with the body, so no request
    // fetches them.
    let inline = inline_parts(&message);
    let html = rewrite_cids(&html, &inline);
    Some(ParsedBody {
        html,
        plain: (!plain.trim().is_empty()).then_some(plain),
        attachments,
        inline_images: inline.into_iter().map(|(_, image)| image).collect(),
    })
}

/// A part's bytes, transfer encoding undone, by the `part_id` that
/// `parse_message` gave its attachment (ATT-4).
pub fn part_bytes(raw: &[u8], part_id: &str) -> Option<Vec<u8>> {
    let id: mail_parser::PartId = part_id.parse().ok()?;
    let message = MessageParser::default().parse(raw)?;
    Some(message.part(id)?.decoded().into_owned())
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
    if !out.reply_to_address.is_empty() {
        m = m.reply_to(Address::new_address(
            None::<&str>,
            out.reply_to_address.as_str(),
        ));
    }
    if build.draft {
        m = m.header(MARKDOWN_HEADER, Raw::new("1"));
        if !out.signature.is_empty() {
            m = m.header(SIGNATURE_HEADER, Raw::new("1"));
        }
    }
    let html = if out.html.is_empty() {
        let mut html = crate::markdown::to_html(&out.markdown);
        if !out.signature.is_empty() {
            html.push_str(&signature_block(&out.signature));
        }
        html
    } else {
        out.html.clone()
    };
    let mut text = out.markdown.clone();
    if !out.signature.is_empty() {
        text.push_str(SIGNATURE_DELIMITER);
        text.push_str(&signature_text(&out.signature));
    }
    m = m.text_body(text).html_body(html);
    for (attachment, bytes) in &build.files {
        m = m.attachment(
            attachment.mime.clone(),
            attachment.name.clone(),
            bytes.as_slice(),
        );
    }
    m.write_to_vec().unwrap_or_default()
}

// Signatures (W3, MST-2).

/// Marks a draft written here that carries its alias's signature, after
/// `SIGNATURE_DELIMITER` in the plain-text part.
pub const SIGNATURE_HEADER: &str = "X-Pigeon-Signature";
/// What comes before the signature in the plain-text part (RFC 3676).
pub const SIGNATURE_DELIMITER: &str = "\n\n-- \n";

/// The signature as Gmail web writes it, so it knows it again there.
pub fn signature_block(signature: &str) -> String {
    format!(
        "<br><span class=\"gmail_signature_prefix\">-- </span><br>\
         <div dir=\"ltr\" class=\"gmail_signature\" data-smartmail=\"gmail_signature\">{signature}</div>"
    )
}

/// The signature's text, for the plain-text part.
pub fn signature_text(signature: &str) -> String {
    crate::html::plain_text(&crate::html::parse(signature))
        .trim()
        .to_string()
}

/// `html` without its Gmail signature block (and the "-- " before it),
/// if it has one.
pub fn strip_signature(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let class = lower.find("class=\"gmail_signature\"")?;
    let start = lower[..class].rfind('<')?;
    let tag = lower[start + 1..]
        .split(|c: char| c.is_whitespace() || c == '>')
        .next()
        .unwrap_or("div")
        .to_string();
    // The element's end: its matching close tag, counting nested ones.
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut depth = 0usize;
    let mut at = start;
    let end = loop {
        let next_open = lower[at..].find(&open).map(|i| i + at);
        let next_close = lower[at..].find(&close).map(|i| i + at);
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                at = o + open.len();
            }
            (_, Some(c)) => {
                depth = depth.saturating_sub(1);
                at = c + close.len();
                if depth == 0 {
                    break at;
                }
            }
            _ => break html.len(),
        }
    };
    // Gmail puts "-- " in a span of its own before it.
    let mut cut = start;
    if let Some(prefix) = lower[..start].rfind("<span class=\"gmail_signature_prefix\"") {
        cut = prefix;
    }
    let before = html[..cut].trim_end();
    let before = before
        .strip_suffix("<br>")
        .or_else(|| before.strip_suffix("<br clear=\"all\">"))
        .unwrap_or(before);
    Some(format!("{before}{}", &html[end..]))
}

/// A draft read back from `drafts.get format=raw`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedDraft {
    /// Written here (`X-Pigeon-Markdown: 1`).
    pub ours: bool,
    /// W3 (MST-2): it carried a signature, which `markdown` leaves out.
    pub signature: bool,
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
    let header = |name: &str| {
        message
            .headers()
            .get(name)
            .is_some_and(|h| String::from_utf8_lossy(h.raw_value()).trim() == "1")
    };
    let ours_signed = ours && header(SIGNATURE_HEADER);
    let plain = message
        .text_body()
        .find(|p| p.is_content_type("text", "plain"))
        .and_then(|p| p.text().map(|t| t.into_owned()));
    let body = parse_message(raw)?;
    // Written in Gmail with its signature: the signature comes out before
    // the text goes to the editor, so it isn't added twice (MST-2).
    let web_signed = (!ours).then(|| strip_signature(&body.html)).flatten();
    let signature = ours_signed || web_signed.is_some();
    // Ours: the plain part is the markdown, then the signature. Others:
    // their plain part, or the HTML's text.
    let markdown = match (web_signed, plain) {
        (Some(html), _) => crate::html::plain_text(&crate::html::parse(&html)),
        (None, Some(plain)) if ours_signed => {
            let plain = plain.replace("\r\n", "\n");
            match plain.rfind(SIGNATURE_DELIMITER) {
                Some(at) => plain[..at].to_string(),
                None => plain,
            }
        }
        (None, Some(plain)) => plain,
        (None, None) => crate::html::plain_text(&crate::html::parse(&body.html)),
    };
    Some(ParsedDraft {
        ours,
        signature,
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
    fn an_invitations_ics_is_an_attachment() {
        let body = parse(include_str!(
            "../../tests/fixtures/google/gmail/invite_request.eml"
        ));
        let names: Vec<_> = body.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["invite.ics"]);
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

    // W3 (MST-1, MST-2).

    #[test]
    fn signatures_go_in_both_parts_and_come_out_once() {
        let out = Outgoing {
            signature: "<b>Me</b><br>Example".into(),
            reply_to_address: "team@example.com".into(),
            ..outgoing()
        };
        let raw = build_message(&out, &build(true, None, Vec::new()));
        let (head, _) = snapshot(&raw);
        assert!(head.contains("X-Pigeon-Signature: 1\r\n"), "{head}");
        assert!(head.contains("Reply-To: <team@example.com>\r\n"), "{head}");
        let message = MessageParser::default().parse(&raw).unwrap();
        assert!(
            message
                .body_text(0)
                .unwrap()
                .ends_with("- no\r\n\r\n-- \r\nMe\r\nExample"),
            "{:?}",
            message.body_text(0)
        );
        let html = message.body_html(0).unwrap();
        assert!(html.contains("class=\"gmail_signature\""), "{html}");
        assert!(html.contains("<b>Me</b>"));
        // Reopened: the markdown is the note alone, flagged as signed.
        let draft = parse_draft(&raw).unwrap();
        assert!(draft.ours && draft.signature);
        assert_eq!(draft.markdown, "Are you **free**?\n\n- yes\n- no");
        // Without one, nothing is added or flagged.
        let raw = build_message(&outgoing(), &build(true, None, Vec::new()));
        assert!(!parse_draft(&raw).unwrap().signature);
        assert!(!String::from_utf8_lossy(&raw).contains("gmail_signature"));
    }

    #[test]
    fn a_gmail_web_draft_loses_its_signature_before_the_editor() {
        let web = "From: me@example.com\r\nSubject: Hi\r\nMIME-Version: 1.0\r\n\
            Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
            --b\r\nContent-Type: text/plain\r\n\r\nHello there\r\n\r\n-- \r\nMe\r\nExample\r\n\
            --b\r\nContent-Type: text/html\r\n\r\n<div dir=\"ltr\"><div>Hello there</div>\
            <br clear=\"all\"><div><br></div><span class=\"gmail_signature_prefix\">-- </span><br>\
            <div dir=\"ltr\" class=\"gmail_signature\" data-smartmail=\"gmail_signature\">\
            <div dir=\"ltr\"><b>Me</b><div>Example</div></div></div></div>\r\n--b--\r\n";
        let draft = parse_draft(web.as_bytes()).unwrap();
        assert!(!draft.ours);
        assert!(draft.signature, "found, so it isn't added again");
        assert_eq!(draft.markdown, "Hello there");
        // A Gmail draft without one keeps its text.
        let plain = "From: me@example.com\r\nContent-Type: text/html\r\n\r\n<p>Just text</p>";
        let draft = parse_draft(plain.as_bytes()).unwrap();
        assert!(!draft.signature);
        assert_eq!(draft.markdown, "Just text");
    }

    #[test]
    fn strip_signature_takes_only_the_signature() {
        let html = format!(
            "<p>Hi</p>{}<blockquote>quoted</blockquote>",
            signature_block("<div>Me</div>")
        );
        assert_eq!(
            strip_signature(&html).unwrap(),
            "<p>Hi</p><blockquote>quoted</blockquote>"
        );
        assert!(strip_signature("<p>Hi</p>").is_none());
    }

    #[test]
    fn message_ids_use_the_senders_domain() {
        let id = new_message_id("me@example.com");
        assert!(id.ends_with("@example.com") && id.len() > 20);
        assert!(new_message_id("").ends_with("@pigeon.local"));
    }

    // W2 (ATT-2, ATT-4).

    /// A 1×1 PNG.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";

    fn nested() -> String {
        use base64::Engine as _;
        let png = base64::engine::general_purpose::STANDARD.encode(PNG);
        let pdf = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.7 nested");
        format!(
            "From: a@example.com\r\nSubject: Nested\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n\
             --outer\r\nContent-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n\
             --alt\r\nContent-Type: text/plain\r\n\r\nHi\r\n\
             --alt\r\nContent-Type: multipart/related; boundary=\"rel\"\r\n\r\n\
             --rel\r\nContent-Type: text/html\r\n\r\n\
             <p>Logo <img src=\"cid:logo%40x\"> and <img src='CID:logo@x'> and <img src=\"cid:missing\"></p>\r\n\
             --rel\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\n\
             Content-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
             --rel--\r\n--alt--\r\n\
             --outer\r\nContent-Type: text/plain; charset=utf-8\r\n\
             Content-Disposition: attachment; filename=\"notes.txt\"\r\n\
             Content-Transfer-Encoding: quoted-printable\r\n\r\nCaf=C3=A9 =3D good\r\n\
             --outer\r\nContent-Type: application/pdf\r\n\
             Content-Disposition: attachment; filename=\"deep.pdf\"\r\n\
             Content-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n\
             --outer--\r\n"
        )
    }

    #[test]
    fn parts_come_out_by_id_decoded() {
        let eml = nested();
        let body = parse(&eml);
        let names: Vec<_> = body.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            ["notes.txt", "deep.pdf"],
            "the inline image isn't a file"
        );
        let bytes = |n: usize| part_bytes(eml.as_bytes(), &body.attachments[n].part_id).unwrap();
        assert_eq!(bytes(0), "Café = good".as_bytes());
        assert_eq!(bytes(1), b"%PDF-1.7 nested");
        assert_eq!(body.attachments[1].size_bytes, 15);
        assert!(part_bytes(eml.as_bytes(), "99").is_none());
        assert!(part_bytes(eml.as_bytes(), "x").is_none());

        // The fixture's files too.
        let raw = include_bytes!("../../tests/fixtures/google/gmail/attachments.eml");
        let body = parse_message(raw).unwrap();
        for a in &body.attachments {
            assert_eq!(
                part_bytes(raw, &a.part_id).unwrap(),
                attachment_bytes(raw, &a.name).unwrap()
            );
        }
    }

    #[test]
    fn cid_images_get_their_own_address() {
        let body = parse(&nested());
        assert_eq!(body.inline_images.len(), 1);
        let image = &body.inline_images[0];
        assert_eq!(image.bytes, PNG);
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.url, inline_url(PNG));
        assert!(image.url.starts_with("cid:pigeon-"));
        // Encoded and upper-case references both point at it; unknown
        // ones are left alone.
        assert_eq!(
            body.html.matches(image.url.as_str()).count(),
            2,
            "{}",
            body.html
        );
        assert!(body.html.contains("cid:missing"));
        assert!(!body.html.contains("logo"));
        assert!(is_cid(&image.url) && !is_cid("https://x"));
    }

    #[test]
    fn raw_decodes_with_or_without_padding() {
        assert_eq!(decode_raw("aGk_").unwrap(), b"hi?");
        assert_eq!(decode_raw("aGk").unwrap(), b"hi");
        assert_eq!(decode_raw("aGk=").unwrap(), b"hi");
    }
}

//! MIME. Parsing raw messages from `messages.get format=raw` (W3, RDR-6);
//! building outgoing ones comes with W6.

use base64::Engine as _;
use mail_parser::MessageParser;

use crate::data::{Attachment, InviteMethod};

// Parse (W3).

/// What the reader needs from a raw message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedBody {
    pub html: String,
    pub plain: Option<String>,
    pub attachments: Vec<Attachment>,
    /// A `text/calendar` part's UID and METHOD.
    pub invite: Option<(String, InviteMethod)>,
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
    for part in message.parts() {
        if part.is_content_type("text", "calendar")
            && invite.is_none()
            && let Some(text) = part.text()
        {
            invite = parse_ics(&text);
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
            }
        })
        .collect();

    Some(ParsedBody {
        html,
        plain: (!plain.trim().is_empty()).then_some(plain),
        attachments,
        invite,
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
    fn raw_decodes_with_or_without_padding() {
        assert_eq!(decode_raw("aGk_").unwrap(), b"hi?");
        assert_eq!(decode_raw("aGk").unwrap(), b"hi");
        assert_eq!(decode_raw("aGk=").unwrap(), b"hi");
    }
}

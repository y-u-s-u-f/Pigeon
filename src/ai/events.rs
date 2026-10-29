//! AI-16, dates in mail (W5): NSDataDetector and the Quick Add parser. A
//! stub until W5.

use chrono::{DateTime, Utc};

use super::EventCandidate;

/// At most 3 per message, earliest first.
pub fn candidates(text: &str, sent: DateTime<Utc>) -> Vec<EventCandidate> {
    let _ = (text, sent);
    Vec::new()
}

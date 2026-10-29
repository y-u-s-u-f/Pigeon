//! AI-10, Clean View (W3). A stub until W3.

use super::{llm, text};

/// The request for a message's Clean View; `None` when it's over budget
/// (never trimmed).
pub fn request(msg: &text::Message) -> Option<llm::Request> {
    let _ = msg;
    None
}

/// The answer with links and images not in the original made plain text.
pub fn clean(markdown: &str, original_html: &str) -> String {
    let _ = (markdown, original_html);
    String::new()
}

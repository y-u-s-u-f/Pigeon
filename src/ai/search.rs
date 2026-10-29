//! AI-13 Best Matches and AI-14 Ask Pigeon (W4). A stub until W4.

use super::{Source, jev, llm};
use crate::data::MailData;

/// A conversation as Jev sees it for reranking, by an opaque id (`c1`…).
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub id: String,
    pub subject: String,
    pub sender: String,
    pub date: String,
    pub snippet: String,
}

/// Indexes into `data.threads` worth reranking, best first, at most `limit`.
pub fn candidates(terms: &[String], data: &MailData, limit: usize) -> Vec<usize> {
    let _ = (terms, data, limit);
    Vec::new()
}

/// One `score` question per record.
pub fn rerank(query: &str, records: &[Record]) -> jev::Request {
    let _ = (query, records);
    jev::Request::default()
}

/// The Ask prompt within `budget` tokens, and the sources it holds.
pub fn ask_request(
    question: &str,
    sources: Vec<Source>,
    budget: usize,
) -> (llm::Request, Vec<Source>) {
    let _ = (question, budget);
    (llm::Request::new("", "", 0), sources)
}

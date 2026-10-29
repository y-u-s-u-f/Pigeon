//! AI-3, the background pass (W2): what to ask about next, from the heads
//! and their `ai_results`. A stub until W2.

use std::collections::HashMap;

use super::{Feature, Job, ThreadAi};
use crate::data::Thread;

/// Jobs for Inbox conversations of the last 30 days whose results are
/// missing or stale, newest first, skipping keys `skip` names (queued,
/// running or failed).
pub fn work_list(
    heads: &[Thread],
    results: &HashMap<String, ThreadAi>,
    skip: &dyn Fn(Feature, &str) -> bool,
) -> Vec<Job> {
    let _ = (heads, results, skip);
    Vec::new()
}

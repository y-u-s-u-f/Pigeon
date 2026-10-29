//! The last 100 AI calls, in memory, for Settings › AI › Activity. Never
//! the text sent or received, and never a key.

use std::collections::VecDeque;

use super::Feature;

pub const LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Unix ms.
    pub at: i64,
    pub feature: Feature,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub ms: u64,
    /// `Err` holds a short reason, such as "HTTP 429" or "Timed out".
    pub result: Result<(), String>,
}

#[derive(Default)]
pub struct Log {
    entries: VecDeque<Entry>,
}

impl Log {
    pub fn push(&mut self, entry: Entry) {
        if self.entries.len() == LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Newest first.
    pub fn entries(&self) -> Vec<Entry> {
        self.entries.iter().rev().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_hundred_newest_first() {
        let mut log = Log::default();
        for at in 0..150 {
            log.push(Entry {
                at,
                feature: Feature::Line,
                model: "m".into(),
                tokens_in: 1,
                tokens_out: 1,
                ms: 1,
                result: Ok(()),
            });
        }
        let entries = log.entries();
        assert_eq!(entries.len(), LIMIT);
        assert_eq!((entries[0].at, entries[99].at), (149, 50));
    }
}

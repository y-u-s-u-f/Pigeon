//! AI-5, Important Only notifications: each batch of new mail waits for
//! Jev's `important` on every message, `DEADLINE` at most, then goes out
//! without the ones Jev finds unimportant. No answer keeps a message: the
//! filter fails open. Pure: the clock comes in as `now`.

use std::time::{Duration, Instant};

use crate::backend::new_mail::NewMail;

/// How long a batch waits for its answers.
pub const DEADLINE: Duration = Duration::from_secs(3);
/// `important` yes below this leaves a message out. Phase 4 tunes it.
pub const UNIMPORTANT: f64 = 0.2;

struct Batch {
    mail: Vec<NewMail>,
    /// By position in `mail`: answered (`Some`), with its yes if any.
    answers: Vec<Option<Option<f64>>>,
    deadline: Instant,
}

impl Batch {
    fn done(&self, now: Instant) -> bool {
        now >= self.deadline || self.answers.iter().all(Option::is_some)
    }

    fn survivors(self) -> Vec<NewMail> {
        self.mail
            .into_iter()
            .zip(self.answers)
            .filter(|(_, answer)| !matches!(answer, Some(Some(p)) if *p < UNIMPORTANT))
            .map(|(m, _)| m)
            .collect()
    }
}

#[derive(Default)]
pub struct Pending {
    batches: Vec<Batch>,
}

impl Pending {
    pub fn hold(&mut self, mail: Vec<NewMail>, now: Instant) {
        if mail.is_empty() {
            return;
        }
        self.batches.push(Batch {
            answers: vec![None; mail.len()],
            mail,
            deadline: now + DEADLINE,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    /// (thread id, message id) of every message still without an answer.
    pub fn waiting(&self) -> Vec<(String, String)> {
        self.batches
            .iter()
            .flat_map(|b| b.mail.iter().zip(&b.answers))
            .filter(|(_, a)| a.is_none())
            .map(|(m, _)| (m.thread_id.clone(), m.message_id.clone()))
            .collect()
    }

    /// Jev answered for `message_id` (`None`: it failed or gave nothing).
    /// One for a batch already sent does nothing.
    pub fn answer(&mut self, message_id: &str, yes: Option<f64>) {
        for batch in &mut self.batches {
            for (m, a) in batch.mail.iter().zip(&mut batch.answers) {
                if m.message_id == message_id && a.is_none() {
                    *a = Some(yes);
                }
            }
        }
    }

    /// The batches all answered or past their deadline, filtered; each
    /// is one call to notify, empty ones left out.
    pub fn take_ready(&mut self, now: Instant) -> Vec<Vec<NewMail>> {
        let (ready, waiting) = std::mem::take(&mut self.batches)
            .into_iter()
            .partition::<Vec<_>, _>(|b| b.done(now));
        self.batches = waiting;
        ready
            .into_iter()
            .map(Batch::survivors)
            .filter(|m| !m.is_empty())
            .collect()
    }

    /// Everything waiting, unfiltered: AI went off, or Notify for went
    /// back to All New Mail.
    pub fn take_all(&mut self) -> Vec<Vec<NewMail>> {
        std::mem::take(&mut self.batches)
            .into_iter()
            .map(|b| b.mail)
            .collect()
    }

    /// Another account or the demo: nothing waiting goes.
    pub fn clear(&mut self) {
        self.batches.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(n: usize) -> NewMail {
        NewMail {
            message_id: format!("m{n}"),
            thread_id: format!("t{n}"),
            from: "Dana".into(),
            subject: format!("S{n}"),
            snippet: String::new(),
        }
    }

    fn ids(batches: &[Vec<NewMail>]) -> Vec<Vec<&str>> {
        batches
            .iter()
            .map(|b| b.iter().map(|m| m.message_id.as_str()).collect())
            .collect()
    }

    #[test]
    fn all_answered_goes_at_once_without_the_unimportant() {
        let start = Instant::now();
        let mut p = Pending::default();
        p.hold(vec![mail(1), mail(2), mail(3)], start);
        assert_eq!(p.waiting().len(), 3);
        p.answer("m1", Some(0.9));
        p.answer("m2", Some(0.05));
        assert!(p.take_ready(start).is_empty());
        // No answer (a failure) keeps it.
        p.answer("m3", None);
        let out = p.take_ready(start);
        assert_eq!(ids(&out), [["m1", "m3"]]);
        assert!(p.is_empty());
    }

    #[test]
    fn the_deadline_sends_what_has_no_answer() {
        let start = Instant::now();
        let mut p = Pending::default();
        p.hold(vec![mail(1), mail(2)], start);
        p.answer("m2", Some(0.1));
        assert!(p.take_ready(start + Duration::from_secs(2)).is_empty());
        let out = p.take_ready(start + DEADLINE);
        assert_eq!(ids(&out), [["m1"]]);
        // A late answer does nothing.
        p.answer("m1", Some(0.0));
        assert!(p.take_ready(start + DEADLINE * 2).is_empty());
    }

    #[test]
    fn all_unimportant_sends_nothing() {
        let start = Instant::now();
        let mut p = Pending::default();
        p.hold(vec![mail(1)], start);
        p.answer("m1", Some(0.01));
        assert!(p.take_ready(start).is_empty());
        assert!(p.is_empty());
    }

    #[test]
    fn ai_off_mid_wait_sends_unfiltered() {
        let start = Instant::now();
        let mut p = Pending::default();
        p.hold(vec![mail(1), mail(2)], start);
        p.answer("m1", Some(0.01));
        assert_eq!(ids(&p.take_all()), [["m1", "m2"]]);
        assert!(p.is_empty());
        p.hold(vec![mail(3)], start);
        p.clear();
        assert!(p.take_all().is_empty());
    }

    #[test]
    fn a_burst_of_twenty_is_one_call() {
        let start = Instant::now();
        let mut p = Pending::default();
        p.hold((0..20).map(mail).collect(), start);
        for n in 0..20 {
            p.answer(&format!("m{n}"), Some(0.5));
        }
        let out = p.take_ready(start);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 20);
    }

    #[test]
    fn batches_keep_to_themselves() {
        let start = Instant::now();
        let mut p = Pending::default();
        p.hold(vec![mail(1)], start);
        p.hold(vec![mail(2)], start + Duration::from_secs(1));
        p.answer("m2", Some(0.9));
        assert_eq!(ids(&p.take_ready(start)), [["m2"]]);
        assert_eq!(ids(&p.take_ready(start + DEADLINE)), [["m1"]]);
    }
}

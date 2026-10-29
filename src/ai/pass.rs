//! AI-3, the background pass: what to ask about next, from the heads and
//! their `ai_results`, and the requests themselves. Also AI-4's rule for
//! the Needs Reply marker. Pure: the backend loads, queues and stores.
//!
//! - Request A (`important`): sender, subject and snippet of new mail, at
//!   once (AI-5).
//! - Request B (`signals`): once a message's body is stored, whether it
//!   needs a reply, whether it's a newsletter, and whether each date found
//!   in it is an event (AI-4, AI-10, AI-16).
//! - The list's one-line summary (`line`, AI-7), by the writing model.

use std::collections::{HashMap, HashSet};

use chrono::Utc;

use super::{
    AiRow, EventCandidate, Feature, Job, Priority, ThreadAi, Work, events, input_hash, jev, llm,
    reading, text,
};
use crate::{
    backend::new_mail::NewMail,
    data::{Account, Folder, Message, Thread},
};

/// The pass looks this far back in the Inbox.
pub const DAYS: i64 = 30;
/// Jobs one run of the pass queues, at most.
pub const MAX_JOBS: usize = 100;
/// AI-4: `needs_reply` yes at least this shows the marker…
pub const NEEDS_REPLY: f64 = 0.8;
/// …unless the same message's `newsletter` yes is at least this (no
/// answer: not a newsletter).
pub const NEEDS_REPLY_NEWSLETTER: f64 = 0.5;
/// AI-7: the longest line shown.
pub const LINE_MAX: usize = 120;
/// The line reads this many of the conversation's latest messages.
const LINE_MESSAGES: usize = 3;
/// Messages of an opened conversation asked about (request B).
const OPEN_MESSAGES: usize = 3;

// The question and prompt versions: a change here changes every hash, so
// old results are asked again.
pub const IMPORTANT_V: &str = "important-v2";
pub const SIGNALS_V: &str = "signals-v1";
pub const LINE_V: &str = "line-v2";

pub const Q_IMPORTANT: &str = "This message needs my attention soon.";
pub const Q_NEEDS_REPLY: &str = "The latest message asks me something or expects my answer.";
pub const Q_NEWSLETTER: &str = "This message is a newsletter, digest or marketing mail.";

const LINE_SYSTEM: &str = "You write the one-line summary an email list shows for a conversation. \
Use at most 12 words (under 75 characters) to say what the conversation is about now and what, if anything, is expected next. \
Plain text on one line: no quotes, no preamble, no markdown, no greeting.";

/// The event question for one date found in a message.
pub fn q_event(candidate: &EventCandidate) -> String {
    let local = candidate.start.with_timezone(&chrono::Local);
    let when = if candidate.all_day {
        local.format("%A, %B %-d, %Y").to_string()
    } else {
        local.format("%A, %B %-d, %Y at %-I:%M %p").to_string()
    };
    format!(
        "The message proposes or confirms an event on {when}, in this sentence: {}",
        candidate.sentence
    )
}

/// What the pass may ask for now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowed {
    /// Jev: request B.
    pub signals: bool,
    /// The writing model and the List Summaries switch: the line.
    pub line: bool,
    /// The writing model and the Clean View switch: newsletters' Clean
    /// View (AI-10).
    pub clean: bool,
}

/// One result missing or stale: what to ask, before its text is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    pub thread_id: String,
    pub feature: Feature,
    pub key: String,
    pub hash: String,
}

/// Gmail's, sent or received: not a draft, not written here and unsent.
fn sendable(thread: &Thread, m: &Message) -> bool {
    let local = |id: &str| id.is_empty() || id.starts_with("local-") || id.contains("-local-");
    thread.folder != Folder::Drafts
        && !local(&thread.id)
        && !local(&m.id)
        && !m
            .label_ids
            .iter()
            .any(|l| l == crate::backend::gmail::DRAFT)
}

fn from_me(m: &Message, me: &Account) -> bool {
    me.is_me(&m.from.email) || m.label_ids.iter().any(|l| l == crate::backend::gmail::SENT)
}

/// The conversation's latest message that isn't a draft or written here.
fn latest(thread: &Thread) -> Option<(usize, &Message)> {
    thread
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| sendable(thread, m))
}

/// Its latest received message: the one request B asks about.
fn latest_received<'a>(thread: &'a Thread, me: &Account) -> Option<(usize, &'a Message)> {
    thread
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| sendable(thread, m) && !from_me(m, me))
}

pub fn signals_hash(message_id: &str) -> String {
    input_hash(SIGNALS_V, &[message_id], &[])
}

pub fn important_hash(message_id: &str) -> String {
    input_hash(IMPORTANT_V, &[message_id], &[])
}

/// The messages the line reads, oldest first.
fn line_ixs(thread: &Thread) -> Vec<usize> {
    let mut ixs: Vec<usize> = (0..thread.messages.len())
        .rev()
        .filter(|&ix| sendable(thread, &thread.messages[ix]))
        .take(LINE_MESSAGES)
        .collect();
    ixs.reverse();
    ixs
}

/// The line's hash: its prompt and the messages it reads. `None` for a
/// conversation with nothing to read.
pub fn line_hash(thread: &Thread) -> Option<String> {
    let ixs = line_ixs(thread);
    if ixs.is_empty() {
        return None;
    }
    let ids: Vec<&str> = ixs
        .iter()
        .map(|&ix| thread.messages[ix].id.as_str())
        .collect();
    Some(input_hash(LINE_V, &ids, &[]))
}

/// AI-7: the conversation's line, if one was written for it as it is now.
pub fn line(thread: &Thread) -> Option<&str> {
    let hash = line_hash(thread)?;
    thread
        .ai
        .fresh(Feature::Line, &thread.id, &hash)
        .map(|r| r.value.as_str())
        .filter(|v| !v.is_empty())
}

/// A stored noul's yes, if it was asked about this input.
pub fn yes(ai: &ThreadAi, feature: Feature, key: &str, hash: &str, id: &str) -> Option<f64> {
    let row = ai.fresh(feature, key, hash)?;
    jev::answers_from(&row.value).get(id)?.yes()
}

/// AI-3: Inbox conversations of the last `DAYS`, newest first, whose
/// results are missing or stale, without the keys `skip` names (queued,
/// running or failed). At most `MAX_JOBS`.
pub fn work_list(
    heads: &[Thread],
    results: &HashMap<String, ThreadAi>,
    me: &Account,
    now_ms: i64,
    allowed: Allowed,
    skip: &dyn Fn(Feature, &str) -> bool,
) -> Vec<Want> {
    let since = now_ms - DAYS * 24 * 60 * 60 * 1000;
    let mut threads: Vec<&Thread> = heads
        .iter()
        .filter(|t| t.folder == Folder::Inbox && !t.messages.is_empty())
        .filter(|t| t.last_message().internal_date >= since)
        .collect();
    threads.sort_by_key(|t| std::cmp::Reverse(t.last_message().internal_date));
    let none = ThreadAi::default();
    let mut out = Vec::new();
    for thread in threads {
        let ai = results.get(&thread.id).unwrap_or(&none);
        let mut want = |feature: Feature, key: &str, hash: String| {
            if ai.fresh(feature, key, &hash).is_none() && !skip(feature, key) {
                out.push(Want {
                    thread_id: thread.id.clone(),
                    feature,
                    key: key.to_string(),
                    hash,
                });
            }
        };
        if allowed.signals
            && let Some((_, m)) = latest_received(thread, me)
        {
            want(Feature::Signals, &m.id, signals_hash(&m.id));
        }
        if allowed.clean
            && let Some((_, m)) = latest_received(thread, me)
            && reading::wants_clean(ai, &m.id)
        {
            want(Feature::Clean, &m.id, reading::clean_hash(&m.id));
        }
        if allowed.line
            && let Some(hash) = line_hash(thread)
        {
            want(Feature::Line, &thread.id, hash);
        }
        if out.len() >= MAX_JOBS {
            out.truncate(MAX_JOBS);
            break;
        }
    }
    out
}

/// Request B for a conversation just opened, in any folder but Spam and
/// Trash: its received messages since the last one from me, newest first,
/// at most `OPEN_MESSAGES`, that lack a fresh answer.
pub fn open_wants(thread: &Thread, me: &Account) -> Vec<Want> {
    if thread.is_hidden() {
        return Vec::new();
    }
    thread
        .messages
        .iter()
        .rev()
        .filter(|m| sendable(thread, m))
        .take_while(|m| !from_me(m, me))
        .take(OPEN_MESSAGES)
        .map(|m| Want {
            thread_id: thread.id.clone(),
            feature: Feature::Signals,
            key: m.id.clone(),
            hash: signals_hash(&m.id),
        })
        .filter(|w| thread.ai.fresh(w.feature, &w.key, &w.hash).is_none())
        .collect()
}

/// The job for `want` from `thread` with its bodies, at Background; `None`
/// while a body it needs isn't stored (the body queue fetches it). A
/// Signals job with dates found also gives the `event` row to store.
pub fn job(thread: &Thread, want: &Want) -> Option<(Job, Option<AiRow>)> {
    let (work, event) = match want.feature {
        Feature::Signals => {
            let ix = thread.messages.iter().position(|m| m.id == want.key)?;
            let message = text::Message::from_thread(thread, ix)?;
            let candidates = events::for_message(&message);
            let automated = crate::contacts::is_automated(&thread.messages[ix].from.email);
            let request = signals_request(&message, automated, &candidates);
            let event = (!candidates.is_empty()).then(|| AiRow {
                feature: Feature::Event,
                key: want.key.clone(),
                input_hash: want.hash.clone(),
                model: String::new(),
                value: serde_json::to_string(&candidates).unwrap_or_default(),
                created: Utc::now().timestamp_millis(),
            });
            (Work::Jev(request), event)
        }
        Feature::Line => {
            let messages = line_ixs(thread)
                .into_iter()
                .map(|ix| text::Message::from_thread(thread, ix))
                .collect::<Option<Vec<_>>>()?;
            (Work::Llm(line_request(&messages)), None)
        }
        // AI-10: `None` over budget too; never trimmed.
        Feature::Clean => {
            let ix = thread.messages.iter().position(|m| m.id == want.key)?;
            let request = reading::request(&reading::message(thread, ix)?)?;
            (Work::Llm(request), None)
        }
        _ => return None,
    };
    let job = Job {
        thread_id: thread.id.clone(),
        input_hash: want.hash.clone(),
        priority: Priority::Background,
        ..Job::new(want.feature, want.key.clone(), work)
    };
    Some((job, event))
}

/// Request B for one message: `needs_reply` (not for automated senders,
/// which count as no), `newsletter`, and `event_1`… per date found.
pub fn signals_request(
    message: &text::Message,
    automated: bool,
    candidates: &[EventCandidate],
) -> jev::Request {
    let mut request = jev::Request::new(message.render_within(text::JEV_BUDGET));
    if !automated {
        request = request.ask("needs_reply", jev::Question::noul(Q_NEEDS_REPLY));
    }
    request = request.ask("newsletter", jev::Question::noul(Q_NEWSLETTER));
    for (n, candidate) in candidates.iter().take(3).enumerate() {
        request = request.ask(
            format!("event_{}", n + 1),
            jev::Question::noul(q_event(candidate)),
        );
    }
    request
}

/// Request A's state: what a notification shows, no body.
pub fn important_state(from: &str, subject: &str, snippet: &str) -> String {
    format!(
        "<mail>\nFrom: {}\nSubject: {}\n\n{}\n</mail>",
        text::defuse(from),
        text::defuse(subject),
        text::defuse(snippet)
    )
}

/// Request A: `important` and `newsletter`, from what a notification
/// shows.
pub fn important_request(from: &str, subject: &str, snippet: &str) -> jev::Request {
    jev::Request::new(important_state(from, subject, snippet))
        .ask("important", jev::Question::noul(Q_IMPORTANT))
        .ask("newsletter", jev::Question::noul(Q_NEWSLETTER))
}

/// Request A for a message that just arrived.
pub fn important_job(mail: &NewMail, priority: Priority) -> Job {
    let request = important_request(&mail.from, &mail.subject, &mail.snippet);
    Job {
        thread_id: mail.thread_id.clone(),
        input_hash: important_hash(&mail.message_id),
        priority,
        ..Job::new(
            Feature::Important,
            mail.message_id.clone(),
            Work::Jev(request),
        )
    }
}

/// AI-7's request: the latest messages, oldest first, within the budget.
pub fn line_request(messages: &[text::Message]) -> llm::Request {
    let each = (super::INPUT_BUDGET - 500) / messages.len().max(1);
    let user = messages
        .iter()
        .map(|m| m.render_within(each))
        .collect::<Vec<_>>()
        .join("\n\n");
    llm::Request {
        validate: Some(clean_line),
        ..llm::Request::new(format!("{LINE_SYSTEM} {}", text::UNTRUSTED), user, 60)
    }
}

/// The model's line as shown: its first line, unquoted, at most
/// `LINE_MAX` characters (cut at a word); `None` when empty.
pub fn clean_line(answer: &str) -> Option<String> {
    let line = answer.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line
        .strip_prefix("Summary:")
        .or_else(|| line.strip_prefix("summary:"))
        .unwrap_or(line)
        .trim()
        .trim_matches(|c| matches!(c, '"' | '“' | '”' | '`' | '*'))
        .trim();
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= LINE_MAX {
        return Some(line.to_string());
    }
    let cut: String = line.chars().take(LINE_MAX - 1).collect();
    let cut = match cut.rfind(' ') {
        Some(ix) if ix > LINE_MAX / 2 => &cut[..ix],
        _ => &cut,
    };
    Some(format!("{}…", cut.trim_end_matches([',', ';', ':', ' '])))
}

/// AI-4: in the Inbox, its latest message (not a draft) is received, Jev
/// says it asks for an answer (yes ≥ `threshold`) and isn't a newsletter
/// (yes < `NEEDS_REPLY_NEWSLETTER`), no reply waits in the outbox, and it
/// wasn't dismissed.
pub fn needs_reply(
    thread: &Thread,
    me: &Account,
    queued_sends: &HashSet<String>,
    threshold: f64,
) -> bool {
    if thread.folder != Folder::Inbox || queued_sends.contains(&thread.id) {
        return false;
    }
    let Some((_, m)) = latest(thread) else {
        return false;
    };
    let hash = signals_hash(&m.id);
    let signal = |id| yes(&thread.ai, Feature::Signals, &m.id, &hash, id);
    !from_me(m, me)
        && thread.ai.get(Feature::NeedsReplyDismissed, &m.id).is_none()
        && signal("needs_reply").is_some_and(|p| p >= threshold)
        && signal("newsletter").is_none_or(|p| p < NEEDS_REPLY_NEWSLETTER)
}

/// The message Dismiss Needs Reply marks: the latest received one.
pub fn dismiss_key(thread: &Thread, me: &Account) -> Option<String> {
    latest_received(thread, me).map(|(_, m)| m.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::store::Store,
        data::{Body, Contact},
    };

    const ME: &str = "me@example.com";
    const NOW: i64 = 1_800_000_000_000;
    const DAY: i64 = 24 * 60 * 60 * 1000;

    fn me() -> Account {
        Account {
            email: ME.into(),
            ..Default::default()
        }
    }

    fn message(id: &str, from: &str, at: i64) -> Message {
        let mut m = Message::new(Contact {
            name: String::new(),
            email: from.into(),
        });
        m.id = id.into();
        m.internal_date = at;
        m.body = Body::Loaded {
            html: format!("<p>Body of {id}</p>"),
        };
        m
    }

    fn thread(id: &str, messages: Vec<Message>) -> Thread {
        Thread::local(id.into(), format!("S {id}"), Folder::Inbox, messages, None)
    }

    fn answer_row(key: &str, feature: Feature, hash: String, json: &str) -> AiRow {
        AiRow {
            feature,
            key: key.into(),
            input_hash: hash,
            model: "m".into(),
            value: json.into(),
            created: 0,
        }
    }

    fn needs(p: f64) -> String {
        format!(r#"{{"needs_reply":{{"type":"noul","noul":{p}}}}}"#)
    }

    const ON: Allowed = Allowed {
        signals: true,
        line: true,
        clean: true,
    };

    #[test]
    fn work_list_is_newest_first_and_skips_stale_failed_and_queued() {
        let heads = vec![
            thread("old", vec![message("o1", "a@x.com", NOW - 40 * DAY)]),
            thread("t1", vec![message("m1", "a@x.com", NOW - 2 * DAY)]),
            thread("t2", vec![message("m2", "b@x.com", NOW - DAY)]),
            thread(
                "mine",
                vec![
                    message("r1", "c@x.com", NOW - 3 * DAY),
                    message("s1", ME, NOW - 3 * DAY + 1),
                ],
            ),
        ];
        let mut archived = thread("arch", vec![message("a1", "a@x.com", NOW)]);
        archived.folder = Folder::Archive;
        let mut heads = heads;
        heads.push(archived);
        let none = |_: Feature, _: &str| false;
        let wants = work_list(&heads, &HashMap::new(), &me(), NOW, ON, &none);
        let got: Vec<(&str, Feature, &str)> = wants
            .iter()
            .map(|w| (w.thread_id.as_str(), w.feature, w.key.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("t2", Feature::Signals, "m2"),
                ("t2", Feature::Line, "t2"),
                ("t1", Feature::Signals, "m1"),
                ("t1", Feature::Line, "t1"),
                // My own message isn't asked about; the one before it is.
                ("mine", Feature::Signals, "r1"),
                ("mine", Feature::Line, "mine"),
            ]
        );

        // Fresh results are done; a stale line (new message) is asked again.
        let mut results = HashMap::new();
        let mut ai = ThreadAi::default();
        ai.rows.push(answer_row(
            "m2",
            Feature::Signals,
            signals_hash("m2"),
            &needs(0.9),
        ));
        ai.rows
            .push(answer_row("t2", Feature::Line, "stale".into(), "Old line"));
        results.insert("t2".to_string(), ai);
        // Queued or failed keys are left alone.
        let skip = |f: Feature, k: &str| f == Feature::Line && k == "t1";
        let wants = work_list(&heads, &results, &me(), NOW, ON, &skip);
        let got: Vec<(&str, &str)> = wants
            .iter()
            .map(|w| (w.feature.as_str(), w.key.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("line", "t2"),
                ("signals", "m1"),
                ("signals", "r1"),
                ("line", "mine")
            ]
        );
        // Only what's allowed.
        let jev_only = Allowed {
            signals: true,
            line: false,
            clean: false,
        };
        assert!(
            work_list(&heads, &results, &me(), NOW, jev_only, &skip)
                .iter()
                .all(|w| w.feature == Feature::Signals)
        );
    }

    #[test]
    fn newsletters_get_a_clean_view_until_reported() {
        let heads = vec![thread("t1", vec![message("m1", "news@shop.com", NOW)])];
        let newsletter = r#"{"newsletter":{"type":"noul","noul":0.95}}"#;
        let mut ai = ThreadAi::default();
        ai.rows.push(answer_row(
            "m1",
            Feature::Signals,
            signals_hash("m1"),
            newsletter,
        ));
        let clean_wants = |ai: &ThreadAi| -> Vec<String> {
            let results = HashMap::from([("t1".to_string(), ai.clone())]);
            work_list(&heads, &results, &me(), NOW, ON, &|_, _| false)
                .into_iter()
                .filter(|w| w.feature == Feature::Clean)
                .map(|w| w.key)
                .collect()
        };
        assert_eq!(clean_wants(&ai), ["m1"]);
        let want = &work_list(
            &heads,
            &HashMap::from([("t1".to_string(), ai.clone())]),
            &me(),
            NOW,
            ON,
            &|_, _| false,
        )
        .into_iter()
        .find(|w| w.feature == Feature::Clean)
        .unwrap();
        let (job, _) = job(&heads[0], want).unwrap();
        assert!(matches!(job.work, Work::Llm(_)));
        assert_eq!(job.input_hash, reading::clean_hash("m1"));
        ai.rows
            .push(answer_row("m1", Feature::CleanRejected, String::new(), ""));
        assert!(clean_wants(&ai).is_empty());
    }

    #[test]
    fn work_list_stops_at_its_budget() {
        let heads: Vec<Thread> = (0..80)
            .map(|n| {
                thread(
                    &format!("t{n}"),
                    vec![message(&format!("m{n}"), "a@x.com", NOW - n)],
                )
            })
            .collect();
        let wants = work_list(&heads, &HashMap::new(), &me(), NOW, ON, &|_, _| false);
        assert_eq!(wants.len(), MAX_JOBS);
        assert_eq!(wants[0].thread_id, "t0");
    }

    #[test]
    fn jobs_wait_for_bodies_and_skip_questions_for_automated_senders() {
        let mut t = thread("t1", vec![message("m1", "no-reply@shop.com", NOW)]);
        let want = &work_list(
            std::slice::from_ref(&t),
            &HashMap::new(),
            &me(),
            NOW,
            ON,
            &|_, _| false,
        )[0];
        let (job, event) = job(&t, want).unwrap();
        assert!(event.is_none());
        let Work::Jev(request) = &job.work else {
            panic!("a Jev job")
        };
        let ids: Vec<&str> = request
            .questions
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(ids, ["newsletter"]);
        assert_eq!(job.input_hash, signals_hash("m1"));
        assert_eq!(job.thread_id, "t1");
        t.messages[0].body = Body::NotLoaded;
        assert!(super::job(&t, want).is_none());
    }

    #[test]
    fn the_line_is_cleaned_and_kept_short() {
        assert_eq!(
            clean_line("\n\"Dana asks for your rates by Thursday\"\nmore").as_deref(),
            Some("Dana asks for your rates by Thursday")
        );
        assert_eq!(
            clean_line("Summary: Lunch moved").as_deref(),
            Some("Lunch moved")
        );
        assert_eq!(clean_line("  \n "), None);
        let long = "word ".repeat(40);
        let cut = clean_line(&long).unwrap();
        assert!(cut.chars().count() <= LINE_MAX, "{cut}");
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn a_line_shows_only_for_the_messages_it_read() {
        let mut t = thread("t1", vec![message("m1", "a@x.com", NOW)]);
        t.ai.rows.push(answer_row(
            "t1",
            Feature::Line,
            line_hash(&t).unwrap(),
            "About the catalog",
        ));
        assert_eq!(line(&t), Some("About the catalog"));
        t.messages.push(message("m2", "a@x.com", NOW + 1));
        assert_eq!(line(&t), None);
    }

    #[test]
    fn needs_reply_follows_answers_sends_and_new_mail() {
        let queued = HashSet::new();
        let mut t = thread("t1", vec![message("m1", "dana@x.com", NOW)]);
        assert!(!needs_reply(&t, &me(), &queued, NEEDS_REPLY));
        t.ai.rows.push(answer_row(
            "m1",
            Feature::Signals,
            signals_hash("m1"),
            &needs(0.92),
        ));
        assert!(needs_reply(&t, &me(), &queued, NEEDS_REPLY));
        // Below the threshold, or asked about another input: no.
        assert!(!needs_reply(&t, &me(), &queued, 0.95));
        // A newsletter that asks something: no.
        let signals = |needs: f64, newsletter: f64| {
            answer_row(
                "m1",
                Feature::Signals,
                signals_hash("m1"),
                &format!(
                    r#"{{"needs_reply":{{"type":"noul","noul":{needs}}},"newsletter":{{"type":"noul","noul":{newsletter}}}}}"#
                ),
            )
        };
        let mut news = thread("t1", vec![message("m1", "dana@x.com", NOW)]);
        news.ai.rows = vec![signals(0.92, 0.5)];
        assert!(!needs_reply(&news, &me(), &queued, NEEDS_REPLY));
        news.ai.rows = vec![signals(0.92, 0.49)];
        assert!(needs_reply(&news, &me(), &queued, NEEDS_REPLY));
        news.ai.rows = vec![signals(0.79, 0.0)];
        assert!(!needs_reply(&news, &me(), &queued, NEEDS_REPLY));
        // A reply waiting to send hides it; undoing the send brings it back.
        let waiting: HashSet<String> = ["t1".to_string()].into();
        assert!(!needs_reply(&t, &me(), &waiting, NEEDS_REPLY));
        assert!(needs_reply(&t, &me(), &queued, NEEDS_REPLY));
        // A draft in it changes nothing.
        let mut draft = message("d1", ME, NOW + 1);
        draft.label_ids = vec!["DRAFT".into()];
        t.messages.push(draft);
        assert!(needs_reply(&t, &me(), &queued, NEEDS_REPLY));
        // My reply ends it.
        t.messages.push(message("s1", ME, NOW + 2));
        assert!(!needs_reply(&t, &me(), &queued, NEEDS_REPLY));
        // Newer incoming mail: only once Jev says it asks again.
        t.messages.push(message("m2", "dana@x.com", NOW + 3));
        assert!(!needs_reply(&t, &me(), &queued, NEEDS_REPLY));
        t.ai.rows.push(answer_row(
            "m2",
            Feature::Signals,
            signals_hash("m2"),
            &needs(0.85),
        ));
        assert!(needs_reply(&t, &me(), &queued, NEEDS_REPLY));
        // Out of the Inbox: no.
        t.folder = Folder::Archive;
        assert!(!needs_reply(&t, &me(), &queued, NEEDS_REPLY));
    }

    #[test]
    fn dismissal_lasts_across_a_store_reopen() {
        let path = std::env::temp_dir().join(format!(
            "pigeon-pass-test-{}-{}.sqlite",
            std::process::id(),
            NOW
        ));
        let _ = std::fs::remove_file(&path);
        let t = thread("t1", vec![message("m1", "dana@x.com", NOW)]);
        {
            let store = Store::open(&path).unwrap();
            store.put_threads(ME, &[t]).unwrap();
            let row = answer_row("m1", Feature::Signals, signals_hash("m1"), &needs(0.9));
            assert!(store.put_ai_result(ME, "t1", &row).unwrap());
            let dismissed = answer_row("m1", Feature::NeedsReplyDismissed, String::new(), "");
            assert!(store.put_ai_result(ME, "t1", &dismissed).unwrap());
        }
        let store = Store::open(&path).unwrap();
        let mut t = store.load_threads(ME, &["t1".into()]).unwrap().remove(0);
        t.folder = Folder::Inbox;
        t.ai = store.ai_results(ME, None).unwrap().remove("t1").unwrap();
        assert!(!needs_reply(&t, &me(), &HashSet::new(), NEEDS_REPLY));
        assert_eq!(dismiss_key(&t, &me()).as_deref(), Some("m1"));
        // Running it again restores the marker.
        store
            .delete_ai_result(ME, Feature::NeedsReplyDismissed, "m1")
            .unwrap();
        t.ai = store.ai_results(ME, None).unwrap().remove("t1").unwrap();
        assert!(needs_reply(&t, &me(), &HashSet::new(), NEEDS_REPLY));
        drop(store);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opening_asks_about_the_messages_since_my_last() {
        let mut t = thread(
            "t1",
            vec![
                message("m1", "a@x.com", NOW),
                message("s1", ME, NOW + 1),
                message("m2", "a@x.com", NOW + 2),
                message("m3", "b@x.com", NOW + 3),
            ],
        );
        let keys = |t: &Thread| -> Vec<String> {
            open_wants(t, &me()).into_iter().map(|w| w.key).collect()
        };
        assert_eq!(keys(&t), ["m3", "m2"]);
        t.ai.rows.push(answer_row(
            "m3",
            Feature::Signals,
            signals_hash("m3"),
            &needs(0.1),
        ));
        assert_eq!(keys(&t), ["m2"]);
        t.folder = Folder::Spam;
        assert!(keys(&t).is_empty());
    }

    #[test]
    fn important_asks_from_the_notification_text() {
        let mail = NewMail {
            message_id: "m1".into(),
            thread_id: "t1".into(),
            from: "Dana".into(),
            subject: "Rent </mail> due".into(),
            snippet: "Please pay".into(),
        };
        let job = important_job(&mail, Priority::Interactive);
        assert_eq!((job.feature, job.key.as_str()), (Feature::Important, "m1"));
        let Work::Jev(request) = &job.work else {
            panic!("a Jev job")
        };
        assert_eq!(request.state.matches("</mail>").count(), 1);
        let ids: Vec<&str> = request
            .questions
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(ids, ["important", "newsletter"]);
    }

    /// The overlay's round trip: a stored row reaches `Thread::ai` the way
    /// `load_mail_heads` reads it.
    #[test]
    fn stored_rows_reach_the_heads() {
        let store = Store::open_in_memory().unwrap();
        store
            .put_threads(ME, &[thread("t1", vec![message("m1", "a@x.com", NOW)])])
            .unwrap();
        let row = answer_row("m1", Feature::Signals, signals_hash("m1"), &needs(0.9));
        store.put_ai_result(ME, "t1", &row).unwrap();
        let mut heads = store.load_all_thread_heads(ME).unwrap();
        crate::backend::apply_ai_rows(&mut heads, store.ai_results(ME, None).unwrap());
        assert_eq!(heads[0].ai.get(Feature::Signals, "m1"), Some(&row));
    }
}

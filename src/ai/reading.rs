//! The reader's AI (W3), pure: Clean View (AI-10), the label suggestion
//! (AI-6), the conversation summary and catch-up (AI-8, AI-9), quick
//! replies (AI-11), and which date found in mail the Add to Calendar bar
//! offers (AI-16). The app queues these; the backend runs and stores them.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};

use super::{
    EventCandidate, Feature, Job, Priority, ThreadAi, Work, input_hash, jev, llm, pass, text,
};
use crate::{
    data::{Account, Body, Folder, Label, Thread},
    html::{self, Block, Document, Inline},
};

/// AI-10: `newsletter` yes at least this gets a Clean View. Phase 4 tunes it.
pub const NEWSLETTER: f64 = 0.8;
/// AI-6: the suggestion's confidence at least this shows it.
pub const LABEL_MIN: f64 = 0.6;
/// AI-16: `event_n` yes at least this offers the date.
pub const EVENT_MIN: f64 = 0.7;
/// AI-6: labels counted over conversations this recent.
const LABEL_DAYS: i64 = 90;
const SHORTLIST: usize = 12;
const LABEL_SUBJECTS: usize = 3;
const SUBJECT_MAX: usize = 60;
/// AI-8: a conversation this long gets its summary on its own.
pub const SUMMARY_MESSAGES: usize = 4;
pub const SUMMARY_WORDS: usize = 1_500;
/// The summary reads at most this many of the latest messages.
const SUMMARY_MAX_MESSAGES: usize = 20;
/// Quick replies read the latest messages.
const REPLY_MESSAGES: usize = 3;
const REPLY_MAX: usize = 200;

pub const CLEAN_V: &str = "clean-v1";
pub const LABEL_V: &str = "label-v1";
pub const SUMMARY_V: &str = "summary-v1";
pub const CATCH_UP_V: &str = "catch-up-v1";
pub const REPLIES_V: &str = "replies-v1";

pub const Q_LABEL: &str =
    "Which of these labels fits this message best? \"none\" when none clearly fits.";
/// The "Asked of you" heading of a catch-up.
pub const ASKED: &str = "Asked of you:";

const CLEAN_SYSTEM: &str = "You turn a newsletter or marketing email into a clean reading view, in Markdown. \
Keep all of its actual content in its order: articles, items, prices, dates and calls to action. \
Drop tracking text, layout leftovers, repeated navigation, social buttons and long footers. \
Use headings, short paragraphs and lists. \
Use only links and images that appear in the email, with their exact URLs, written as [text](url) and ![alt](url); never add others. \
No commentary, no summary, no preamble: answer with the Markdown only.";

const SUMMARY_SYSTEM: &str = "You summarize an email conversation for the person reading it. \
Write 2 to 5 short bullet points, each starting with \"- \": what it is about, what was decided, and what is still open, oldest to newest. \
Plain bullets only: no headings, no preamble, no closing line.";

const CATCH_UP_SYSTEM: &str = "The reader was away while these new messages arrived in an email conversation. \
Write 2 to 4 short bullet points, each starting with \"- \", of what is new. \
Then, only if the new messages ask the reader for something, a line \"Asked of you:\" followed by one bullet point per request. \
No headings, no preamble, no closing line.";

const REPLIES_SYSTEM: &str = "You suggest three short replies the reader could send to the latest message of this email conversation. \
Make them different in intent (for example agreeing, declining, and asking for details or time), each at most two sentences and under 120 characters, \
written as the reader, in the conversation's language, with no greeting and no signature. \
Answer with only a JSON array of exactly three strings.";

// AI-10, Clean View.

/// A message's text for its Clean View: the body without its quoted
/// history, with links and images written as Markdown so they can be kept.
pub fn source(html_body: &str) -> String {
    let mut doc = html::parse(html_body);
    if let Some(ix) = html::trailing_quote_start(&doc) {
        doc.blocks.truncate(ix);
    }
    let mut out = String::new();
    write_blocks(&doc.blocks, "", &mut out);
    out.trim().to_string()
}

fn write_blocks(blocks: &[Block], prefix: &str, out: &mut String) {
    let par = |text: &str, out: &mut String| {
        let text = text.trim();
        if !text.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(prefix);
            out.push_str(text);
        }
    };
    for block in blocks {
        match block {
            Block::Text { text, kind, .. } => {
                let line = inline(text);
                match kind {
                    html::TextKind::Heading(n) if !line.trim().is_empty() => par(
                        &format!("{} {}", "#".repeat((*n).clamp(1, 6) as usize), line.trim()),
                        out,
                    ),
                    _ => par(&line, out),
                }
            }
            Block::Pre(text) => par(&text.text, out),
            Block::List { items, .. } => {
                for item in items {
                    let mut inner = String::new();
                    write_blocks(item, "", &mut inner);
                    par(&format!("- {}", inner.replace("\n\n", " ")), out);
                }
            }
            Block::Quote(blocks) => write_blocks(blocks, "> ", out),
            Block::Table(table) => {
                for row in &table.rows {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|cell| {
                            let mut inner = String::new();
                            write_blocks(&cell.blocks, "", &mut inner);
                            inner.replace("\n\n", " ")
                        })
                        .filter(|c| !c.trim().is_empty())
                        .collect();
                    par(&cells.join(" | "), out);
                }
            }
            Block::Image { alt, src, link, .. } => {
                let alt = alt.as_deref().unwrap_or_default();
                let image = if src.is_empty() {
                    alt.to_string()
                } else {
                    format!("![{alt}]({src})")
                };
                match link {
                    Some(url) => par(&format!("[{image}]({url})"), out),
                    None => par(&image, out),
                }
            }
            Block::Button { label, url, .. } => par(&format!("[{label}]({url})"), out),
            Block::Rule => par("---", out),
        }
    }
}

/// An inline run with its links as `[text](url)`.
fn inline(text: &Inline) -> String {
    let s = text.text.as_ref();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    for (range, url) in &text.links {
        let (Some(before), Some(inside)) = (s.get(at..range.start), s.get(range.clone())) else {
            continue;
        };
        out.push_str(before);
        out.push_str(&format!("[{inside}]({url})"));
        at = range.end;
    }
    out.push_str(s.get(at..).unwrap_or_default());
    out
}

/// The message at `ix` as Clean View sends it: `text::Message` with the
/// body from `source`.
pub fn message(thread: &Thread, ix: usize) -> Option<text::Message> {
    let mut msg = text::Message::from_thread(thread, ix)?;
    msg.body = source(thread.messages[ix].html());
    Some(msg)
}

/// The request for a message's Clean View; `None` when it's over budget
/// (never trimmed).
pub fn request(msg: &text::Message) -> Option<llm::Request> {
    let system = format!("{CLEAN_SYSTEM} {}", text::UNTRUSTED);
    let user = msg.render();
    if text::tokens(&system) + text::tokens(&user) > super::INPUT_BUDGET {
        return None;
    }
    Some(llm::Request {
        validate: Some(clean_answer),
        ..llm::Request::new(system, user, 4_000)
    })
}

/// The model's Markdown without a code fence around it; `None` when empty.
fn clean_answer(answer: &str) -> Option<String> {
    let mut text = answer.trim();
    if let Some(rest) = text.strip_prefix("```") {
        text = rest.split_once('\n').map_or("", |(_, body)| body);
        text = text.trim_end().strip_suffix("```").unwrap_or(text);
    }
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub fn clean_hash(message_id: &str) -> String {
    input_hash(CLEAN_V, &[message_id], &[])
}

/// A message Clean View is for: Jev says it's a newsletter, and it wasn't
/// reported.
pub fn wants_clean(ai: &ThreadAi, message_id: &str) -> bool {
    ai.get(Feature::CleanRejected, message_id).is_none()
        && pass::yes(
            ai,
            Feature::Signals,
            message_id,
            &pass::signals_hash(message_id),
            "newsletter",
        )
        .is_some_and(|p| p >= NEWSLETTER)
}

/// The links and image addresses an original shows.
fn original_urls(original_html: &str) -> HashSet<String> {
    fn images(blocks: &[Block], out: &mut HashSet<String>) {
        for block in blocks {
            match block {
                Block::Image { src, .. } if !src.is_empty() => {
                    out.insert(src.trim().to_string());
                }
                Block::List { items, .. } => items.iter().for_each(|i| images(i, out)),
                Block::Quote(blocks) => images(blocks, out),
                Block::Table(t) => t.rows.iter().flatten().for_each(|c| images(&c.blocks, out)),
                _ => {}
            }
        }
    }
    let doc: Document = html::parse(original_html);
    let mut out: HashSet<String> = html::links(&doc.blocks)
        .iter()
        .map(|u| u.trim().to_string())
        .collect();
    images(&doc.blocks, &mut out);
    out
}

/// Text that Markdown reads as itself: every ASCII punctuation escaped, so
/// a bare address can't turn back into a link.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A node's text, images as their alt text.
fn plain(node: &markdown::mdast::Node) -> String {
    use markdown::mdast::Node;
    match node {
        Node::Text(n) => n.value.clone(),
        Node::InlineCode(n) => n.value.clone(),
        Node::Image(n) => n.alt.clone(),
        other => other
            .children()
            .map(|c| c.iter().map(plain).collect())
            .unwrap_or_default(),
    }
}

fn parse_md(markdown: &str) -> Option<markdown::mdast::Node> {
    markdown::to_mdast(markdown, &markdown::ParseOptions::gfm()).ok()
}

/// The answer with links and images not in the original made plain text.
pub fn clean(markdown: &str, original_html: &str) -> String {
    use markdown::mdast::Node;
    fn walk(node: &Node, allowed: &HashSet<String>, edits: &mut Vec<(usize, usize, String)>) {
        let url = match node {
            Node::Link(n) => Some(&n.url),
            Node::Image(n) => Some(&n.url),
            _ => None,
        };
        if let Some(url) = url
            && !allowed.contains(url.trim())
        {
            if let Some(p) = node.position() {
                edits.push((p.start.offset, p.end.offset, literal(&plain(node))));
            }
            return;
        }
        for child in node.children().into_iter().flatten() {
            walk(child, allowed, edits);
        }
    }
    let Some(root) = parse_md(markdown) else {
        return literal(markdown);
    };
    let allowed = original_urls(original_html);
    let mut edits = Vec::new();
    walk(&root, &allowed, &mut edits);
    let mut out = markdown.to_string();
    for (start, end, text) in edits.into_iter().rev() {
        if out.is_char_boundary(start) && out.is_char_boundary(end) && start <= end {
            out.replace_range(start..end, &text);
        }
    }
    out
}

/// A cleaned answer as HTML for the reader, its images with their
/// addresses (the composer's Markdown leaves them out). Only its kept
/// `[text](url)` links are links: bare addresses, such as the ones
/// `clean` dropped, stay text (`html::NO_AUTOLINK`).
pub fn to_html(markdown: &str) -> String {
    use markdown::mdast::Node;
    fn images(node: &Node, out: &mut Vec<String>) {
        if let Node::Image(n) = node {
            out.push(n.url.clone());
        }
        for child in node.children().into_iter().flatten() {
            images(child, out);
        }
    }
    let html = crate::markdown::to_html(markdown);
    let mut urls = Vec::new();
    if let Some(root) = parse_md(markdown) {
        images(&root, &mut urls);
    }
    let mut urls = urls.into_iter();
    let mut out = String::with_capacity(html.len());
    let mut rest = html.as_str();
    while let Some(at) = rest.find("<img alt=\"") {
        out.push_str(&rest[..at]);
        match urls.next() {
            Some(url) => out.push_str(&format!("<img src=\"{}\" alt=\"", html::escape(&url))),
            None => out.push_str("<img alt=\""),
        }
        rest = &rest[at + "<img alt=\"".len()..];
    }
    out.push_str(rest);
    let tag = html::NO_AUTOLINK;
    format!("<{tag}>{out}</{tag}>")
}

// AI-6, the label suggestion.

/// Up to `SHORTLIST` labels, by how many conversations of the last
/// `LABEL_DAYS` carry them, most first; none unless two are in use.
pub fn shortlist(labels: &[Label], threads: &[Thread], now_ms: i64) -> Vec<usize> {
    let since = now_ms - LABEL_DAYS * 24 * 60 * 60 * 1000;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for t in threads.iter().filter(|t| {
        !t.messages.is_empty() && !t.is_hidden() && t.last_message().internal_date >= since
    }) {
        for id in &t.labels {
            *counts.entry(id.as_str()).or_default() += 1;
        }
    }
    let mut ixs: Vec<usize> = (0..labels.len())
        .filter(|&ix| counts.contains_key(labels[ix].id.as_str()))
        .collect();
    if ixs.len() < 2 {
        return Vec::new();
    }
    ixs.sort_by_key(|&ix| std::cmp::Reverse(counts[labels[ix].id.as_str()]));
    ixs.truncate(SHORTLIST);
    ixs
}

fn short(subject: &str) -> String {
    let subject = subject.trim();
    if subject.chars().count() <= SUBJECT_MAX {
        return subject.to_string();
    }
    let cut: String = subject.chars().take(SUBJECT_MAX - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The choice's options: "none" first (Jev leans to the first), then each
/// shortlisted label's id and name with up to three recent subjects of
/// other conversations.
pub fn label_options(
    labels: &[Label],
    shortlist: &[usize],
    threads: &[Thread],
    this: &str,
) -> Vec<(String, String)> {
    let mut recent: Vec<&Thread> = threads
        .iter()
        .filter(|t| t.id != this && !t.messages.is_empty() && !t.is_hidden())
        .collect();
    recent.sort_by_key(|t| std::cmp::Reverse(t.last_message().internal_date));
    let mut options = vec![("none".to_string(), "None of these labels fits.".to_string())];
    for &ix in shortlist {
        let label = &labels[ix];
        let subjects: Vec<String> = recent
            .iter()
            .filter(|t| t.labels.contains(&label.id) && t.has_subject())
            .take(LABEL_SUBJECTS)
            .map(|t| format!("\"{}\"", short(&t.subject)))
            .collect();
        let name = label.name.replace('/', " › ");
        let description = if subjects.is_empty() {
            format!("{name}.")
        } else {
            format!("{name}. Recent: {}", subjects.join(", "))
        };
        options.push((label.id.clone(), description));
    }
    options
}

pub fn label_hash(message_id: &str, label_ids: &[&str]) -> String {
    input_hash(LABEL_V, &[message_id], label_ids)
}

/// The message a suggestion reads: the latest one sent to me, else the
/// latest one that may be sent.
pub fn label_message(thread: &Thread, me: &Account) -> Option<usize> {
    let all: Vec<usize> = (0..thread.messages.len())
        .rev()
        .filter(|&ix| sendable(thread, ix))
        .collect();
    all.iter()
        .copied()
        .find(|&ix| !me.is_me(&thread.messages[ix].from.email))
        .or(all.first().copied())
}

/// Gmail's, sent or received: not a draft, not written here and unsent.
/// Its body may not be stored yet.
fn sendable(thread: &Thread, ix: usize) -> bool {
    let local = |id: &str| id.is_empty() || id.starts_with("local-") || id.contains("-local-");
    let m = &thread.messages[ix];
    thread.folder != Folder::Drafts
        && !local(&thread.id)
        && !local(&m.id)
        && !m
            .label_ids
            .iter()
            .any(|l| l == crate::backend::gmail::DRAFT)
}

pub fn label_request(msg: &text::Message, options: Vec<(String, String)>) -> jev::Request {
    jev::Request::new(msg.render_within(text::JEV_BUDGET)).ask(
        "label",
        jev::Question::Choice {
            instructions: Q_LABEL.into(),
            options,
        },
    )
}

/// The label id Jev picked, if it's sure enough and not "none".
pub fn suggested(value: &str) -> Option<String> {
    let answers = jev::answers_from(value);
    let (choice, p) = answers.get("label")?.choice()?;
    (choice != "none" && p >= LABEL_MIN).then(|| choice.to_string())
}

// AI-8, AI-9: the summary and the catch-up.

/// AI-9: the messages unread at open, when an older one was read.
pub fn catch_up_snapshot(messages: &[(&str, bool)]) -> Option<Vec<String>> {
    let first = messages.iter().position(|(_, unread)| *unread)?;
    if first == 0 {
        return None;
    }
    Some(
        messages[first..]
            .iter()
            .filter(|(_, unread)| *unread)
            .map(|(id, _)| id.to_string())
            .collect(),
    )
}

/// The messages a summary reads, oldest first: the latest that may be sent.
fn summary_ixs(thread: &Thread) -> Vec<usize> {
    let mut ixs: Vec<usize> = (0..thread.messages.len())
        .rev()
        .filter(|&ix| sendable(thread, ix))
        .take(SUMMARY_MAX_MESSAGES)
        .collect();
    ixs.reverse();
    ixs
}

pub fn summary_hash(thread: &Thread) -> Option<String> {
    let ixs = summary_ixs(thread);
    let ids: Vec<&str> = ixs
        .iter()
        .map(|&ix| thread.messages[ix].id.as_str())
        .collect();
    (!ids.is_empty()).then(|| input_hash(SUMMARY_V, &ids, &[]))
}

pub fn catch_up_hash(snapshot: &[String]) -> String {
    let ids: Vec<&str> = snapshot.iter().map(String::as_str).collect();
    input_hash(CATCH_UP_V, &ids, &ids)
}

/// AI-8 shows on its own: 4+ messages, or 1,500+ words. `None` while the
/// words can't be counted yet (a body isn't stored).
pub fn summary_due(thread: &Thread) -> Option<bool> {
    if thread.messages.len() >= SUMMARY_MESSAGES {
        return Some(true);
    }
    let mut words = 0;
    for m in &thread.messages {
        match &m.body {
            Body::Loaded { html } => words += text::body_text(html).split_whitespace().count(),
            _ => return None,
        }
    }
    Some(words >= SUMMARY_WORDS)
}

fn messages_request(system: &str, messages: &[text::Message], max_output: u32) -> llm::Request {
    let each = (super::INPUT_BUDGET - 600) / messages.len().max(1);
    let user = messages
        .iter()
        .map(|m| m.render_within(each))
        .collect::<Vec<_>>()
        .join("\n\n");
    llm::Request::new(format!("{system} {}", text::UNTRUSTED), user, max_output)
}

/// A reader job: Interactive, in `JobGroup::Open`.
pub fn job(feature: Feature, key: &str, thread: &Thread, hash: String, work: Work) -> Job {
    Job {
        thread_id: thread.id.clone(),
        input_hash: hash,
        priority: Priority::Interactive,
        group: Some(super::JobGroup::Open),
        ..Job::new(feature, key, work)
    }
}

/// AI-8's job; `None` while a body it reads isn't stored.
pub fn summary_job(thread: &Thread) -> Option<Job> {
    let messages = summary_ixs(thread)
        .into_iter()
        .map(|ix| text::Message::from_thread(thread, ix))
        .collect::<Option<Vec<_>>>()?;
    let request = messages_request(SUMMARY_SYSTEM, &messages, 400);
    Some(job(
        Feature::Summary,
        &thread.id,
        thread,
        summary_hash(thread)?,
        Work::Llm(request),
    ))
}

/// AI-9's job: only the snapshot's messages.
pub fn catch_up_job(thread: &Thread, snapshot: &[String]) -> Option<Job> {
    let messages = snapshot
        .iter()
        .map(|id| {
            let ix = thread.messages.iter().position(|m| &m.id == id)?;
            text::Message::from_thread(thread, ix)
        })
        .collect::<Option<Vec<_>>>()?;
    if messages.is_empty() {
        return None;
    }
    let request = messages_request(CATCH_UP_SYSTEM, &messages, 400);
    Some(job(
        Feature::CatchUp,
        &thread.id,
        thread,
        catch_up_hash(snapshot),
        Work::Llm(request),
    ))
}

// AI-11, quick replies.

fn reply_ixs(thread: &Thread) -> Vec<usize> {
    let mut ixs: Vec<usize> = (0..thread.messages.len())
        .rev()
        .filter(|&ix| sendable(thread, ix))
        .take(REPLY_MESSAGES)
        .collect();
    ixs.reverse();
    ixs
}

/// Keyed by the latest message.
pub fn replies_key(thread: &Thread) -> Option<String> {
    reply_ixs(thread)
        .last()
        .map(|&ix| thread.messages[ix].id.clone())
}

pub fn replies_hash(thread: &Thread) -> Option<String> {
    let ixs = reply_ixs(thread);
    let ids: Vec<&str> = ixs
        .iter()
        .map(|&ix| thread.messages[ix].id.as_str())
        .collect();
    (!ids.is_empty()).then(|| input_hash(REPLIES_V, &ids, &[]))
}

pub fn replies_job(thread: &Thread) -> Option<Job> {
    let messages = reply_ixs(thread)
        .into_iter()
        .map(|ix| text::Message::from_thread(thread, ix))
        .collect::<Option<Vec<_>>>()?;
    let request = llm::Request {
        validate: Some(valid_replies),
        ..messages_request(REPLIES_SYSTEM, &messages, 200)
    };
    Some(job(
        Feature::Replies,
        &replies_key(thread)?,
        thread,
        replies_hash(thread)?,
        Work::Llm(request),
    ))
}

/// Three short, distinct, non-empty strings, as a JSON array.
pub fn valid_replies(answer: &str) -> Option<String> {
    let start = answer.find('[')?;
    let end = answer.rfind(']')?;
    let list: Vec<serde_json::Value> = serde_json::from_str(answer.get(start..=end)?).ok()?;
    let replies: Vec<String> = list
        .iter()
        .map(|v| v.as_str().map(|s| s.trim().to_string()))
        .collect::<Option<_>>()?;
    let distinct: HashSet<&str> = replies.iter().map(String::as_str).collect();
    let ok = replies.len() == 3
        && distinct.len() == 3
        && replies
            .iter()
            .all(|r| !r.is_empty() && r.chars().count() <= REPLY_MAX);
    ok.then(|| serde_json::to_string(&replies).unwrap_or_default())
}

pub fn replies(value: &str) -> Vec<String> {
    serde_json::from_str(value).unwrap_or_default()
}

// AI-16, the Add to Calendar bar.

/// A conversation's dates found in mail: the message each is in, and its
/// `event_n` yes.
pub fn event_candidates(thread: &Thread) -> Vec<(usize, EventCandidate, Option<f64>)> {
    let mut out = Vec::new();
    for (ix, m) in thread.messages.iter().enumerate() {
        let hash = pass::signals_hash(&m.id);
        let Some(row) = thread.ai.fresh(Feature::Event, &m.id, &hash) else {
            continue;
        };
        let found: Vec<EventCandidate> = serde_json::from_str(&row.value).unwrap_or_default();
        let answers: BTreeMap<String, jev::Answer> = thread
            .ai
            .fresh(Feature::Signals, &m.id, &hash)
            .map(|r| jev::answers_from(&r.value))
            .unwrap_or_default();
        for (n, candidate) in found.into_iter().take(3).enumerate() {
            let yes = answers
                .get(&format!("event_{}", n + 1))
                .and_then(|a| a.yes());
            out.push((ix, candidate, yes));
        }
    }
    out
}

/// The date to offer: the highest `event_n` yes at least `EVENT_MIN`,
/// still to come, not starting when an event already does. An index into
/// `candidates`.
pub fn pick_candidate(
    candidates: &[EventCandidate],
    answers: &[Option<f64>],
    now: DateTime<Utc>,
    existing_starts: &[DateTime<Utc>],
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter_map(|(ix, c)| Some((ix, c, answers.get(ix).copied().flatten()?)))
        .filter(|(_, _, yes)| *yes >= EVENT_MIN)
        .filter(|(_, c, _)| {
            let until = if c.all_day {
                c.start + Duration::days(1)
            } else {
                c.start
            };
            until > now
        })
        .filter(|(_, c, _)| !existing_starts.contains(&c.start))
        .max_by(|a, b| a.2.total_cmp(&b.2).then(b.0.cmp(&a.0)))
        .map(|(ix, _, _)| ix)
}

/// Spam and Trash get none of the reader's AI.
pub fn reader_allowed(thread: &Thread) -> bool {
    !thread.is_hidden() && thread.folder != Folder::Drafts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::AiRow,
        data::{Contact, Message},
    };

    fn message(id: &str, html: &str) -> Message {
        let mut m = Message::new(Contact {
            name: "News".into(),
            email: "news@shop.example".into(),
        });
        m.id = id.into();
        m.body = Body::Loaded { html: html.into() };
        m
    }

    fn thread(messages: Vec<Message>) -> Thread {
        Thread::local("t1".into(), "Weekly".into(), Folder::Inbox, messages, None)
    }

    const ORIGINAL: &str = r#"<p>Read about <a href="https://blog.example/async">async closures</a>.</p>
        <p><img src="https://cdn.example/hero.png" alt="Hero"></p>
        <p><a href="https://shop.example/sale" style="background:#0a7;padding:8px">Shop now</a></p>"#;

    #[test]
    fn clean_keeps_real_links_and_drops_invented_ones() {
        let md = "# This week\n\nRead about [async closures](https://blog.example/async).\n\n\
            ![Hero](https://cdn.example/hero.png)\n\n\
            [Log in](https://evil.example/login) to your account.\n\n\
            ![Pixel](https://evil.example/p.gif)\n\n\
            See https://evil.example/bare and <https://evil.example/auto> or www.evil.example.\n\n\
            [Shop now](https://shop.example/sale)";
        let cleaned = clean(md, ORIGINAL);
        let html = to_html(&cleaned);
        assert!(!html.contains("href=\"https://evil"), "{html}");
        assert!(!html.contains("src=\"https://evil"), "{html}");
        assert!(!html.contains("href=\"http://www.evil"), "{html}");
        assert!(
            html.contains("<a href=\"https://blog.example/async\">"),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"https://shop.example/sale\">"),
            "{html}"
        );
        assert!(
            html.contains("<img src=\"https://cdn.example/hero.png\" alt=\"Hero\">"),
            "{html}"
        );
        // Their text stays.
        let doc = html::parse(&html);
        let text = html::plain_text(&doc);
        assert!(text.contains("Log in to your account."), "{text}");
        assert!(text.contains("https://evil.example/bare"), "{text}");
        assert!(text.contains("Pixel"), "{text}");
        assert_eq!(
            html::links(&doc.blocks),
            ["https://blog.example/async", "https://shop.example/sale"]
        );
        assert_eq!(doc.remote_images, ["https://cdn.example/hero.png"]);
    }

    #[test]
    fn the_source_keeps_links_and_images_for_the_model() {
        let source = source(ORIGINAL);
        assert!(source.contains("[async closures](https://blog.example/async)"));
        assert!(source.contains("![Hero](https://cdn.example/hero.png)"));
        assert!(source.contains("[Shop now](https://shop.example/sale)"));
    }

    #[test]
    fn an_over_budget_newsletter_gets_no_request() {
        let t = thread(vec![message("m1", ORIGINAL)]);
        let msg = super::message(&t, 0).unwrap();
        assert!(request(&msg).is_some());
        let long = format!("<p>{}</p>", "word ".repeat(60_000));
        let t = thread(vec![message("m1", &long)]);
        let msg = super::message(&t, 0).unwrap();
        assert!(request(&msg).is_none());
    }

    #[test]
    fn clean_answers_lose_their_fence() {
        assert_eq!(
            clean_answer("```markdown\n# Hi\n\nText\n```").as_deref(),
            Some("# Hi\n\nText")
        );
        assert_eq!(clean_answer("  "), None);
    }

    fn label(id: &str, name: &str) -> Label {
        Label::new(id, name, gpui::Hsla::default())
    }

    #[test]
    fn the_shortlist_counts_recent_conversations_and_puts_none_first() {
        const NOW: i64 = 1_800_000_000_000;
        const DAY: i64 = 24 * 60 * 60 * 1000;
        let labels = vec![
            label("L1", "Work"),
            label("L2", "Receipts"),
            label("L3", "Old"),
        ];
        let conv = |id: &str, subject: &str, at: i64, labels: &[&str]| {
            let mut m = message(&format!("{id}-m"), "<p>x</p>");
            m.internal_date = at;
            let mut t = Thread::local(id.into(), subject.into(), Folder::Inbox, vec![m], None);
            t.labels = labels.iter().map(|l| l.to_string()).collect();
            t
        };
        let mut threads = vec![
            conv("a", "Q4 planning", NOW - DAY, &["L1"]),
            conv("b", "Standup", NOW - 2 * DAY, &["L1"]),
            conv("c", "Your order", NOW - 3 * DAY, &["L2"]),
            conv("d", "Ancient", NOW - 200 * DAY, &["L3"]),
        ];
        let list = shortlist(&labels, &threads, NOW);
        assert_eq!(list, [0, 1]);
        let options = label_options(&labels, &list, &threads, "a");
        assert_eq!(options[0].0, "none");
        assert_eq!(
            options[1],
            ("L1".into(), "Work. Recent: \"Standup\"".into())
        );
        assert_eq!(
            options[2],
            ("L2".into(), "Receipts. Recent: \"Your order\"".into())
        );
        // One label in use: nothing to choose from.
        threads.retain(|t| t.id != "c");
        assert!(shortlist(&labels, &threads, NOW).is_empty());
    }

    #[test]
    fn a_suggestion_needs_confidence_and_a_label() {
        let choice = |c: &str, p: f64| {
            format!(r#"{{"label":{{"type":"choice","choice":"{c}","confidence":{p}}}}}"#)
        };
        assert_eq!(suggested(&choice("L1", 0.8)).as_deref(), Some("L1"));
        assert_eq!(suggested(&choice("L1", 0.5)), None);
        assert_eq!(suggested(&choice("none", 0.9)), None);
        assert_eq!(suggested("{}"), None);
    }

    #[test]
    fn catch_up_needs_an_older_read_message() {
        assert_eq!(
            catch_up_snapshot(&[("m1", false), ("m2", true), ("m3", true)]),
            Some(vec!["m2".to_string(), "m3".to_string()])
        );
        // All unread: the first open.
        assert_eq!(catch_up_snapshot(&[("m1", true), ("m2", true)]), None);
        assert_eq!(catch_up_snapshot(&[("m1", false)]), None);
        assert_ne!(catch_up_hash(&["m2".into()]), catch_up_hash(&["m3".into()]));
    }

    #[test]
    fn replies_must_be_three_short_strings() {
        assert_eq!(
            valid_replies(
                "Sure:\n[\"Yes, works for me.\", \"Sorry, I can't.\", \"Can we do Friday?\"]"
            )
            .as_deref(),
            Some(r#"["Yes, works for me.","Sorry, I can't.","Can we do Friday?"]"#)
        );
        assert_eq!(valid_replies(r#"["a", "b"]"#), None);
        assert_eq!(valid_replies(r#"["a", "b", "c", "d"]"#), None);
        assert_eq!(valid_replies(r#"["a", "a", "b"]"#), None);
        assert_eq!(valid_replies(r#"["a", "", "b"]"#), None);
        assert_eq!(valid_replies(r#"["a", 2, "b"]"#), None);
        assert_eq!(
            valid_replies(&format!(r#"["a", "b", "{}"]"#, "x".repeat(300))),
            None
        );
        assert_eq!(valid_replies("no list"), None);
        assert_eq!(replies(r#"["a","b","c"]"#), ["a", "b", "c"]);
    }

    fn candidate(start: DateTime<Utc>, all_day: bool) -> EventCandidate {
        EventCandidate {
            title: "Lunch".into(),
            start,
            end: None,
            all_day,
            zone: None,
            location: None,
            sentence: "Lunch then?".into(),
        }
    }

    #[test]
    fn the_bar_offers_the_likeliest_future_new_date() {
        let now: DateTime<Utc> = "2026-10-28T12:00:00Z".parse().unwrap();
        let past = candidate(now - Duration::hours(1), false);
        let soon = candidate(now + Duration::days(1), false);
        let later = candidate(now + Duration::days(2), false);
        let today = candidate("2026-10-28T00:00:00Z".parse().unwrap(), true);
        let all = vec![past.clone(), soon.clone(), later.clone(), today];
        // The past one is out, however sure.
        assert_eq!(
            pick_candidate(&all, &[Some(0.99), Some(0.8), Some(0.9), None], now, &[]),
            Some(2)
        );
        // An event already starts then.
        assert_eq!(
            pick_candidate(
                &all,
                &[None, Some(0.8), Some(0.9), None],
                now,
                &[later.start]
            ),
            Some(1)
        );
        // Below the threshold, or no answer.
        assert_eq!(
            pick_candidate(&all, &[None, Some(0.6), None, None], now, &[]),
            None
        );
        // An all-day date today is still to come.
        assert_eq!(
            pick_candidate(&all, &[None, None, None, Some(0.75)], now, &[]),
            Some(3)
        );
    }

    #[test]
    fn candidates_come_with_their_answers() {
        let mut t = thread(vec![message("m1", "<p>Lunch Friday at noon?</p>")]);
        let now: DateTime<Utc> = "2026-10-28T12:00:00Z".parse().unwrap();
        let hash = pass::signals_hash("m1");
        let row = |feature, value: String| AiRow {
            feature,
            key: "m1".into(),
            input_hash: hash.clone(),
            model: String::new(),
            value,
            created: 0,
        };
        t.ai.rows.push(row(
            Feature::Event,
            serde_json::to_string(&[candidate(now, false)]).unwrap(),
        ));
        t.ai.rows.push(row(
            Feature::Signals,
            r#"{"event_1":{"type":"noul","noul":0.9}}"#.into(),
        ));
        let found = event_candidates(&t);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].0, found[0].2), (0, Some(0.9)));
    }
}

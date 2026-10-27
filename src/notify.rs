//! System notifications (NTF-1, sprint 2 W1), through gpui's own API over
//! `UNUserNotificationCenter`: `notify` posts one, `dismiss` takes it back,
//! and a click goes where the notice says (`shell::show`). gpui posts at
//! once and reports no delivery result, so every notice submitted is
//! logged (`notify: tag=… title=…`) for testers to check. In test-instance
//! mode (`PIGEON_DEV=1`) tags get a `dev-` prefix.
//!
//! New mail: one notice per conversation, tagged `mail-<thread id>`, so a
//! newer message in it replaces the older; more than three at once become
//! one summary under `mail-summary`, which the next one replaces. Both go
//! once their conversations are read.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use gpui::{App, Global, SystemNotification, SystemNotificationAction, SystemNotificationResponse};

use crate::{app::shell, backend::new_mail::NewMail, instance, logging};

/// Where a click on the notice's body goes.
#[allow(dead_code)] // `Event`: W5's reminders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeTarget {
    Thread {
        id: String,
    },
    Event {
        calendar: String,
        id: String,
        start: NaiveDateTime,
    },
    Inbox,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Prefixed in test-instance mode. A notice with the tag of one still
    /// showing replaces it.
    pub tag: String,
    pub title: String,
    pub body: String,
    pub target: NoticeTarget,
    /// Adds a "Join" action that opens the URL.
    pub join_url: Option<String>,
}

/// The tag of the notice standing for more than `MAX_SINGLE` new messages.
pub const SUMMARY_TAG: &str = "mail-summary";
/// New messages notified one by one; more become one summary.
pub const MAX_SINGLE: usize = 3;
const JOIN: &str = "join";
const DEV_PREFIX: &str = "dev-";

/// What's showing, by full tag, for routing clicks and dismissing.
#[derive(Default)]
struct Posted(HashMap<String, Posting>);

struct Posting {
    target: NoticeTarget,
    join_url: Option<String>,
    /// The conversations it stands for: once they're all read, it goes.
    threads: Vec<String>,
}

impl Global for Posted {}

/// Route clicks. Once, at launch: registering also brings back the clicks
/// on notices posted before a relaunch.
pub fn init(cx: &mut App) {
    cx.set_global(Posted::default());
    cx.on_system_notification_response(respond);
}

pub fn notify(notice: Notice, cx: &mut App) {
    let threads = match &notice.target {
        NoticeTarget::Thread { id } => vec![id.clone()],
        _ => Vec::new(),
    };
    post(notice, threads, cx);
}

#[allow(dead_code)] // W5 and W7 dismiss their own.
pub fn dismiss(tag: &str, cx: &mut App) {
    dismiss_full(&full_tag(tag), cx);
}

fn post(notice: Notice, threads: Vec<String>, cx: &mut App) {
    let tag = full_tag(&notice.tag);
    logging::info(&format!("notify: tag={tag} title={}", notice.title));
    let actions = notice
        .join_url
        .iter()
        .map(|_| SystemNotificationAction {
            id: JOIN.into(),
            label: "Join".into(),
        })
        .collect();
    cx.default_global::<Posted>().0.insert(
        tag.clone(),
        Posting {
            target: notice.target,
            join_url: notice.join_url,
            threads,
        },
    );
    cx.show_system_notification(SystemNotification {
        tag: tag.into(),
        title: notice.title.into(),
        body: notice.body.into(),
        actions,
    });
}

fn dismiss_full(tag: &str, cx: &mut App) {
    cx.default_global::<Posted>().0.remove(tag);
    logging::info(&format!("notify: dismissed tag={tag}"));
    cx.dismiss_system_notification(tag);
}

pub fn full_tag(tag: &str) -> String {
    if instance::dev_mode() {
        format!("{DEV_PREFIX}{tag}")
    } else {
        tag.to_string()
    }
}

fn respond(response: SystemNotificationResponse, cx: &mut App) {
    let tag = response.tag.to_string();
    logging::info(&format!(
        "notify: clicked tag={tag} action={}",
        response.action_id.as_deref().unwrap_or("open")
    ));
    let posting = cx
        .default_global::<Posted>()
        .0
        .get(&tag)
        .map(|p| (p.target.clone(), p.join_url.clone()));
    if response.action_id.as_deref() == Some(JOIN) {
        if let Some((_, Some(url))) = posting {
            cx.open_url(&url);
        }
        return;
    }
    let target = posting.map_or_else(|| target_from_tag(&tag), |(target, _)| target);
    shell::show(shell::target_show(target), cx);
}

/// A click on a mail notice from before a relaunch: where its tag says.
pub fn target_from_tag(tag: &str) -> NoticeTarget {
    let tag = tag.strip_prefix(DEV_PREFIX).unwrap_or(tag);
    match tag.strip_prefix("mail-") {
        Some(id) if tag != SUMMARY_TAG && !id.is_empty() => NoticeTarget::Thread { id: id.into() },
        _ => NoticeTarget::Inbox,
    }
}

/// The notices for mail that just arrived, each with its conversations.
pub fn mail_notices(new: &[NewMail]) -> Vec<(Notice, Vec<String>)> {
    if new.len() > MAX_SINGLE {
        let mut threads: Vec<String> = Vec::new();
        let mut senders: Vec<&str> = Vec::new();
        for m in new {
            if !threads.contains(&m.thread_id) {
                threads.push(m.thread_id.clone());
            }
            if !senders.contains(&m.from.as_str()) {
                senders.push(&m.from);
            }
        }
        let body = match senders.as_slice() {
            [one] => format!("From {one}"),
            [a, b] => format!("From {a} and {b}"),
            [a, b, rest @ ..] => format!("From {a}, {b} and {} others", rest.len()),
            [] => String::new(),
        };
        let notice = Notice {
            tag: SUMMARY_TAG.into(),
            title: format!("{} new messages", new.len()),
            body,
            target: NoticeTarget::Inbox,
            join_url: None,
        };
        return vec![(notice, threads)];
    }
    new.iter()
        .map(|m| {
            let subject = if m.subject.is_empty() {
                "(no subject)"
            } else {
                &m.subject
            };
            let notice = Notice {
                tag: format!("mail-{}", m.thread_id),
                title: m.from.clone(),
                body: [subject, m.snippet.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
                target: NoticeTarget::Thread {
                    id: m.thread_id.clone(),
                },
                join_url: None,
            };
            (notice, vec![m.thread_id.clone()])
        })
        .collect()
}

/// Post the notices for `new`, already filtered by the caller.
pub fn notify_new_mail(new: &[NewMail], cx: &mut App) {
    for (notice, threads) in mail_notices(new) {
        post(notice, threads, cx);
    }
}

/// The notices whose conversations are among `changed`, to check whether
/// they're read now.
pub fn showing_for(changed: &[String], cx: &App) -> Vec<(String, Vec<String>)> {
    cx.try_global::<Posted>()
        .map(|posted| {
            posted
                .0
                .iter()
                .filter(|(_, p)| p.threads.iter().any(|t| changed.contains(t)))
                .map(|(tag, p)| (tag.clone(), p.threads.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Take back a notice by its full tag, as `showing_for` gives it.
pub fn dismiss_tag(full_tag: &str, cx: &mut App) {
    dismiss_full(full_tag, cx);
}

/// Settings › Notifications › Send test notification, and ⌘K. The first
/// notice is what brings up macOS's permission prompt.
pub fn send_test(cx: &mut App) {
    notify(
        Notice {
            tag: "test".into(),
            title: "Pigeon notifications are on".into(),
            body: "New mail in the Inbox shows up like this.".into(),
            target: NoticeTarget::Inbox,
            join_url: None,
        },
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(ix: usize, thread: &str, from: &str) -> NewMail {
        NewMail {
            message_id: format!("m{ix}"),
            thread_id: thread.into(),
            from: from.into(),
            subject: format!("Subject {ix}"),
            snippet: format!("Snippet {ix}"),
        }
    }

    #[test]
    fn up_to_three_are_notified_one_by_one() {
        let new = [
            mail(1, "t1", "Ann"),
            mail(2, "t2", "Bo"),
            mail(3, "t1", "Ann"),
        ];
        let notices = mail_notices(&new);
        assert_eq!(notices.len(), 3);
        assert_eq!(notices[0].0.tag, "mail-t1");
        assert_eq!(notices[0].0.title, "Ann");
        assert_eq!(notices[0].0.body, "Subject 1\nSnippet 1");
        assert_eq!(
            notices[0].0.target,
            NoticeTarget::Thread { id: "t1".into() }
        );
        // A newer message in the same conversation replaces its notice.
        assert_eq!(notices[2].0.tag, notices[0].0.tag);
    }

    #[test]
    fn more_than_three_become_one_summary() {
        let new = [
            mail(1, "t1", "Ann"),
            mail(2, "t2", "Bo"),
            mail(3, "t3", "Cy"),
            mail(4, "t3", "Cy"),
            mail(5, "t4", "Di"),
        ];
        let notices = mail_notices(&new);
        assert_eq!(notices.len(), 1);
        let (notice, threads) = &notices[0];
        assert_eq!(notice.tag, SUMMARY_TAG);
        assert_eq!(notice.title, "5 new messages");
        assert_eq!(notice.body, "From Ann, Bo and 2 others");
        assert_eq!(notice.target, NoticeTarget::Inbox);
        assert_eq!(threads, &["t1", "t2", "t3", "t4"]);
        // The next batch reuses the tag, so it replaces this one.
        let next = mail_notices(&[
            mail(6, "a", "X"),
            mail(7, "b", "X"),
            mail(8, "c", "X"),
            mail(9, "d", "Y"),
        ]);
        assert_eq!(next[0].0.tag, SUMMARY_TAG);
        assert_eq!(next[0].0.body, "From X and Y");
    }

    #[test]
    fn clicks_from_before_a_relaunch_route_by_tag() {
        assert_eq!(
            target_from_tag("mail-abc"),
            NoticeTarget::Thread { id: "abc".into() }
        );
        assert_eq!(
            target_from_tag("dev-mail-abc"),
            NoticeTarget::Thread { id: "abc".into() }
        );
        assert_eq!(target_from_tag(SUMMARY_TAG), NoticeTarget::Inbox);
        assert_eq!(target_from_tag("dev-mail-summary"), NoticeTarget::Inbox);
        assert_eq!(target_from_tag("test"), NoticeTarget::Inbox);
    }
}

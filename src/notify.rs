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
//! once their conversations are read. Mail outside Primary comes at the
//! Passive level (MB-Q13). Single mail and reminders play the default
//! sound (MA-073); mail offers Mark as Read and Archive, reminders Join and
//! Snooze, which all work with the window closed (MB-050, MB-051).
//!
//! Sound, grouping and the level go through the vendored gpui-pre-macos
//! (`@` options, see `vendor/gpui-pre-macos/PIGEON_PATCHES.md`).
//!
//! Permission (MA-073): nothing is posted before macOS has been asked,
//! which happens from the card after the first sync, when notifications
//! are turned on in Settings, or with the test notice; not at a random
//! first post.

use std::{collections::HashMap, ptr::NonNull, sync::mpsc, time::Duration};

use block2::RcBlock;
use chrono::NaiveDateTime;
use gpui::{App, Global, SystemNotification, SystemNotificationAction, SystemNotificationResponse};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool},
};
use objc2_foundation::{NSBundle, NSString, NSUserDefaults};

use crate::{
    actions::OpenSettingsSection,
    app::{shell, today},
    backend::{
        categories::{Category, in_category},
        new_mail::NewMail,
    },
    instance, logging,
    settings::{self, AppSettings, Section},
    ui::menu_bar,
};

/// Where a click on the notice's body goes.
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

/// What a notice is about: its sound, level, group and buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One new message; outside Primary it comes quietly.
    Mail {
        primary: bool,
    },
    /// More than `MAX_SINGLE` at once.
    Summary {
        primary: bool,
    },
    Reminder,
    /// From Pigeon itself: the test, an expired sign-in.
    App,
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
    pub kind: Kind,
}

/// The tag of the notice standing for more than `MAX_SINGLE` new messages.
pub const SUMMARY_TAG: &str = "mail-summary";
/// The notice for a sign-in that expired with no window in front (MB-054).
pub const SIGN_IN_TAG: &str = "sign-in";
/// New messages notified one by one; more become one summary.
pub const MAX_SINGLE: usize = 3;
const JOIN: &str = "join";
const MARK_READ: &str = "read";
const ARCHIVE: &str = "archive";
const SNOOZE: &str = "snooze";
const DEV_PREFIX: &str = "dev-";
/// Remembers "Not Now" on the permission card, per bundle.
const CARD_DISMISSED_KEY: &str = "PigeonNotificationCardDismissed";

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

/// Whether macOS lets Pigeon notify (MA-073).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Permission {
    /// Not read yet.
    #[default]
    Unknown,
    /// Not run from an app bundle (`cargo run`): notifications can't work.
    Unavailable,
    NotAsked,
    Allowed,
    /// Turned off in System Settings, or refused.
    Denied,
}

impl Permission {
    /// For Settings › Notifications.
    pub fn label(self) -> &'static str {
        match self {
            Permission::Unknown => "Checking…",
            Permission::Unavailable => "Unavailable outside the app bundle",
            Permission::NotAsked => "Not asked yet",
            Permission::Allowed => "Allowed",
            Permission::Denied => "Off in System Settings",
        }
    }
}

#[derive(Default)]
struct PermissionState {
    permission: Permission,
    /// The card's "Not Now", or an answer given: it doesn't come back.
    card_dismissed: bool,
    /// The test notice waiting for the answer to the prompt.
    pending_test: bool,
}

impl Global for PermissionState {}

/// Route clicks. Once, at launch: registering also brings back the clicks
/// on notices posted before a relaunch.
pub fn init(cx: &mut App) {
    cx.set_global(Posted::default());
    cx.set_global(PermissionState {
        card_dismissed: card_dismissed(),
        ..Default::default()
    });
    cx.on_system_notification_response(respond);
    refresh_permission(cx);
    // Turned on in Settings: ask macOS then, not at the first post.
    let mut was_on = settings::get(cx).notifications;
    cx.observe_global::<AppSettings>(move |cx| {
        let on = settings::get(cx).notifications;
        if on && !was_on && permission(cx) == Permission::NotAsked {
            ask_permission(cx);
        }
        was_on = on;
    })
    .detach();
}

pub fn notify(notice: Notice, cx: &mut App) {
    let threads = match &notice.target {
        NoticeTarget::Thread { id } => vec![id.clone()],
        _ => Vec::new(),
    };
    post(notice, threads, cx);
}

pub fn dismiss(tag: &str, cx: &mut App) {
    dismiss_tag(&full_tag(tag), cx);
}

/// The `@` options and buttons a notice of `kind` carries.
fn actions(kind: Kind, join: bool, sound: bool) -> Vec<SystemNotificationAction> {
    let option = |id: &str| SystemNotificationAction {
        id: id.to_string().into(),
        label: "".into(),
    };
    let button = |id: &'static str, label: &'static str| SystemNotificationAction {
        id: id.into(),
        label: label.into(),
    };
    let (group, hidden) = match kind {
        Kind::Mail { .. } | Kind::Summary { .. } => ("mail", Some("New mail")),
        Kind::Reminder => ("events", Some("Event reminder")),
        Kind::App => ("pigeon", None),
    };
    let mut actions = vec![option(&format!("@thread={group}"))];
    actions.extend(hidden.map(|h| option(&format!("@hidden={h}"))));
    if matches!(
        kind,
        Kind::Mail { primary: false } | Kind::Summary { primary: false }
    ) {
        actions.push(option("@level=passive"));
    }
    if sound && matches!(kind, Kind::Mail { primary: true } | Kind::Reminder) {
        actions.push(option("@sound"));
    }
    match kind {
        Kind::Mail { .. } => {
            actions.push(button(MARK_READ, "Mark as Read"));
            actions.push(button(ARCHIVE, "Archive"));
        }
        Kind::Reminder => {
            if join {
                actions.push(button(JOIN, "Join"));
            }
            actions.push(button(SNOOZE, "Snooze 5 Minutes"));
        }
        Kind::Summary { .. } | Kind::App => {}
    }
    actions
}

fn post(notice: Notice, threads: Vec<String>, cx: &mut App) {
    let tag = full_tag(&notice.tag);
    let body: String = notice
        .body
        .chars()
        .take(80)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    let permission = permission(cx);
    if matches!(permission, Permission::NotAsked | Permission::Denied) {
        logging::info(&format!(
            "notify: tag={tag} not shown ({})",
            permission.label()
        ));
        return;
    }
    logging::info(&format!(
        "notify: tag={tag} title={} body={body}",
        notice.title
    ));
    let actions = actions(
        notice.kind,
        notice.join_url.is_some(),
        settings::get(cx).notification_sound,
    );
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

/// Take back a notice by its full tag, as `showing_for` gives it.
pub fn dismiss_tag(tag: &str, cx: &mut App) {
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

fn base_tag(tag: &str) -> &str {
    tag.strip_prefix(DEV_PREFIX).unwrap_or(tag)
}

fn respond(response: SystemNotificationResponse, cx: &mut App) {
    let tag = response.tag.to_string();
    let action = response.action_id.as_deref();
    logging::info(&format!(
        "notify: clicked tag={tag} action={}",
        action.unwrap_or("open")
    ));
    let posting = cx
        .default_global::<Posted>()
        .0
        .get(&tag)
        .map(|p| (p.target.clone(), p.join_url.clone(), p.threads.clone()));
    let target = posting
        .as_ref()
        .map_or_else(|| target_from_tag(&tag), |(target, ..)| target.clone());
    match action {
        Some(JOIN) => {
            if let Some((_, Some(url), _)) = posting {
                cx.open_url(&url);
            }
        }
        // MB-050: done in place, without bringing the window up.
        Some(MARK_READ | ARCHIVE) => {
            let threads = match (posting, target) {
                (Some((.., threads)), _) if !threads.is_empty() => threads,
                (_, NoticeTarget::Thread { id }) => vec![id],
                _ => return,
            };
            // From the shell, which is there before any click is (the
            // menu bar's Today isn't, at relaunch).
            let backend = shell::backend_of(cx);
            // The panel's own ops, so both behave the same (MB-050).
            for id in &threads {
                if action == Some(ARCHIVE) {
                    menu_bar::archive_thread(&backend, id, cx);
                } else {
                    menu_bar::mark_thread_read(&backend, id, cx);
                }
            }
            dismiss_tag(&tag, cx);
        }
        Some(SNOOZE) => today::snooze(base_tag(&tag), cx),
        _ if base_tag(&tag) == SIGN_IN_TAG => {
            let main = shell::ensure_main_window(cx);
            cx.activate(true);
            main.update(cx, |app, window, cx| {
                app.open_settings_section(&OpenSettingsSection(Section::Accounts), window, cx)
            });
        }
        _ => shell::show(shell::target_show(target), cx),
    }
}

/// A click on a mail notice from before a relaunch: where its tag says.
pub fn target_from_tag(tag: &str) -> NoticeTarget {
    let tag = base_tag(tag);
    match tag.strip_prefix("mail-") {
        Some(id) if tag != SUMMARY_TAG && !id.is_empty() => NoticeTarget::Thread { id: id.into() },
        _ => NoticeTarget::Inbox,
    }
}

/// The notices for mail that just arrived, each with its conversations.
/// `primary` says whether a conversation is in Primary.
pub fn mail_notices(new: &[NewMail], primary: impl Fn(&str) -> bool) -> Vec<(Notice, Vec<String>)> {
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
            title: format!("{} New Messages", new.len()),
            body,
            target: NoticeTarget::Inbox,
            join_url: None,
            kind: Kind::Summary {
                primary: threads.iter().any(|t| primary(t)),
            },
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
                kind: Kind::Mail {
                    primary: primary(&m.thread_id),
                },
            };
            (notice, vec![m.thread_id.clone()])
        })
        .collect()
}

/// Post the notices for `new`, already filtered by the caller.
pub fn notify_new_mail(new: &[NewMail], cx: &mut App) {
    if new.is_empty() {
        return;
    }
    let ids: Vec<String> = new.iter().map(|m| m.thread_id.clone()).collect();
    let threads = shell::backend_of(cx).read(cx).load_threads(&ids);
    // Unknown to the store: as Primary, so it isn't missed.
    let primary = |id: &str| {
        threads
            .iter()
            .find(|t| t.id == id)
            .is_none_or(|t| in_category(t, Category::Primary))
    };
    for (notice, threads) in mail_notices(new, primary) {
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

fn test_notice() -> Notice {
    Notice {
        tag: "test".into(),
        title: "Test Notification".into(),
        body: "New mail and event reminders show up like this.".into(),
        target: NoticeTarget::Inbox,
        join_url: None,
        kind: Kind::App,
    }
}

/// Settings › Notifications › Send Test Notification, and ⌘K. Asks macOS
/// first if it hasn't been asked; Settings says when it's off.
pub fn send_test(cx: &mut App) {
    match permission(cx) {
        Permission::NotAsked => {
            cx.global_mut::<PermissionState>().pending_test = true;
            ask_permission(cx);
        }
        _ => notify(test_notice(), cx),
    }
}

// Permission (MA-073). gpui has no call for these, so they go to
// `UNUserNotificationCenter` directly; only reading and asking, so gpui's
// categories stay its own.

/// macOS's answer, as last read.
pub fn permission(cx: &App) -> Permission {
    cx.try_global::<PermissionState>()
        .map(|s| s.permission)
        .unwrap_or_default()
}

/// The shared center; none outside an app bundle, where touching it aborts.
fn center() -> Option<Retained<AnyObject>> {
    NSBundle::mainBundle().bundleIdentifier()?;
    let class = AnyClass::get(c"UNUserNotificationCenter")?;
    // SAFETY: a class method that returns the shared center.
    unsafe { msg_send![class, currentNotificationCenter] }
}

/// Wait off the main thread for a completion handler's answer, then hand it
/// to `then` on it.
fn on_answer<T: Send + 'static>(
    answer: mpsc::Receiver<T>,
    cx: &mut App,
    then: impl FnOnce(T, &mut App) + 'static,
) {
    let wait = cx
        .background_executor()
        .spawn(async move { answer.recv_timeout(Duration::from_secs(60)).ok() });
    cx.spawn(async move |cx| {
        if let Some(value) = wait.await {
            cx.update(|cx| then(value, cx));
        }
    })
    .detach();
}

/// Read the permission again: at launch, after asking, and when Settings
/// › Notifications shows it.
pub fn refresh_permission(cx: &mut App) {
    let Some(center) = center() else {
        set_permission(Permission::Unavailable, cx);
        return;
    };
    let (send, answer) = mpsc::channel();
    let block = RcBlock::new(move |settings: NonNull<AnyObject>| {
        // SAFETY: `settings` is a `UNNotificationSettings` for the call.
        let status: isize = unsafe { msg_send![settings.as_ref(), authorizationStatus] };
        send.send(status).ok();
    });
    // SAFETY: the block matches `void (^)(UNNotificationSettings *)`.
    let () = unsafe { msg_send![&center, getNotificationSettingsWithCompletionHandler: &*block] };
    on_answer(answer, cx, |status, cx| {
        // Not determined 0, denied 1; authorized, provisional, ephemeral.
        let permission = match status {
            0 => Permission::NotAsked,
            1 => Permission::Denied,
            _ => Permission::Allowed,
        };
        set_permission(permission, cx);
    });
}

fn set_permission(permission: Permission, cx: &mut App) {
    let state = cx.global_mut::<PermissionState>();
    if state.permission == permission {
        return;
    }
    state.permission = permission;
    let pending_test = permission == Permission::Allowed && std::mem::take(&mut state.pending_test);
    logging::info(&format!("notify: permission {}", permission.label()));
    if pending_test {
        notify(test_notice(), cx);
    }
    cx.refresh_windows();
}

/// Bring up macOS's prompt; once answered, the card goes for good.
pub fn ask_permission(cx: &mut App) {
    let Some(center) = center() else {
        return;
    };
    dismiss_card(cx);
    let (send, answer) = mpsc::channel();
    let block = RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
        send.send(granted.as_bool()).ok();
    });
    // Alert and sound; the Dock badge isn't a notification.
    let options: usize = 4 | 2;
    // SAFETY: the block matches `void (^)(BOOL, NSError *)`.
    let () = unsafe {
        msg_send![&center, requestAuthorizationWithOptions: options, completionHandler: &*block]
    };
    on_answer(answer, cx, |granted, cx| {
        logging::info(&format!("notify: asked, granted={granted}"));
        refresh_permission(cx);
    });
}

/// System Settings › Notifications › Pigeon.
pub fn open_system_settings(cx: &mut App) {
    let id = NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
        .unwrap_or_default();
    cx.open_url(&format!(
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={id}"
    ));
}

fn card_dismissed() -> bool {
    NSUserDefaults::standardUserDefaults().boolForKey(&NSString::from_str(CARD_DISMISSED_KEY))
}

/// The card after the first sync (`ui::status::notifications_card`) shows:
/// notifications on, macOS not asked yet, and no "Not Now".
pub fn offer_card(cx: &App) -> bool {
    cx.try_global::<PermissionState>()
        .is_some_and(|s| s.permission == Permission::NotAsked && !s.card_dismissed)
        && settings::get(cx).notifications
}

/// "Not Now", or asked: the card doesn't come back.
pub fn dismiss_card(cx: &mut App) {
    cx.global_mut::<PermissionState>().card_dismissed = true;
    NSUserDefaults::standardUserDefaults()
        .setBool_forKey(true, &NSString::from_str(CARD_DISMISSED_KEY));
    cx.refresh_windows();
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
        let notices = mail_notices(&new, |_| true);
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
        let notices = mail_notices(&new, |_| true);
        assert_eq!(notices.len(), 1);
        let (notice, threads) = &notices[0];
        assert_eq!(notice.tag, SUMMARY_TAG);
        assert_eq!(notice.title, "5 New Messages");
        assert_eq!(notice.kind, Kind::Summary { primary: true });
        assert_eq!(notice.body, "From Ann, Bo and 2 others");
        assert_eq!(notice.target, NoticeTarget::Inbox);
        assert_eq!(threads, &["t1", "t2", "t3", "t4"]);
        // The next batch reuses the tag, so it replaces this one.
        let next = mail_notices(
            &[
                mail(6, "a", "X"),
                mail(7, "b", "X"),
                mail(8, "c", "X"),
                mail(9, "d", "Y"),
            ],
            |_| false,
        );
        assert_eq!(next[0].0.tag, SUMMARY_TAG);
        assert_eq!(next[0].0.body, "From X and Y");
        assert_eq!(next[0].0.kind, Kind::Summary { primary: false });
    }

    #[test]
    fn mail_outside_primary_comes_quietly() {
        let new = [mail(1, "t1", "Ann"), mail(2, "promo", "Shop")];
        let notices = mail_notices(&new, |id| id == "t1");
        assert_eq!(notices[0].0.kind, Kind::Mail { primary: true });
        assert_eq!(notices[1].0.kind, Kind::Mail { primary: false });
        let ids = |kind, join, sound| -> Vec<String> {
            actions(kind, join, sound)
                .into_iter()
                .map(|a| a.id.to_string())
                .collect()
        };
        assert_eq!(
            ids(Kind::Mail { primary: true }, false, true),
            [
                "@thread=mail",
                "@hidden=New mail",
                "@sound",
                "read",
                "archive"
            ]
        );
        assert_eq!(
            ids(Kind::Mail { primary: false }, false, true),
            [
                "@thread=mail",
                "@hidden=New mail",
                "@level=passive",
                "read",
                "archive"
            ]
        );
        // Summaries never sound; the setting turns sound off.
        assert!(!ids(Kind::Summary { primary: true }, false, true).contains(&"@sound".into()));
        assert_eq!(
            ids(Kind::Reminder, true, false),
            ["@thread=events", "@hidden=Event reminder", "join", "snooze"]
        );
        assert_eq!(ids(Kind::App, false, true), ["@thread=pigeon"]);
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

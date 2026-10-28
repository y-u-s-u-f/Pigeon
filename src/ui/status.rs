//! The sync status line under the account card (ACC-5, STA-1/3/4), and the
//! empty state for a feature whose scope wasn't granted (ACC-6, CAL-1).

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div,
};

use super::style::*;
use crate::{
    actions::SignIn,
    assets::Icon,
    backend::{
        Backend, Source, SyncStatus,
        auth::{CALENDAR_SCOPE, CONTACTS_SCOPE, GMAIL_SCOPE},
    },
    theme::Theme,
};

/// "Synced just now", "Syncing 340 of 1,000", "Offline · 3 changes waiting".
pub fn label(status: &SyncStatus, now_ms: i64) -> String {
    match status {
        SyncStatus::Demo => "Demo data".into(),
        SyncStatus::SignedOut => "Not signed in".into(),
        SyncStatus::Syncing { total: 0, .. } => "Syncing…".into(),
        SyncStatus::Syncing { done, total } => {
            format!("Syncing {} of {}", thousands(*done), thousands(*total))
        }
        SyncStatus::Synced { at } => format!("Synced {}", ago(*at, now_ms)),
        SyncStatus::Offline { waiting: 0 } => "Offline".into(),
        SyncStatus::Offline { waiting: 1 } => "Offline · 1 change waiting".into(),
        SyncStatus::Offline { waiting } => {
            format!("Offline · {} changes waiting", thousands(*waiting))
        }
        SyncStatus::AuthExpired => "Sign in again".into(),
        SyncStatus::Waiting => "Waiting for Gmail…".into(),
    }
}

/// "just now", "4 min ago", "2 h ago", "3 days ago".
pub fn ago(at_ms: i64, now_ms: i64) -> String {
    let minutes = (now_ms - at_ms).max(0) / 60_000;
    match minutes {
        0 => "just now".into(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} h ago", minutes / 60),
        1440..2880 => "1 day ago".into(),
        _ => format!("{} days ago", minutes / 1440),
    }
}

pub(crate) fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (ix, c) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One quiet line; "Sign in again" is a button.
pub fn status_line(status: &SyncStatus, now_ms: i64, theme: &Theme) -> AnyElement {
    let text = label(status, now_ms);
    if *status == SyncStatus::AuthExpired {
        return focusable_control(div().id("sign-in-again"), theme)
            .role(gpui::Role::Button)
            .aria_label("Sign in again")
            .px(SPACE_MD)
            .py(SPACE_XS)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .text_size(TEXT_SM)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.danger)
            .hover(|s| s.bg(theme.hover_bg))
            .cursor_pointer()
            .child(super::icon(Icon::User, gpui::px(12.), theme.danger))
            .child(text)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(SignIn), cx))
            .into_any_element();
    }
    div()
        .id("sync-status")
        .role(gpui::Role::Status)
        .aria_label(text.clone())
        .px(SPACE_MD)
        .text_size(TEXT_XS)
        .text_color(theme.text_faint)
        .truncate()
        .child(text)
        .into_any_element()
}

/// Which features Google allowed (ACC-6): "Mail, Calendar and Contacts
/// allowed", and what wasn't.
pub fn granted(backend: &Backend) -> String {
    match backend.source() {
        Source::Demo => return "Demo data".into(),
        Source::SignedOut => return String::new(),
        Source::Google => {}
    }
    let features = [
        ("Mail", GMAIL_SCOPE),
        ("Calendar", CALENDAR_SCOPE),
        ("Contacts", CONTACTS_SCOPE),
    ];
    let (on, off): (Vec<_>, Vec<_>) = features.iter().partition(|(_, s)| backend.has_scope(s));
    let names = |list: &[&(&str, &str)]| {
        let names: Vec<&str> = list.iter().map(|(name, _)| *name).collect();
        match names.as_slice() {
            [] => String::new(),
            [one] => one.to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        }
    };
    match (on.is_empty(), off.is_empty()) {
        (_, true) => format!("{} allowed", names(&on)),
        (true, false) => "Nothing allowed: sign in again".into(),
        (false, false) => format!("{} allowed · {} not", names(&on), names(&off)),
    }
}

/// Every scope Google granted, in plain words (Settings › Accounts).
pub fn scopes(backend: &Backend) -> String {
    match backend.source() {
        Source::Demo => return "Demo data: everything, on this Mac only".into(),
        Source::SignedOut => return "Not signed in".into(),
        Source::Google => {}
    }
    let mut names: Vec<String> = Vec::new();
    for scope in backend.granted_scopes() {
        let name = scope_name(scope);
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    if names.is_empty() {
        "Nothing: sign in again to allow mail, calendar and contacts".into()
    } else {
        names.join(" · ")
    }
}

fn scope_name(scope: &str) -> &str {
    match scope.rsplit('/').next().unwrap_or(scope) {
        "openid" => "",
        "userinfo.email" | "userinfo.profile" | "email" | "profile" => "Name and email",
        "gmail.modify" => "Mail: read, organize and send",
        "calendar" => "Calendar",
        "contacts.readonly" => "Contacts (read-only)",
        "contacts.other.readonly" => "Other contacts (read-only)",
        "directory.readonly" => "Directory (read-only)",
        other => other,
    }
}

/// "3 min ago · Oct 27, 2:02 PM", or why there's none (Settings › Accounts).
pub fn last_sync(backend: &Backend, now_ms: i64) -> String {
    match backend.source() {
        Source::Demo => return "Demo data is always up to date".into(),
        Source::SignedOut => return "Not signed in".into(),
        Source::Google => {}
    }
    let Some(at) = backend.last_synced() else {
        return "Not yet".into();
    };
    let when = chrono::DateTime::from_timestamp_millis(at)
        .map(|t| {
            let t = t.with_timezone(&chrono::Local);
            format!(
                "{}, {}",
                t.format("%b %-d"),
                crate::calendar::time::time_label(t.time())
            )
        })
        .unwrap_or_default();
    let status = match backend.status() {
        SyncStatus::Synced { .. } => String::new(),
        status => format!(" · now {}", label(status, now_ms)),
    };
    format!("{} · {when}{status}", capitalize(&ago(at, now_ms)))
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// In place of a feature Google wasn't allowed to show.
pub fn missing_scope(what: &'static str, theme: &Theme) -> AnyElement {
    let title = format!("{what} access wasn't granted");
    div()
        .id("missing-scope")
        .role(gpui::Role::Region)
        .aria_label(title.clone())
        .flex_1()
        .h_full()
        .child(super::empty_state(
            Icon::Calendar,
            title,
            Some("Sign in again to allow it."),
            theme,
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_plainly() {
        let now = 10_000_000;
        assert_eq!(
            label(&SyncStatus::Synced { at: now - 5_000 }, now),
            "Synced just now"
        );
        assert_eq!(
            label(&SyncStatus::Synced { at: now - 180_000 }, now),
            "Synced 3 min ago"
        );
        assert_eq!(label(&SyncStatus::Waiting, now), "Waiting for Gmail…");
        assert_eq!(
            label(
                &SyncStatus::Syncing {
                    done: 340,
                    total: 1000
                },
                now
            ),
            "Syncing 340 of 1,000"
        );
        assert_eq!(
            label(&SyncStatus::Syncing { done: 0, total: 0 }, now),
            "Syncing…"
        );
        assert_eq!(
            label(&SyncStatus::Offline { waiting: 3 }, now),
            "Offline · 3 changes waiting"
        );
        assert_eq!(label(&SyncStatus::AuthExpired, now), "Sign in again");
    }

    #[test]
    fn scopes_read_plainly() {
        assert_eq!(scope_name("openid"), "");
        assert_eq!(scope_name(GMAIL_SCOPE), "Mail: read, organize and send");
        assert_eq!(scope_name(CALENDAR_SCOPE), "Calendar");
        assert_eq!(scope_name("https://example.com/auth/other"), "other");
        assert_eq!(capitalize("just now"), "Just now");
    }

    #[test]
    fn ago_rounds_down() {
        let min = 60_000;
        let now = 100 * 24 * 60 * min;
        assert_eq!(ago(now - 59_000, now), "just now");
        assert_eq!(ago(now - min, now), "1 min ago");
        assert_eq!(ago(now - 59 * min, now), "59 min ago");
        assert_eq!(ago(now - 60 * min, now), "1 h ago");
        assert_eq!(ago(now - 23 * 60 * min - 59 * min, now), "23 h ago");
        assert_eq!(ago(now - 24 * 60 * min, now), "1 day ago");
        assert_eq!(ago(now - 3 * 24 * 60 * min, now), "3 days ago");
        // A clock that went back reads as now.
        assert_eq!(ago(now + min, now), "just now");
    }
}

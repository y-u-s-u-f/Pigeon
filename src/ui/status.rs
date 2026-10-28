//! The sync status line under the account card (ACC-5, STA-1/3/4), the
//! card that asks to turn on notifications after the first sync (MA-073),
//! and the empty state for a feature whose scope wasn't granted (ACC-6,
//! CAL-1).

use gpui::{
    AnyElement, App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};

use super::{
    controls::{ButtonKind, push_button},
    style::*,
};
use crate::{
    actions::SignIn,
    assets::Icon,
    backend::{
        Backend, Source, SyncStatus,
        auth::{CALENDAR_SCOPE, CONTACTS_SCOPE, GMAIL_SCOPE},
    },
    notify,
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

/// "just now", "4 min ago", "2 hr ago", "3 days ago".
pub fn ago(at_ms: i64, now_ms: i64) -> String {
    let minutes = (now_ms - at_ms).max(0) / 60_000;
    match minutes {
        0 => "just now".into(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} hr ago", minutes / 60),
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
    if *status == SyncStatus::AuthExpired {
        return focusable_control(div().id("sign-in-again"), theme)
            .role(gpui::Role::Button)
            .aria_label("Sign In Again")
            .px(SPACE_MD)
            .py(SPACE_XS)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .text_size(TEXT_SM)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text_danger)
            .hover(|s| s.bg(theme.hover_bg))
            .child(super::icon(Icon::LogIn, ICON_SM, theme.text_danger))
            .child("Sign In Again")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(SignIn), cx))
            .into_any_element();
    }
    let text = label(status, now_ms);
    // ME-057: Offline needs noticing; syncing turns while it's under way
    // (MA-066).
    let (dot, color) = match status {
        SyncStatus::Offline { .. } => (Some(theme.warning), theme.text_muted),
        _ => (None, theme.text_faint),
    };
    let syncing = matches!(status, SyncStatus::Syncing { .. } | SyncStatus::Waiting);
    div()
        .id("sync-status")
        .role(gpui::Role::Status)
        .aria_label(text.clone())
        .px(SPACE_MD)
        .flex()
        .items_center()
        .gap(SPACE_XS)
        .text_size(TEXT_XS)
        .text_color(color)
        .children(dot.map(|dot| {
            div()
                .flex_none()
                .size(SPACE_XS + SPACE_2XS)
                .rounded_full()
                .bg(dot)
        }))
        .when(syncing, |this| {
            this.child(super::motion::spinner(
                "sync-spinner",
                super::icon(Icon::RefreshCw, ICON_SM, theme.text_faint),
            ))
        })
        .child(div().min_w_0().truncate().child(text))
        .into_any_element()
}

/// MA-073: Pigeon asks before macOS does, once there's something to tell:
/// after the first sync, or with the demo data. "Turn On" brings up macOS's
/// prompt; "Not Now" puts it away for good (Settings can still turn it on).
pub fn notifications_card(backend: &Backend, theme: &Theme, cx: &App) -> Option<AnyElement> {
    let ready = match backend.source() {
        Source::Demo => true,
        Source::Google => backend.last_synced().is_some(),
        Source::SignedOut => false,
    };
    if !ready || !notify::offer_card(cx) {
        return None;
    }
    let title = "Turn on notifications?";
    Some(
        div()
            .id("notifications-card")
            .role(gpui::Role::Region)
            .aria_label(title)
            .p(SPACE_MD)
            .flex()
            .flex_col()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .bg(theme.chip_bg)
            .line_height(LINE_UI)
            .child(
                super::label("notifications-card-title", title)
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text),
            )
            .child(
                super::label(
                    "notifications-card-text",
                    "New mail and event reminders, while Pigeon is in the background.",
                )
                .text_size(TEXT_XS)
                .text_color(theme.text_muted),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(SPACE_XS)
                    .child(
                        push_button(
                            "notifications-turn-on",
                            "Turn On",
                            ButtonKind::Primary,
                            theme,
                        )
                        .on_click(|_, _, cx| notify::ask_permission(cx)),
                    )
                    .child(
                        push_button("notifications-not-now", "Not Now", ButtonKind::Quiet, theme)
                            .on_click(|_, _, cx| notify::dismiss_card(cx)),
                    ),
            )
            .into_any_element(),
    )
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

/// In place of a feature Google wasn't allowed to show, with the way to
/// allow it (ME-058).
pub fn missing_scope(what: &'static str, glyph: Icon, theme: &Theme) -> AnyElement {
    let title = format!("{what} access wasn't granted");
    div()
        .id("missing-scope")
        .role(gpui::Role::Region)
        .aria_label(title.clone())
        .flex_1()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(SPACE_SM)
        .line_height(LINE_UI)
        .child(super::icon(glyph, ICON_HERO, theme.text_faint))
        .child(
            super::label("missing-scope-title", title)
                .text_size(TEXT_BASE)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted),
        )
        .child(
            super::label("missing-scope-hint", "Sign in again and allow it.")
                .mb(SPACE_SM)
                .text_size(TEXT_SM)
                .text_color(theme.text_faint),
        )
        .child(
            push_button("sign-in-again", "Sign In Again", ButtonKind::Primary, theme)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(SignIn), cx)),
        )
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
        assert_eq!(ago(now - 60 * min, now), "1 hr ago");
        assert_eq!(ago(now - 23 * 60 * min - 59 * min, now), "23 hr ago");
        assert_eq!(ago(now - 24 * 60 * min, now), "1 day ago");
        assert_eq!(ago(now - 3 * 24 * 60 * min, now), "3 days ago");
        // A clock that went back reads as now.
        assert_eq!(ago(now + min, now), "just now");
    }
}

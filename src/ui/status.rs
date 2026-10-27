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
        SyncStatus::Synced { at } => {
            let minutes = (now_ms - at).max(0) / 60_000;
            match minutes {
                0 => "Synced just now".into(),
                1 => "Synced 1 minute ago".into(),
                2..60 => format!("Synced {minutes} minutes ago"),
                _ => format!("Synced {} h ago", minutes / 60),
            }
        }
        SyncStatus::Offline { waiting: 0 } => "Offline".into(),
        SyncStatus::Offline { waiting: 1 } => "Offline · 1 change waiting".into(),
        SyncStatus::Offline { waiting } => {
            format!("Offline · {} changes waiting", thousands(*waiting))
        }
        SyncStatus::AuthExpired => "Sign in again".into(),
    }
}

fn thousands(n: usize) -> String {
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
            "Synced 3 minutes ago"
        );
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
}

//! Every user-facing command is an action, so keyboard shortcuts, toolbar
//! buttons and (later) the command palette all go through one path. Handlers
//! live on `PigeonApp` in `app.rs`.

use gpui::{App, KeyBinding, actions};

actions!(
    pigeon,
    [
        Quit,
        ToggleSidebar,
        ToggleSearch,
        ToggleArchivedSection,
        SelectNextThread,
        SelectPrevThread,
        Archive,
        Delete,
        ToggleStar,
        MarkUnread,
        Reply,
        ReplyAll,
        Forward,
    ]
);

/// Key context set on the root view.
pub const CONTEXT: &str = "Pigeon";

/// The keymap. Shortcut and command palette work extends this.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("down", SelectNextThread, Some(CONTEXT)),
        KeyBinding::new("up", SelectPrevThread, Some(CONTEXT)),
    ]);
}

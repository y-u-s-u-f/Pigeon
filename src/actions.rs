//! Every user-facing command is an action, so keyboard shortcuts, toolbar
//! buttons and the command palette all go through one path. Handlers live on
//! `PigeonApp` in `app.rs`.
//!
//! Key contexts, outermost first:
//! - `Pigeon`: the root view. Shortcuts with modifiers work everywhere.
//! - `Mailbox`: the panes, which hold focus while no overlay is open. Gets the
//!   extra identifier `Reader` while the reading pane has keyboard focus.
//! - `CommandPalette` and `ShortcutsHelp`: the overlays, siblings of
//!   `Mailbox`, so single-letter shortcuts are off while they are open.
//! - `Input`: gpui-component's text inputs. Single-letter shortcuts also
//!   require `!Input`, so typing never triggers them.

use gpui::{Action, App, KeyBinding, KeyContext, actions};

use crate::{assets::Icon, ui::command_palette};

actions!(
    pigeon,
    [
        Quit,
        ToggleSidebar,
        ToggleSearch,
        ToggleArchivedSection,
        ToggleCommandPalette,
        ToggleShortcutsHelp,
        SelectNextThread,
        SelectPrevThread,
        OpenThread,
        BackToList,
        ScrollReaderDown,
        ScrollReaderUp,
        Archive,
        Delete,
        ToggleStar,
        MarkUnread,
        MarkRead,
        Compose,
        Reply,
        ReplyAll,
        Forward,
        GoToInbox,
        GoToStarred,
        GoToSent,
        GoToDrafts,
        GoToArchive,
        GoToSpam,
        GoToTrash,
        GoToLabel,
    ]
);

/// Key context set on the root view.
pub const CONTEXT: &str = "Pigeon";
/// Key context of the panes; see the module docs.
pub const MAILBOX: &str = "Mailbox";
/// Extra identifier on the `Mailbox` context while the reader has focus.
pub const READER: &str = "Reader";
pub const PALETTE: &str = "CommandPalette";
pub const HELP: &str = "ShortcutsHelp";

/// The keymap. Where an action has several bindings, the one added last is
/// the one the palette shows.
pub fn bind_keys(cx: &mut App) {
    const MAIL: &str = "Mailbox && !Input";
    const LIST: &str = "Mailbox && !Reader && !Input";
    const READING: &str = "Mailbox && Reader && !Input";
    const PALETTE_INPUT: &str = "CommandPalette > Input";

    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-shift-p", ToggleCommandPalette, Some(CONTEXT)),
        KeyBinding::new("cmd-k", ToggleCommandPalette, Some(CONTEXT)),
        KeyBinding::new("cmd-\\", ToggleSidebar, Some(CONTEXT)),
        // Moving through the list.
        KeyBinding::new("down", SelectNextThread, Some(LIST)),
        KeyBinding::new("up", SelectPrevThread, Some(LIST)),
        KeyBinding::new("j", SelectNextThread, Some(MAIL)),
        KeyBinding::new("k", SelectPrevThread, Some(MAIL)),
        KeyBinding::new("enter", OpenThread, Some(LIST)),
        KeyBinding::new("o", OpenThread, Some(LIST)),
        // Reading.
        KeyBinding::new("down", ScrollReaderDown, Some(READING)),
        KeyBinding::new("up", ScrollReaderUp, Some(READING)),
        KeyBinding::new("escape", BackToList, Some(READING)),
        KeyBinding::new("u", BackToList, Some(READING)),
        // Thread actions.
        KeyBinding::new("e", Archive, Some(MAIL)),
        KeyBinding::new("#", Delete, Some(MAIL)),
        KeyBinding::new("s", ToggleStar, Some(MAIL)),
        KeyBinding::new("shift-u", MarkUnread, Some(MAIL)),
        KeyBinding::new("shift-i", MarkRead, Some(MAIL)),
        KeyBinding::new("c", Compose, Some(MAIL)),
        KeyBinding::new("r", Reply, Some(MAIL)),
        KeyBinding::new("a", ReplyAll, Some(MAIL)),
        KeyBinding::new("f", Forward, Some(MAIL)),
        KeyBinding::new("/", ToggleSearch, Some(MAIL)),
        KeyBinding::new("?", ToggleShortcutsHelp, Some(MAIL)),
        // Go to.
        KeyBinding::new("g i", GoToInbox, Some(MAIL)),
        KeyBinding::new("g s", GoToStarred, Some(MAIL)),
        KeyBinding::new("g t", GoToSent, Some(MAIL)),
        KeyBinding::new("g d", GoToDrafts, Some(MAIL)),
        KeyBinding::new("g a", GoToArchive, Some(MAIL)),
        KeyBinding::new("g !", GoToSpam, Some(MAIL)),
        KeyBinding::new("g l", GoToLabel, Some(MAIL)),
        // Overlays.
        KeyBinding::new("escape", ToggleShortcutsHelp, Some(HELP)),
        KeyBinding::new("?", ToggleShortcutsHelp, Some(HELP)),
        KeyBinding::new("enter", command_palette::Confirm, Some(PALETTE)),
        KeyBinding::new("escape", command_palette::Dismiss, Some(PALETTE)),
        // The input swallows up and down, so these bind inside it.
        KeyBinding::new("up", command_palette::SelectPrev, Some(PALETTE_INPUT)),
        KeyBinding::new("down", command_palette::SelectNext, Some(PALETTE_INPUT)),
        KeyBinding::new("ctrl-p", command_palette::SelectPrev, Some(PALETTE_INPUT)),
        KeyBinding::new("ctrl-n", command_palette::SelectNext, Some(PALETTE_INPUT)),
    ]);
}

/// The context stack of the panes, flattened into one context, for looking up
/// shortcuts to show while focus is elsewhere (in the palette, say).
pub fn mailbox_context(reader: bool) -> KeyContext {
    let mut context = KeyContext::default();
    context.add(CONTEXT);
    context.add(MAILBOX);
    if reader {
        context.add(READER);
    }
    context
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Navigation,
    Conversation,
    Compose,
    GoTo,
    App,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Navigation => "Navigation",
            Group::Conversation => "Conversation",
            Group::Compose => "Compose",
            Group::GoTo => "Go to",
            Group::App => "Application",
        }
    }
}

/// A named action, for the command palette and the shortcuts overlay.
pub struct Command {
    pub name: &'static str,
    pub group: Group,
    pub icon: Icon,
    pub action: fn() -> Box<dyn Action>,
    /// Shown in the shortcuts overlay but not offered in the palette.
    pub help_only: bool,
}

macro_rules! commands {
    ($($group:ident: $name:literal, $icon:ident, $action:ident $(, $help_only:ident)?;)*) => {
        &[$(Command {
            name: $name,
            group: Group::$group,
            icon: Icon::$icon,
            action: || Box::new($action),
            help_only: commands!(@help $($help_only)?),
        }),*]
    };
    (@help) => { false };
    (@help help_only) => { true };
}

/// Every command, in palette order.
pub const COMMANDS: &[Command] = commands! {
    Compose: "Compose New Message", SquarePen, Compose;
    Compose: "Reply", Reply, Reply;
    Compose: "Reply All", ReplyAll, ReplyAll;
    Compose: "Forward", Forward, Forward;
    Conversation: "Archive", Archive, Archive;
    Conversation: "Delete", Trash, Delete;
    Conversation: "Star / Unstar", Star, ToggleStar;
    Conversation: "Mark as Unread", MailUnread, MarkUnread;
    Conversation: "Mark as Read", MailOpen, MarkRead;
    Navigation: "Next Conversation", ChevronDown, SelectNextThread;
    Navigation: "Previous Conversation", ChevronUp, SelectPrevThread;
    Navigation: "Open Conversation", Mail, OpenThread;
    Navigation: "Back to List", Inbox, BackToList;
    Navigation: "Scroll Conversation Down", ChevronDown, ScrollReaderDown, help_only;
    Navigation: "Scroll Conversation Up", ChevronUp, ScrollReaderUp, help_only;
    Navigation: "Search Mail", Search, ToggleSearch;
    GoTo: "Go to Inbox", Inbox, GoToInbox;
    GoTo: "Go to Starred", Star, GoToStarred;
    GoTo: "Go to Sent", Send, GoToSent;
    GoTo: "Go to Drafts", FilePen, GoToDrafts;
    GoTo: "Go to Archive", Archive, GoToArchive;
    GoTo: "Go to Spam", OctagonAlert, GoToSpam;
    GoTo: "Go to Trash", Trash, GoToTrash;
    GoTo: "Go to Label…", ChevronRight, GoToLabel;
    App: "Command Palette", Command, ToggleCommandPalette, help_only;
    App: "Keyboard Shortcuts", Keyboard, ToggleShortcutsHelp;
    App: "Toggle Sidebar", PanelLeft, ToggleSidebar;
    App: "Show / Hide Archived in Inbox", Archive, ToggleArchivedSection;
    App: "Quit Pigeon", Command, Quit;
};

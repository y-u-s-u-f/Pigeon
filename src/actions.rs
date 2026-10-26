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
//! - `Calendar`: the calendar pane, in place of `Mailbox` while the calendar
//!   is showing. Its event panel adds `EventPanel`, and the panel's date and
//!   time fields `DateField` and `TimeField`, and its guest field `Guests`,
//!   around their inputs. Its buttons that take focus (calendar, all day,
//!   reminder) are `PanelControl`.
//! - `Composer`: the draft in the reading pane, inside `Mailbox`; its
//!   recipient fields add `Recipients`. `SearchBar`: the search field above the
//!   list, inside `Mailbox`.
//! - `Input`: gpui-component's text inputs. Single-letter shortcuts also
//!   require `!Input`, so typing never triggers them.

use gpui::{Action, App, KeyBinding, KeyContext, actions};

use crate::{
    assets::Icon,
    search,
    ui::{calendar::event_panel, command_palette, composer},
};

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
        SendMessage,
        CloseDraft,
        DiscardDraft,
        ShowCcBcc,
        Undo,
        CloseSearch,
        NextMessage,
        PrevMessage,
        GoToInbox,
        GoToStarred,
        GoToSent,
        GoToDrafts,
        GoToArchive,
        GoToSpam,
        GoToTrash,
        GoToLabel,
        GoToMail,
        GoToCalendar,
        ShowDayView,
        ShowWeekView,
        ShowMonthView,
        GoToToday,
        NextPeriod,
        PrevPeriod,
        NewEvent,
        OpenEvent,
        DeleteEvent,
        SelectNextEvent,
        SelectPrevEvent,
        CloseEvent,
        ToggleCalendarsMenu,
        AcceptInvite,
        MaybeInvite,
        DeclineInvite,
        OpenInviteInCalendar,
        RemoveCancelledEvent,
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
pub const COMPOSER: &str = "Composer";
pub const RECIPIENTS: &str = "Recipients";
pub const SEARCH: &str = "SearchBar";
pub const CALENDAR: &str = "Calendar";
pub const EVENT_PANEL: &str = "EventPanel";
pub const DATE_FIELD: &str = "DateField";
pub const TIME_FIELD: &str = "TimeField";
pub const GUESTS: &str = "Guests";
pub const PANEL_CONTROL: &str = "PanelControl";

/// The keymap. Where an action has several bindings, the one added last is
/// the one the palette shows.
pub fn bind_keys(cx: &mut App) {
    const MAIL: &str = "Mailbox && !Input";
    const LIST: &str = "Mailbox && !Reader && !Input";
    const READING: &str = "Mailbox && Reader && !Input";
    const PALETTE_INPUT: &str = "CommandPalette > Input";
    const COMPOSER_INPUT: &str = "Composer > Input";
    const RECIPIENTS_INPUT: &str = "Recipients > Input";
    const SEARCH_INPUT: &str = "SearchBar > Input";
    const CAL: &str = "Calendar && !Input";
    const DATE_INPUT: &str = "DateField > Input";
    const TIME_INPUT: &str = "TimeField > Input";
    const GUESTS_INPUT: &str = "Guests > Input";

    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-shift-p", ToggleCommandPalette, Some(CONTEXT)),
        KeyBinding::new("cmd-k", ToggleCommandPalette, Some(CONTEXT)),
        KeyBinding::new("cmd-\\", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("cmd-1", GoToMail, Some(CONTEXT)),
        KeyBinding::new("cmd-2", GoToCalendar, Some(CONTEXT)),
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
        KeyBinding::new("n", NextMessage, Some(READING)),
        KeyBinding::new("p", PrevMessage, Some(READING)),
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
        // Answering the invite in the selected conversation.
        KeyBinding::new("shift-y", AcceptInvite, Some(MAIL)),
        KeyBinding::new("shift-m", MaybeInvite, Some(MAIL)),
        KeyBinding::new("shift-n", DeclineInvite, Some(MAIL)),
        KeyBinding::new("shift-c", OpenInviteInCalendar, Some(MAIL)),
        KeyBinding::new("shift-x", RemoveCancelledEvent, Some(MAIL)),
        // Search. Escape in the list clears a search too.
        KeyBinding::new("escape", CloseSearch, Some(LIST)),
        KeyBinding::new("escape", CloseSearch, Some(SEARCH)),
        KeyBinding::new("down", search::FocusResults, Some(SEARCH_INPUT)),
        // Compose. Inputs take enter and escape themselves, so these also bind
        // inside them, where they win.
        KeyBinding::new("cmd-enter", SendMessage, Some(CONTEXT)),
        KeyBinding::new("cmd-enter", SendMessage, Some(COMPOSER_INPUT)),
        KeyBinding::new("cmd-shift-c", ShowCcBcc, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-d", DiscardDraft, Some(COMPOSER)),
        KeyBinding::new("escape", CloseDraft, Some(COMPOSER)),
        KeyBinding::new("cmd-z", Undo, Some(MAIL)),
        KeyBinding::new("tab", composer::NextField, Some(COMPOSER)),
        KeyBinding::new("shift-tab", composer::PrevField, Some(COMPOSER)),
        KeyBinding::new("down", composer::SuggestNext, Some(RECIPIENTS_INPUT)),
        KeyBinding::new("up", composer::SuggestPrev, Some(RECIPIENTS_INPUT)),
        KeyBinding::new("ctrl-n", composer::SuggestNext, Some(RECIPIENTS_INPUT)),
        KeyBinding::new("ctrl-p", composer::SuggestPrev, Some(RECIPIENTS_INPUT)),
        KeyBinding::new(
            "backspace",
            composer::RemoveLastRecipient,
            Some(RECIPIENTS_INPUT),
        ),
        KeyBinding::new(
            "escape",
            composer::DismissSuggestions,
            Some(RECIPIENTS_INPUT),
        ),
        KeyBinding::new("?", ToggleShortcutsHelp, Some(MAIL)),
        // Go to.
        KeyBinding::new("g i", GoToInbox, Some(MAIL)),
        KeyBinding::new("g s", GoToStarred, Some(MAIL)),
        KeyBinding::new("g t", GoToSent, Some(MAIL)),
        KeyBinding::new("g d", GoToDrafts, Some(MAIL)),
        KeyBinding::new("g a", GoToArchive, Some(MAIL)),
        KeyBinding::new("g !", GoToSpam, Some(MAIL)),
        KeyBinding::new("g l", GoToLabel, Some(MAIL)),
        // Calendar, like Google Calendar.
        KeyBinding::new("d", ShowDayView, Some(CAL)),
        KeyBinding::new("w", ShowWeekView, Some(CAL)),
        KeyBinding::new("m", ShowMonthView, Some(CAL)),
        KeyBinding::new("t", GoToToday, Some(CAL)),
        KeyBinding::new("n", NextPeriod, Some(CAL)),
        KeyBinding::new("j", NextPeriod, Some(CAL)),
        KeyBinding::new("p", PrevPeriod, Some(CAL)),
        KeyBinding::new("k", PrevPeriod, Some(CAL)),
        KeyBinding::new("c", NewEvent, Some(CAL)),
        KeyBinding::new("enter", OpenEvent, Some(CAL)),
        KeyBinding::new("e", OpenEvent, Some(CAL)),
        KeyBinding::new("delete", DeleteEvent, Some(CAL)),
        KeyBinding::new("backspace", DeleteEvent, Some(CAL)),
        KeyBinding::new("tab", SelectNextEvent, Some(CAL)),
        KeyBinding::new("right", SelectNextEvent, Some(CAL)),
        KeyBinding::new("down", SelectNextEvent, Some(CAL)),
        KeyBinding::new("shift-tab", SelectPrevEvent, Some(CAL)),
        KeyBinding::new("left", SelectPrevEvent, Some(CAL)),
        KeyBinding::new("up", SelectPrevEvent, Some(CAL)),
        // Inputs pass escape on, so this closes the panel from them too.
        KeyBinding::new("escape", CloseEvent, Some(CALENDAR)),
        KeyBinding::new("cmd-z", Undo, Some(CAL)),
        KeyBinding::new("?", ToggleShortcutsHelp, Some(CAL)),
        KeyBinding::new("tab", event_panel::NextField, Some(EVENT_PANEL)),
        KeyBinding::new("shift-tab", event_panel::PrevField, Some(EVENT_PANEL)),
        KeyBinding::new("down", event_panel::StepDown, Some(DATE_INPUT)),
        KeyBinding::new("up", event_panel::StepUp, Some(DATE_INPUT)),
        KeyBinding::new("down", event_panel::StepDown, Some(TIME_INPUT)),
        KeyBinding::new("up", event_panel::StepUp, Some(TIME_INPUT)),
        KeyBinding::new("enter", event_panel::Activate, Some(PANEL_CONTROL)),
        KeyBinding::new("space", event_panel::Activate, Some(PANEL_CONTROL)),
        KeyBinding::new("down", event_panel::MenuNext, Some(PANEL_CONTROL)),
        KeyBinding::new("up", event_panel::MenuPrev, Some(PANEL_CONTROL)),
        KeyBinding::new("down", composer::SuggestNext, Some(GUESTS_INPUT)),
        KeyBinding::new("up", composer::SuggestPrev, Some(GUESTS_INPUT)),
        KeyBinding::new(
            "backspace",
            composer::RemoveLastRecipient,
            Some(GUESTS_INPUT),
        ),
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

/// Flattened contexts to look shortcuts up in. Mail: the list, the reader, a
/// draft and the search field. Calendar: the grid and the event panel.
pub fn lookup_contexts(scope: Scope, reader_first: bool) -> Vec<KeyContext> {
    let with = |mut context: KeyContext, extra: &str| {
        context.add(extra);
        context
    };
    match scope {
        Scope::Calendar => {
            let mut calendar = KeyContext::default();
            calendar.add(CONTEXT);
            calendar.add(CALENDAR);
            vec![calendar.clone(), with(calendar, EVENT_PANEL)]
        }
        Scope::Mail | Scope::Global => vec![
            mailbox_context(reader_first),
            mailbox_context(!reader_first),
            with(mailbox_context(false), COMPOSER),
            with(mailbox_context(false), SEARCH),
        ],
    }
}

/// Which app a command belongs to. The palette and the shortcuts sheet show
/// the current app's commands and the global ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Mail,
    Calendar,
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Navigation,
    Conversation,
    Invite,
    Compose,
    GoTo,
    App,
    CalendarNavigation,
    CalendarView,
    CalendarEvents,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Navigation => "Navigation",
            Group::Conversation => "Conversation",
            Group::Invite => "Invitations",
            Group::Compose => "Compose",
            Group::GoTo => "Go to",
            Group::App => "Application",
            Group::CalendarNavigation => "Navigation",
            Group::CalendarView => "View",
            Group::CalendarEvents => "Events",
        }
    }

    pub fn scope(self) -> Scope {
        match self {
            Group::App => Scope::Global,
            Group::CalendarNavigation | Group::CalendarView | Group::CalendarEvents => {
                Scope::Calendar
            }
            _ => Scope::Mail,
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
    Compose: "Send", Send, SendMessage;
    Compose: "Show Cc and Bcc", Mail, ShowCcBcc;
    Compose: "Close Draft (Saves to Drafts)", FilePen, CloseDraft;
    Compose: "Discard Draft", Trash, DiscardDraft;
    Compose: "Undo Send / Discard", Undo, Undo;
    Conversation: "Archive", Archive, Archive;
    Conversation: "Delete", Trash, Delete;
    Conversation: "Star / Unstar", Star, ToggleStar;
    Conversation: "Mark as Unread", MailUnread, MarkUnread;
    Conversation: "Mark as Read", MailOpen, MarkRead;
    Invite: "Accept Invitation (Yes)", Check, AcceptInvite;
    Invite: "Maybe to Invitation", Help, MaybeInvite;
    Invite: "Decline Invitation (No)", X, DeclineInvite;
    Invite: "Open Invitation in Calendar", Calendar, OpenInviteInCalendar;
    Invite: "Remove Cancelled Event from Calendar", Trash, RemoveCancelledEvent;
    Navigation: "Next Conversation", ChevronDown, SelectNextThread;
    Navigation: "Previous Conversation", ChevronUp, SelectPrevThread;
    Navigation: "Open Conversation", Mail, OpenThread;
    Navigation: "Back to List", Inbox, BackToList;
    Navigation: "Next Message in Conversation", ChevronDown, NextMessage;
    Navigation: "Previous Message in Conversation", ChevronUp, PrevMessage;
    Navigation: "Scroll Conversation Down", ChevronDown, ScrollReaderDown, help_only;
    Navigation: "Scroll Conversation Up", ChevronUp, ScrollReaderUp, help_only;
    Navigation: "Search Mail", Search, ToggleSearch;
    Navigation: "Clear Search", X, CloseSearch;
    GoTo: "Go to Inbox", Inbox, GoToInbox;
    GoTo: "Go to Starred", Star, GoToStarred;
    GoTo: "Go to Sent", Send, GoToSent;
    GoTo: "Go to Drafts", FilePen, GoToDrafts;
    GoTo: "Go to Archive", Archive, GoToArchive;
    GoTo: "Go to Spam", OctagonAlert, GoToSpam;
    GoTo: "Go to Trash", Trash, GoToTrash;
    GoTo: "Go to Label…", ChevronRight, GoToLabel;
    Navigation: "Toggle Sidebar", PanelLeft, ToggleSidebar;
    Navigation: "Show / Hide Archived in Inbox", Archive, ToggleArchivedSection;
    CalendarEvents: "New Event", Plus, NewEvent;
    CalendarEvents: "Open Event", Calendar, OpenEvent;
    CalendarEvents: "Delete Event", Trash, DeleteEvent;
    CalendarEvents: "Close Event", X, CloseEvent;
    CalendarEvents: "Undo Delete", Undo, Undo;
    CalendarNavigation: "Today", Calendar, GoToToday;
    CalendarNavigation: "Next Day / Week / Month", ChevronRight, NextPeriod;
    CalendarNavigation: "Previous Day / Week / Month", ChevronLeft, PrevPeriod;
    CalendarNavigation: "Next Event", ChevronDown, SelectNextEvent;
    CalendarNavigation: "Previous Event", ChevronUp, SelectPrevEvent;
    CalendarView: "Day View", Calendar, ShowDayView;
    CalendarView: "Week View", Calendar, ShowWeekView;
    CalendarView: "Month View", Calendar, ShowMonthView;
    CalendarView: "Show / Hide Calendars…", Layers, ToggleCalendarsMenu;
    App: "Go to Mail", Mail, GoToMail;
    App: "Go to Calendar", Calendar, GoToCalendar;
    App: "Command Palette", Command, ToggleCommandPalette, help_only;
    App: "Keyboard Shortcuts", Keyboard, ToggleShortcutsHelp;
    App: "Quit Pigeon", Command, Quit;
};

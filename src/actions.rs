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
//! - `Settings`: the settings page (⌘,), in place of `Mailbox` or
//!   `Calendar` while it shows. Its search field adds `SettingsSearch`.
//! - `ContactCard` (W6): the contact card overlay, a sibling of `Mailbox`
//!   like the palette.
//! - `Input`: gpui-component's text inputs. Single-letter shortcuts also
//!   require `!Input`, so typing never triggers them.

use gpui::{Action, App, KeyBinding, KeyContext, actions};

use crate::{
    assets::Icon,
    search,
    ui::{
        calendar::{self, GoToDate, NextDay, PrevDay, ToggleWeekNumbers, event_panel},
        command_palette,
        composer::{
            self, BulletedList, ClearFormatting, FormatBold, FormatCode, FormatItalic,
            FormatStrikethrough, FormatUnderline, InsertLink, NumberedList, QuoteLines,
        },
        contact_card, menu_bar, settings as settings_page,
    },
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
        SelectMessageText,
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
        ZoomIn,
        ZoomOut,
        ResetZoom,
        FocusNextControl,
        FocusPrevControl,
        FocusPanes,
        About,
        OpenSettings,
        Hide,
        HideOthers,
        ShowAll,
        CloseWindow,
        Minimize,
        ZoomWindow,
        // Mail: selecting several threads, moving and labelling them.
        ToggleSelected,
        ExtendSelectionDown,
        ExtendSelectionUp,
        ClearSelection,
        MoveTo,
        ApplyLabel,
        ToggleDensity,
        // The account and the backend (docs/BACKEND_SPRINT_1.md, 2.11).
        SignIn,
        SignOut,
        SyncNow,
        RetryLoadingMessage,
        LoadImages,
        LoadOlderConversations,
        UseDemoData,
        // W1: the app shell and notifications (sprint 2).
        ShowMainWindow,
        SendTestNotification,
        ToggleNotifications,
        ToggleOpenAtLogin,
        // W2: attachments and the original message.
        OpenAttachment,
        SaveAttachment,
        ShowOriginal,
        SaveOriginal,
        CopyOriginal,
        CloseOriginal,
        // W5: calendar (docs/BACKEND_SPRINT_2.md).
        AddGoogleMeet,
        SetRepeat,
        AddReminder,
        NewEventFromText,
        // W7b: the menu bar.
        OpenMenuBarPanel,
        JoinNextMeeting,
        // W4b: Inbox categories, labels and Important.
        GoToSocial,
        GoToPromotions,
        GoToUpdates,
        GoToForums,
        ToggleImportant,
        NewLabel,
        RenameLabel,
        ChangeLabelColor,
        DeleteLabel,
        ChangeLabelVisibility,
        ToggleLabelFolded,
        ToggleHiddenLabels,
        // W3: compose and identity.
        ChangeFrom,
        ToggleSignature,
        MakeDefaultMailApp,
        // W6: the contact card.
        ShowContact,
    ]
);

/// W4b (FOL-10): where the sidebar lists the open label; from its context
/// menu.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct SetLabelListVisibility(pub crate::data::ListVisibility);

/// W4b (FOL-10): whether rows show the open label.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct SetLabelMessageVisibility(pub bool);

/// W2: open one attachment of the selected conversation, by message and
/// attachment index; from its chip and the picker.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct OpenAttachmentAt(pub usize, pub usize);

/// W2: save one attachment, as `OpenAttachmentAt`.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct SaveAttachmentAt(pub usize, pub usize);

/// Move the selected threads to a folder; from the context menu and the
/// toolbar's "…" menu.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct MoveToFolder(pub crate::data::Folder);

/// Add a label (by index into `MailData::labels`) to the selected threads,
/// or remove it when they all have it.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct ToggleLabel(pub usize);

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
/// W3: the composer's From button and its menu.
pub const FROM_PICKER: &str = "FromPicker";
/// A focusable button or row outside the event panel (see
/// `ui::style::focusable_control`): enter and space press it.
pub const CONTROL: &str = "Control";
/// W2: the original message's source, in the reading pane.
pub const ORIGINAL: &str = "Original";

/// The keymap. Where an action has several bindings, the one added last is
/// the one the palette shows.
pub fn bind_keys(cx: &mut App) {
    const MAIL: &str = "Mailbox && !Input";
    const LIST: &str = "Mailbox && !Reader && !Input && !Control";
    const TABBING: &str = "Mailbox && !Input && !Composer";
    const READING: &str = "Mailbox && Reader && !Input";
    const PALETTE_INPUT: &str = "CommandPalette > Input";
    const COMPOSER_INPUT: &str = "Composer > Input";
    const RECIPIENTS_INPUT: &str = "Recipients > Input";
    const SEARCH_INPUT: &str = "SearchBar > Input";
    const CAL: &str = "Calendar && !Input";
    const DATE_INPUT: &str = "DateField > Input";
    const TIME_INPUT: &str = "TimeField > Input";
    const GUESTS_INPUT: &str = "Guests > Input";
    const SET: &str = "Settings && !Input";
    const SET_INPUT: &str = "SettingsSearch > Input";
    const MENU_BAR: &str = menu_bar::MENU_BAR;
    const CARD: &str = contact_card::CONTACT_CARD;

    cx.bind_keys([
        // Menu-bar shortcuts first: the menu shows an action's first binding.
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd-+", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd--", ZoomOut, Some(CONTEXT)),
        KeyBinding::new("cmd-0", ResetZoom, Some(CONTEXT)),
        KeyBinding::new("cmd-n", Compose, Some(MAILBOX)),
        KeyBinding::new("cmd-n", NewEvent, Some(CALENDAR)),
        KeyBinding::new("cmd-f", ToggleSearch, Some(MAILBOX)),
        KeyBinding::new("tab", FocusNextControl, Some(TABBING)),
        KeyBinding::new("shift-tab", FocusPrevControl, Some(TABBING)),
        KeyBinding::new("escape", FocusPanes, Some(CONTROL)),
        KeyBinding::new("cmd-k", ToggleCommandPalette, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-p", ToggleCommandPalette, Some(CONTEXT)),
        KeyBinding::new("cmd-\\", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("cmd-1", GoToMail, Some(CONTEXT)),
        KeyBinding::new("cmd-2", GoToCalendar, Some(CONTEXT)),
        KeyBinding::new("cmd-r", SyncNow, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-i", LoadImages, Some(MAILBOX)),
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
        KeyBinding::new("cmd-a", SelectMessageText, Some(READING)),
        // Thread actions.
        KeyBinding::new("e", Archive, Some(MAIL)),
        KeyBinding::new("#", Delete, Some(MAIL)),
        KeyBinding::new("delete", Delete, Some(MAIL)),
        KeyBinding::new("backspace", Delete, Some(MAIL)),
        KeyBinding::new("v", MoveTo, Some(MAIL)),
        KeyBinding::new("l", ApplyLabel, Some(MAIL)),
        // Selecting several threads, like Gmail.
        KeyBinding::new("x", ToggleSelected, Some(MAIL)),
        KeyBinding::new("shift-down", ExtendSelectionDown, Some(LIST)),
        KeyBinding::new("shift-up", ExtendSelectionUp, Some(LIST)),
        KeyBinding::new("shift-j", ExtendSelectionDown, Some(MAIL)),
        KeyBinding::new("shift-k", ExtendSelectionUp, Some(MAIL)),
        KeyBinding::new("shift-e", ToggleArchivedSection, Some(MAIL)),
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
        // Bound after, so it wins: escape clears a selection first.
        KeyBinding::new("escape", ClearSelection, Some(LIST)),
        KeyBinding::new("escape", CloseSearch, Some(SEARCH)),
        KeyBinding::new("down", search::FocusResults, Some(SEARCH_INPUT)),
        // While the suggestions show, the arrows move through them.
        KeyBinding::new("down", search::NextSuggestion, Some(SEARCH_INPUT)),
        KeyBinding::new("up", search::PrevSuggestion, Some(SEARCH_INPUT)),
        // Compose. Inputs take enter and escape themselves, so these also bind
        // inside them, where they win.
        KeyBinding::new("cmd-enter", SendMessage, Some(COMPOSER)),
        KeyBinding::new("cmd-enter", SendMessage, Some(COMPOSER_INPUT)),
        KeyBinding::new("cmd-shift-c", ShowCcBcc, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-d", DiscardDraft, Some(COMPOSER)),
        KeyBinding::new("escape", CloseDraft, Some(COMPOSER)),
        KeyBinding::new("z", Undo, Some(MAIL)),
        KeyBinding::new("cmd-z", Undo, Some(MAIL)),
        KeyBinding::new("tab", composer::NextField, Some(COMPOSER)),
        KeyBinding::new("shift-tab", composer::PrevField, Some(COMPOSER)),
        // Formatting the body, like Gmail, except links: ⌘⇧K, so ⌘K is the
        // palette everywhere. The composer passes them on from its other
        // fields, so there ⌘\ is still the sidebar. On a US layout ⌘⇧7
        // arrives as ⌘&, and so on.
        KeyBinding::new("cmd-b", composer::FormatBold, Some(COMPOSER)),
        KeyBinding::new("cmd-i", composer::FormatItalic, Some(COMPOSER)),
        KeyBinding::new("cmd-u", composer::FormatUnderline, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-x", composer::FormatStrikethrough, Some(COMPOSER)),
        KeyBinding::new("cmd-e", composer::FormatCode, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-k", composer::InsertLink, Some(COMPOSER)),
        KeyBinding::new("cmd-&", composer::NumberedList, Some(COMPOSER)),
        KeyBinding::new("cmd-*", composer::BulletedList, Some(COMPOSER)),
        KeyBinding::new("cmd-(", composer::QuoteLines, Some(COMPOSER)),
        KeyBinding::new("cmd-\\", composer::ClearFormatting, Some(COMPOSER)),
        // The editor takes these itself, so they bind inside it, where they
        // win; the composer passes them on outside lists.
        KeyBinding::new("enter", composer::BodyEnter, Some(COMPOSER_INPUT)),
        KeyBinding::new("tab", composer::BodyIndent, Some(COMPOSER_INPUT)),
        KeyBinding::new("shift-tab", composer::BodyOutdent, Some(COMPOSER_INPUT)),
        KeyBinding::new("pageup", composer::BodyPageUp, Some(COMPOSER_INPUT)),
        KeyBinding::new("pagedown", composer::BodyPageDown, Some(COMPOSER_INPUT)),
        KeyBinding::new(
            "shift-pageup",
            composer::BodySelectPageUp,
            Some(COMPOSER_INPUT),
        ),
        KeyBinding::new(
            "shift-pagedown",
            composer::BodySelectPageDown,
            Some(COMPOSER_INPUT),
        ),
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
        // Between the apps, from either one.
        KeyBinding::new("g m", GoToMail, Some(MAIL)),
        KeyBinding::new("g c", GoToCalendar, Some(MAIL)),
        KeyBinding::new("g m", GoToMail, Some(CAL)),
        KeyBinding::new("g c", GoToCalendar, Some(CAL)),
        // Calendar, like Google Calendar.
        KeyBinding::new("d", ShowDayView, Some(CAL)),
        KeyBinding::new("w", ShowWeekView, Some(CAL)),
        KeyBinding::new("m", ShowMonthView, Some(CAL)),
        KeyBinding::new("t", GoToToday, Some(CAL)),
        KeyBinding::new("n", NextPeriod, Some(CAL)),
        KeyBinding::new("p", PrevPeriod, Some(CAL)),
        KeyBinding::new("right", calendar::NextDay, Some(CAL)),
        KeyBinding::new("left", calendar::PrevDay, Some(CAL)),
        KeyBinding::new("cmd-shift-t", calendar::GoToDate, Some(CALENDAR)),
        KeyBinding::new("g d", calendar::GoToDate, Some(CAL)),
        KeyBinding::new("c", NewEvent, Some(CAL)),
        // W5.
        KeyBinding::new("shift-c", NewEventFromText, Some(CAL)),
        KeyBinding::new("shift-g", AddGoogleMeet, Some(CAL)),
        KeyBinding::new("shift-r", SetRepeat, Some(CAL)),
        KeyBinding::new("shift-a", AddReminder, Some(CAL)),
        KeyBinding::new("enter", OpenEvent, Some(CAL)),
        KeyBinding::new("e", OpenEvent, Some(CAL)),
        KeyBinding::new("delete", DeleteEvent, Some(CAL)),
        KeyBinding::new("backspace", DeleteEvent, Some(CAL)),
        // Down and up go by event, like j and k in Mail; tab too.
        KeyBinding::new("tab", SelectNextEvent, Some(CAL)),
        KeyBinding::new("down", SelectNextEvent, Some(CAL)),
        KeyBinding::new("j", SelectNextEvent, Some(CAL)),
        KeyBinding::new("shift-tab", SelectPrevEvent, Some(CAL)),
        KeyBinding::new("up", SelectPrevEvent, Some(CAL)),
        KeyBinding::new("k", SelectPrevEvent, Some(CAL)),
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
        // The settings page.
        KeyBinding::new("down", settings_page::SelectNext, Some(SET)),
        KeyBinding::new("j", settings_page::SelectNext, Some(SET)),
        KeyBinding::new("up", settings_page::SelectPrev, Some(SET)),
        KeyBinding::new("k", settings_page::SelectPrev, Some(SET)),
        KeyBinding::new("tab", settings_page::EnterRows, Some(SET)),
        KeyBinding::new("shift-tab", settings_page::LeaveRows, Some(SET)),
        KeyBinding::new("enter", settings_page::Activate, Some(SET)),
        KeyBinding::new("space", settings_page::Activate, Some(SET)),
        KeyBinding::new("right", settings_page::Increase, Some(SET)),
        KeyBinding::new("left", settings_page::Decrease, Some(SET)),
        KeyBinding::new("/", settings_page::FocusSearch, Some(SET)),
        KeyBinding::new(
            "cmd-f",
            settings_page::FocusSearch,
            Some(settings_page::SETTINGS),
        ),
        KeyBinding::new("?", ToggleShortcutsHelp, Some(SET)),
        // Inputs pass escape on, so this clears the search from it too.
        KeyBinding::new(
            "escape",
            settings_page::Close,
            Some(settings_page::SETTINGS),
        ),
        KeyBinding::new("down", settings_page::SelectNext, Some(SET_INPUT)),
        KeyBinding::new("up", settings_page::SelectPrev, Some(SET_INPUT)),
        KeyBinding::new("tab", settings_page::EnterRows, Some(SET_INPUT)),
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
        // W1: works with no window open too.
        KeyBinding::new("cmd-o", ShowMainWindow, None),
        // W2: attachments and the original message.
        KeyBinding::new("shift-o", OpenAttachment, Some(MAIL)),
        KeyBinding::new("cmd-shift-s", SaveAttachment, Some(MAILBOX)),
        KeyBinding::new("cmd-alt-u", ShowOriginal, Some(MAILBOX)),
        KeyBinding::new("cmd-alt-s", SaveOriginal, Some(MAILBOX)),
        KeyBinding::new("cmd-c", CopyOriginal, Some(ORIGINAL)),
        KeyBinding::new("escape", CloseOriginal, Some(ORIGINAL)),
        // W7b: the menu bar panel.
        KeyBinding::new("down", menu_bar::SelectNext, Some(MENU_BAR)),
        KeyBinding::new("up", menu_bar::SelectPrev, Some(MENU_BAR)),
        KeyBinding::new("ctrl-n", menu_bar::SelectNext, Some(MENU_BAR)),
        KeyBinding::new("ctrl-p", menu_bar::SelectPrev, Some(MENU_BAR)),
        KeyBinding::new("enter", menu_bar::OpenSelected, Some(MENU_BAR)),
        KeyBinding::new("j", menu_bar::Join, Some(MENU_BAR)),
        KeyBinding::new("e", menu_bar::Archive, Some(MENU_BAR)),
        KeyBinding::new("shift-i", menu_bar::MarkRead, Some(MENU_BAR)),
        KeyBinding::new("z", menu_bar::Undo, Some(MENU_BAR)),
        KeyBinding::new("cmd-z", menu_bar::Undo, Some(MENU_BAR)),
        KeyBinding::new("c", menu_bar::Compose, Some(MENU_BAR)),
        KeyBinding::new("n", menu_bar::NewEvent, Some(MENU_BAR)),
        KeyBinding::new("cmd-o", menu_bar::OpenPigeon, Some(MENU_BAR)),
        KeyBinding::new("escape", menu_bar::Close, Some(MENU_BAR)),
        KeyBinding::new("cmd-w", menu_bar::Close, Some(MENU_BAR)),
        // W4b: the Inbox's other categories, Important (Gmail's `+` and
        // `=`), and labels under `L`.
        KeyBinding::new("g o", GoToSocial, Some(MAIL)),
        KeyBinding::new("g p", GoToPromotions, Some(MAIL)),
        KeyBinding::new("g u", GoToUpdates, Some(MAIL)),
        KeyBinding::new("g f", GoToForums, Some(MAIL)),
        KeyBinding::new("+", ToggleImportant, Some(MAIL)),
        KeyBinding::new("=", ToggleImportant, Some(MAIL)),
        KeyBinding::new("shift-l n", NewLabel, Some(MAIL)),
        KeyBinding::new("shift-l r", RenameLabel, Some(MAIL)),
        KeyBinding::new("shift-l c", ChangeLabelColor, Some(MAIL)),
        KeyBinding::new("shift-l d", DeleteLabel, Some(MAIL)),
        KeyBinding::new("shift-l v", ChangeLabelVisibility, Some(MAIL)),
        KeyBinding::new("shift-l f", ToggleLabelFolded, Some(MAIL)),
        KeyBinding::new("shift-l h", ToggleHiddenLabels, Some(MAIL)),
        // W3: From and the signature.
        KeyBinding::new("cmd-shift-f", ChangeFrom, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-e", ToggleSignature, Some(COMPOSER)),
        KeyBinding::new("enter", composer::FromActivate, Some(FROM_PICKER)),
        KeyBinding::new("space", composer::FromActivate, Some(FROM_PICKER)),
        KeyBinding::new("down", composer::FromNext, Some(FROM_PICKER)),
        KeyBinding::new("up", composer::FromPrev, Some(FROM_PICKER)),
        KeyBinding::new("escape", composer::FromDismiss, Some(FROM_PICKER)),
        // W6: the contact card.
        KeyBinding::new("shift-p", ShowContact, Some(MAIL)),
        KeyBinding::new("down", contact_card::SelectNext, Some(CARD)),
        KeyBinding::new("j", contact_card::SelectNext, Some(CARD)),
        KeyBinding::new("up", contact_card::SelectPrev, Some(CARD)),
        KeyBinding::new("k", contact_card::SelectPrev, Some(CARD)),
        KeyBinding::new("enter", contact_card::Confirm, Some(CARD)),
        KeyBinding::new("c", contact_card::ComposeTo, Some(CARD)),
        KeyBinding::new("cmd-c", contact_card::CopyAddress, Some(CARD)),
        KeyBinding::new("escape", contact_card::Dismiss, Some(CARD)),
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
            with(mailbox_context(false), ORIGINAL),
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
    Selection,
    Invite,
    Compose,
    GoTo,
    App,
    CalendarNavigation,
    CalendarView,
    CalendarEvents,
    /// W4b.
    Labels,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Navigation => "Navigation",
            Group::Conversation => "Conversation",
            Group::Selection => "Selection",
            Group::Invite => "Invitations",
            Group::Compose => "Compose",
            Group::GoTo => "Go to",
            Group::App => "Application",
            Group::CalendarNavigation => "Navigation",
            Group::CalendarView => "View",
            Group::CalendarEvents => "Events",
            Group::Labels => "Labels",
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
    /// Offered in the palette in Mail too, though its group is Calendar's.
    pub global: bool,
    /// When the palette offers it (`PigeonApp::command_available`).
    pub when: When,
}

impl Command {
    /// Where the palette offers it.
    pub fn scope(&self) -> Scope {
        if self.global {
            Scope::Global
        } else {
            self.group.scope()
        }
    }
}

/// The state a command needs to do anything; the palette leaves out the
/// ones that can't run now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Always,
    /// A conversation is selected (or several are).
    Thread,
    /// A conversation that can be replied to is selected.
    Replyable,
    /// The selected conversation has more than one message.
    Messages,
    /// A draft is open.
    Draft,
    /// The toast offers an undo.
    Undo,
    /// The selected conversation has an invitation to answer.
    Invite,
    /// The selected conversation has a cancelled event to remove.
    Cancelled,
    /// The search bar is open.
    Searching,
    /// Several conversations are selected.
    Selection,
    /// The Inbox, with its archived section, is showing.
    Inbox,
    /// W4b: the user has labels.
    Labels,
    // W3.
    /// A draft is open and there's more than one address to send from.
    FromChoice,
}

macro_rules! commands {
    ($($group:ident: $name:literal, $icon:ident, $action:ident $(, $flag:ident)? $(if $when:ident)?;)*) => {
        &[$(Command {
            name: $name,
            group: Group::$group,
            icon: Icon::$icon,
            action: || Box::new($action),
            help_only: commands!(@help $($flag)?),
            global: commands!(@global $($flag)?),
            when: commands!(@when $($when)?),
        }),*]
    };
    (@help) => { false };
    (@help help_only) => { true };
    (@help global) => { false };
    (@global) => { false };
    (@global global) => { true };
    (@global help_only) => { false };
    (@when) => { When::Always };
    (@when $when:ident) => { When::$when };
}

/// Every command, in palette order.
pub const COMMANDS: &[Command] = commands! {
    Compose: "Compose New Message", SquarePen, Compose;
    Compose: "Reply", Reply, Reply if Replyable;
    Compose: "Reply All", ReplyAll, ReplyAll if Replyable;
    Compose: "Forward", Forward, Forward if Replyable;
    Compose: "Send", Send, SendMessage if Draft;
    Compose: "Show Cc and Bcc", Mail, ShowCcBcc if Draft;
    Compose: "Close Draft (Saves to Drafts)", FilePen, CloseDraft if Draft;
    Compose: "Discard Draft", Trash, DiscardDraft if Draft;
    Compose: "Bold", Bold, FormatBold if Draft;
    Compose: "Italic", Italic, FormatItalic if Draft;
    Compose: "Underline", Underline, FormatUnderline if Draft;
    Compose: "Strikethrough", Strikethrough, FormatStrikethrough if Draft;
    Compose: "Code", Code, FormatCode if Draft;
    Compose: "Insert Link", Link, InsertLink if Draft;
    Compose: "Bulleted List", List, BulletedList if Draft;
    Compose: "Numbered List", ListOrdered, NumberedList if Draft;
    Compose: "Quote", TextQuote, QuoteLines if Draft;
    Compose: "Clear Formatting", RemoveFormatting, ClearFormatting if Draft;
    Conversation: "Undo", Undo, Undo if Undo;
    Conversation: "Archive", Archive, Archive if Thread;
    Conversation: "Delete", Trash, Delete if Thread;
    Conversation: "Star / Unstar", Star, ToggleStar if Thread;
    Conversation: "Mark as Unread", MailUnread, MarkUnread if Thread;
    Conversation: "Mark as Read", MailOpen, MarkRead if Thread;
    Conversation: "Move to…", FolderInput, MoveTo if Thread;
    Conversation: "Label…", Tag, ApplyLabel if Thread;
    Selection: "Select / Deselect Conversation", Check, ToggleSelected if Thread;
    Selection: "Extend Selection Down", ChevronDown, ExtendSelectionDown if Thread;
    Selection: "Extend Selection Up", ChevronUp, ExtendSelectionUp if Thread;
    Selection: "Clear Selection", X, ClearSelection if Selection;
    Invite: "Answer Yes", Check, AcceptInvite if Invite;
    Invite: "Answer Maybe", Help, MaybeInvite if Invite;
    Invite: "Answer No", X, DeclineInvite if Invite;
    Invite: "Open Invitation in Calendar", Calendar, OpenInviteInCalendar if Invite;
    Invite: "Remove Cancelled Event from Calendar", Trash, RemoveCancelledEvent if Cancelled;
    Navigation: "Next Conversation", ChevronDown, SelectNextThread;
    Navigation: "Previous Conversation", ChevronUp, SelectPrevThread;
    Navigation: "Open Conversation", Mail, OpenThread;
    Navigation: "Back to List", Inbox, BackToList if Thread;
    Navigation: "Next Message in Conversation", ChevronDown, NextMessage if Messages;
    Navigation: "Previous Message in Conversation", ChevronUp, PrevMessage if Messages;
    Conversation: "Select Message Text", Text, SelectMessageText if Thread;
    Conversation: "Retry Loading Message", Repeat, RetryLoadingMessage if Thread;
    Conversation: "Load Images", Image, LoadImages if Thread;
    Navigation: "Scroll Conversation Down", ChevronDown, ScrollReaderDown, help_only;
    Navigation: "Scroll Conversation Up", ChevronUp, ScrollReaderUp, help_only;
    Navigation: "Search Mail", Search, ToggleSearch;
    Navigation: "Clear Search", X, CloseSearch if Searching;
    Navigation: "Compact / Comfortable List", Rows, ToggleDensity;
    Navigation: "Load Older Conversations", ChevronDown, LoadOlderConversations;
    GoTo: "Go to Inbox", Inbox, GoToInbox;
    GoTo: "Go to Starred", Star, GoToStarred;
    GoTo: "Go to Sent", Send, GoToSent;
    GoTo: "Go to Drafts", FilePen, GoToDrafts;
    GoTo: "Go to Archive", Archive, GoToArchive;
    GoTo: "Go to Spam", OctagonAlert, GoToSpam;
    GoTo: "Go to Trash", Trash, GoToTrash;
    GoTo: "Go to Label…", ChevronRight, GoToLabel;
    Navigation: "Toggle Sidebar", PanelLeft, ToggleSidebar;
    Navigation: "Show / Hide Archived in Inbox", Archive, ToggleArchivedSection if Inbox;
    // From Mail too: the app switches to Calendar (`Show::NewEvent`).
    CalendarEvents: "New Event", Plus, NewEvent, global;
    // W5.
    CalendarEvents: "New Event from Text…", Plus, NewEventFromText, global;
    CalendarEvents: "Add Google Meet", Video, AddGoogleMeet;
    CalendarEvents: "Repeat…", Repeat, SetRepeat;
    CalendarEvents: "Add Reminder…", Bell, AddReminder;
    CalendarEvents: "Open Event", Calendar, OpenEvent;
    CalendarEvents: "Delete Event", Trash, DeleteEvent;
    CalendarEvents: "Close Event", X, CloseEvent;
    CalendarEvents: "Undo Delete", Undo, Undo;
    CalendarNavigation: "Today", Calendar, GoToToday;
    CalendarNavigation: "Go to Date…", Calendar, GoToDate;
    CalendarNavigation: "Next Day", ChevronRight, NextDay;
    CalendarNavigation: "Previous Day", ChevronLeft, PrevDay;
    CalendarNavigation: "Next Event", ChevronDown, SelectNextEvent;
    CalendarNavigation: "Previous Event", ChevronUp, SelectPrevEvent;
    CalendarNavigation: "Next Period (Day / Week / Month)", ChevronRight, NextPeriod;
    CalendarNavigation: "Previous Period (Day / Week / Month)", ChevronLeft, PrevPeriod;
    CalendarView: "Day View", Calendar, ShowDayView;
    CalendarView: "Week View", Calendar, ShowWeekView;
    CalendarView: "Month View", Calendar, ShowMonthView;
    CalendarView: "Show / Hide Calendars…", Layers, ToggleCalendarsMenu;
    CalendarView: "Show / Hide Week Numbers", Calendar, ToggleWeekNumbers;
    App: "Go to Mail", Mail, GoToMail;
    App: "Go to Calendar", Calendar, GoToCalendar;
    App: "Command Palette", Command, ToggleCommandPalette, help_only;
    App: "Keyboard Shortcuts", Keyboard, ToggleShortcutsHelp;
    App: "Settings…", Settings, OpenSettings;
    App: "Sync Now", Repeat, SyncNow;
    App: "Sign In with Google", User, SignIn;
    App: "Sign Out", User, SignOut;
    App: "Use Demo Data", Layers, UseDemoData;
    App: "Quit Pigeon", Command, Quit;
    // W1
    App: "Open Pigeon Window", Mail, ShowMainWindow, help_only;
    App: "Send Test Notification", Bell, SendTestNotification;
    App: "Turn New Mail Notifications On / Off", Bell, ToggleNotifications;
    App: "Turn Open at Login On / Off", Settings, ToggleOpenAtLogin;
    // W2: attachments and the original message.
    Conversation: "Open Attachment", Paperclip, OpenAttachment if Thread;
    Conversation: "Save Attachment…", Download, SaveAttachment if Thread;
    Conversation: "Show Original", Code, ShowOriginal if Thread;
    Conversation: "Save Message as .eml…", Download, SaveOriginal if Thread;
    Conversation: "Copy Original", Code, CopyOriginal, help_only;
    Conversation: "Close Original", X, CloseOriginal, help_only;
    // W7b: the menu bar.
    App: "Open Menu Bar Panel", Calendar, OpenMenuBarPanel;
    App: "Join Next Meeting", Video, JoinNextMeeting;
    // W4b: Inbox categories, labels and Important.
    GoTo: "Go to Social", Users, GoToSocial;
    GoTo: "Go to Promotions", Tag, GoToPromotions;
    GoTo: "Go to Updates", Bell, GoToUpdates;
    GoTo: "Go to Forums", Layers, GoToForums;
    Conversation: "Mark as Important / Not Important", Important, ToggleImportant if Thread;
    Labels: "New Label…", Plus, NewLabel;
    Labels: "Rename Label…", Tag, RenameLabel if Labels;
    Labels: "Change Label Color…", Tag, ChangeLabelColor if Labels;
    Labels: "Delete Label…", Trash, DeleteLabel if Labels;
    Labels: "Label Visibility…", Tag, ChangeLabelVisibility if Labels;
    Labels: "Fold / Unfold Sublabels", ChevronRight, ToggleLabelFolded if Labels;
    Labels: "Show / Hide Hidden Labels", Tag, ToggleHiddenLabels if Labels;
    // W3: compose and identity.
    Compose: "Change From…", User, ChangeFrom if FromChoice;
    Compose: "Add / Remove Signature", SquarePen, ToggleSignature if Draft;
    App: "Make Pigeon the Default Mail App", Mail, MakeDefaultMailApp;
    // W6: the contact card.
    Conversation: "Show Contact", User, ShowContact if Thread;
};

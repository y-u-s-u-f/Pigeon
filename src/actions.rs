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
//!   like the palette. `LaterPicker`: the Snooze and Send Later time
//!   picker, likewise.
//! - `Input`: gpui-component's text inputs. Single-letter shortcuts also
//!   require `!Input`, so typing never triggers them.

use gpui::{Action, App, KeyBinding, KeyContext, actions};

use crate::{
    assets::Icon,
    search,
    settings::{KeymapPreset, Section},
    ui::{
        calendar::{self, GoToDate, NextDay, PrevDay, ToggleWeekNumbers, event_panel},
        command_palette,
        composer::{
            self, BulletedList, ClearFormatting, FormatBold, FormatCode, FormatItalic,
            FormatStrikethrough, FormatUnderline, InsertLink, NumberedList, QuoteLines,
        },
        contact_card, later_picker, menu_bar, onboarding, settings as settings_page,
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
        SaveEvent,
        DiscardEventChanges,
        ToggleCalendarsMenu,
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
        MoveLabelUp,
        MoveLabelDown,
        // W3: compose and identity.
        ChangeFrom,
        ToggleSignature,
        MakeDefaultMailApp,
        // W6: the contact card.
        ShowContact,
        // Polish sprint (W1-KEY): declared here, handled by their owners.
        // The Edit menu: to the focused input, else Pigeon's own (MA-070).
        EditUndo,
        EditRedo,
        EditCopy,
        EditSelectAll,
        SelectAllThreads,
        ToggleFullScreen,
        BringAllToFront,
        SaveDraft,
        // Paging the reader and the list (MB-045).
        PageDown,
        PageUp,
        ScrollToTop,
        ScrollToBottom,
        SelectFirstThread,
        SelectLastThread,
        // The selected event (ME-034, ME-096, MB-046, MB-061).
        AcceptEvent,
        MaybeEvent,
        DeclineEvent,
        JoinEvent,
        OpenLocation,
        CopyLink,
        OpenInGoogleCalendar,
        ToggleEventReminders,
        // Moving the selected event by keyboard (MA-092).
        NudgeEventEarlier,
        NudgeEventLater,
        NudgeEventDayBack,
        NudgeEventDayForward,
        // The event panel's "…" menu (ME-097).
        DuplicateEvent,
        EmailGuests,
        // Mail (MB-061, MB-044, MB-059, MA-003, MB-062, ME-015, MB-047).
        OpenInGmail,
        QuickLookAttachment,
        SaveAllAttachments,
        CycleMessageLinks,
        MoveToCategory,
        ClearRecentSearches,
        OpenDraftInNewWindow,
        // Onboarding (MC-089) and the contact card.
        CancelSignIn,
        SearchFromContact,
        // Polish stretch B: Snooze (ACT-11) and Send Later (CMP-17).
        Snooze,
        Unsnooze,
        SendLater,
        GoToSnoozed,
        GoToScheduled,
        EditScheduledSend,
        CancelScheduledSend,
        // AI sprint (AI-1): declared here, handled by their owners in
        // `app/ai.rs`, `app/ai_reader.rs` and `app/ai_palette.rs`.
        GoToNeedsReply,
        DismissNeedsReply,
        ApplySuggestedLabel,
        SummarizeConversation,
        ToggleCleanView,
        RetryCleanView,
        ReportBadCleanView,
        AddToCalendar,
        AskPigeon,
        DoItForMe,
    ]
);

/// AI-11: start a reply from quick reply 0, 1 or 2 (⌥1–⌥3).
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct QuickReply(pub usize);

/// ME-060: open Settings at one section; one ⌘K row each.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct OpenSettingsSection(pub Section);

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
/// The sign-in screen (MC-089).
pub const ONBOARDING: &str = "Onboarding";
/// The Snooze and Send Later time picker, a sibling of `Mailbox`.
pub const LATER: &str = later_picker::LATER_PICKER;
/// An attachment chip that has focus (MB-044).
pub const ATTACHMENT: &str = "Attachment";

/// gpui-component's own bindings, as they were before Pigeon's, so a preset
/// change can clear the keymap and put them back.
struct BaseKeymap(Vec<KeyBinding>);

impl gpui::Global for BaseKeymap {}

/// The keymap, with `preset`'s ⌘ shortcuts (MA-038); replaces Pigeon's
/// bindings when called again. Where an action has several bindings, the
/// one added first is the one the menus show, the one added last the one the
/// palette shows.
pub fn bind_keys(cx: &mut App, preset: KeymapPreset) {
    let base = match cx.try_global::<BaseKeymap>() {
        Some(base) => base.0.clone(),
        None => {
            let base: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
            cx.set_global(BaseKeymap(base.clone()));
            base
        }
    };
    cx.clear_key_bindings();
    cx.bind_keys(base);
    cx.bind_keys(preset_bindings(preset));
    cx.bind_keys(shared_bindings());
    crate::ui::index_shortcuts(cx);
}

/// The ⌘ shortcuts that differ between the presets. Bound before the rest,
/// so the menus show them. Single-key shortcuts are the same in both.
fn preset_bindings(preset: KeymapPreset) -> Vec<KeyBinding> {
    const COMPOSER_INPUT: &str = "Composer > Input";
    const NOT_COMPOSING: &str = "Mailbox && !Composer";
    match preset {
        KeymapPreset::Gmail => vec![
            KeyBinding::new("cmd-r", SyncNow, Some(CONTEXT)),
            KeyBinding::new("cmd-shift-d", DiscardDraft, Some(COMPOSER)),
            KeyBinding::new("cmd-shift-d", DiscardEventChanges, Some(CALENDAR)),
        ],
        // Mail's: ⇧⌘D sends, so discarding moves to ⇧⌘⌫, which also works
        // while typing (⌘⌫ deletes to the line start there).
        KeymapPreset::AppleMail => vec![
            KeyBinding::new("cmd-shift-n", SyncNow, Some(CONTEXT)),
            KeyBinding::new("cmd-r", Reply, Some(NOT_COMPOSING)),
            KeyBinding::new("cmd-shift-r", ReplyAll, Some(NOT_COMPOSING)),
            KeyBinding::new("cmd-shift-f", Forward, Some(NOT_COMPOSING)),
            KeyBinding::new("cmd-shift-d", SendMessage, Some(COMPOSER)),
            KeyBinding::new("cmd-shift-d", SendMessage, Some(COMPOSER_INPUT)),
            KeyBinding::new("cmd-shift-backspace", DiscardDraft, Some(COMPOSER)),
            KeyBinding::new("cmd-shift-backspace", DiscardEventChanges, Some(CALENDAR)),
        ],
    }
}

fn shared_bindings() -> Vec<KeyBinding> {
    const MAIL: &str = "Mailbox && !Input";
    const LIST: &str = "Mailbox && !Reader && !Input && !Control";
    const TABBING: &str = "Mailbox && !Input && !Composer";
    const READING: &str = "Mailbox && Reader && !Input";
    // Space presses a focused control instead (W2-R04).
    const READING_TEXT: &str = "Mailbox && Reader && !Input && !Control";
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
    // Return on a focused button presses it (W2-R05, W2-R16).
    const CARD_LIST: &str = "ContactCard && !Control";
    const ONBOARDING_KEYS: &str = "Onboarding && !Control";
    const HELP_INPUT: &str = "ShortcutsHelp > Input";
    const CAL_INPUT: &str = "Calendar > Input";
    const CAL_TABBING: &str = "Calendar && !Input && !EventPanel";
    const SETTINGS: &str = settings_page::SETTINGS;
    const LATER_INPUT: &str = "LaterPicker > Input";

    vec![
        // Menu-bar shortcuts first: the menu shows an action's first binding.
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        // Inputs bind these too, deeper, so they win while typing.
        KeyBinding::new("cmd-z", EditUndo, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-z", EditRedo, Some(CONTEXT)),
        KeyBinding::new("cmd-c", EditCopy, Some(CONTEXT)),
        KeyBinding::new("cmd-a", EditSelectAll, Some(CONTEXT)),
        KeyBinding::new("cmd-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd-+", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("cmd--", ZoomOut, Some(CONTEXT)),
        KeyBinding::new("cmd-0", ResetZoom, Some(CONTEXT)),
        KeyBinding::new("cmd-n", Compose, Some(MAILBOX)),
        // ⌥⌘N first, so File › New Event doesn't show New Message's ⌘N.
        KeyBinding::new("alt-cmd-n", NewEvent, Some(CONTEXT)),
        KeyBinding::new("cmd-n", NewEvent, Some(CALENDAR)),
        KeyBinding::new("cmd-f", ToggleSearch, Some(MAILBOX)),
        // In Calendar, Find searches the events (MA-081); ⌥⌘F is Mail's
        // mailbox search.
        KeyBinding::new("cmd-f", ToggleSearch, Some(CALENDAR)),
        KeyBinding::new("alt-cmd-f", ToggleSearch, Some(CONTEXT)),
        KeyBinding::new("tab", FocusNextControl, Some(TABBING)),
        KeyBinding::new("shift-tab", FocusPrevControl, Some(TABBING)),
        KeyBinding::new("escape", FocusPanes, Some(CONTROL)),
        KeyBinding::new("cmd-k", ToggleCommandPalette, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-p", ToggleCommandPalette, Some(CONTEXT)),
        // ⌃⌘S first, so the menu doesn't show Clear Formatting's ⌘\.
        KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("cmd-\\", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("cmd-1", GoToMail, Some(CONTEXT)),
        KeyBinding::new("cmd-2", GoToCalendar, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-i", LoadImages, Some(MAILBOX)),
        // Moving through the list.
        KeyBinding::new("down", SelectNextThread, Some(LIST)),
        KeyBinding::new("up", SelectPrevThread, Some(LIST)),
        KeyBinding::new("j", SelectNextThread, Some(MAIL)),
        KeyBinding::new("k", SelectPrevThread, Some(MAIL)),
        KeyBinding::new("enter", OpenThread, Some(LIST)),
        KeyBinding::new("o", OpenThread, Some(LIST)),
        KeyBinding::new("home", SelectFirstThread, Some(LIST)),
        KeyBinding::new("end", SelectLastThread, Some(LIST)),
        KeyBinding::new("pagedown", PageDown, Some(LIST)),
        KeyBinding::new("pageup", PageUp, Some(LIST)),
        // Reading.
        KeyBinding::new("down", ScrollReaderDown, Some(READING)),
        KeyBinding::new("up", ScrollReaderUp, Some(READING)),
        KeyBinding::new("pagedown", PageDown, Some(READING)),
        KeyBinding::new("space", PageDown, Some(READING_TEXT)),
        KeyBinding::new("pageup", PageUp, Some(READING)),
        KeyBinding::new("shift-space", PageUp, Some(READING_TEXT)),
        KeyBinding::new("cmd-up", ScrollToTop, Some(READING)),
        KeyBinding::new("home", ScrollToTop, Some(READING)),
        KeyBinding::new("cmd-down", ScrollToBottom, Some(READING)),
        KeyBinding::new("end", ScrollToBottom, Some(READING)),
        KeyBinding::new("alt-tab", CycleMessageLinks, Some(READING)),
        KeyBinding::new("escape", BackToList, Some(READING)),
        KeyBinding::new("u", BackToList, Some(READING)),
        KeyBinding::new("n", NextMessage, Some(READING)),
        KeyBinding::new("p", PrevMessage, Some(READING)),
        KeyBinding::new("cmd-a", SelectMessageText, Some(READING)),
        // Thread actions. ⌫ first, so the menu shows it (MB-014), and last,
        // so the palette does too.
        KeyBinding::new("e", Archive, Some(MAIL)),
        KeyBinding::new("backspace", Delete, Some(MAIL)),
        KeyBinding::new("#", Delete, Some(MAIL)),
        KeyBinding::new("delete", Delete, Some(MAIL)),
        KeyBinding::new("backspace", Delete, Some(MAIL)),
        KeyBinding::new("v", MoveTo, Some(MAIL)),
        KeyBinding::new("l", ApplyLabel, Some(MAIL)),
        KeyBinding::new("cmd-y", QuickLookAttachment, Some(MAILBOX)),
        KeyBinding::new("space", QuickLookAttachment, Some(ATTACHMENT)),
        // Selecting several threads, like Gmail.
        KeyBinding::new("x", ToggleSelected, Some(MAIL)),
        KeyBinding::new("cmd-a", SelectAllThreads, Some(LIST)),
        KeyBinding::new("cmd-shift-a", ClearSelection, Some(LIST)),
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
        KeyBinding::new("cmd-s", SaveDraft, Some(COMPOSER)),
        KeyBinding::new("cmd-s", SaveDraft, Some(COMPOSER_INPUT)),
        KeyBinding::new("escape", CloseDraft, Some(COMPOSER)),
        KeyBinding::new("z", Undo, Some(MAIL)),
        KeyBinding::new("cmd-z", Undo, Some(MAIL)),
        KeyBinding::new("tab", composer::NextField, Some(COMPOSER)),
        KeyBinding::new("shift-tab", composer::PrevField, Some(COMPOSER)),
        // Formatting the body, like Gmail, except links: ⌘⇧K, so ⌘K is the
        // palette everywhere. The composer passes them on from its other
        // fields, so there ⌘\ is still the sidebar. On a US layout ⌘⇧7
        // arrives as ⌘&, and so on; other layouts have those symbols
        // elsewhere, so the letter aliases come first (MA-088), and the
        // menus show them.
        KeyBinding::new("cmd-b", composer::FormatBold, Some(COMPOSER)),
        KeyBinding::new("cmd-i", composer::FormatItalic, Some(COMPOSER)),
        KeyBinding::new("cmd-u", composer::FormatUnderline, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-x", composer::FormatStrikethrough, Some(COMPOSER)),
        KeyBinding::new("cmd-e", composer::FormatCode, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-k", composer::InsertLink, Some(COMPOSER)),
        KeyBinding::new("alt-cmd-o", composer::NumberedList, Some(COMPOSER)),
        KeyBinding::new("alt-cmd-b", composer::BulletedList, Some(COMPOSER)),
        KeyBinding::new("alt-cmd-q", composer::QuoteLines, Some(COMPOSER)),
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
        KeyBinding::new(
            "cmd-.",
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
        KeyBinding::new("g x", GoToSpam, Some(MAIL)),
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
        // RSVP and Join for the selected event (ME-034, ME-096).
        KeyBinding::new("shift-y", AcceptEvent, Some(CAL)),
        KeyBinding::new("shift-m", MaybeEvent, Some(CAL)),
        KeyBinding::new("shift-n", DeclineEvent, Some(CAL)),
        KeyBinding::new("cmd-j", JoinEvent, Some(CALENDAR)),
        // ⌥ and an arrow moves the selected event (MA-092).
        KeyBinding::new("alt-up", NudgeEventEarlier, Some(CAL)),
        KeyBinding::new("alt-down", NudgeEventLater, Some(CAL)),
        KeyBinding::new("alt-left", NudgeEventDayBack, Some(CAL)),
        KeyBinding::new("alt-right", NudgeEventDayForward, Some(CAL)),
        // Down and up go by event, like j and k in Mail; tab goes through
        // the header's controls (MA-005).
        KeyBinding::new("tab", FocusNextControl, Some(CAL_TABBING)),
        KeyBinding::new("shift-tab", FocusPrevControl, Some(CAL_TABBING)),
        KeyBinding::new("down", SelectNextEvent, Some(CAL)),
        KeyBinding::new("j", SelectNextEvent, Some(CAL)),
        KeyBinding::new("up", SelectPrevEvent, Some(CAL)),
        KeyBinding::new("k", SelectPrevEvent, Some(CAL)),
        // Inputs pass escape on, so this closes the panel from them too.
        // They take ⌘. themselves, so it binds inside them as well.
        KeyBinding::new("escape", CloseEvent, Some(CALENDAR)),
        KeyBinding::new("cmd-.", CloseEvent, Some(CALENDAR)),
        KeyBinding::new("cmd-.", CloseEvent, Some(CAL_INPUT)),
        KeyBinding::new("cmd-s", SaveEvent, Some(CALENDAR)),
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
        KeyBinding::new("cmd-f", settings_page::FocusSearch, Some(SETTINGS)),
        KeyBinding::new("?", ToggleShortcutsHelp, Some(SET)),
        // Inputs pass escape on, so this clears the search from it too.
        // ⌘W closes Settings, not the window (MA-077).
        KeyBinding::new("escape", settings_page::Close, Some(SETTINGS)),
        KeyBinding::new("cmd-.", settings_page::Close, Some(SETTINGS)),
        KeyBinding::new("cmd-.", settings_page::Close, Some(SET_INPUT)),
        KeyBinding::new("cmd-w", settings_page::Close, Some(SETTINGS)),
        KeyBinding::new("down", settings_page::SelectNext, Some(SET_INPUT)),
        KeyBinding::new("up", settings_page::SelectPrev, Some(SET_INPUT)),
        KeyBinding::new("tab", settings_page::EnterRows, Some(SET_INPUT)),
        // Overlays close on ⌘W and ⌘. too (MA-077, MA-089).
        KeyBinding::new("escape", ToggleShortcutsHelp, Some(HELP)),
        KeyBinding::new("cmd-.", ToggleShortcutsHelp, Some(HELP)),
        KeyBinding::new("cmd-.", ToggleShortcutsHelp, Some(HELP_INPUT)),
        KeyBinding::new("cmd-w", ToggleShortcutsHelp, Some(HELP)),
        KeyBinding::new("?", ToggleShortcutsHelp, Some(HELP)),
        KeyBinding::new("enter", command_palette::Confirm, Some(PALETTE)),
        KeyBinding::new("escape", command_palette::Dismiss, Some(PALETTE)),
        KeyBinding::new("cmd-.", command_palette::Dismiss, Some(PALETTE)),
        KeyBinding::new("cmd-.", command_palette::Dismiss, Some(PALETTE_INPUT)),
        KeyBinding::new("cmd-w", command_palette::Dismiss, Some(PALETTE)),
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
        KeyBinding::new("shift-l up", MoveLabelUp, Some(MAIL)),
        KeyBinding::new("shift-l down", MoveLabelDown, Some(MAIL)),
        // W3: From and the signature.
        KeyBinding::new("cmd-shift-f", ChangeFrom, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-e", ToggleSignature, Some(COMPOSER)),
        KeyBinding::new("enter", composer::FromActivate, Some(FROM_PICKER)),
        KeyBinding::new("space", composer::FromActivate, Some(FROM_PICKER)),
        KeyBinding::new("down", composer::FromNext, Some(FROM_PICKER)),
        KeyBinding::new("up", composer::FromPrev, Some(FROM_PICKER)),
        KeyBinding::new("escape", composer::FromDismiss, Some(FROM_PICKER)),
        KeyBinding::new("cmd-.", composer::FromDismiss, Some(FROM_PICKER)),
        // W6: the contact card.
        KeyBinding::new("shift-p", ShowContact, Some(MAIL)),
        KeyBinding::new("down", contact_card::SelectNext, Some(CARD)),
        KeyBinding::new("j", contact_card::SelectNext, Some(CARD)),
        KeyBinding::new("up", contact_card::SelectPrev, Some(CARD)),
        KeyBinding::new("k", contact_card::SelectPrev, Some(CARD)),
        KeyBinding::new("enter", contact_card::Confirm, Some(CARD_LIST)),
        KeyBinding::new("c", contact_card::ComposeTo, Some(CARD)),
        KeyBinding::new("cmd-c", contact_card::CopyAddress, Some(CARD)),
        KeyBinding::new("s", SearchFromContact, Some(CARD)),
        KeyBinding::new("escape", contact_card::Dismiss, Some(CARD)),
        KeyBinding::new("cmd-.", contact_card::Dismiss, Some(CARD)),
        KeyBinding::new("cmd-w", contact_card::Dismiss, Some(CARD)),
        // The sign-in screen (MC-089); its handlers ignore ↩ and D while a
        // sign-in is under way.
        KeyBinding::new("escape", CancelSignIn, Some(ONBOARDING)),
        KeyBinding::new("enter", onboarding::Confirm, Some(ONBOARDING_KEYS)),
        KeyBinding::new("d", onboarding::TryDemo, Some(ONBOARDING_KEYS)),
        // Snooze (Gmail's `b`) and Send Later (ACT-11, CMP-17). The picker's
        // field swallows the arrows, so they bind inside it too.
        KeyBinding::new("b", Snooze, Some(MAIL)),
        KeyBinding::new("shift-b", Unsnooze, Some(MAIL)),
        KeyBinding::new("g b", GoToSnoozed, Some(MAIL)),
        KeyBinding::new("g shift-t", GoToScheduled, Some(MAIL)),
        KeyBinding::new("cmd-shift-enter", SendLater, Some(COMPOSER)),
        KeyBinding::new("cmd-shift-enter", SendLater, Some(COMPOSER_INPUT)),
        KeyBinding::new("down", later_picker::SelectNext, Some(LATER)),
        KeyBinding::new("up", later_picker::SelectPrev, Some(LATER)),
        KeyBinding::new("down", later_picker::SelectNext, Some(LATER_INPUT)),
        KeyBinding::new("up", later_picker::SelectPrev, Some(LATER_INPUT)),
        KeyBinding::new("ctrl-n", later_picker::SelectNext, Some(LATER_INPUT)),
        KeyBinding::new("ctrl-p", later_picker::SelectPrev, Some(LATER_INPUT)),
        KeyBinding::new("escape", later_picker::Dismiss, Some(LATER)),
        KeyBinding::new("cmd-.", later_picker::Dismiss, Some(LATER)),
        KeyBinding::new("cmd-.", later_picker::Dismiss, Some(LATER_INPUT)),
        KeyBinding::new("cmd-w", later_picker::Dismiss, Some(LATER)),
        // AI sprint (AI-1).
        KeyBinding::new("alt-cmd-a", OpenSettingsSection(Section::Ai), Some(CONTEXT)),
        KeyBinding::new("g r", GoToNeedsReply, Some(MAIL)),
        KeyBinding::new("shift-h", DismissNeedsReply, Some(MAIL)),
        KeyBinding::new("alt-l", ApplySuggestedLabel, Some(MAIL)),
        KeyBinding::new("shift-s", SummarizeConversation, Some(MAIL)),
        KeyBinding::new("shift-v", ToggleCleanView, Some(MAIL)),
        KeyBinding::new("alt-cmd-r", RetryCleanView, Some(MAILBOX)),
        KeyBinding::new("alt-shift-cmd-b", ReportBadCleanView, Some(MAILBOX)),
        KeyBinding::new("alt-1", QuickReply(0), Some(MAIL)),
        KeyBinding::new("alt-2", QuickReply(1), Some(MAIL)),
        KeyBinding::new("alt-3", QuickReply(2), Some(MAIL)),
        KeyBinding::new("shift-t", AddToCalendar, Some(MAIL)),
        KeyBinding::new("cmd-shift-j", AskPigeon, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-l", DoItForMe, Some(MAILBOX)),
    ]
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
    /// The action's name, without building it (MC-047).
    pub action_name: fn() -> &'static str,
    /// Shown in the shortcuts overlay but not offered in the palette.
    pub help_only: bool,
    /// Offered in the palette in Mail too, though its group is Calendar's.
    pub global: bool,
    /// When the palette offers it (`PigeonApp::command_available`).
    pub when: When,
    /// ME-055: the state its palette title follows; `name` stays for the
    /// shortcuts sheet and settings search.
    pub toggle: Option<Toggle>,
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

    /// The palette title: what running it will do, given `on`, the state
    /// of its toggle.
    pub fn title(&self, on: impl FnOnce(Toggle) -> bool) -> &'static str {
        match self.toggle {
            Some(toggle) => toggle.title(on(toggle)),
            None => self.name,
        }
    }
}

/// A state a command turns on and off (ME-055); the palette and menus name
/// what it will do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// The selected conversation is starred.
    Star,
    Important,
    /// The selected conversation is checked.
    Selected,
    ArchivedSection,
    CompactList,
    Sidebar,
    WeekNumbers,
    FullScreen,
    Notifications,
    OpenAtLogin,
    EventReminders,
    HiddenLabels,
    /// The open label's sublabels are folded.
    LabelFolded,
    /// The draft carries its signature.
    Signature,
    /// The selected conversation is snoozed.
    Snoozed,
    /// AI-4: the selected conversation's Needs Reply marker was dismissed.
    NeedsReplyDismissed,
}

impl Toggle {
    /// The title while the state is `on`, so the command turns it off, and
    /// the other way round.
    pub fn title(self, on: bool) -> &'static str {
        let (on_title, off_title) = match self {
            Toggle::Star => ("Unstar", "Star"),
            Toggle::Important => ("Mark as Not Important", "Mark as Important"),
            Toggle::Selected => ("Deselect Conversation", "Select Conversation"),
            Toggle::ArchivedSection => ("Hide Archived in Inbox", "Show Archived in Inbox"),
            Toggle::CompactList => ("Use Comfortable List", "Use Compact List"),
            Toggle::Sidebar => ("Hide Sidebar", "Show Sidebar"),
            Toggle::WeekNumbers => ("Hide Week Numbers", "Show Week Numbers"),
            Toggle::FullScreen => ("Exit Full Screen", "Enter Full Screen"),
            Toggle::Notifications => (
                "Turn Off New Mail Notifications",
                "Turn On New Mail Notifications",
            ),
            Toggle::OpenAtLogin => ("Turn Off Open at Login", "Turn On Open at Login"),
            Toggle::EventReminders => ("Turn Off Event Reminders", "Turn On Event Reminders"),
            Toggle::HiddenLabels => ("Hide Hidden Labels", "Show Hidden Labels"),
            Toggle::LabelFolded => ("Unfold Sublabels", "Fold Sublabels"),
            Toggle::Signature => ("Remove Signature", "Add Signature"),
            Toggle::Snoozed => ("Change Snooze…", "Snooze…"),
            Toggle::NeedsReplyDismissed => ("Restore Needs Reply", "Dismiss Needs Reply"),
        };
        if on { on_title } else { off_title }
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
    /// The search bar is open.
    Searching,
    /// Several conversations are selected.
    Selection,
    /// The Inbox, with its archived section, is showing.
    Inbox,
    /// W4b: the user has labels.
    Labels,
    /// A label's view is open.
    LabelOpen,
    // W3.
    /// A draft is open and there's more than one address to send from.
    FromChoice,
    // Polish sprint (ME-054); `Undo` reads the undo stack (MA-071).
    /// There's something to redo.
    Redo,
    /// An event is selected in the calendar.
    Event,
    /// The event panel is open.
    EventOpen,
    /// The selected event is an invitation the user can answer (ME-034).
    CalendarInvite,
    /// Signed in with Google.
    SignedIn,
    /// Neither signed in nor on the demo data.
    SignedOut,
    /// Signed out, or on demo data picked on the sign-in screen (ME-050).
    CanSignIn,
    /// A snoozed conversation is selected (ACT-11).
    Snoozed,
    /// A draft scheduled to send is selected (CMP-17).
    Scheduled,
    /// Signed in, or on demo data picked on the sign-in screen (ME-050).
    CanSignOut,
    /// AI-1: an AI feature may run, with what else it needs.
    Ai(AiNeed),
}

/// What an AI command needs (`When::Ai`): its feature allowed now
/// (`Backend::ai_allows`), and for the reader's commands, in Mail, one
/// selected conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiNeed {
    /// Needs Reply is on.
    NeedsReply,
    /// Needs Reply, on the selected conversation.
    NeedsReplyThread,
    Label,
    Summary,
    CleanView,
    /// Quick replies, on a conversation that can be replied to.
    QuickReply,
    /// Email to event.
    Event,
    Ask,
    DoIt,
}

macro_rules! commands {
    ($($group:ident: $name:literal, $icon:ident, $action:ident $(($arg:expr))? $(, $flag:ident)? $(if $when:ident $(($need:ident))?)? $(=> $toggle:ident)?;)*) => {
        &[$(Command {
            name: $name,
            group: Group::$group,
            icon: Icon::$icon,
            action: || Box::new($action $(($arg))?),
            action_name: <$action as Action>::name_for_type,
            help_only: commands!(@help $($flag)?),
            global: commands!(@global $($flag)?),
            when: commands!(@when $($when $($need)?)?),
            toggle: commands!(@toggle $($toggle)?),
        }),*]
    };
    (@toggle) => { None };
    (@toggle $toggle:ident) => { Some(Toggle::$toggle) };
    (@help) => { false };
    (@help help_only) => { true };
    (@help global) => { false };
    (@global) => { false };
    (@global global) => { true };
    (@global help_only) => { false };
    (@when) => { When::Always };
    (@when $when:ident) => { When::$when };
    (@when $when:ident $need:ident) => { When::$when(AiNeed::$need) };
}

/// Every command, in palette order.
pub const COMMANDS: &[Command] = commands! {
    Compose: "Compose New Message", SquarePen, Compose;
    Compose: "Reply", Reply, Reply if Replyable;
    Compose: "Reply All", ReplyAll, ReplyAll if Replyable;
    Compose: "Forward", Forward, Forward if Replyable;
    Compose: "Send", Send, SendMessage if Draft;
    Compose: "Show Cc and Bcc", Mail, ShowCcBcc if Draft;
    Compose: "Save Draft", FilePen, SaveDraft if Draft;
    Compose: "Close Draft (Saves to Drafts)", FilePen, CloseDraft if Draft;
    Compose: "Discard Draft", Trash, DiscardDraft if Draft;
    Compose: "Open Draft in New Window", SquarePen, OpenDraftInNewWindow if Draft;
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
    Conversation: "Archive", Archive, Archive if Thread;
    Conversation: "Delete", Trash, Delete if Thread;
    Conversation: "Star / Unstar", Star, ToggleStar if Thread => Star;
    Conversation: "Mark as Unread", MailUnread, MarkUnread if Thread;
    Conversation: "Mark as Read", MailOpen, MarkRead if Thread;
    Conversation: "Move to…", FolderInput, MoveTo if Thread;
    Conversation: "Move to Category…", FolderInput, MoveToCategory if Thread;
    Conversation: "Label…", Tag, ApplyLabel if Thread;
    Selection: "Select / Deselect Conversation", Check, ToggleSelected if Thread => Selected;
    Selection: "Select All Conversations", Check, SelectAllThreads;
    Selection: "Extend Selection Down", ChevronDown, ExtendSelectionDown if Thread;
    Selection: "Extend Selection Up", ChevronUp, ExtendSelectionUp if Thread;
    Selection: "Clear Selection", X, ClearSelection if Selection;
    Navigation: "Next Conversation", ChevronDown, SelectNextThread;
    Navigation: "Previous Conversation", ChevronUp, SelectPrevThread;
    Navigation: "First Conversation", ChevronUp, SelectFirstThread;
    Navigation: "Last Conversation", ChevronDown, SelectLastThread;
    Navigation: "Open Conversation", Mail, OpenThread;
    Navigation: "Back to List", Inbox, BackToList if Thread;
    Navigation: "Next Message in Conversation", ChevronDown, NextMessage if Messages;
    Navigation: "Previous Message in Conversation", ChevronUp, PrevMessage if Messages;
    Conversation: "Select Message Text", Text, SelectMessageText if Thread;
    Conversation: "Retry Loading Message", RefreshCw, RetryLoadingMessage if Thread;
    Conversation: "Load Images", Image, LoadImages if Thread;
    Conversation: "Open in Gmail", Mail, OpenInGmail if Thread;
    Navigation: "Scroll Conversation Down", ChevronDown, ScrollReaderDown, help_only;
    Navigation: "Scroll Conversation Up", ChevronUp, ScrollReaderUp, help_only;
    Navigation: "Page Down", ChevronDown, PageDown, help_only;
    Navigation: "Page Up", ChevronUp, PageUp, help_only;
    Navigation: "Scroll to Top", ChevronUp, ScrollToTop, help_only;
    Navigation: "Scroll to Bottom", ChevronDown, ScrollToBottom, help_only;
    Navigation: "Next Link in Message", Link, CycleMessageLinks, help_only;
    Navigation: "Search Mail", Search, ToggleSearch;
    CalendarNavigation: "Search Events", Search, ToggleSearch;
    Navigation: "Clear Search", X, CloseSearch if Searching;
    Navigation: "Clear Recent Searches", X, ClearRecentSearches if Searching;
    Navigation: "Compact / Comfortable List", Rows, ToggleDensity => CompactList;
    Navigation: "Load Older Conversations", ChevronDown, LoadOlderConversations;
    GoTo: "Go to Inbox", Inbox, GoToInbox;
    GoTo: "Go to Starred", Star, GoToStarred;
    GoTo: "Go to Sent", Send, GoToSent;
    GoTo: "Go to Drafts", FilePen, GoToDrafts;
    GoTo: "Go to Archive", Archive, GoToArchive;
    GoTo: "Go to Spam", ShieldAlert, GoToSpam;
    GoTo: "Go to Trash", Trash, GoToTrash;
    GoTo: "Go to Label…", ChevronRight, GoToLabel;
    Navigation: "Show / Hide Sidebar", PanelLeft, ToggleSidebar => Sidebar;
    Navigation: "Show / Hide Archived in Inbox", Archive, ToggleArchivedSection if Inbox => ArchivedSection;
    // From Mail too: the app switches to Calendar (`Show::NewEvent`).
    CalendarEvents: "New Event", Plus, NewEvent, global;
    // W5.
    CalendarEvents: "New Event from Text…", Plus, NewEventFromText, global;
    CalendarEvents: "Join Meeting", Video, JoinEvent if Event;
    CalendarEvents: "Accept Invitation", Check, AcceptEvent if CalendarInvite;
    CalendarEvents: "Reply Maybe", Calendar, MaybeEvent if CalendarInvite;
    CalendarEvents: "Decline Invitation", X, DeclineEvent if CalendarInvite;
    CalendarEvents: "Add Google Meet", Video, AddGoogleMeet if Event;
    CalendarEvents: "Repeat…", Repeat, SetRepeat if Event;
    CalendarEvents: "Add Reminder…", Bell, AddReminder if Event;
    CalendarEvents: "Open Location in Maps", MapPin, OpenLocation if Event;
    CalendarEvents: "Copy Link to Event", Copy, CopyLink if Event;
    CalendarEvents: "Duplicate Event", Copy, DuplicateEvent if Event;
    CalendarEvents: "Email Guests", Mail, EmailGuests if Event;
    CalendarEvents: "Open in Google Calendar", Calendar, OpenInGoogleCalendar if Event;
    CalendarEvents: "Open Event", Calendar, OpenEvent if Event;
    CalendarEvents: "Move Event 15 Minutes Earlier", ChevronUp, NudgeEventEarlier if Event;
    CalendarEvents: "Move Event 15 Minutes Later", ChevronDown, NudgeEventLater if Event;
    CalendarEvents: "Move Event to the Day Before", ChevronLeft, NudgeEventDayBack if Event;
    CalendarEvents: "Move Event to the Day After", ChevronRight, NudgeEventDayForward if Event;
    CalendarEvents: "Delete Event", Trash, DeleteEvent if Event;
    CalendarEvents: "Save Event", Check, SaveEvent if EventOpen;
    CalendarEvents: "Discard Changes to Event", Undo, DiscardEventChanges if EventOpen;
    CalendarEvents: "Close Event (Discards a New One)", X, CloseEvent if EventOpen;
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
    CalendarView: "Show / Hide Week Numbers", Calendar, ToggleWeekNumbers => WeekNumbers;
    // Undo and Redo work in both apps (ME-054: Undo Delete is Undo now).
    App: "Undo", Undo, Undo if Undo;
    App: "Redo", Repeat, EditRedo if Redo;
    App: "Go to Mail", Mail, GoToMail;
    App: "Go to Calendar", Calendar, GoToCalendar;
    App: "Command Palette", Command, ToggleCommandPalette, help_only;
    App: "Keyboard Shortcuts", Keyboard, ToggleShortcutsHelp;
    App: "Settings…", Settings, OpenSettings;
    App: "Sync Now", RefreshCw, SyncNow if SignedIn;
    App: "Sign In with Google…", LogIn, SignIn if CanSignIn;
    App: "Sign Out", LogOut, SignOut if CanSignOut;
    App: "Use Demo Data", Layers, UseDemoData if SignedOut;
    App: "Quit Pigeon", LogOut, Quit;
    // ME-060: the menus' window and text commands, and Settings' sections.
    App: "About Pigeon", Mail, About;
    App: "Make Text Bigger", Plus, ZoomIn;
    App: "Make Text Smaller", Minus, ZoomOut;
    App: "Make Text Normal Size", Text, ResetZoom;
    App: "Enter / Exit Full Screen", PanelLeft, ToggleFullScreen => FullScreen;
    App: "Minimize", Minus, Minimize;
    App: "Zoom", Rows, ZoomWindow;
    App: "Bring All to Front", Layers, BringAllToFront;
    App: "Appearance Settings", SunMoon, OpenSettingsSection(Section::Appearance);
    App: "Mail Settings", Settings, OpenSettingsSection(Section::Mail);
    App: "Calendar Settings", Settings, OpenSettingsSection(Section::Calendar);
    App: "Notification Settings", Settings, OpenSettingsSection(Section::Notifications);
    App: "Menu Bar Settings", Settings, OpenSettingsSection(Section::MenuBar);
    App: "MCP Settings", Code, OpenSettingsSection(Section::Mcp);
    App: "General Settings", Settings, OpenSettingsSection(Section::General);
    App: "Account Settings", Settings, OpenSettingsSection(Section::Accounts);
    App: "Keyboard Shortcut Settings", Settings, OpenSettingsSection(Section::Shortcuts);
    // W1
    App: "Open Pigeon Window", Mail, ShowMainWindow, help_only;
    App: "Send Test Notification", Bell, SendTestNotification;
    App: "Turn New Mail Notifications On / Off", Bell, ToggleNotifications => Notifications;
    App: "Turn Event Reminders On / Off", Bell, ToggleEventReminders => EventReminders;
    App: "Turn Open at Login On / Off", Settings, ToggleOpenAtLogin => OpenAtLogin;
    // W2: attachments and the original message.
    Conversation: "Open Attachment", Paperclip, OpenAttachment if Thread;
    Conversation: "Quick Look Attachment", Paperclip, QuickLookAttachment if Thread;
    Conversation: "Save Attachment…", Download, SaveAttachment if Thread;
    Conversation: "Save All Attachments…", Download, SaveAllAttachments if Thread;
    Conversation: "Show Original", Code, ShowOriginal if Thread;
    Conversation: "Save Message as .eml…", Download, SaveOriginal if Thread;
    Conversation: "Copy Original", Copy, CopyOriginal, help_only;
    Conversation: "Close Original", X, CloseOriginal, help_only;
    // W7b: the menu bar.
    App: "Open Menu Bar Panel", Calendar, OpenMenuBarPanel;
    App: "Join Next Meeting", Video, JoinNextMeeting;
    // W4b: Inbox categories, labels and Important.
    GoTo: "Go to Social", Users, GoToSocial;
    GoTo: "Go to Promotions", Megaphone, GoToPromotions;
    GoTo: "Go to Updates", Bell, GoToUpdates;
    GoTo: "Go to Forums", MessagesSquare, GoToForums;
    Conversation: "Mark as Important / Not Important", Important, ToggleImportant if Thread => Important;
    Labels: "New Label…", Plus, NewLabel;
    Labels: "Rename Label…", Tag, RenameLabel if Labels;
    Labels: "Change Label Color…", Tag, ChangeLabelColor if Labels;
    Labels: "Delete Label…", Trash, DeleteLabel if Labels;
    Labels: "Label Visibility…", Tag, ChangeLabelVisibility if Labels;
    Labels: "Fold / Unfold Sublabels", ChevronRight, ToggleLabelFolded if Labels => LabelFolded;
    Labels: "Show / Hide Hidden Labels", Tag, ToggleHiddenLabels if Labels => HiddenLabels;
    Labels: "Move Label Up", ChevronUp, MoveLabelUp if LabelOpen;
    Labels: "Move Label Down", ChevronDown, MoveLabelDown if LabelOpen;
    // W3: compose and identity.
    Compose: "Change From…", User, ChangeFrom if FromChoice;
    Compose: "Add / Remove Signature", SquarePen, ToggleSignature if Draft => Signature;
    App: "Make Pigeon the Default Mail App", Mail, MakeDefaultMailApp;
    // W6: the contact card.
    Conversation: "Show Contact", User, ShowContact if Thread;
    // Polish stretch B: Snooze (ACT-11) and Send Later (CMP-17).
    Conversation: "Snooze…", AlarmClock, Snooze if Thread => Snoozed;
    Conversation: "Unsnooze", Inbox, Unsnooze if Snoozed;
    Compose: "Send Later…", Clock, SendLater if Draft;
    Compose: "Edit Scheduled Message", SquarePen, EditScheduledSend if Scheduled;
    Compose: "Cancel Scheduled Send", X, CancelScheduledSend if Scheduled;
    GoTo: "Go to Snoozed", AlarmClock, GoToSnoozed;
    GoTo: "Go to Scheduled", Clock, GoToScheduled;
    // AI sprint (AI-1).
    App: "AI Settings", Settings, OpenSettingsSection(Section::Ai);
    GoTo: "Go to Needs Reply", Reply, GoToNeedsReply if Ai(NeedsReply);
    Conversation: "Dismiss / Restore Needs Reply", Check, DismissNeedsReply if Ai(NeedsReplyThread) => NeedsReplyDismissed;
    Conversation: "Apply Suggested Label", Tag, ApplySuggestedLabel if Ai(Label);
    Conversation: "Summarize Conversation", TextQuote, SummarizeConversation if Ai(Summary);
    Conversation: "Toggle Clean View", FileText, ToggleCleanView if Ai(CleanView);
    Conversation: "Retry Clean View", RefreshCw, RetryCleanView if Ai(CleanView);
    Conversation: "Report Bad Clean View", OctagonAlert, ReportBadCleanView if Ai(CleanView);
    Conversation: "Quick Reply 1", Reply, QuickReply(0) if Ai(QuickReply);
    Conversation: "Quick Reply 2", Reply, QuickReply(1) if Ai(QuickReply);
    Conversation: "Quick Reply 3", Reply, QuickReply(2) if Ai(QuickReply);
    Conversation: "Add to Calendar", Calendar, AddToCalendar if Ai(Event);
    App: "Ask Pigeon", MessagesSquare, AskPigeon if Ai(Ask);
    Conversation: "Do It for Me…", Check, DoItForMe if Ai(DoIt);
};

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_of(bindings: &[KeyBinding], action: &dyn Action) -> Vec<String> {
        bindings
            .iter()
            .filter(|b| b.action().partial_eq(action))
            .map(|b| {
                b.keystrokes()
                    .iter()
                    .map(|k| k.unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn every_binding_parses() {
        assert!(!shared_bindings().is_empty());
        for preset in KeymapPreset::ALL {
            assert!(!preset_bindings(preset).is_empty());
        }
    }

    fn key(source: &str) -> String {
        gpui::Keystroke::parse(source).unwrap().unparse()
    }

    #[test]
    fn apple_mail_sends_on_shift_cmd_d() {
        let send = key("cmd-shift-d");
        let apple = preset_bindings(KeymapPreset::AppleMail);
        assert!(keys_of(&apple, &SendMessage).contains(&send));
        assert!(!keys_of(&apple, &DiscardDraft).contains(&send));
        let gmail = preset_bindings(KeymapPreset::Gmail);
        assert_eq!(keys_of(&gmail, &DiscardDraft), [send]);
        assert!(keys_of(&gmail, &SendMessage).is_empty());
    }

    /// Where the bindings can be active together, innermost last.
    fn context_stacks() -> Vec<Vec<KeyContext>> {
        let stack = |names: &[&str]| -> Vec<KeyContext> {
            names
                .iter()
                .map(|n| KeyContext::parse(n).expect("context parses"))
                .collect()
        };
        [
            &["Pigeon", "Mailbox"][..],
            &["Pigeon", "Mailbox Reader"],
            &["Pigeon", "Mailbox Reader", "Control"],
            &["Pigeon", "Mailbox Reader", "Attachment"],
            &["Pigeon", "Mailbox", "Composer"],
            &["Pigeon", "Mailbox", "Composer", "Input"],
            &["Pigeon", "Mailbox", "Composer", "Recipients", "Input"],
            &["Pigeon", "Mailbox", "SearchBar", "Input"],
            &["Pigeon", "Mailbox Reader", "Original"],
            &["Pigeon", "Calendar"],
            &["Pigeon", "Calendar", "EventPanel", "Input"],
            &["Pigeon", "Settings"],
            &["Pigeon", "Settings", "SettingsSearch", "Input"],
            &["Pigeon", "CommandPalette", "Input"],
            &["Pigeon", "ShortcutsHelp"],
            &["Pigeon", "ContactCard"],
            &["Pigeon", "LaterPicker", "Input"],
        ]
        .iter()
        .map(|names| stack(names))
        .collect()
    }

    fn strokes(binding: &KeyBinding) -> Vec<String> {
        binding.keystrokes().iter().map(|k| k.unparse()).collect()
    }

    /// AI-1: none of the AI bindings is another binding, a prefix of one,
    /// or starts with one, wherever both can be active.
    #[test]
    fn ai_bindings_clash_with_nothing() {
        let ai: [&dyn Action; 14] = [
            &OpenSettingsSection(Section::Ai),
            &GoToNeedsReply,
            &DismissNeedsReply,
            &ApplySuggestedLabel,
            &SummarizeConversation,
            &ToggleCleanView,
            &RetryCleanView,
            &ReportBadCleanView,
            &QuickReply(0),
            &QuickReply(1),
            &QuickReply(2),
            &AddToCalendar,
            &AskPigeon,
            &DoItForMe,
        ];
        let is_ai = |b: &KeyBinding| ai.iter().any(|a| b.action().partial_eq(*a));
        for preset in KeymapPreset::ALL {
            let all: Vec<KeyBinding> = preset_bindings(preset)
                .into_iter()
                .chain(shared_bindings())
                .collect();
            for action in ai {
                assert!(
                    all.iter().any(|b| b.action().partial_eq(action)),
                    "{} has no binding",
                    action.name()
                );
            }
            for stack in context_stacks() {
                let active: Vec<&KeyBinding> = all
                    .iter()
                    // As the keymap does: at any depth of the stack.
                    .filter(|b| {
                        b.predicate().is_none_or(|p| {
                            (1..=stack.len()).any(|n| p.eval_inner(&stack[..n], &stack))
                        })
                    })
                    .collect();
                for new in active.iter().filter(|b| is_ai(b)) {
                    let keys = strokes(new);
                    for other in &active {
                        if std::ptr::eq(*new, *other) {
                            continue;
                        }
                        let theirs = strokes(other);
                        let n = keys.len().min(theirs.len());
                        assert!(
                            keys[..n] != theirs[..n],
                            "{preset:?}: {} ({}) clashes with {} ({}) in {stack:?}",
                            new.action().name(),
                            keys.join(" "),
                            other.action().name(),
                            theirs.join(" "),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ai_commands_are_in_the_table() {
        for name in [
            "AI Settings",
            "Go to Needs Reply",
            "Dismiss / Restore Needs Reply",
            "Apply Suggested Label",
            "Summarize Conversation",
            "Toggle Clean View",
            "Retry Clean View",
            "Report Bad Clean View",
            "Quick Reply 1",
            "Quick Reply 2",
            "Quick Reply 3",
            "Add to Calendar",
            "Ask Pigeon",
            "Do It for Me…",
        ] {
            let command = COMMANDS.iter().find(|c| c.name == name);
            assert!(command.is_some_and(|c| !c.help_only), "{name}");
        }
        let ask = COMMANDS.iter().find(|c| c.name == "Ask Pigeon").unwrap();
        assert_eq!(ask.when, When::Ai(AiNeed::Ask));
    }

    #[test]
    fn toggle_titles_name_the_change() {
        assert_eq!(Toggle::Star.title(true), "Unstar");
        assert_eq!(Toggle::Star.title(false), "Star");
        let star = COMMANDS.iter().find(|c| c.name == "Star / Unstar").unwrap();
        assert_eq!(star.title(|_| true), "Unstar");
        let archive = COMMANDS.iter().find(|c| c.name == "Archive").unwrap();
        assert_eq!(archive.title(|_| true), "Archive");
    }
}

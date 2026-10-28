mod actions;
mod app;
mod assets;
mod backend;
mod calendar;
mod contacts;
mod data;
mod html;
mod instance;
mod logging;
mod markdown;
mod mcp;
mod native;
mod notify;
mod search;
mod settings;
mod theme;
mod ui;

use gpui::{
    App, Entity, Global, Menu, MenuItem, OsAction, SharedString, SystemMenuType, component::input,
};

use crate::{
    actions::*,
    app::shell::{self, Show},
    backend::Backend,
    settings::KeymapPreset,
    ui::calendar::ViewKind,
};

/// What the menus show that changes while Pigeon runs (MB-006): checkmarks,
/// Show/Hide titles and the undo's name. Set by the app with
/// `set_menu_state`; the menus rebuild only when it changes.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    /// Calendar is showing, rather than Mail.
    pub calendar: bool,
    pub calendar_view: ViewKind,
    pub sidebar: bool,
    pub compact_list: bool,
    pub week_numbers: bool,
    /// The selected conversation is starred, or important.
    pub starred: bool,
    pub important: bool,
    pub full_screen: bool,
    pub signed_in: bool,
    /// "Undo Archive", or `None` for a plain "Undo"; the same for redo.
    pub undo: Option<SharedString>,
    pub redo: Option<SharedString>,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            calendar: false,
            calendar_view: ViewKind::Week,
            sidebar: true,
            compact_list: false,
            week_numbers: false,
            starred: false,
            important: false,
            full_screen: false,
            signed_in: false,
            undo: None,
            redo: None,
        }
    }
}

/// The menus as last built, and the keymap they show.
#[derive(Default)]
struct Menus {
    state: MenuState,
    keymap: Option<KeymapPreset>,
}

impl Global for Menus {}

/// Rebuild the menus for `state`, when it differs from the last one.
pub fn set_menu_state(state: MenuState, cx: &mut App) {
    let menus = cx.default_global::<Menus>();
    if menus.state == state {
        return;
    }
    menus.state = state;
    refresh_menus(cx);
}

/// Switch the keyboard shortcuts preset (MA-038): rebind the keymap, then
/// rebuild the menus and the Dock menu, which show its shortcuts.
pub fn set_keymap(preset: KeymapPreset, cx: &mut App) {
    if cx.default_global::<Menus>().keymap == Some(preset) {
        return;
    }
    actions::bind_keys(cx, preset);
    cx.default_global::<Menus>().keymap = Some(preset);
    refresh_menus(cx);
}

fn refresh_menus(cx: &mut App) {
    let state = cx.default_global::<Menus>().state.clone();
    cx.set_menus(menus(&state));
    cx.set_dock_menu(dock_menu());
}

/// The standard macOS menus. Every item runs an existing action, so it shows
/// that action's shortcut, and is greyed out where the action can't run.
/// Single-key shortcuts show too (MB-Q3): they teach the keyboard.
fn menus(state: &MenuState) -> Vec<Menu> {
    let undo = state.undo.clone().unwrap_or_else(|| "Undo".into());
    let redo = state.redo.clone().unwrap_or_else(|| "Redo".into());
    let account = if state.signed_in {
        MenuItem::action("Sign Out", SignOut)
    } else {
        MenuItem::action("Sign In with Google…", SignIn)
    };
    let view = |kind| state.calendar && state.calendar_view == kind;
    vec![
        Menu::new("Pigeon").items([
            MenuItem::action("About Pigeon", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            account,
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Pigeon", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Pigeon", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Message", Compose),
            MenuItem::action("New Event", NewEvent),
            MenuItem::action("New Event from Text…", NewEventFromText),
            MenuItem::action("Open Draft in New Window", OpenDraftInNewWindow),
            MenuItem::separator(),
            MenuItem::action("Close", CloseWindow),
            MenuItem::action("Save Draft", SaveDraft),
            MenuItem::action("Discard Draft", DiscardDraft),
            MenuItem::separator(),
            MenuItem::action("Open Attachment", OpenAttachment),
            MenuItem::action("Quick Look Attachment", QuickLookAttachment),
            MenuItem::action("Save Attachment…", SaveAttachment),
            MenuItem::action("Save All Attachments…", SaveAllAttachments),
            MenuItem::action("Save Message As…", SaveOriginal),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action(undo, EditUndo, OsAction::Undo),
            MenuItem::os_action(redo, EditRedo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", EditCopy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::os_action("Select All", EditSelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Find…", ToggleSearch),
        ]),
        Menu::new("Format").items([
            MenuItem::action("Bold", ui::composer::FormatBold),
            MenuItem::action("Italic", ui::composer::FormatItalic),
            MenuItem::action("Underline", ui::composer::FormatUnderline),
            MenuItem::action("Strikethrough", ui::composer::FormatStrikethrough),
            MenuItem::action("Code", ui::composer::FormatCode),
            MenuItem::separator(),
            MenuItem::action("Insert Link…", ui::composer::InsertLink),
            MenuItem::separator(),
            MenuItem::action("Bulleted List", ui::composer::BulletedList),
            MenuItem::action("Numbered List", ui::composer::NumberedList),
            MenuItem::action("Quote", ui::composer::QuoteLines),
            MenuItem::separator(),
            MenuItem::action("Clear Formatting", ui::composer::ClearFormatting),
        ]),
        Menu::new("View").items([
            MenuItem::action("Mail", GoToMail).checked(!state.calendar),
            MenuItem::action("Calendar", GoToCalendar).checked(state.calendar),
            MenuItem::separator(),
            MenuItem::action(Toggle::Sidebar.title(state.sidebar), ToggleSidebar),
            MenuItem::action("Compact List", ToggleDensity).checked(state.compact_list),
            MenuItem::separator(),
            MenuItem::action("Day", ShowDayView).checked(view(ViewKind::Day)),
            MenuItem::action("Week", ShowWeekView).checked(view(ViewKind::Week)),
            MenuItem::action("Month", ShowMonthView).checked(view(ViewKind::Month)),
            MenuItem::action("Week Numbers", ui::calendar::ToggleWeekNumbers)
                .checked(state.week_numbers),
            MenuItem::action("Calendars…", ToggleCalendarsMenu),
            MenuItem::separator(),
            MenuItem::action("Make Text Bigger", ZoomIn),
            MenuItem::action("Make Text Smaller", ZoomOut),
            MenuItem::action("Make Text Normal Size", ResetZoom),
            MenuItem::separator(),
            MenuItem::action("Command Palette", ToggleCommandPalette),
            MenuItem::separator(),
            MenuItem::action(
                Toggle::FullScreen.title(state.full_screen),
                ToggleFullScreen,
            ),
        ]),
        Menu::new("Mailbox").items([
            MenuItem::action("Inbox", GoToInbox),
            MenuItem::action("Starred", GoToStarred),
            MenuItem::action("Sent", GoToSent),
            MenuItem::action("Drafts", GoToDrafts),
            MenuItem::action("Archive", GoToArchive),
            MenuItem::action("Spam", GoToSpam),
            MenuItem::action("Trash", GoToTrash),
            MenuItem::separator(),
            MenuItem::action("Social", GoToSocial),
            MenuItem::action("Promotions", GoToPromotions),
            MenuItem::action("Updates", GoToUpdates),
            MenuItem::action("Forums", GoToForums),
            MenuItem::separator(),
            MenuItem::action("Go to Label…", GoToLabel),
            MenuItem::action("New Label…", NewLabel),
            MenuItem::action("Rename Label…", RenameLabel),
            MenuItem::action("Change Label Color…", ChangeLabelColor),
            MenuItem::action("Label Visibility…", ChangeLabelVisibility),
            MenuItem::action("Delete Label…", DeleteLabel),
            MenuItem::separator(),
            MenuItem::action("Sync Now", SyncNow),
            MenuItem::action("Load Older Conversations", LoadOlderConversations),
        ]),
        Menu::new("Message").items([
            MenuItem::action("Send", SendMessage),
            MenuItem::action("Show Cc and Bcc", ShowCcBcc),
            MenuItem::separator(),
            MenuItem::action("Reply", Reply),
            MenuItem::action("Reply All", ReplyAll),
            MenuItem::action("Forward", Forward),
            MenuItem::separator(),
            MenuItem::action("Archive", Archive),
            MenuItem::action("Delete", Delete),
            MenuItem::action("Move to…", MoveTo),
            MenuItem::action("Move to Category…", MoveToCategory),
            MenuItem::action("Label…", ApplyLabel),
            MenuItem::separator(),
            MenuItem::action(Toggle::Star.title(state.starred), ToggleStar),
            MenuItem::action(Toggle::Important.title(state.important), ToggleImportant),
            MenuItem::action("Mark as Read", MarkRead),
            MenuItem::action("Mark as Unread", MarkUnread),
            MenuItem::separator(),
            MenuItem::action("Load Images", LoadImages),
            MenuItem::action("Show Original", ShowOriginal),
            MenuItem::action("Show Contact", ShowContact),
            MenuItem::action("Open in Gmail", OpenInGmail),
        ]),
        Menu::new("Event").items([
            MenuItem::action("Today", GoToToday),
            MenuItem::action("Go to Date…", ui::calendar::GoToDate),
            MenuItem::action("Next Period", NextPeriod),
            MenuItem::action("Previous Period", PrevPeriod),
            MenuItem::separator(),
            MenuItem::action("Open Event", OpenEvent),
            MenuItem::action("Join Meeting", JoinEvent),
            MenuItem::action("Accept Invitation", AcceptEvent),
            MenuItem::action("Reply Maybe", MaybeEvent),
            MenuItem::action("Decline Invitation", DeclineEvent),
            MenuItem::separator(),
            MenuItem::action("Add Google Meet", AddGoogleMeet),
            MenuItem::action("Repeat…", SetRepeat),
            MenuItem::action("Add Reminder…", AddReminder),
            MenuItem::separator(),
            MenuItem::action("Open Location in Maps", OpenLocation),
            MenuItem::action("Copy Link to Event", CopyLink),
            MenuItem::action("Open in Google Calendar", OpenInGoogleCalendar),
            MenuItem::separator(),
            MenuItem::action("Save Event", SaveEvent),
            MenuItem::action("Discard Changes", DiscardEventChanges),
            MenuItem::action("Close Event", CloseEvent),
            MenuItem::action("Delete Event", DeleteEvent),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::separator(),
            MenuItem::action("Main Window", ShowMainWindow),
            MenuItem::separator(),
            MenuItem::action("Bring All to Front", BringAllToFront),
        ]),
        Menu::new("Help").items([MenuItem::action("Keyboard Shortcuts", ToggleShortcutsHelp)]),
    ]
}

/// The Dock icon's menu (MB-009); every item has a global handler, so it
/// works with no window open.
fn dock_menu() -> Vec<MenuItem> {
    vec![
        MenuItem::action("New Message", Compose),
        MenuItem::action("New Event", NewEvent),
        MenuItem::separator(),
        MenuItem::action("Sync Now", SyncNow),
        MenuItem::action("Join Next Meeting", JoinNextMeeting),
    ]
}

/// Quit once the queue has had its chance (2.7): sends inside their undo
/// window go now, and Quit waits up to 5 s for what's going. Works with no
/// window open too (NTF-3).
fn quit(backend: &Entity<Backend>, cx: &mut App) {
    let flush = backend.update(cx, |b, cx| {
        b.flush_settings();
        b.flush_on_quit(cx)
    });
    cx.spawn(async move |cx| {
        flush.await;
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// When Pigeon started, for the time to its first frame (MA-072).
pub static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn main() {
    STARTED.get_or_init(std::time::Instant::now);
    let dir = backend::data_dir();
    logging::init(&dir);
    // PER-5: one Pigeon per data folder; a second one hands over and goes.
    let _lock = match instance::acquire(&dir) {
        Ok(lock) => lock,
        Err(running) => {
            logging::info(&format!(
                "instance: Pigeon (pid {}) already runs on {}; handing over",
                running.pid.map_or("?".into(), |p| p.to_string()),
                dir.display()
            ));
            native::hand_over(&dir.to_string_lossy(), running.pid);
            return;
        }
    };
    let app = gpui::application().with_assets(assets::Assets);
    app.on_open_urls(shell::open_urls);
    // The Dock icon clicked, or Pigeon opened again (NTF-3).
    app.on_reopen(|cx| shell::show(Show::Main, cx));
    app.run(move |cx: &mut App| {
        gpui::init(cx);
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        // MB-008: works with no window open too.
        cx.on_action(|_: &About, _| native::show_about());
        cx.on_action(|_: &BringAllToFront, _| native::bring_all_to_front());
        let backend = Backend::open(cx);
        let quit_backend = backend.clone();
        cx.on_action(move |_: &Quit, cx| quit(&quit_backend, cx));
        // Closing the window leaves Pigeon running, in the Dock (NTF-3).
        shell::init(backend, cx);
        // The saved preset is known once the shell has read the settings.
        set_keymap(settings::get(cx).keymap, cx);
        // SYS-4: the MCP server, when it's on.
        mcp::init(cx);
        shell::show(Show::Main, cx);
        // W7b: the menu bar item, its panel and the reminders.
        shell::install_menu_bar(cx);
    });
}

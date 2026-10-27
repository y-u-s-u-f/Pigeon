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
mod native;
mod notify;
mod search;
mod settings;
mod theme;
mod ui;

use gpui::{App, Entity, Menu, MenuItem, OsAction, SystemMenuType, component::input};

use crate::{
    actions::*,
    app::shell::{self, Show},
    backend::Backend,
};

/// The standard macOS menus. Every item runs an existing action, so it shows
/// that action's shortcut, and is greyed out where the action can't run.
fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Pigeon").items([
            MenuItem::action("About Pigeon", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
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
            MenuItem::separator(),
            MenuItem::action("Sync Now", SyncNow),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Find…", ToggleSearch),
        ]),
        Menu::new("View").items([
            MenuItem::action("Mail", GoToMail),
            MenuItem::action("Calendar", GoToCalendar),
            MenuItem::separator(),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Compact List", ToggleDensity),
            MenuItem::separator(),
            MenuItem::action("Day", ShowDayView),
            MenuItem::action("Week", ShowWeekView),
            MenuItem::action("Month", ShowMonthView),
            MenuItem::action("Week Numbers", ui::calendar::ToggleWeekNumbers),
            MenuItem::separator(),
            MenuItem::action("Make Text Bigger", ZoomIn),
            MenuItem::action("Make Text Smaller", ZoomOut),
            MenuItem::action("Make Text Normal Size", ResetZoom),
        ]),
        Menu::new("Message").items([
            MenuItem::action("Reply", Reply),
            MenuItem::action("Reply All", ReplyAll),
            MenuItem::action("Forward", Forward),
            MenuItem::separator(),
            MenuItem::action("Archive", Archive),
            MenuItem::action("Delete", Delete),
            MenuItem::action("Move to…", MoveTo),
            MenuItem::action("Label…", ApplyLabel),
            MenuItem::separator(),
            MenuItem::action("Star / Unstar", ToggleStar),
            MenuItem::action("Mark as Read", MarkRead),
            MenuItem::action("Mark as Unread", MarkUnread),
        ]),
        Menu::new("Go").items([
            MenuItem::action("Inbox", GoToInbox),
            MenuItem::action("Starred", GoToStarred),
            MenuItem::action("Sent", GoToSent),
            MenuItem::action("Drafts", GoToDrafts),
            MenuItem::action("Archive", GoToArchive),
            MenuItem::action("Spam", GoToSpam),
            MenuItem::action("Trash", GoToTrash),
            MenuItem::action("Label…", GoToLabel),
            MenuItem::separator(),
            MenuItem::action("Today", GoToToday),
            MenuItem::action("Go to Date…", ui::calendar::GoToDate),
            MenuItem::action("Next Period", NextPeriod),
            MenuItem::action("Previous Period", PrevPeriod),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::separator(),
            MenuItem::action("Pigeon", ShowMainWindow),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Keyboard Shortcuts", ToggleShortcutsHelp),
            MenuItem::action("Command Palette", ToggleCommandPalette),
        ]),
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

fn main() {
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
        actions::bind_keys(cx);
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.set_menus(menus());
        let backend = Backend::open(cx);
        let quit_backend = backend.clone();
        cx.on_action(move |_: &Quit, cx| quit(&quit_backend, cx));
        // Closing the window leaves Pigeon running, in the Dock (NTF-3).
        shell::init(backend, cx);
        shell::show(Show::Main, cx);
    });
}

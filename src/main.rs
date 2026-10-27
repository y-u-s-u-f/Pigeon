mod actions;
mod app;
mod assets;
mod calendar;
mod data;
mod html;
mod markdown;
mod search;
mod settings;
mod theme;
mod ui;

use gpui::{
    App, AppContext as _, Bounds, Menu, MenuItem, OsAction, SystemMenuType, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, component::input, point, px, size,
};

use crate::{
    actions::*,
    app::{PigeonApp, RAIL_WIDTH},
    calendar::CalendarData,
    data::MailData,
    theme::Theme,
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
        ]),
        Menu::new("Help").items([
            MenuItem::action("Keyboard Shortcuts", ToggleShortcutsHelp),
            MenuItem::action("Command Palette", ToggleCommandPalette),
        ]),
    ]
}

fn main() {
    let data = MailData::load().expect("bundled mailbox.json should parse");
    let calendar = CalendarData::load().expect("bundled calendar.json should parse");

    gpui::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            gpui::init(cx);
            actions::bind_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_action(|_: &Hide, cx| cx.hide());
            cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
            cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
            cx.set_menus(menus());
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
            // The traffic lights (60pt wide on macOS 26) sit centered in the
            // rail, level with the middle of the pane headers.
            let lights_x = (RAIL_WIDTH - px(60.)) / 2.;
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(900.), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Pigeon".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(lights_x, px(19.))),
                }),
                // Panes move the window themselves (see `ui::window_drag_area`).
                app_owns_titlebar_drag: true,
                window_background: WindowBackgroundAppearance::Blurred,
                ..Default::default()
            };

            gpui::open_window(options, cx, |window, cx| {
                Theme::sync(window, cx);
                cx.new(|cx| PigeonApp::new(data, calendar, window, cx))
            })
            .expect("failed to open the main window");
            cx.activate(true);
        });
}

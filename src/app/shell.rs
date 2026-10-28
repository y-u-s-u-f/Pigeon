//! The app around the window (sprint 2 W1, NTF-3). Pigeon keeps running
//! with its window closed; `ensure_main_window` is the one way to the main
//! window, open or not, and `show` the one way to put something in it: the
//! Dock, notification clicks, the menu bar, `mailto:` and a second launch
//! all go through them. App-level handlers run Compose, New Event, Open
//! Thread and the rest of the menus with no window open. Also here: what
//! the backend reports app-wide (new-mail notifications, the Dock badge),
//! wake from sleep, and the system's display options (MA-009, MA-010).

use std::cell::RefCell;

use chrono::NaiveDateTime;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, DisplayId, Entity, Global, Pixels,
    Size, Task, TitlebarOptions, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, point, px, size,
};

use super::{Mode, PigeonApp, RAIL_WIDTH};
use crate::{
    actions::*,
    backend::{
        self, Backend, BackendEvent, SyncStatus, categories::Category, http::percent_decode,
    },
    logging, native,
    notify::{self, NoticeTarget},
    settings::{self, DockBadge, Saved, TimeFormat, WindowFrame},
    theme::{Display, Theme},
};

/// A `mailto:` link's fields (LNK-3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mailto {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
}

impl Mailto {
    /// RFC 6068: addresses before `?`, then `to`, `cc`, `bcc`, `subject`
    /// and `body` fields, percent-decoded; `+` stays a plus. Other fields
    /// are ignored.
    pub fn parse(url: &str) -> Option<Self> {
        let scheme = url.get(..7)?;
        if !scheme.eq_ignore_ascii_case("mailto:") {
            return None;
        }
        let rest = &url[7..];
        let (to, query) = rest.split_once('?').unwrap_or((rest, ""));
        let mut out = Self::default();
        add_addresses(&mut out.to, to);
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match percent_decode(key).to_ascii_lowercase().as_str() {
                "to" => add_addresses(&mut out.to, value),
                "cc" => add_addresses(&mut out.cc, value),
                "bcc" => add_addresses(&mut out.bcc, value),
                "subject" if out.subject.is_empty() => out.subject = percent_decode(value),
                "body" if out.body.is_empty() => {
                    out.body = percent_decode(value).replace("\r\n", "\n")
                }
                _ => {}
            }
        }
        Some(out)
    }
}

/// The comma-separated addresses in `field`, decoded, onto `list`.
fn add_addresses(list: &mut Vec<String>, field: &str) {
    let decoded = percent_decode(field);
    list.extend(
        decoded
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(String::from),
    );
}

/// What to bring up in the main window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Show {
    Main,
    Thread(String),
    Event {
        calendar: String,
        id: String,
        start: NaiveDateTime,
    },
    Compose(Option<Mailto>),
    NewEvent,
    Settings,
    /// Mail's search, holding this text (MB-063).
    Search(String),
    /// Calendar, on today (MB-063).
    Today,
}

/// A `pigeon://` link (MB-063), for Shortcuts, Raycast, Alfred and the
/// like. Any web page can open one, so each only shows or fills in
/// something; nothing is sent, saved or changed.
///
/// - `pigeon://compose[/addresses][?to=&cc=&bcc=&subject=&body=]`, as `mailto:`
/// - `pigeon://thread/<id>`
/// - `pigeon://search?q=<query>`
/// - `pigeon://event/new`
/// - `pigeon://today`
pub fn parse_pigeon_url(url: &str) -> Option<Show> {
    const SCHEME: &str = "pigeon://";
    if !url.get(..SCHEME.len())?.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let rest = &url[SCHEME.len()..];
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, tail) = path.split_once('/').unwrap_or((path, ""));
    let tail = tail.trim_end_matches('/');
    let field = |name: &str| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key) == name).then(|| percent_decode(&value.replace('+', " ")))
        })
    };
    match (host.to_ascii_lowercase().as_str(), tail) {
        ("compose", to) => Mailto::parse(&format!("mailto:{to}?{query}"))
            .map(|m| Show::Compose((m != Mailto::default()).then_some(m))),
        ("thread", id) if !id.is_empty() && !id.contains('/') => {
            Some(Show::Thread(percent_decode(id)))
        }
        ("search", "") => Some(Show::Search(field("q").unwrap_or_default())),
        ("event", "new") => Some(Show::NewEvent),
        ("today", "") => Some(Show::Today),
        _ => None,
    }
}

/// The main window: its handle, and its view while it's open.
#[derive(Clone)]
pub struct MainWindow {
    pub window: AnyWindowHandle,
    pub app: WeakEntity<PigeonApp>,
}

impl MainWindow {
    /// Run `f` on the view, if the window is still open.
    pub fn update<R>(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut PigeonApp, &mut Window, &mut Context<PigeonApp>) -> R,
    ) -> Option<R> {
        let app = self.app.upgrade()?;
        self.window
            .update(cx, |_, window, cx| {
                app.update(cx, |this, cx| f(this, window, cx))
            })
            .ok()
    }
}

struct Shell {
    backend: Entity<Backend>,
    main: Option<MainWindow>,
    /// The Dock badge's count as last set.
    badge: Option<u64>,
    /// The Transparency slider's pending save, replaced on every move.
    glass_save: Option<Task<()>>,
}

impl Global for Shell {}

type UrlSink = Box<dyn Fn(Vec<String>)>;

thread_local! {
    /// `mailto:` URLs that came before the app was ready, and where they
    /// go once it is.
    static URLS: RefCell<(Vec<String>, Option<UrlSink>)> =
        RefCell::new((Vec::new(), None));
}

/// For `Application::on_open_urls`, which may run before launch finishes.
pub fn open_urls(urls: Vec<String>) {
    URLS.with(|cell| {
        let mut cell = cell.borrow_mut();
        match &cell.1 {
            Some(sink) => sink(urls),
            None => cell.0.extend(urls),
        }
    });
}

/// Set the shell up on `backend`: settings, notifications, the badge, the
/// app-level handlers, wake and hand-over. Once, at launch, before
/// `show(Show::Main)`.
pub fn init(backend: Entity<Backend>, cx: &mut App) {
    let saved = Saved::from_map(backend.read(cx).load_settings());
    settings::remember(&saved, cx);
    cx.set_global(Display::read());
    apply_time_format(saved.time_format);
    backend.update(cx, |b, _| b.set_notifications(saved.notifications));
    cx.set_global(Shell {
        backend: backend.clone(),
        main: None,
        badge: None,
        glass_save: None,
    });
    notify::init(cx);
    cx.subscribe(&backend, on_backend_event).detach();

    // With no window open (or none taking them), the menus still work.
    cx.on_action(|_: &Compose, cx| show(Show::Compose(None), cx));
    cx.on_action(|_: &NewEvent, cx| show(Show::NewEvent, cx));
    cx.on_action(|_: &OpenThread, cx| show(Show::Main, cx));
    cx.on_action(|_: &ShowMainWindow, cx| show(Show::Main, cx));
    cx.on_action(|_: &OpenSettings, cx| show(Show::Settings, cx));
    cx.on_action(|_: &SyncNow, cx| {
        backend_of(cx).update(cx, |b, cx| b.sync_now(cx));
    });
    cx.on_action(|_: &SendTestNotification, cx| notify::send_test(cx));
    cx.on_action(|_: &ToggleNotifications, cx| {
        let on = !settings::get(cx).notifications;
        set_notifications(on, cx);
    });
    cx.on_action(|_: &ToggleEventReminders, cx| {
        change_app_settings(cx, |s| s.event_reminders = !s.event_reminders);
        cx.refresh_windows();
    });
    cx.on_action(|_: &MakeDefaultMailApp, cx| make_default_mail_app(cx));
    cx.on_action(|_: &ToggleOpenAtLogin, cx| {
        let on = !matches!(
            login_item(),
            native::LoginItem::On | native::LoginItem::NeedsApproval
        );
        set_open_at_login(on, cx);
    });

    cx.on_system_wake(|cx| {
        logging::info("wake: syncing now");
        backend_of(cx).update(cx, |b, cx| b.wake(cx));
    })
    .detach();

    // The rest come from AppKit, outside gpui's callbacks: hop onto the
    // foreground executor to reach the app.
    let async_cx = cx.to_async();
    let executor = cx.foreground_executor().clone();
    native::listen(&backend::data_dir().to_string_lossy(), move |signal| {
        let cx = async_cx.clone();
        executor
            .spawn(async move { cx.update(|cx| on_signal(signal, cx)) })
            .detach();
    });
    let async_cx = cx.to_async();
    let executor = cx.foreground_executor().clone();
    let pending = URLS.with(|cell| {
        let mut cell = cell.borrow_mut();
        cell.1 = Some(Box::new(move |urls| {
            let cx = async_cx.clone();
            executor
                .spawn(async move { cx.update(|cx| open_urls_now(urls, cx)) })
                .detach();
        }));
        std::mem::take(&mut cell.0)
    });
    if !pending.is_empty() {
        // After the main window opens, at launch.
        cx.defer(move |cx| open_urls_now(pending, cx));
    }
    update_badge(cx);
}

pub(crate) fn backend_of(cx: &App) -> Entity<Backend> {
    cx.global::<Shell>().backend.clone()
}

fn on_signal(signal: native::Signal, cx: &mut App) {
    match signal {
        native::Signal::ShowRequested => {
            logging::info("instance: opened again; showing the window");
            show(Show::Main, cx);
        }
        native::Signal::LocaleChanged => {
            apply_time_format(settings::get(cx).time_format);
            cx.refresh_windows();
        }
        native::Signal::TimeZoneChanged => {
            logging::info("time zone changed");
            crate::calendar::time::reset_zone_name();
            cx.refresh_windows();
        }
        native::Signal::DisplayChanged => {
            let display = Display::read();
            if cx.try_global::<Display>() != Some(&display) {
                logging::info(&format!("display options: {display:?}"));
                cx.set_global(display);
                refresh_theme(cx);
            }
        }
    }
}

/// Re-apply the theme in every window, after the display options or the
/// glass changed. Not from inside a window's update: that window is skipped.
fn refresh_theme(cx: &mut App) {
    let appearance = settings::get(cx).appearance;
    for window in cx.windows() {
        window
            .update(cx, |_, window, cx| Theme::sync_for(appearance, window, cx))
            .ok();
    }
    cx.refresh_windows();
}

/// MC-094: change one app-wide setting, remember it and save it.
pub fn change_app_settings(cx: &mut App, change: impl FnOnce(&mut Saved)) {
    let mut saved = settings::current(cx);
    change(&mut saved);
    settings::remember(&saved, cx);
    backend_of(cx).update(cx, |b, cx| b.save_settings(saved.to_map(), cx));
}

/// Settings › Appearance › Transparency (MA-010): `Saved::glass`, 0–100.
/// Re-themes at once; saves once the slider has rested a moment, not on
/// every drag tick.
pub fn set_glass(glass: u8, cx: &mut App) {
    let mut saved = settings::current(cx);
    saved.glass = glass.min(100);
    settings::remember(&saved, cx);
    // After the update this came from, when every window can be reached.
    cx.defer(refresh_theme);
    let save = cx.spawn(async |cx| {
        cx.background_executor().timer(GLASS_SAVE_DELAY).await;
        cx.update(|cx| change_app_settings(cx, |_| {}));
    });
    cx.global_mut::<Shell>().glass_save = Some(save);
}

const GLASS_SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(400);

/// Times show on the clock `format` asks for, `System` reading macOS's now.
fn apply_time_format(format: TimeFormat) {
    crate::calendar::time::set_24_hour(format.is_24_hour(native::system_uses_24_hour));
}

/// Settings › General › Time format, for mail and calendar alike.
pub fn set_time_format(format: TimeFormat, cx: &mut App) {
    change_app_settings(cx, |s| s.time_format = format);
    apply_time_format(format);
    cx.refresh_windows();
}

/// `mailto:` and `pigeon://` links (LNK-3, MB-063).
fn open_urls_now(urls: Vec<String>, cx: &mut App) {
    for url in urls {
        // Not the whole link: it can hold a message's text.
        logging::info(&format!(
            "open url: {}:",
            url.split(':').next().unwrap_or_default()
        ));
        let what = match Mailto::parse(&url) {
            Some(mailto) => Some(Show::Compose(Some(mailto))),
            None => parse_pigeon_url(&url),
        };
        match what {
            Some(what) => show(what, cx),
            None => logging::info("open url: not one Pigeon knows"),
        }
    }
}

/// The main window, opened if it isn't, and brought forward.
pub fn ensure_main_window(cx: &mut App) -> MainWindow {
    if let Some(main) = cx.global::<Shell>().main.clone()
        && main.app.upgrade().is_some()
        && cx.windows().contains(&main.window)
    {
        // Fails while the window is mid-update (an action from it reached an
        // app-level handler); it's open all the same, so no second one.
        main.window
            .update(cx, |_, window, _| window.activate_window())
            .ok();
        return main;
    }
    let backend = backend_of(cx);
    let saved = settings::current(cx);
    let (window, app) = gpui::open_window(window_options(cx), cx, |window, cx| {
        Theme::sync(window, cx);
        let app = cx.new(|cx| PigeonApp::new(backend, saved, window, cx));
        let weak = app.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| this.before_close(cx)).ok();
            true
        });
        app
    })
    .expect("failed to open the main window");
    let main = MainWindow {
        window,
        app: app.downgrade(),
    };
    cx.global_mut::<Shell>().main = Some(main.clone());
    main
}

/// Bring `what` up in the main window, opening it if need be.
pub fn show(what: Show, cx: &mut App) {
    if let Err(what) = show_now(what, cx) {
        // The main window is mid-update: once it's done.
        cx.defer(move |cx| {
            if show_now(what, cx).is_err() {
                logging::info("window: couldn't reach the main window");
            }
        });
    }
}

/// Calls into the main window's views directly: an action dispatched there
/// that nothing takes falls through to the app-level handlers in `init`,
/// which would come back here.
fn show_now(what: Show, cx: &mut App) -> Result<(), Show> {
    let main = ensure_main_window(cx);
    cx.activate(true);
    let mut what = Some(what);
    main.update(cx, |this, window, cx| {
        if let Some(what) = what.take() {
            this.show(what, window, cx);
        }
    });
    match what {
        Some(what) => Err(what),
        None => Ok(()),
    }
}

/// The smallest the main window gets.
const WINDOW_MIN: Size<Pixels> = size(px(900.), px(560.));
/// How much of a saved frame must still be on a display to reopen there.
const WINDOW_ON_SCREEN: Pixels = px(100.);

/// The window's frame now, to save (MA-034).
pub fn window_frame(window: &Window, cx: &App) -> WindowFrame {
    let (bounds, full_screen) = match window.window_bounds() {
        WindowBounds::Windowed(b) | WindowBounds::Maximized(b) => (b, false),
        WindowBounds::Fullscreen(b) => (b, true),
    };
    WindowFrame {
        x: bounds.origin.x.into(),
        y: bounds.origin.y.into(),
        width: bounds.size.width.into(),
        height: bounds.size.height.into(),
        full_screen,
        display: window
            .display(cx)
            .and_then(|d| d.uuid().ok())
            .map(|uuid| uuid.to_string()),
    }
}

/// The saved frame on its display, or the main one, fitted inside it;
/// none when too little of it would show there.
fn restored_frame(frame: &WindowFrame, cx: &App) -> Option<(WindowBounds, DisplayId)> {
    let display = cx
        .displays()
        .into_iter()
        .find(|d| {
            frame.display.is_some() && d.uuid().ok().map(|uuid| uuid.to_string()) == frame.display
        })
        .or_else(|| cx.primary_display())?;
    let bounds = fit_frame(frame, display.bounds().size)?;
    let bounds = if frame.full_screen {
        WindowBounds::Fullscreen(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    };
    Some((bounds, display.id()))
}

/// `frame`, relative to a display `area` big, moved and shrunk to fit it;
/// none when less than `WINDOW_ON_SCREEN` of it overlaps it each way.
fn fit_frame(frame: &WindowFrame, area: Size<Pixels>) -> Option<Bounds<Pixels>> {
    let saved = Bounds::new(
        point(px(frame.x), px(frame.y)),
        size(px(frame.width), px(frame.height)),
    );
    let overlap = saved.intersect(&Bounds::new(point(px(0.), px(0.)), area));
    if overlap.size.width < WINDOW_ON_SCREEN || overlap.size.height < WINDOW_ON_SCREEN {
        return None;
    }
    let width = saved.size.width.max(WINDOW_MIN.width).min(area.width);
    let height = saved.size.height.max(WINDOW_MIN.height).min(area.height);
    let x = saved.origin.x.min(area.width - width).max(px(0.));
    let y = saved.origin.y.min(area.height - height).max(px(0.));
    Some(Bounds::new(point(x, y), size(width, height)))
}

fn window_options(cx: &mut App) -> WindowOptions {
    // MA-034: where it was left, else centered on the main display.
    let restored = settings::get(cx)
        .window
        .clone()
        .and_then(|frame| restored_frame(&frame, cx));
    let (window_bounds, display_id) = match restored {
        Some((bounds, display)) => (bounds, Some(display)),
        None => (
            WindowBounds::Windowed(Bounds::centered(None, size(px(1360.), px(860.)), cx)),
            None,
        ),
    };
    WindowOptions {
        window_bounds: Some(window_bounds),
        display_id,
        window_min_size: Some(WINDOW_MIN),
        titlebar: Some(TitlebarOptions {
            title: Some("Pigeon".into()),
            appears_transparent: true,
            traffic_light_position: Some(traffic_lights()),
        }),
        // Panes move the window themselves (see `ui::window_drag_area`).
        app_owns_titlebar_drag: true,
        window_background: window_background(cx),
        ..Default::default()
    }
}

/// Where the traffic lights sit: centered in the rail (they're 60pt wide
/// on macOS 26), level with the middle of the pane headers. Draft windows
/// put them in the same place (MB-047).
pub fn traffic_lights() -> gpui::Point<Pixels> {
    point((RAIL_WIDTH - px(60.)) / 2., px(19.))
}

/// Glass, or opaque under Reduce Transparency, Increase Contrast or a
/// glass of 100, for a window about to open; `Theme::apply` keeps it in step.
pub fn window_background(cx: &App) -> WindowBackgroundAppearance {
    let display = cx.try_global::<Display>().copied().unwrap_or_default();
    let opaque =
        display.reduce_transparency || display.increase_contrast || settings::get(cx).glass >= 100;
    if opaque {
        WindowBackgroundAppearance::Opaque
    } else {
        WindowBackgroundAppearance::Blurred
    }
}

/// The menu bar, its model and the reminders (W7b). Once, at launch, after
/// the main window opened (it sets the theme).
pub fn install_menu_bar(cx: &mut App) {
    let today = super::today::init(backend_of(cx), cx);
    super::menu_bar::install(today, cx);
}

/// Settings › Notifications and ⌘K: on or off, app-wide.
pub fn set_notifications(on: bool, cx: &mut App) {
    change_app_settings(cx, |s| s.notifications = on);
    logging::info(&format!(
        "notify: new mail notifications {}",
        if on { "on" } else { "off" }
    ));
    backend_of(cx).update(cx, |b, _| b.set_notifications(on));
}

/// W3: Settings › Mail › Signatures (MST-2).
pub fn set_signatures(on: bool, cx: &mut App) {
    change_app_settings(cx, |s| s.signatures = on);
    cx.refresh_windows();
}

/// W3: Settings › General and ⌘K (LNK-3). macOS may ask to confirm. A
/// failure stays in Settings' row and shows as a toast (ME-074).
pub fn make_default_mail_app(cx: &mut App) {
    let result = native::make_default_mail_app();
    logging::info(&format!("default mail app: {result:?}"));
    refresh_system_items();
    let error = result.err();
    SYSTEM.with(|s| s.borrow_mut().mail_error = error.clone());
    if let Some(error) = error
        && let Some(main) = cx.global::<Shell>().main.clone()
    {
        // Called from inside the main window's update: show it after.
        cx.defer(move |cx| {
            main.update(cx, |this, _, cx| {
                this.show_error_toast(
                    format!("Pigeon couldn't become the default mail app. {error}"),
                    cx,
                )
            });
        });
    }
    cx.refresh_windows();
}

/// Settings › General and ⌘K (NTF-6). Needing approval opens System
/// Settings where it's given.
pub fn set_open_at_login(on: bool, cx: &mut App) {
    if login_item() == native::LoginItem::NeedsApproval && on {
        native::open_login_items_settings();
        return;
    }
    let result = native::set_login_item(on);
    logging::info(&format!(
        "login item: {} → {result:?}",
        if on { "on" } else { "off" }
    ));
    refresh_system_items();
    SYSTEM.with(|s| s.borrow_mut().login_error = result.err());
    if on && login_item() == native::LoginItem::NeedsApproval {
        native::open_login_items_settings();
    }
    cx.refresh_windows();
}

/// The login item and the mail handler as macOS last said, and what went
/// wrong changing them (MC-038). Asking macOS goes through XPC and Launch
/// Services, so it happens on first use, after a change and each time
/// Pigeon comes forward (`refresh_system_items`), not every frame.
#[derive(Default)]
struct SystemItems {
    login_item: Option<native::LoginItem>,
    mail_handler: Option<native::MailHandler>,
    login_error: Option<String>,
    mail_error: Option<String>,
}

thread_local! {
    static SYSTEM: RefCell<SystemItems> = RefCell::new(SystemItems::default());
}

/// Ask macOS again the next time they're read: after a change, and when
/// Pigeon comes forward (approval in System Settings shows then).
pub fn refresh_system_items() {
    SYSTEM.with(|s| {
        let mut s = s.borrow_mut();
        s.login_item = None;
        s.mail_handler = None;
    });
}

/// Pigeon's login item, as macOS last said.
pub fn login_item() -> native::LoginItem {
    SYSTEM.with(|s| {
        s.borrow_mut()
            .login_item
            .get_or_insert_with(native::login_item)
            .clone()
    })
}

/// Who opens `mailto:` links, as macOS last said.
pub fn mail_handler() -> native::MailHandler {
    SYSTEM.with(|s| {
        s.borrow_mut()
            .mail_handler
            .get_or_insert_with(native::mail_handler)
            .clone()
    })
}

/// Why "Make Default" last failed, until it works (ME-074).
pub fn default_mail_app_error() -> Option<String> {
    SYSTEM.with(|s| s.borrow().mail_error.clone())
}

/// What Settings › General says about the login item, as macOS has it.
pub fn login_item_status() -> String {
    if let Some(error) = SYSTEM.with(|s| s.borrow().login_error.clone()) {
        return format!("Couldn't change it. {error}");
    }
    match login_item() {
        native::LoginItem::Unavailable(why) => why.to_string(),
        native::LoginItem::On => "On: Pigeon opens when you log in.".into(),
        native::LoginItem::Off => "Off.".into(),
        native::LoginItem::NeedsApproval => {
            "Needs your approval in System Settings › General › Login Items.".into()
        }
    }
}

fn on_backend_event(backend: Entity<Backend>, event: &BackendEvent, cx: &mut App) {
    match event {
        BackendEvent::NewMail(new) => {
            if !settings::get(cx).notifications {
                logging::info(&format!(
                    "notify: {} new, not shown (notifications off)",
                    new.len()
                ));
                return;
            }
            let new: Vec<_> = new
                .iter()
                .filter(|m| !in_view(&m.thread_id, cx))
                .cloned()
                .collect();
            notify::notify_new_mail(&new, cx);
        }
        BackendEvent::ThreadsChanged(ids) | BackendEvent::ThreadsRemoved(ids) => {
            for (tag, threads) in notify::showing_for(ids, cx) {
                if backend.read(cx).threads_done(&threads) {
                    notify::dismiss_tag(&tag, cx);
                }
            }
            update_badge(cx);
        }
        BackendEvent::CountsChanged
        | BackendEvent::OpCommitted(_)
        | BackendEvent::Status(_)
        | BackendEvent::SignedIn
        | BackendEvent::SignedOut => update_badge(cx),
        _ => {}
    }
}

/// Whether the main window is frontmost on Mail and its view shows the
/// thread: no notification for it there (MB-004). Elsewhere, or with the
/// window in the background or closed, one shows.
fn in_view(thread_id: &str, cx: &App) -> bool {
    let Some(main) = cx.global::<Shell>().main.clone() else {
        return false;
    };
    if cx.active_window() != Some(main.window) {
        return false;
    }
    let Some(app) = main.app.upgrade() else {
        return false;
    };
    let app = app.read(cx);
    if app.mode != Mode::Mail || app.settings_page.open {
        return false;
    }
    match app.data.thread_ix(thread_id) {
        Some(ix) => {
            app.selected == Some(ix)
                || app.rows.contains(&super::Row::Thread(ix))
                || app.nav_contains(app.nav, &app.data.threads[ix])
        }
        // Not in the window's list yet: as the store has it.
        None => app
            .backend
            .read(cx)
            .load_threads(&[thread_id.to_string()])
            .first()
            .is_some_and(|t| app.nav_contains(app.nav, t)),
    }
}

/// Settings › Notifications › Dock badge (MB-055).
pub fn set_dock_badge(badge: DockBadge, cx: &mut App) {
    change_app_settings(cx, |s| s.dock_badge = badge);
    update_badge(cx);
}

/// NTF-2: the unread count on the Dock icon, as Settings asks (MB-055).
fn update_badge(cx: &mut App) {
    let backend = backend_of(cx).read(cx);
    // MB-054: stale counts don't stand while the sign-in has expired.
    let expired = *backend.status() == SyncStatus::AuthExpired;
    let counts = backend.category_counts();
    let count = match settings::get(cx).dock_badge {
        _ if expired => 0,
        DockBadge::Off => 0,
        DockBadge::Primary => counts.unread(Category::Primary).into(),
        DockBadge::AllInbox => Category::ALL
            .iter()
            .map(|&c| u64::from(counts.unread(c)))
            .sum(),
    };
    let shell = cx.global_mut::<Shell>();
    if shell.badge != Some(count) {
        shell.badge = Some(count);
        logging::info(&format!("badge: {count}"));
        native::set_dock_badge(count);
    }
}

impl PigeonApp {
    /// The window is closing: open drafts go to Drafts, the settings are
    /// written, and sync polls as in the background.
    pub(super) fn before_close(&mut self, cx: &mut Context<Self>) {
        self.stash_composer(cx);
        self.close_draft_windows(cx);
        self.save_settings(cx);
        self.backend.update(cx, |b, cx| {
            b.flush_settings();
            b.set_window_active(false, cx);
        });
        logging::info("window: closed");
    }

    fn show(&mut self, what: Show, window: &mut Window, cx: &mut Context<Self>) {
        match what {
            Show::Main => {}
            Show::Thread(id) => {
                self.set_mode(Mode::Mail, window, cx);
                if let Some(ix) = self.data.thread_ix(&id) {
                    self.reveal_thread(ix, window, cx);
                }
            }
            Show::Event { calendar, id, .. } => {
                self.set_mode(Mode::Calendar, window, cx);
                self.calendar.update(cx, |c, cx| {
                    if let Some(ix) = c
                        .data
                        .events
                        .iter()
                        .position(|e| e.id == id && e.calendar == calendar)
                    {
                        c.reveal_event(ix, window, cx);
                    }
                });
            }
            Show::Compose(None) => self.compose_new(&Compose, window, cx),
            // W3 (LNK-3): the link's fields fill the draft.
            Show::Compose(Some(mailto)) => self.compose_mailto(mailto, window, cx),
            Show::NewEvent => {
                self.set_mode(Mode::Calendar, window, cx);
                cx.on_next_frame(window, |this, window, cx| {
                    this.calendar
                        .update(cx, |c, cx| c.new_event(&NewEvent, window, cx));
                });
            }
            Show::Settings => {
                if !self.settings_page.open {
                    self.toggle_settings(&OpenSettings, window, cx);
                }
            }
            // MB-063: shown, not saved to the recent searches.
            Show::Search(text) => {
                self.close_overlays(window, cx);
                self.set_mode(Mode::Mail, window, cx);
                self.search_open = true;
                self.set_search_text(&text, window, cx);
                self.search_input
                    .update(cx, |input, cx| input.focus(window, cx));
            }
            Show::Today => {
                self.set_mode(Mode::Calendar, window, cx);
                self.calendar.update(cx, |c, cx| {
                    c.go_to_date(crate::ui::calendar::today(), window, cx)
                });
            }
        }
    }
}

/// The target a notice for `show` would have; for W5 and W7.
pub fn target_show(target: NoticeTarget) -> Show {
    match target {
        NoticeTarget::Thread { id } => Show::Thread(id),
        NoticeTarget::Event {
            calendar,
            id,
            start,
        } => Show::Event {
            calendar,
            id,
            start,
        },
        NoticeTarget::Inbox => Show::Main,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailto_takes_its_addresses() {
        assert_eq!(
            Mailto::parse("mailto:a@x.com,b@y.com?subject=Hi")
                .unwrap()
                .to,
            ["a@x.com", "b@y.com"]
        );
        assert!(Mailto::parse("https://x.com").is_none());
    }

    /// MA-034: a saved frame comes back where it was, fitted to its display,
    /// or not at all when it would be (nearly) off screen.
    #[test]
    fn saved_frames_fit_their_display() {
        let frame = |x: f32, y: f32, width: f32, height: f32| WindowFrame {
            x,
            y,
            width,
            height,
            full_screen: false,
            display: None,
        };
        let area = size(px(1440.), px(900.));
        let fit = |f: WindowFrame| {
            fit_frame(&f, area).map(|b| {
                (
                    f32::from(b.origin.x),
                    f32::from(b.origin.y),
                    f32::from(b.size.width),
                    f32::from(b.size.height),
                )
            })
        };
        assert_eq!(
            fit(frame(40., 30., 1200., 800.)),
            Some((40., 30., 1200., 800.))
        );
        // Partly off the right edge: moved in.
        assert_eq!(
            fit(frame(1000., 100., 1200., 700.)),
            Some((240., 100., 1200., 700.))
        );
        // Bigger than the display: shrunk to it; smaller than the minimum:
        // grown to it.
        assert_eq!(
            fit(frame(0., 0., 3000., 2000.)),
            Some((0., 0., 1440., 900.))
        );
        assert_eq!(
            fit(frame(10., 10., 400., 300.)),
            Some((10., 10., 900., 560.))
        );
        // On a display that's gone, or barely on this one.
        assert_eq!(fit(frame(2000., 100., 1200., 800.)), None);
        assert_eq!(fit(frame(1400., 100., 1200., 800.)), None);
    }

    /// W3 (LNK-3): RFC 6068's fields and its edge cases.
    #[test]
    fn mailto_fields_per_rfc_6068() {
        let m = Mailto::parse(
            "MAILTO:ana%40x.com?cc=bo@x.com,%20cy@x.com&bcc=dee@x.com&to=eve@x.com\
             &subject=Caf%C3%A9%20%26%20cake%3F&body=Line%201%0D%0ALine+2&x-other=1",
        )
        .unwrap();
        assert_eq!(m.to, ["ana@x.com", "eve@x.com"]);
        assert_eq!(m.cc, ["bo@x.com", "cy@x.com"]);
        assert_eq!(m.bcc, ["dee@x.com"]);
        assert_eq!(m.subject, "Café & cake?");
        // CRLF becomes a newline; `+` is a plus, not a space.
        assert_eq!(m.body, "Line 1\nLine+2");

        // No address, only fields; a name with its address; empty parts.
        let m = Mailto::parse("mailto:?to=%22Ana%20Lima%22%20%3Cana@x.com%3E&subject=").unwrap();
        assert_eq!(m.to, ["\"Ana Lima\" <ana@x.com>"]);
        assert_eq!(m.subject, "");
        let m = Mailto::parse("mailto:,,a@x.com,?&&body=hi%").unwrap();
        assert_eq!(m.to, ["a@x.com"]);
        assert_eq!(m.body, "hi%", "a broken escape stays");
        assert_eq!(Mailto::parse("mailto:").unwrap(), Mailto::default());
        assert!(Mailto::parse("mail").is_none());
    }

    /// MB-063: each `pigeon://` link shows or fills in, and nothing else.
    #[test]
    fn pigeon_urls() {
        let parse = parse_pigeon_url;
        assert_eq!(parse("pigeon://compose"), Some(Show::Compose(None)));
        assert_eq!(
            parse("PIGEON://compose/ana@x.com?subject=Hi%20there&body=a+b"),
            Some(Show::Compose(Some(Mailto {
                to: vec!["ana@x.com".into()],
                subject: "Hi there".into(),
                body: "a+b".into(),
                ..Default::default()
            })))
        );
        assert_eq!(
            parse("pigeon://thread/18c2f%2Fa"),
            Some(Show::Thread("18c2f/a".into()))
        );
        assert_eq!(parse("pigeon://thread/"), None);
        assert_eq!(
            parse("pigeon://search?q=from%3Aana+is%3Aunread"),
            Some(Show::Search("from:ana is:unread".into()))
        );
        assert_eq!(parse("pigeon://search"), Some(Show::Search(String::new())));
        assert_eq!(parse("pigeon://event/new"), Some(Show::NewEvent));
        assert_eq!(parse("pigeon://today/"), Some(Show::Today));
        assert_eq!(parse("pigeon://send?to=a@x.com"), None);
        assert_eq!(parse("pigeon://event/delete"), None);
        assert_eq!(parse("https://x.com"), None);
        assert_eq!(parse("pig"), None);
    }
}

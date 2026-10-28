//! The app around the window (sprint 2 W1, NTF-3). Pigeon keeps running
//! with its window closed; `ensure_main_window` is the one way to the main
//! window, open or not, and `show` the one way to put something in it: the
//! Dock, notification clicks, the menu bar, `mailto:` and a second launch
//! all go through them. App-level handlers run Compose, New Event, Open
//! Thread and the rest of the menus with no window open. Also here: what
//! the backend reports app-wide (new-mail notifications, the Dock badge)
//! and wake from sleep.

use std::cell::RefCell;

use chrono::NaiveDateTime;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, Global, TitlebarOptions,
    WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, point, px, size,
};

use super::{Mode, PigeonApp, RAIL_WIDTH};
use crate::{
    actions::*,
    backend::{self, Backend, BackendEvent},
    logging, native,
    notify::{self, NoticeTarget},
    settings::{self, Saved, TimeFormat},
    theme::Theme,
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

/// `%XX` escapes as UTF-8; a broken escape stays as written.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
    settings::remember(saved, cx);
    apply_time_format(saved.time_format);
    backend.update(cx, |b, _| b.set_notifications(saved.notifications));
    cx.set_global(Shell {
        backend: backend.clone(),
        main: None,
        badge: None,
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
        let on = !settings::current(cx).notifications;
        set_notifications(on, cx);
    });
    cx.on_action(|_: &MakeDefaultMailApp, cx| make_default_mail_app(cx));
    cx.on_action(|_: &ToggleOpenAtLogin, cx| {
        let on = !matches!(
            native::login_item(),
            native::LoginItem::On | native::LoginItem::NeedsApproval
        );
        set_open_at_login(on, cx);
    });

    // Wake and hand-over come from AppKit, outside gpui's callbacks: hop
    // onto the foreground executor to reach the app.
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
                .spawn(async move { cx.update(|cx| open_mailto(urls, cx)) })
                .detach();
        }));
        std::mem::take(&mut cell.0)
    });
    if !pending.is_empty() {
        // After the main window opens, at launch.
        cx.defer(move |cx| open_mailto(pending, cx));
    }
    update_badge(cx);
}

fn backend_of(cx: &App) -> Entity<Backend> {
    cx.global::<Shell>().backend.clone()
}

fn on_signal(signal: native::Signal, cx: &mut App) {
    match signal {
        native::Signal::Wake => {
            logging::info("wake: syncing now");
            backend_of(cx).update(cx, |b, cx| b.wake(cx));
        }
        native::Signal::ShowRequested => {
            logging::info("instance: opened again; showing the window");
            show(Show::Main, cx);
        }
        native::Signal::LocaleChanged => {
            apply_time_format(settings::current(cx).time_format);
            cx.refresh_windows();
        }
    }
}

/// Times show on the clock `format` asks for, `System` reading macOS's now.
fn apply_time_format(format: TimeFormat) {
    crate::calendar::time::set_24_hour(format.is_24_hour(native::system_uses_24_hour));
}

/// Settings › General › Time format, for mail and calendar alike.
pub fn set_time_format(format: TimeFormat, cx: &mut App) {
    let mut saved = settings::current(cx);
    saved.time_format = format;
    settings::remember(saved, cx);
    apply_time_format(format);
    backend_of(cx).update(cx, |b, cx| b.save_settings(saved.to_map(), cx));
    cx.refresh_windows();
}

fn open_mailto(urls: Vec<String>, cx: &mut App) {
    for url in urls {
        logging::info(&format!("open url: {url}"));
        if let Some(mailto) = Mailto::parse(&url) {
            show(Show::Compose(Some(mailto)), cx);
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

fn window_options(cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
    // The traffic lights (60pt wide on macOS 26) sit centered in the rail,
    // level with the middle of the pane headers.
    let lights_x = (RAIL_WIDTH - px(60.)) / 2.;
    WindowOptions {
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
    }
}

/// Save the app's settings as they are now (W7b's menu bar rows).
pub fn save_app_settings(cx: &mut App) {
    let map = settings::current(cx).to_map();
    backend_of(cx).update(cx, |b, cx| b.save_settings(map, cx));
}

/// The menu bar, its model and the reminders (W7b). Once, at launch, after
/// the main window opened (it sets the theme).
pub fn install_menu_bar(cx: &mut App) {
    let today = super::today::init(backend_of(cx), cx);
    super::menu_bar::install(today, cx);
}

/// Settings › Notifications and ⌘K: on or off, app-wide.
pub fn set_notifications(on: bool, cx: &mut App) {
    let mut saved = settings::current(cx);
    saved.notifications = on;
    settings::remember(saved, cx);
    logging::info(&format!(
        "notify: new mail notifications {}",
        if on { "on" } else { "off" }
    ));
    backend_of(cx).update(cx, |b, cx| {
        b.set_notifications(on);
        b.save_settings(saved.to_map(), cx);
    });
}

/// W3: Settings › Mail › Signatures (MST-2).
pub fn set_signatures(on: bool, cx: &mut App) {
    let mut saved = settings::current(cx);
    saved.signatures = on;
    settings::remember(saved, cx);
    backend_of(cx).update(cx, |b, cx| b.save_settings(saved.to_map(), cx));
    cx.refresh_windows();
}

/// W3: Settings › General and ⌘K (LNK-3). macOS may ask to confirm.
pub fn make_default_mail_app(cx: &mut App) {
    let result = native::make_default_mail_app();
    logging::info(&format!("default mail app: {result:?}"));
    if let Err(e) = result {
        eprintln!("Pigeon: couldn't become the default mail app: {e}");
    }
    cx.refresh_windows();
}

/// Settings › General and ⌘K (NTF-6). Needing approval opens System
/// Settings where it's given.
pub fn set_open_at_login(on: bool, cx: &mut App) {
    if native::login_item() == native::LoginItem::NeedsApproval && on {
        native::open_login_items_settings();
        return;
    }
    let result = native::set_login_item(on);
    logging::info(&format!(
        "login item: {} → {result:?}",
        if on { "on" } else { "off" }
    ));
    LOGIN_ERROR.with(|e| *e.borrow_mut() = result.err());
    if on && native::login_item() == native::LoginItem::NeedsApproval {
        native::open_login_items_settings();
    }
    cx.refresh_windows();
}

thread_local! {
    static LOGIN_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// What Settings › General says about the login item, as macOS has it.
pub fn login_item_status() -> String {
    if let Some(error) = LOGIN_ERROR.with(|e| e.borrow().clone()) {
        return format!("Couldn't change it: {error}");
    }
    match native::login_item() {
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
            if !settings::current(cx).notifications {
                logging::info(&format!(
                    "notify: {} new, not shown (notifications off)",
                    new.len()
                ));
                return;
            }
            let open = frontmost_thread(cx);
            let new: Vec<_> = new
                .iter()
                .filter(|m| Some(&m.thread_id) != open.as_ref())
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
        | BackendEvent::SignedIn
        | BackendEvent::SignedOut => update_badge(cx),
        _ => {}
    }
}

/// The conversation open in the main window while it's frontmost: no
/// notification for that one.
fn frontmost_thread(cx: &App) -> Option<String> {
    let main = cx.global::<Shell>().main.clone()?;
    if cx.active_window() != Some(main.window) {
        return None;
    }
    let app = main.app.upgrade()?;
    let app = app.read(cx);
    (app.mode == Mode::Mail && !app.settings_page.open)
        .then(|| app.selected.map(|ix| app.data.threads[ix].id.clone()))
        .flatten()
}

/// NTF-2: Primary's unread count on the Dock icon.
fn update_badge(cx: &mut App) {
    let count = backend_of(cx).read(cx).inbox_unread();
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
}

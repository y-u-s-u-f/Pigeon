//! The menu bar item and its panel (sprint 2 W7b, NTF-4), and quick join
//! (NTF-5). The panel is one gpui pop-up window (a non-activating
//! `NSPanel` on every Space), opened hidden at launch, drawn once off
//! sight, then only shown and hidden natively (`native::menu_bar`).
//!
//! Every native show, hide, focus or resize runs from a foreground task,
//! outside any gpui update (`schedule`): AppKit calls back into gpui at
//! once, which would otherwise borrow the app twice.

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use gpui::{
    App, AppContext as _, AsyncApp, Bounds, Entity, SharedString, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, point, px, size,
};

use super::{
    shell::{self, Show},
    today::{self, Today},
};
use crate::{
    actions::{JoinNextMeeting, OpenMenuBarPanel},
    instance, logging,
    native::menu_bar::{self as native, Anchor, HOT_KEY_TAKEN, Signal},
    settings::{self, MenuBarPrefs},
    ui::menu_bar::{MenuBarPanel, PANEL_START_HEIGHT, PANEL_WIDTH},
};

const PANEL_HOT_KEY: u32 = 1;
const JOIN_HOT_KEY: u32 = 2;
/// A status-item click this soon after the panel closed itself (lost
/// focus, a click away) is the click that closed it: it doesn't reopen.
const REOPEN_GUARD: Duration = Duration::from_millis(300);
/// How long the hidden panel shows, see-through, at launch to be drawn.
const WARM_UP: Duration = Duration::from_millis(300);

/// What opened the panel.
#[derive(Clone)]
enum Source {
    Item(Anchor),
    HotKey,
    Command,
    /// Quick join found no meeting to join.
    NoMeeting,
}

impl Source {
    fn label(&self) -> &'static str {
        match self {
            Source::Item(_) => "status item",
            Source::HotKey => "hot key",
            Source::Command => "command",
            Source::NoMeeting => "quick join",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HideReason {
    Toggle,
    Escape,
    LostKey,
    ClickAway,
    /// Something in it was opened.
    Opened,
}

struct State {
    cx: AsyncApp,
    panel: WindowHandle<MenuBarPanel>,
    closed_at: Option<Instant>,
    /// What Settings › Menu bar says about the shortcuts.
    hot_keys: String,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The item, the hidden panel and the shortcuts. Once, at launch, after
/// the main window (it sets the theme).
pub fn install(today: Entity<Today>, cx: &mut App) {
    native::listen(|signal| match signal {
        Signal::Clicked(anchor) => schedule(move || toggle(Source::Item(anchor))),
        Signal::ClickAway => {
            if native::panel_visible() {
                schedule(|| hide(HideReason::ClickAway));
            }
        }
        Signal::HotKey(PANEL_HOT_KEY) => schedule(|| toggle(Source::HotKey)),
        Signal::HotKey(JOIN_HOT_KEY) => schedule(|| with_app(join_next)),
        Signal::HotKey(_) => {}
    });
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(PANEL_WIDTH, PANEL_START_HEIGHT),
        })),
        titlebar: None,
        focus: true,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: shell::window_background(cx),
        ..Default::default()
    };
    let panel = cx.open_window(options, |window, cx| {
        if !native::adopt_panel(window) {
            logging::info("menu-bar: no NSWindow behind the panel");
        }
        cx.new(|cx| MenuBarPanel::new(today.clone(), window, cx))
    });
    let panel = match panel {
        Ok(panel) => panel,
        Err(e) => {
            logging::info(&format!("menu-bar: couldn't open the panel: {e:#}"));
            return;
        }
    };
    STATE.with_borrow_mut(|state| {
        *state = Some(State {
            cx: cx.to_async(),
            panel,
            closed_at: None,
            hot_keys: String::new(),
        })
    });
    apply(settings::get(cx).menu_bar, cx);
    cx.observe(&today, |_, cx| update_status(cx)).detach();
    cx.on_action(|_: &OpenMenuBarPanel, _| schedule(|| toggle(Source::Command)));
    cx.on_action(|_: &JoinNextMeeting, cx| join_next(cx));

    // Draw it once now, so the first open is as quick as the next.
    schedule(move || {
        native::warm_up_panel();
        with_app(|cx| {
            panel.update(cx, |_, window, _| window.refresh()).ok();
        });
    });
    cx.spawn(async |cx| {
        cx.background_executor().timer(WARM_UP).await;
        native::finish_warm_up();
        // For testers: open and close a few times, the timings logged.
        if instance::dev_mode() && std::env::var_os("PIGEON_MENU_BAR_SELFTEST").is_some() {
            for _ in 0..3 {
                cx.background_executor()
                    .timer(Duration::from_millis(800))
                    .await;
                toggle(Source::Command);
                cx.background_executor()
                    .timer(Duration::from_millis(800))
                    .await;
                hide(HideReason::Toggle);
            }
        }
    })
    .detach();
}

/// Run `f` on the main thread soon, outside any gpui update.
fn schedule(f: impl FnOnce() + 'static) {
    let Some(cx) = STATE.with_borrow(|s| s.as_ref().map(|s| s.cx.clone())) else {
        return;
    };
    cx.foreground_executor().spawn(async move { f() }).detach();
}

/// From a scheduled task: an update.
fn with_app(f: impl FnOnce(&mut App)) {
    if let Some(cx) = STATE.with_borrow(|s| s.as_ref().map(|s| s.cx.clone())) {
        cx.update(f);
    }
}

/// From a scheduled task: a value read from the app.
fn with_app_value<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let cx = STATE.with_borrow(|s| s.as_ref().map(|s| s.cx.clone()))?;
    Some(cx.update(f))
}

fn toggle(source: Source) {
    let closed_at = STATE.with_borrow(|s| s.as_ref().and_then(|s| s.closed_at));
    if native::panel_visible() {
        hide(HideReason::Toggle);
    } else if matches!(source, Source::Item(_))
        && closed_at.is_some_and(|t| t.elapsed() < REOPEN_GUARD)
    {
        logging::info("menu-bar: ignored the click that closed the panel");
    } else {
        show(source, None);
    }
}

fn show(source: Source, notice: Option<SharedString>) {
    let started = Instant::now();
    let Some(panel) = STATE.with_borrow(|s| s.as_ref().map(|s| s.panel)) else {
        return;
    };
    // gpui first, in its own update, done before AppKit calls back below.
    let mut height = None;
    with_app(|cx| {
        if let Some(today) = today::model(cx) {
            today.update(cx, |t, cx| t.mark_seen(cx));
        }
        height = panel
            .update(cx, |panel, window, cx| {
                panel.will_show(source.label(), notice, started, window, cx);
                panel.content_height()
            })
            .ok();
    });
    if let Some(height) = height {
        native::resize_panel(height);
    }
    let anchor = match &source {
        Source::Item(anchor) => Some(anchor),
        _ => None,
    };
    // A short fade and slide, unless Reduce Motion is on.
    let animate = with_app_value(|cx| !cx.reduce_motion()).unwrap_or(false);
    native::show_panel(anchor, animate);
    logging::info(&format!(
        "menu-bar: {} → ordered front in {:.1} ms",
        source.label(),
        started.elapsed().as_secs_f64() * 1000.
    ));
}

fn hide(reason: HideReason) {
    if !native::panel_visible() {
        return;
    }
    // A short fade, except under Reduce Motion or when the main window
    // brings up what was opened.
    let animate =
        reason != HideReason::Opened && with_app_value(|cx| !cx.reduce_motion()).unwrap_or(false);
    native::hide_panel(animate);
    STATE.with_borrow_mut(|s| {
        if let Some(s) = s
            && matches!(reason, HideReason::LostKey | HideReason::ClickAway)
        {
            s.closed_at = Some(Instant::now());
        }
    });
    let panel = STATE.with_borrow(|s| s.as_ref().map(|s| s.panel));
    if let Some(panel) = panel {
        with_app(|cx| {
            panel.update(cx, |p, _, cx| p.did_hide(cx)).ok();
        });
    }
}

/// Close the panel, from inside gpui (Esc, losing focus).
pub fn hide_later(reason: HideReason) {
    schedule(move || hide(reason));
}

/// Close the panel and bring `what` up in the main window.
pub fn open_in_main(what: Show) {
    schedule(move || {
        hide(HideReason::Opened);
        with_app(|cx| shell::show(what, cx));
    });
}

/// Size the window to its content, top edge kept.
pub fn resize_later(height: f64) {
    schedule(move || native::resize_panel(height));
}

/// Open a meeting's video link. In test-instance mode a dry run that only
/// logs it.
pub fn open_link(url: &str, cx: &mut App) {
    if instance::dev_mode() {
        logging::info(&format!("quick-join: {url}"));
    } else {
        logging::info(&format!("quick-join: opening {url}"));
        cx.open_url(url);
    }
}

/// ⌃⌥⌘J and ⌘K "Join Next Meeting" (NTF-5): the meeting under way or
/// starting within 10 minutes; otherwise the panel says there's none.
pub fn join_next(cx: &mut App) {
    let Some(today) = today::model(cx) else {
        return;
    };
    let found = today::quick_join(&today.read(cx).calendar, today::now());
    match found {
        Some((_, url)) => open_link(&url, cx),
        None => {
            logging::info("quick-join: no meeting within 10 minutes");
            let notice: SharedString = "No meeting to join in the next 10 minutes.".into();
            schedule(move || {
                if native::panel_visible() {
                    let panel = STATE.with_borrow(|s| s.as_ref().map(|s| s.panel));
                    if let Some(panel) = panel {
                        with_app(|cx| {
                            panel.update(cx, |p, _, cx| p.say(notice, cx)).ok();
                        });
                    }
                } else {
                    show(Source::NoMeeting, Some(notice));
                }
            });
        }
    }
}

/// The title and dot beside the icon; cheap, run on every model change
/// and each minute.
fn update_status(cx: &mut App) {
    let Some(today) = today::model(cx) else {
        return;
    };
    let prefs = settings::get(cx).menu_bar;
    let today = today.read(cx);
    let title = prefs
        .next_meeting
        .then(|| today::status_title(&today.calendar, today::now()))
        .flatten();
    native::set_status(title.as_deref(), today.has_new_mail());
}

/// Settings › Menu bar: save, then put into effect.
pub fn set_prefs(prefs: MenuBarPrefs, cx: &mut App) {
    if settings::get(cx).menu_bar == prefs {
        return;
    }
    shell::change_app_settings(cx, |s| s.menu_bar = prefs);
    apply(prefs, cx);
}

fn apply(prefs: MenuBarPrefs, cx: &mut App) {
    native::set_status_item(prefs.icon);
    update_status(cx);
    let status = register_hot_keys(prefs);
    logging::info(&format!("menu-bar: {status}"));
    STATE.with_borrow_mut(|s| {
        if let Some(s) = s {
            s.hot_keys = status;
        }
    });
    cx.refresh_windows();
}

/// Whether global shortcuts are registered: not in test-instance mode,
/// unless `PIGEON_HOTKEY=1`.
fn hot_keys_allowed() -> bool {
    instance::global_shortcuts_allowed()
        || std::env::var("PIGEON_HOTKEY").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn register_hot_keys(prefs: MenuBarPrefs) -> String {
    if !hot_keys_allowed() {
        native::unregister_hot_key(PANEL_HOT_KEY);
        native::unregister_hot_key(JOIN_HOT_KEY);
        return "Off in test-instance mode (PIGEON_DEV). PIGEON_HOTKEY=1 turns them on.".into();
    }
    let mut problems = Vec::new();
    let mut on = Vec::new();
    for (id, key) in [
        (PANEL_HOT_KEY, prefs.panel_key),
        (JOIN_HOT_KEY, prefs.join_key),
    ] {
        let Some(code) = key.key_code() else {
            native::unregister_hot_key(id);
            continue;
        };
        match native::register_hot_key(id, code) {
            Ok(()) => on.push(key.label()),
            Err(HOT_KEY_TAKEN) => problems.push(format!(
                "{} is taken by another app; pick another.",
                key.label()
            )),
            Err(status) => {
                eprintln!("Pigeon: RegisterEventHotKey {}: {status}", key.label());
                problems.push(format!(
                    "macOS didn't turn on {}. Pick another shortcut.",
                    key.label()
                ))
            }
        }
    }
    if !problems.is_empty() {
        problems.join(" ")
    } else if on.is_empty() {
        "Both shortcuts are off.".into()
    } else {
        format!("{} registered.", on.join(" and "))
    }
}

/// For Settings › Menu bar.
pub fn hot_key_status() -> String {
    STATE.with_borrow(|s| s.as_ref().map(|s| s.hot_keys.clone()).unwrap_or_default())
}

//! The settings page (⌘,). It replaces the panes beside the rail: the
//! sections, with a search over every setting, on the left, and the
//! section's rows on the right. A row's value is read from where the app
//! keeps it, so the shortcuts and menus that change the same things stay in
//! step.
//!
//! Keys: ↑/↓ (j/k) move through the sections or the rows, Tab and ⇧Tab go
//! between the two, Space/Enter toggles or opens a pop-up (then ↑/↓ and
//! Enter pick), ←/→ change a value, `/` or ⌘F search, Esc closes a pop-up,
//! clears the search, then goes back.

use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, FocusHandle,
    Focusable as _, FontWeight, HighlightStyle, Hsla, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, ParentElement, Pixels, Rems, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, StyledText, Subscription, Task, Window, actions,
    anchored,
    component::{
        input::{Input, InputEvent, InputState},
        scroll::ScrollableElement as _,
    },
    deferred, div, point,
    prelude::FluentBuilder as _,
    px, rems,
};

use super::{
    HEADER_HEIGHT, account_avatar,
    controls::{self, ButtonKind, dropdown, menu_check, menu_row, popup, push_button},
    empty_state, icon, icon_button_small, keys_caps,
    motion::{self, Rise, SPRING_PANE},
    window_drag_area,
};
use crate::ui::style::*;
use crate::{
    actions::{COMMANDS, Scope, SignOut, SyncNow},
    app::{Density, PigeonApp, menu_bar, shell},
    assets::Icon,
    backend::Source,
    data::Contact,
    native::{self, LoginItem},
    notify::{self, Permission},
    settings::{
        self, Appearance, DockBadge, Entry, HotKey, KeymapPreset, Match, RemoteImages, Section,
        SettingId, TimeFormat,
    },
    theme::Theme,
    ui::{calendar::ViewKind, shortcuts_help::live_bindings},
};

actions!(
    settings,
    [
        SelectNext,
        SelectPrev,
        EnterRows,
        LeaveRows,
        Activate,
        Increase,
        Decrease,
        FocusSearch,
        Close,
    ]
);

/// Key context of the page, and of its search field.
pub const SETTINGS: &str = "Settings";
pub const SETTINGS_SEARCH: &str = "SettingsSearch";

/// How long a setting jumped to from the search stays fully lit before it
/// fades to the cursor's color (MD-026).
const FLASH_HOLD: Duration = Duration::from_millis(900);
const SECTIONS_WIDTH: gpui::Rems = type_size(220.);
const CONTENT_MAX: Pixels = px(680.);
const MENU_WIDTH: gpui::Rems = type_size(220.);
const MENU_MAX_HEIGHT: Pixels = px(280.);
/// A setting's row, and a shortcut's, at least.
const ROW_MIN: Rems = rems(56. / BASE_REM);
const ROW_MIN_COMPACT: Rems = rems(36. / BASE_REM);

const APPEARANCES: &[&str] = &["System", "Light", "Dark"];
const SCOPES: &[&str] = &["Current View", "All Mail"];
const VIEWS: &[&str] = &["Day", "Week", "Month"];
const IMAGES: &[&str] = &["Ask", "Always", "Never"];
// As `TimeFormat::ALL`.
const TIME_FORMATS: &[&str] = &["System", "12-hour", "24-hour"];
// W5 (CAL-41).
const EVENT_LENGTHS: &[&str] = &[
    "15 minutes",
    "30 minutes",
    "45 minutes",
    "1 hour",
    "1.5 hours",
    "2 hours",
];
const EVENT_MINUTES: [u32; 6] = [15, 30, 45, 60, 90, 120];
// W7b: as `HotKey::PANEL` and `HotKey::JOIN`.
const PANEL_KEYS: &[&str] = &["⌃⌥⌘Space", "⌃⌥⌘P", "Off"];
const JOIN_KEYS: &[&str] = &["⌃⌥⌘J", "⌃⌥⌘M", "Off"];
// As `KeymapPreset::ALL`.
const KEYMAPS: &[&str] = &["Gmail", "Apple Mail"];
// As `DockBadge::ALL`.
const DOCK_BADGES: &[&str] = &["Off", "Primary", "All Inbox"];
/// A slider's keyboard step.
const SLIDER_STEP: f32 = 0.1;

pub struct SettingsPage {
    pub open: bool,
    section: Section,
    /// The row the keyboard is on; none while it's in the section list.
    row: Option<usize>,
    /// The highlighted search result.
    result: usize,
    /// Moved with the keys since the last click, so the ring shows.
    keyboard: bool,
    /// A setting jumped to, lit for a moment; fading once the hold is over.
    flash: Option<Entry>,
    flash_fading: bool,
    /// Counts jumps, so each one's light starts at full.
    flash_serial: usize,
    /// Scroll the cursor into view at the next render, once its rows exist.
    pending_scroll: Option<Reveal>,
    pub focus: FocusHandle,
    pub search: Entity<InputState>,
    scroll: ScrollHandle,
    /// Sign Out was pressed once: the row asks to confirm.
    confirm_sign_out: bool,
    /// While confirming, the keyboard is on Sign Out rather than Cancel
    /// (MB-028: Return cancels unless → moved it).
    sign_out_armed: bool,
    /// The pop-up menu of this setting is open.
    menu: Option<SettingId>,
    /// The option the keyboard is on in the open menu (MC-069).
    menu_cursor: usize,
    /// Every command's bindings, by `COMMANDS` index, for the preset they
    /// were read for (MC-040).
    bindings: Option<(KeymapPreset, Vec<Vec<KeyBinding>>)>,
    _flash: Option<Task<()>>,
}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<PigeonApp>) -> (Self, Subscription) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        let events = cx.subscribe_in(&search, window, PigeonApp::on_settings_search);
        let page = Self {
            open: false,
            section: Section::ALL[0],
            row: None,
            result: 0,
            keyboard: false,
            flash: None,
            flash_fading: false,
            flash_serial: 0,
            pending_scroll: None,
            focus: cx.focus_handle(),
            search,
            scroll: ScrollHandle::new(),
            confirm_sign_out: false,
            sign_out_armed: false,
            menu: None,
            menu_cursor: 0,
            bindings: None,
            _flash: None,
        };
        (page, events)
    }
}

/// What a row shows on its right.
enum Control {
    Switch(bool),
    Choice(usize, &'static [&'static str]),
    /// A pop-up menu; options may have a color swatch.
    Menu(usize, Vec<(String, Option<Hsla>)>),
    /// 0…1 in steps of `SLIDER_STEP`; `locked` while the system decides.
    Slider {
        value: f32,
        text: String,
        locked: bool,
    },
    Stepper {
        value: String,
        can_down: bool,
        can_up: bool,
    },
    Account(Contact),
    Keys(Vec<KeyBinding>),
    /// Read-only text, shown under the label.
    Info(String),
    /// Runs something; `danger` asks first.
    Button {
        label: &'static str,
        danger: bool,
    },
}

#[derive(Clone, Copy)]
enum Change {
    Switch(bool),
    Choice(usize),
    Step(i32),
    /// A slider's new value, 0…1.
    Set(f32),
    Reset,
}

/// Every command's bindings, looked up in its app's contexts.
fn all_bindings(window: &Window) -> Vec<Vec<KeyBinding>> {
    COMMANDS
        .iter()
        .map(|command| {
            let scope = match command.group.scope() {
                Scope::Calendar => Scope::Calendar,
                Scope::Mail | Scope::Global => Scope::Mail,
            };
            live_bindings(&*(command.action)(), scope, window)
        })
        .collect()
}

impl SettingsPage {
    fn shortcut_bindings(&self, ix: usize) -> &[KeyBinding] {
        self.bindings
            .as_ref()
            .and_then(|(_, all)| all.get(ix))
            .map_or(&[], Vec::as_slice)
    }

    /// Shortcuts without a binding are left out, as in the `?` sheet.
    fn shown(&self, entry: Entry) -> bool {
        match entry {
            Entry::Shortcut(ix) => !self.shortcut_bindings(ix).is_empty(),
            _ => true,
        }
    }
}

/// How the cursor's row is scrolled into view.
#[derive(Clone, Copy)]
enum Reveal {
    /// The least scroll that shows it with a neighbour on the side it's nearer.
    Nearest,
    /// Near the top, under the item above it; for jumps, whose layout is new.
    Top,
}

/// Scroll child `ix` of the rows into view, keeping one neighbour visible as margin.
fn reveal_child(scroll: &ScrollHandle, ix: usize, len: usize, reveal: Reveal) {
    let above = ix.saturating_sub(1);
    let below = (ix + 1).min(len.saturating_sub(1));
    match reveal {
        Reveal::Top => scroll.scroll_to_top_of_item(above),
        // Last frame's layout still holds for a move between rows.
        Reveal::Nearest => match scroll.bounds_for_item(ix) {
            Some(item) => {
                let view = scroll.bounds();
                let upper = item.top() + scroll.offset().y < view.center().y;
                scroll.scroll_to_item(if upper { above } else { below });
            }
            None => scroll.scroll_to_item(ix),
        },
    }
}

impl PigeonApp {
    // Opening and closing.

    pub(crate) fn toggle_settings(
        &mut self,
        _: &crate::actions::OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page.open {
            self.close_settings(window, cx);
        } else {
            self.open_settings_page(window, cx);
            let page = &mut self.settings_page;
            page.row = None;
            page.keyboard = false;
            window.focus(&page.focus, cx);
            cx.notify();
        }
    }

    fn open_settings_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings_page.open {
            self.close_overlays(window, cx);
            self.settings_page.open = true;
            shell::refresh_system_items();
            notify::refresh_permission(cx);
        }
        self.read_settings_bindings(window, cx);
    }

    /// The bindings shown, read again once the preset changed.
    fn read_settings_bindings(&mut self, window: &Window, cx: &App) {
        let preset = settings::get(cx).keymap;
        if self.settings_page.bindings.as_ref().map(|(p, _)| *p) != Some(preset) {
            self.settings_page.bindings = Some((preset, all_bindings(window)));
        }
    }

    /// ⌘K's "… Settings" rows (ME-060): Settings open at that section.
    pub(crate) fn open_settings_section(
        &mut self,
        action: &crate::actions::OpenSettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings_page(window, cx);
        self.jump_to_setting(Entry::Section(action.0), window, cx);
    }

    /// Sign Out from ⌘K or the menu (ACC-7): Settings › Accounts with the
    /// confirmation showing, as its own button asks.
    pub(crate) fn ask_sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_page(window, cx);
        self.jump_to_setting(Entry::Setting(SettingId::SignOut), window, cx);
        self.settings_page.confirm_sign_out = true;
        self.settings_page.sign_out_armed = false;
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_page.open = false;
        self.clear_settings_search(window, cx);
        window.focus(&self.pane_focus(cx), cx);
        cx.notify();
    }

    pub(crate) fn clear_settings_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_page
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings_page.result = 0;
    }

    pub(crate) fn focus_settings_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_page
            .search
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn settings_query(&self, cx: &App) -> String {
        self.settings_page
            .search
            .read(cx)
            .value()
            .trim()
            .to_string()
    }

    fn settings_results(&self, cx: &App) -> Vec<Match> {
        settings::search(&self.settings_query(cx))
            .into_iter()
            .filter(|m| self.settings_page.shown(m.entry))
            .collect()
    }

    fn section_rows(&self, section: Section) -> Vec<Entry> {
        settings::entries(section)
            .into_iter()
            .filter(|e| self.settings_page.shown(*e))
            .collect()
    }

    fn on_settings_search(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.settings_page.result = 0;
                self.settings_page.scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.open_settings_result(window, cx),
            _ => {}
        }
    }

    fn open_settings_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let results = self.settings_results(cx);
        if let Some(m) = results.get(self.settings_page.result) {
            self.jump_to_setting(m.entry, window, cx);
        }
    }

    /// Show `entry` in its section, lit for a moment.
    fn jump_to_setting(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_settings_search(window, cx);
        let section = entry.section();
        let row = match entry {
            Entry::Section(_) => None,
            entry => self.section_rows(section).iter().position(|e| *e == entry),
        };
        let page = &mut self.settings_page;
        page.section = section;
        page.row = row;
        if section == Section::Notifications {
            notify::refresh_permission(cx);
        }
        let page = &mut self.settings_page;
        page.keyboard = true;
        page.pending_scroll = row.is_some().then_some(Reveal::Top);
        if row.is_none() {
            page.scroll.set_offset(point(px(0.), px(0.)));
        }
        page.flash = row.is_some().then_some(entry);
        page.flash_fading = false;
        page.flash_serial += 1;
        page._flash = row.is_some().then(|| {
            cx.spawn(async |this, cx| {
                cx.background_executor().timer(FLASH_HOLD).await;
                this.update(cx, |this, cx| {
                    this.settings_page.flash_fading = true;
                    cx.notify();
                })
                .ok();
            })
        });
        window.focus(&page.focus, cx);
        cx.notify();
    }

    fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        let page = &mut self.settings_page;
        if page.section != section {
            page.section = section;
            page.scroll.set_offset(point(px(0.), px(0.)));
            if section == Section::Notifications {
                notify::refresh_permission(cx);
            }
        }
        let page = &mut self.settings_page;
        page.row = None;
        page.confirm_sign_out = false;
        page.menu = None;
        cx.notify();
    }

    // Keyboard.

    fn settings_step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let step = |ix: usize, len: usize| {
            if forward {
                (ix + 1).min(len.saturating_sub(1))
            } else {
                ix.saturating_sub(1)
            }
        };
        self.settings_page.keyboard = true;
        self.settings_page.confirm_sign_out = false;
        // ↑/↓ move through an open menu's options (MC-069).
        if let Some(id) = self.settings_page.menu {
            if let Control::Menu(_, options) = self.control(id, window, cx) {
                let page = &mut self.settings_page;
                page.menu_cursor = step(page.menu_cursor, options.len());
            }
            return cx.notify();
        }
        if !self.settings_query(cx).is_empty() {
            let len = self.settings_results(cx).len();
            self.settings_page.result = step(self.settings_page.result, len);
            self.settings_page.pending_scroll = Some(Reveal::Nearest);
        } else if let Some(row) = self.settings_page.row {
            let len = self.section_rows(self.settings_page.section).len();
            self.settings_page.row = Some(step(row, len));
            self.settings_page.flash = None;
            self.settings_page.pending_scroll = Some(Reveal::Nearest);
        } else {
            let ix = Section::ALL
                .iter()
                .position(|s| *s == self.settings_page.section)
                .unwrap_or(0);
            self.set_section(Section::ALL[step(ix, Section::ALL.len())], cx);
        }
        cx.notify();
    }

    fn settings_enter_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = &mut self.settings_page;
        page.keyboard = true;
        window.focus(&page.focus, cx);
        if self.settings_query(cx).is_empty()
            && !self.section_rows(self.settings_page.section).is_empty()
        {
            let page = &mut self.settings_page;
            page.row = Some(page.row.unwrap_or(0));
            page.pending_scroll = Some(Reveal::Nearest);
        }
        cx.notify();
    }

    fn settings_leave_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings_query(cx).is_empty() {
            return self.focus_settings_search(window, cx);
        }
        self.settings_page.keyboard = true;
        self.settings_page.row = None;
        cx.notify();
    }

    /// The setting under the keyboard, when it's on one.
    fn cursor_setting(&self) -> Option<SettingId> {
        let row = self.settings_page.row?;
        match self.section_rows(self.settings_page.section).get(row)? {
            Entry::Setting(id) => Some(*id),
            _ => None,
        }
    }

    fn settings_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings_query(cx).is_empty() {
            return self.open_settings_result(window, cx);
        }
        if let Some(id) = self.settings_page.menu {
            let option = self.settings_page.menu_cursor;
            return self.change_setting(id, Change::Choice(option), window, cx);
        }
        if self.settings_page.row.is_none() {
            return self.settings_enter_rows(window, cx);
        }
        let Some(id) = self.cursor_setting() else {
            return;
        };
        let change = match self.control(id, window, cx) {
            Control::Switch(on) => Change::Switch(!on),
            Control::Choice(ix, options) => Change::Choice((ix + 1) % options.len()),
            Control::Menu(ix, _) => return self.open_settings_menu(id, ix, cx),
            Control::Stepper { .. } => Change::Reset,
            Control::Slider { locked, .. } if !locked => Change::Reset,
            // Return on the confirmation is Cancel unless → moved to Sign
            // Out (MB-028).
            Control::Button { .. }
                if id == SettingId::SignOut
                    && self.settings_page.confirm_sign_out
                    && !self.settings_page.sign_out_armed =>
            {
                self.settings_page.confirm_sign_out = false;
                return cx.notify();
            }
            Control::Button { .. } => return self.press_setting(id, window, cx),
            Control::Account(_) | Control::Keys(_) | Control::Info(_) | Control::Slider { .. } => {
                return;
            }
        };
        self.change_setting(id, change, window, cx);
    }

    /// Open setting `id`'s pop-up, the keyboard on option `at`.
    fn open_settings_menu(&mut self, id: SettingId, at: usize, cx: &mut Context<Self>) {
        let page = &mut self.settings_page;
        page.menu = Some(id);
        page.menu_cursor = at;
        cx.notify();
    }

    /// Sync Now and Sign Out; Sign Out asks first, signed in to Google.
    fn press_setting(&mut self, id: SettingId, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            SettingId::SyncNow => window.dispatch_action(Box::new(SyncNow), cx),
            SettingId::TestNotification => crate::notify::send_test(cx),
            SettingId::NotificationPermission => match notify::permission(cx) {
                Permission::NotAsked => notify::ask_permission(cx),
                Permission::Denied => notify::open_system_settings(cx),
                _ => {}
            },
            SettingId::DefaultMailApp => shell::make_default_mail_app(cx),
            SettingId::SignOut => {
                let google = self.backend.read(cx).source() == Source::Google;
                if google && !self.settings_page.confirm_sign_out {
                    self.settings_page.confirm_sign_out = true;
                    self.settings_page.sign_out_armed = false;
                } else {
                    self.settings_page.confirm_sign_out = false;
                    if google {
                        self.close_settings(window, cx);
                        self.backend.update(cx, |b, cx| b.sign_out(cx));
                    } else {
                        window.dispatch_action(Box::new(SignOut), cx);
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn settings_adjust(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_page.keyboard = true;
        if self.settings_page.row.is_none() {
            if up {
                self.settings_enter_rows(window, cx);
            }
            return;
        }
        let Some(id) = self.cursor_setting() else {
            if !up {
                self.settings_leave_rows(window, cx);
            }
            return;
        };
        if id == SettingId::SignOut && self.settings_page.confirm_sign_out {
            self.settings_page.sign_out_armed = up;
            return cx.notify();
        }
        let change = match self.control(id, window, cx) {
            Control::Switch(_) => Change::Switch(up),
            Control::Choice(ix, options) if up => Change::Choice((ix + 1).min(options.len() - 1)),
            Control::Menu(ix, options) if up => Change::Choice((ix + 1).min(options.len() - 1)),
            Control::Choice(ix, _) | Control::Menu(ix, _) => Change::Choice(ix.saturating_sub(1)),
            Control::Stepper { .. } => Change::Step(if up { 1 } else { -1 }),
            Control::Slider { locked, .. } if !locked => Change::Step(if up { 1 } else { -1 }),
            Control::Account(_)
            | Control::Keys(_)
            | Control::Info(_)
            | Control::Button { .. }
            | Control::Slider { .. } => {
                return;
            }
        };
        self.change_setting(id, change, window, cx);
    }

    fn settings_close(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.menu.take().is_some()
            || std::mem::take(&mut self.settings_page.confirm_sign_out)
        {
            cx.notify();
        } else if !self.settings_query(cx).is_empty() {
            self.clear_settings_search(window, cx);
            cx.notify();
        } else {
            self.close_settings(window, cx);
        }
    }

    // Values: read from, and written to, where the app keeps them.

    fn control(&self, id: SettingId, window: &Window, cx: &App) -> Control {
        let calendar = self.calendar.read(cx);
        match id {
            SettingId::Appearance => Control::Choice(
                Appearance::ALL
                    .iter()
                    .position(|a| *a == self.settings.appearance)
                    .unwrap_or(0),
                APPEARANCES,
            ),
            SettingId::TextSize => {
                let (min, max) = TEXT_SCALE_RANGE;
                Control::Stepper {
                    value: format!("{:.0}%", self.text_scale * 100.),
                    can_down: self.text_scale > min + 0.01,
                    can_up: self.text_scale < max - 0.01,
                }
            }
            SettingId::CompactList => Control::Switch(self.density == Density::Compact),
            SettingId::Sidebar => {
                Control::Switch(self.pane_layout(window.viewport_size().width).sidebar > px(0.))
            }
            SettingId::ArchivedInInbox => Control::Switch(self.archived_expanded),
            SettingId::AutoAdvance => Control::Switch(self.settings.auto_advance),
            SettingId::SearchScope => {
                Control::Choice(self.settings.search_all_mail as usize, SCOPES)
            }
            SettingId::CalendarView => Control::Choice(
                match calendar.view {
                    ViewKind::Day => 0,
                    ViewKind::Week => 1,
                    ViewKind::Month => 2,
                },
                VIEWS,
            ),
            SettingId::WeekNumbers => Control::Switch(calendar.show_week_numbers),
            SettingId::RemoteImages => Control::Choice(
                match self.saved.remote_images {
                    RemoteImages::Ask => 0,
                    RemoteImages::Always => 1,
                    RemoteImages::Never => 2,
                },
                IMAGES,
            ),
            SettingId::TimeFormat => {
                let format = settings::get(cx).time_format;
                let ix = TimeFormat::ALL.iter().position(|f| *f == format);
                Control::Choice(ix.unwrap_or(0), TIME_FORMATS)
            }
            SettingId::EventLength => {
                let minutes = calendar.event_minutes();
                let ix = (0..EVENT_MINUTES.len())
                    .min_by_key(|&i| EVENT_MINUTES[i].abs_diff(minutes))
                    .unwrap_or(3);
                Control::Menu(ix, menu_options(EVENT_LENGTHS))
            }
            SettingId::DefaultCalendar => {
                let writable: Vec<_> = calendar.writable_calendars().collect();
                let selected = calendar
                    .prefs
                    .default_calendar
                    .as_deref()
                    .and_then(|id| writable.iter().position(|c| c.id == id))
                    .map_or(0, |ix| ix + 1);
                let mut options = vec![("Automatic".to_string(), None)];
                options.extend(writable.iter().map(|c| (c.name.clone(), Some(c.color))));
                Control::Menu(selected, options)
            }
            SettingId::TimeZone => Control::Info(calendar.time_zone_note()),
            SettingId::Account => Control::Account(self.data.me()),
            SettingId::Access => Control::Info(super::status::scopes(self.backend.read(cx))),
            SettingId::LastSync => Control::Info(super::status::last_sync(
                self.backend.read(cx),
                super::status::now_ms(),
            )),
            SettingId::SyncNow => Control::Button {
                label: "Sync Now",
                danger: false,
            },
            SettingId::SignOut => Control::Button {
                label: "Sign Out",
                danger: true,
            },
            // Sprint 2 W1.
            SettingId::Notifications => Control::Switch(settings::get(cx).notifications),
            SettingId::TestNotification => Control::Button {
                label: "Send Test",
                danger: false,
            },
            SettingId::NotificationPermission => match notify::permission(cx) {
                Permission::NotAsked => Control::Button {
                    label: "Turn On",
                    danger: false,
                },
                Permission::Denied => Control::Button {
                    label: "Open System Settings",
                    danger: false,
                },
                permission => Control::Info(permission.label().into()),
            },
            SettingId::NotificationSound => Control::Switch(settings::get(cx).notification_sound),
            SettingId::EventReminders => Control::Switch(settings::get(cx).event_reminders),
            SettingId::DockBadge => {
                let badge = settings::get(cx).dock_badge;
                let ix = DockBadge::ALL.iter().position(|b| *b == badge);
                Control::Choice(ix.unwrap_or(1), DOCK_BADGES)
            }
            // Cached by `shell`, asked again on opening and activation (MC-038).
            SettingId::OpenAtLogin => match shell::login_item() {
                LoginItem::Unavailable(why) => Control::Info(why.into()),
                LoginItem::On | LoginItem::NeedsApproval => Control::Switch(true),
                LoginItem::Off => Control::Switch(false),
            },
            SettingId::LoginItemStatus => Control::Info(shell::login_item_status()),
            // Sprint 2 W7b.
            SettingId::MenuBarIcon => Control::Switch(settings::get(cx).menu_bar.icon),
            SettingId::MenuBarNextMeeting => {
                Control::Switch(settings::get(cx).menu_bar.next_meeting)
            }
            // ME-068: pop-ups, as the options are long and uneven.
            SettingId::MenuBarPanelKey => {
                let key = settings::get(cx).menu_bar.panel_key;
                let ix = HotKey::PANEL.iter().position(|k| *k == key).unwrap_or(0);
                Control::Menu(ix, menu_options(PANEL_KEYS))
            }
            SettingId::MenuBarJoinKey => {
                let key = settings::get(cx).menu_bar.join_key;
                let ix = HotKey::JOIN.iter().position(|k| *k == key).unwrap_or(0);
                Control::Menu(ix, menu_options(JOIN_KEYS))
            }
            SettingId::MenuBarShortcutStatus => Control::Info(menu_bar::hot_key_status()),
            // Sprint 2 W3.
            SettingId::Signatures => Control::Switch(settings::get(cx).signatures),
            SettingId::DefaultMailApp => match shell::mail_handler() {
                native::MailHandler::Unavailable(why) => Control::Info(why.into()),
                native::MailHandler::Pigeon => Control::Info("Pigeon opens email links.".into()),
                native::MailHandler::Other(_) => Control::Button {
                    label: "Make Default",
                    danger: false,
                },
            },
            // W1-THM contract; W1-KITA gives these their slider and choice.
            SettingId::Transparency => {
                let transparency = 100 - settings::get(cx).glass.min(100);
                Control::Slider {
                    value: transparency as f32 / 100.,
                    text: format!("{transparency}%"),
                    locked: native::reduce_transparency() || native::increase_contrast(),
                }
            }
            SettingId::ShortcutPreset => {
                let keymap = settings::get(cx).keymap;
                let ix = KeymapPreset::ALL.iter().position(|k| *k == keymap);
                Control::Choice(ix.unwrap_or(0), KEYMAPS)
            }
        }
    }

    fn change_setting(
        &mut self,
        id: SettingId,
        change: Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on = match change {
            Change::Switch(on) => on,
            _ => false,
        };
        let choice = match change {
            Change::Choice(ix) => ix,
            _ => 0,
        };
        let current = match self.control(id, window, cx) {
            Control::Switch(on) => Some(on),
            _ => None,
        };
        // Switches here only flip, so setting one to its value does nothing.
        if current == Some(on) && matches!(change, Change::Switch(_)) {
            return;
        }
        // A pick closes the pop-up it came from.
        self.settings_page.menu = None;
        match id {
            SettingId::Appearance => self.set_appearance(Appearance::ALL[choice], window, cx),
            SettingId::TextSize => match change {
                Change::Step(delta) => {
                    self.set_text_scale(self.text_scale + delta as f32 * TEXT_SCALE_STEP, cx)
                }
                _ => self.set_text_scale(1., cx),
            },
            SettingId::CompactList => self.toggle_density(),
            SettingId::Sidebar => self.toggle_sidebar(&crate::actions::ToggleSidebar, window, cx),
            SettingId::ArchivedInInbox => {
                self.toggle_archived_section(&crate::actions::ToggleArchivedSection, window, cx)
            }
            SettingId::AutoAdvance => self.settings.auto_advance = on,
            SettingId::SearchScope => self.set_default_search_scope(choice == 1, cx),
            SettingId::CalendarView => {
                let view = [ViewKind::Day, ViewKind::Week, ViewKind::Month][choice];
                self.calendar.update(cx, |c, cx| c.set_view(view, cx));
            }
            SettingId::WeekNumbers => self.calendar.update(cx, |c, cx| {
                c.show_week_numbers = on;
                cx.notify();
            }),
            SettingId::RemoteImages => {
                // Also on `saved` itself, where the reader reads it (ATT-1).
                let images = [RemoteImages::Ask, RemoteImages::Always, RemoteImages::Never][choice];
                self.saved.remote_images = images;
                shell::change_app_settings(cx, |s| s.remote_images = images);
            }
            SettingId::Notifications => shell::set_notifications(on, cx),
            SettingId::NotificationSound => {
                shell::change_app_settings(cx, |s| s.notification_sound = on)
            }
            SettingId::EventReminders => shell::change_app_settings(cx, |s| s.event_reminders = on),
            SettingId::DockBadge => shell::set_dock_badge(DockBadge::ALL[choice], cx),
            SettingId::Signatures => shell::set_signatures(on, cx),
            SettingId::OpenAtLogin => shell::set_open_at_login(on, cx),
            SettingId::MenuBarIcon
            | SettingId::MenuBarNextMeeting
            | SettingId::MenuBarPanelKey
            | SettingId::MenuBarJoinKey => {
                let mut prefs = settings::get(cx).menu_bar;
                match id {
                    SettingId::MenuBarIcon => prefs.icon = on,
                    SettingId::MenuBarNextMeeting => prefs.next_meeting = on,
                    SettingId::MenuBarPanelKey => prefs.panel_key = HotKey::PANEL[choice],
                    _ => prefs.join_key = HotKey::JOIN[choice],
                }
                menu_bar::set_prefs(prefs, cx);
            }
            SettingId::TimeFormat => shell::set_time_format(TimeFormat::ALL[choice], cx),
            SettingId::EventLength => self.calendar.update(cx, |c, cx| {
                c.prefs.event_minutes = Some(EVENT_MINUTES[choice]);
                c.apply_display();
                cx.notify();
            }),
            SettingId::DefaultCalendar => {
                self.calendar.update(cx, |c, cx| {
                    c.prefs.default_calendar = choice
                        .checked_sub(1)
                        .and_then(|ix| c.writable_calendars().nth(ix))
                        .map(|c| c.id.clone());
                    cx.notify();
                });
            }
            SettingId::TimeZone
            | SettingId::Account
            | SettingId::Access
            | SettingId::LastSync
            | SettingId::SyncNow
            | SettingId::SignOut
            | SettingId::TestNotification
            | SettingId::NotificationPermission
            | SettingId::LoginItemStatus
            | SettingId::MenuBarShortcutStatus
            | SettingId::DefaultMailApp => {}
            SettingId::Transparency => {
                let transparency = 100 - settings::get(cx).glass.min(100);
                let transparency = match change {
                    Change::Step(delta) => transparency as f32 / 100. + delta as f32 * SLIDER_STEP,
                    Change::Set(value) => value,
                    _ => 1.,
                };
                let glass = 100 - (transparency.clamp(0., 1.) * 100.).round() as u8;
                shell::set_glass(glass, cx);
            }
            SettingId::ShortcutPreset => {
                let preset = KeymapPreset::ALL[choice];
                shell::change_app_settings(cx, |s| s.keymap = preset);
                crate::set_keymap(preset, cx);
                self.read_settings_bindings(window, cx);
            }
        }
        cx.notify();
    }

    /// Light or dark whatever the system says, or follow it again. The
    /// window chrome follows too.
    pub(crate) fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.appearance = appearance;
        cx.set_window_appearance(appearance.window_appearance());
        self.sync_theme(window, cx);
    }

    /// The theme for the user's choice, or for the window's appearance.
    pub(crate) fn sync_theme(&self, window: &mut Window, cx: &mut App) {
        Theme::sync_for(self.settings.appearance, window, cx);
    }

    // Rendering.

    pub(crate) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.read_settings_bindings(window, cx);
        let theme = Theme::of(cx).clone();
        let query = self.settings_query(cx);
        let searching = !query.is_empty();
        let results = if searching {
            self.settings_results(cx)
        } else {
            Vec::new()
        };
        let focused = self.settings_page.focus.is_focused(window);
        // Not in a background window (MB-042).
        let ring = focused && self.settings_page.keyboard && window.is_window_active();
        let section = self.settings_page.section;

        // The scroll view's children, and which one holds the cursor.
        let mut children: Vec<AnyElement> = Vec::new();
        let mut cursor_child = None;
        if searching {
            let mut last_section = None;
            for (ix, m) in results.iter().enumerate() {
                let entry_section = m.entry.section();
                if last_section != Some(entry_section) {
                    last_section = Some(entry_section);
                    children.push(group_header(entry_section.title(), &theme));
                }
                let cursor = ix == self.settings_page.result;
                if cursor {
                    cursor_child = Some(children.len());
                }
                children.push(self.render_settings_row(
                    m.entry,
                    Some(m),
                    cursor,
                    ring,
                    ix,
                    window,
                    cx,
                ));
            }
        } else {
            let rows = self.section_rows(section);
            let mut last_group = None;
            for (ix, entry) in rows.iter().enumerate() {
                if let Entry::Shortcut(command) = entry {
                    let group = COMMANDS[*command].group;
                    if last_group != Some(group) {
                        last_group = Some(group);
                        children.push(group_header(settings::group_label(group), &theme));
                    }
                }
                let cursor = self.settings_page.row == Some(ix);
                if cursor {
                    cursor_child = Some(children.len());
                }
                children.push(self.render_settings_row(*entry, None, cursor, ring, ix, window, cx));
            }
        }
        if let Some(reveal) = self.settings_page.pending_scroll.take()
            && let Some(child) = cursor_child
        {
            reveal_child(&self.settings_page.scroll, child, children.len(), reveal);
        }

        let title: SharedString = if searching {
            "Search results".into()
        } else {
            section.title().into()
        };
        let empty = searching && results.is_empty();

        div()
            .id("settings")
            .key_context(SETTINGS)
            .track_focus(&self.settings_page.focus)
            .role(gpui::Role::Region)
            .aria_label("Settings")
            .on_action(
                cx.listener(|this, _: &SelectNext, window, cx| {
                    this.settings_step(true, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SelectPrev, window, cx| {
                    this.settings_step(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &EnterRows, window, cx| this.settings_enter_rows(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &LeaveRows, window, cx| this.settings_leave_rows(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Activate, window, cx| this.settings_activate(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Increase, window, cx| {
                    this.settings_adjust(true, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &Decrease, window, cx| {
                    this.settings_adjust(false, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.focus_settings_search(window, cx)
            }))
            .on_action(cx.listener(Self::settings_close))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_row()
            .child(self.render_settings_sections(&results, searching, ring, &theme, window, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(theme.reader_bg)
                    .child(
                        window_drag_area(div().id("settings-header"))
                            .h(super::HEADER_MIN)
                            .min_h(HEADER_HEIGHT)
                            .flex_none()
                            .px(SPACE_2XL)
                            .flex()
                            .items_center()
                            .gap(SPACE_LG)
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .id("settings-title")
                                    .role(gpui::Role::Heading)
                                    .text_size(TEXT_LG)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(SPACE_SM)
                                    .text_size(TEXT_SM)
                                    .text_color(theme.text_faint)
                                    .child(if searching { "Clear" } else { "Back" })
                                    .child(keys_caps("escape", &theme)),
                            ),
                    )
                    .map(|this| {
                        if empty {
                            this.child(empty_state(
                                Icon::Search,
                                format!("No settings match “{query}”"),
                                Some("Try a word like dark, font or density"),
                                &theme,
                            ))
                        } else {
                            // The scroll bar sits beside the rows, outside
                            // the list (MB-068).
                            this.child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        div()
                                            .id("settings-rows")
                                            .role(gpui::Role::List)
                                            .aria_label(title.clone())
                                            .size_full()
                                            .overflow_y_scroll()
                                            .track_scroll(&self.settings_page.scroll)
                                            .px(SPACE_2XL)
                                            .pt(SPACE_LG)
                                            .pb(SPACE_2XL)
                                            .flex()
                                            .flex_col()
                                            // Rows are the scroll view's own children,
                                            // so `scroll_to_item` can find them by index.
                                            .children(children.into_iter().map(|child| {
                                                div()
                                                    .flex_none()
                                                    .max_w(CONTENT_MAX)
                                                    .flex()
                                                    .flex_col()
                                                    .child(child)
                                            })),
                                    )
                                    .vertical_scrollbar(&self.settings_page.scroll),
                            )
                        }
                    }),
            )
    }

    fn render_settings_sections(
        &self,
        results: &[Match],
        searching: bool,
        ring: bool,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let window_active = window.is_window_active();
        let search = self.render_settings_search(searching, theme, window, cx);
        let items = Section::ALL.iter().map(|&section| {
            let count = results
                .iter()
                .filter(|m| m.entry.section() == section)
                .count();
            let active = !searching && self.settings_page.section == section;
            let dim = searching && count == 0;
            let cursor = active && ring && self.settings_page.row.is_none();
            div()
                .id(("settings-section", section as usize))
                .role(gpui::Role::Tab)
                .aria_label(if searching {
                    format!(
                        "{}, {}",
                        section.title(),
                        super::status::plural(count, "result", "results")
                    )
                } else {
                    section.title().to_string()
                })
                .aria_selected(active)
                .h(ROW_SM)
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .text_size(TEXT_BASE)
                .map(|this| {
                    if active {
                        this.bg(if window_active {
                            theme.nav_active_bg
                        } else {
                            theme.selected_bg
                        })
                        .text_color(theme.text)
                        .font_weight(FontWeight::MEDIUM)
                    } else {
                        this.text_color(if dim {
                            theme.text_faint
                        } else {
                            theme.text_muted
                        })
                        .hover(|s| s.bg(theme.hover_bg))
                    }
                })
                .when(cursor, |this| this.shadow(focus_ring(theme)))
                .child(icon(
                    section.icon(),
                    ICON_LG,
                    if active {
                        theme.icon_active
                    } else {
                        theme.icon
                    },
                ))
                .child(div().flex_1().truncate().child(section.title()))
                .when(searching && count > 0, |this| {
                    this.child(
                        div()
                            .text_size(TEXT_SM)
                            .font_features(tabular_figures())
                            .text_color(theme.text_muted)
                            .child(count.to_string()),
                    )
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.clear_settings_search(window, cx);
                    this.set_section(section, cx);
                    this.settings_page.keyboard = false;
                    window.focus(&this.settings_page.focus, cx);
                }))
        });

        div()
            .w(SECTIONS_WIDTH)
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.sidebar_bg)
            .border_r_1()
            .border_color(theme.border)
            .child(
                window_drag_area(div().id("settings-sections-header"))
                    .h(super::HEADER_MIN)
                    .min_h(HEADER_HEIGHT)
                    .flex_none()
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .text_size(TEXT_LG)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child(search)
            .child(
                div()
                    .id("settings-sections")
                    .role(gpui::Role::TabList)
                    .aria_label("Settings sections")
                    .flex_1()
                    .min_h_0()
                    .px(SPACE_MD)
                    .pb(SPACE_MD)
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .overflow_y_scrollbar()
                    .children(items),
            )
    }

    /// The search field: an accent border while it has the keyboard, and
    /// ✕ to clear it, keeping the keyboard there (MA-082).
    fn render_settings_search(
        &self,
        searching: bool,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focused = self
            .settings_page
            .search
            .focus_handle(cx)
            .is_focused(window);
        super::search_field(Some(focused), theme)
            .key_context(SETTINGS_SEARCH)
            .mx(SPACE_MD)
            .mb(SPACE_SM)
            .h(CONTROL_LG)
            .pl(SPACE_SM)
            .pr(if searching { SPACE_2XS } else { SPACE_SM })
            .child(icon(Icon::Search, ICON_MD, theme.text_faint))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.settings_page.search)
                        .appearance(false)
                        .px(px(0.)),
                ),
            )
            .map(|this| {
                if searching {
                    this.child(
                        icon_button_small("settings-clear-search", Icon::X, "Clear search", theme)
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                window.prevent_default()
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.clear_settings_search(window, cx);
                                this.focus_settings_search(window, cx);
                            })),
                    )
                } else {
                    this.child(keys_caps("/", theme))
                }
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_settings_row(
        &self,
        entry: Entry,
        found: Option<&Match>,
        cursor: bool,
        ring: bool,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        // By color alone, so the text doesn't shift as it's typed (ME-079).
        let highlight = HighlightStyle {
            color: Some(theme.match_text),
            ..Default::default()
        };
        let styled = |text: &'static str, ranges: Option<&Vec<std::ops::Range<usize>>>| {
            let ranges: Vec<_> = ranges
                .map(|r| r.iter().map(|r| (r.clone(), highlight)).collect())
                .unwrap_or_default();
            StyledText::new(text).with_highlights(ranges)
        };
        // Full at once, then a fade to the cursor's color; moving the
        // cursor away ends it.
        let page = &self.settings_page;
        let flash = (page.flash == Some(entry) && cursor).then(|| {
            let id = (
                ElementId::from(("settings-flash", page.flash_serial)),
                "lit",
            );
            let target = if page.flash_fading { 0. } else { 1. };
            if cx.reduce_motion() {
                motion::fade(id, 1., target, window, cx)
            } else {
                gpui::base::spring(id, target, SPRING_PANE, window, cx)
            }
        });
        let flash = flash.filter(|p| *p > 0.);
        let in_results = found.is_some();
        let control = match entry {
            Entry::Setting(id) => Some(self.control(id, window, cx)),
            Entry::Shortcut(command) => Some(Control::Keys(
                self.settings_page.shortcut_bindings(command).to_vec(),
            )),
            Entry::Section(_) => None,
        };
        let compact = matches!(entry, Entry::Shortcut(_));
        let label = entry.label();
        let description = match (&control, found) {
            // The account's own row says whose it is; info rows say it.
            (Some(Control::Account(_) | Control::Info(_)), None) => "",
            (Some(Control::Slider { locked: true, .. }), None) => {
                "Reduce Transparency or Increase Contrast is on in System Settings, so the window is opaque."
            }
            _ => entry.description(),
        };
        let confirming = entry == Entry::Setting(SettingId::SignOut)
            && self.settings_page.confirm_sign_out
            && !in_results;
        // Why Make Default failed, until it works (ME-074).
        let error = (entry == Entry::Setting(SettingId::DefaultMailApp) && !in_results)
            .then(shell::default_mail_app_error)
            .flatten();

        // MA-073: what macOS says, above Turn On or Open System Settings.
        let permission = (entry == Entry::Setting(SettingId::NotificationPermission)
            && !in_results
            && matches!(control, Some(Control::Button { .. })))
        .then(|| notify::permission(cx).label());

        let leading = match (&control, entry) {
            (Some(Control::Account(me)), _) => Some(
                account_avatar(
                    self.backend.read(cx).profile_photo(),
                    me.initials(),
                    theme.avatar_color(&me.email),
                    AVATAR_MD,
                    &theme,
                )
                .into_any_element(),
            ),
            (_, Entry::Section(section)) => {
                Some(icon(section.icon(), ICON_LG, theme.icon).into_any_element())
            }
            _ => None,
        };
        let (title, subtitle): (AnyElement, Option<AnyElement>) = match &control {
            Some(Control::Account(me)) if !in_results => (
                div().child(me.name.clone()).into_any_element(),
                Some(
                    div()
                        .child(account_line(&me.email, self.backend.read(cx)))
                        .into_any_element(),
                ),
            ),
            // Selectable, to copy a status into a report (MB-035).
            Some(Control::Info(text)) if !in_results => (
                div().child(label).into_any_element(),
                Some(
                    div()
                        .child(
                            super::selectable::Selectable::plain(
                                ("settings-info", ix),
                                text.clone(),
                                theme.text_selection,
                            )
                            .silent(),
                        )
                        .into_any_element(),
                ),
            ),
            _ if permission.is_some() => (
                div().child(label).into_any_element(),
                Some(div().children(permission).into_any_element()),
            ),
            _ if error.is_some() => (
                div().child(label).into_any_element(),
                Some(
                    div()
                        .text_color(theme.text_danger)
                        .children(error.clone().map(|error| {
                            super::selectable::Selectable::plain(
                                ("settings-error", ix),
                                error,
                                theme.text_selection,
                            )
                            .silent()
                        }))
                        .into_any_element(),
                ),
            ),
            _ if confirming => (
                div().child("Sign out?").into_any_element(),
                Some(
                    div()
                        .text_color(theme.text_danger)
                        .child(format!(
                            "{} and its cached mail and calendar are removed from this Mac.",
                            self.data.me().email
                        ))
                        .into_any_element(),
                ),
            ),
            _ => (
                styled(label, found.map(|m| &m.label)).into_any_element(),
                (!description.is_empty())
                    .then(|| styled(description, found.map(|m| &m.description)).into_any_element()),
            ),
        };

        let trailing: Option<AnyElement> = match control {
            Some(Control::Switch(on)) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
                };
                Some(
                    controls::switch(
                        ("settings-switch", id as usize),
                        on,
                        label,
                        &theme,
                        window,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.settings_control_changed(id, Change::Switch(!on), window, cx);
                    }))
                    .into_any_element(),
                )
            }
            Some(Control::Choice(selected, options)) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
                };
                let pick = Self::settings_listener(id, Change::Choice, cx);
                Some(
                    controls::segmented(
                        ("settings-choice", id as usize),
                        label,
                        options,
                        selected,
                        &theme,
                        window,
                        cx,
                        pick,
                    )
                    .into_any_element(),
                )
            }
            Some(Control::Menu(selected, options)) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
                };
                Some(self.settings_menu(ix, id, label, selected, options, &theme, window, cx))
            }
            Some(Control::Stepper {
                value,
                can_down,
                can_up,
            }) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
                };
                Some(
                    controls::stepper(
                        ("settings-stepper", id as usize),
                        label,
                        value,
                        (can_down, can_up),
                        &theme,
                        Self::settings_listener(id, Change::Step, cx),
                    )
                    .into_any_element(),
                )
            }
            Some(Control::Slider {
                value,
                text,
                locked,
            }) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
                };
                Some(if locked {
                    div()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .child("Opaque")
                        .into_any_element()
                } else {
                    controls::slider(
                        ("settings-slider", id as usize),
                        label,
                        value,
                        text,
                        &theme,
                        Self::settings_listener(id, Change::Set, cx),
                    )
                    .into_any_element()
                })
            }
            Some(Control::Keys(bindings)) => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_XS)
                    .children(super::bindings_caps(bindings.iter(), &theme))
                    .into_any_element(),
            ),
            Some(Control::Info(_)) => None,
            Some(Control::Button { label, danger }) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
                };
                let armed = self.settings_page.sign_out_armed;
                let keyboard_ring = |on: bool| {
                    let ring = focus_ring(&theme);
                    move |el: gpui::Stateful<gpui::Div>| {
                        if on { el.shadow(ring) } else { el }
                    }
                };
                let press = |name: &'static str| {
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if name == "settings-cancel" {
                            this.settings_page.confirm_sign_out = false;
                            cx.notify();
                        } else {
                            this.press_setting(id, window, cx);
                        }
                    })
                };
                Some(if confirming {
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child(
                            push_button(
                                ("settings-cancel", ix),
                                "Cancel",
                                ButtonKind::Plain,
                                &theme,
                            )
                            .map(keyboard_ring(cursor && ring && !armed))
                            .on_click(press("settings-cancel")),
                        )
                        .child(
                            push_button(
                                ("settings-confirm", ix),
                                label,
                                ButtonKind::Destructive,
                                &theme,
                            )
                            .map(keyboard_ring(cursor && ring && armed))
                            .on_click(press("settings-confirm")),
                        )
                        .into_any_element()
                } else {
                    let kind = if danger {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Plain
                    };
                    push_button(("settings-button", ix), label, kind, &theme)
                        .on_click(press("settings-button"))
                        .into_any_element()
                })
            }
            Some(Control::Account(_)) => Some(super::status::status_line(
                self.backend.read(cx).status(),
                chrono::Utc::now().timestamp_millis(),
                &theme,
            )),
            None => Some(icon(Icon::ChevronRight, ICON_MD, theme.text_faint).into_any_element()),
        };

        div()
            .id(("settings-row", ix))
            .role(gpui::Role::ListItem)
            .aria_label(match entry {
                Entry::Section(_) => format!("Open {label}"),
                _ => label.to_string(),
            })
            .aria_selected(cursor)
            .when(cursor, |this| this.aria_active_descendant())
            .when(!description.is_empty(), |this| {
                this.aria_description(description)
            })
            .when_some(error, |this, error| this.aria_description(error))
            .min_h(if compact { ROW_MIN_COMPACT } else { ROW_MIN })
            .px(SPACE_MD)
            .py(if compact { SPACE_XS } else { SPACE_SM })
            .flex()
            .items_center()
            .gap(SPACE_LG)
            .rounded(RADIUS_MD)
            .map(|this| {
                // The keyboard's row as in every overlay (ME-080).
                if let Some(p) = flash {
                    let rest = if ring {
                        theme.selected_bg
                    } else {
                        theme.selected_focused_bg.opacity(0.)
                    };
                    this.bg(mix(rest, theme.selected_focused_bg, p))
                } else if cursor && ring {
                    this.bg(theme.selected_bg)
                } else {
                    this.hover(|s| s.bg(theme.hover_bg))
                }
            })
            .children(leading)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .child(
                        div()
                            .text_size(TEXT_BASE)
                            .font_weight(if compact {
                                FontWeight::NORMAL
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(if compact {
                                theme.text_muted
                            } else {
                                theme.text
                            })
                            .child(title),
                    )
                    .when(!compact || in_results, |this| {
                        this.children(subtitle.map(|subtitle| {
                            div()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_muted)
                                .child(subtitle)
                        }))
                    }),
            )
            .children(trailing.map(|t| div().flex_none().child(t)))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                if !confirming {
                    this.settings_page.confirm_sign_out = false;
                }
                if in_results {
                    this.jump_to_setting(entry, window, cx);
                } else {
                    this.settings_page.row = Some(ix);
                    this.settings_page.flash = None;
                    window.focus(&this.settings_page.focus, cx);
                }
                this.settings_page.keyboard = false;
                cx.notify();
            }))
            .into_any_element()
    }
}

impl PigeonApp {
    /// A change made with the mouse on a row's control. The control took
    /// focus on the press; the page takes it back so its keys keep working.
    fn settings_control_changed(
        &mut self,
        id: SettingId,
        change: Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_page.keyboard = false;
        self.change_setting(id, change, window, cx);
        window.focus(&self.settings_page.focus, cx);
    }

    /// A shared control's callback that changes setting `id`.
    fn settings_listener<T: 'static>(
        id: SettingId,
        change: impl Fn(T) -> Change + 'static,
        cx: &mut Context<Self>,
    ) -> impl Fn(T, &mut Window, &mut App) + 'static {
        let this = cx.entity().downgrade();
        move |value, window, cx| {
            this.update(cx, |this, cx| {
                this.settings_control_changed(id, change(value), window, cx)
            })
            .ok();
        }
    }

    /// A pop-up menu: a button showing the choice, and under it, while
    /// open, the list. Default calendar and Event length.
    #[allow(clippy::too_many_arguments)]
    fn settings_menu(
        &self,
        ix: usize,
        id: SettingId,
        label: &'static str,
        selected: usize,
        options: Vec<(String, Option<Hsla>)>,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.settings_page.menu == Some(id);
        let width = MENU_WIDTH;
        let highlighted = self.settings_page.menu_cursor;
        let swatches = options.iter().any(|(_, color)| color.is_some());
        let swatch = |color: Option<Hsla>| {
            swatches.then(|| {
                div()
                    .flex_none()
                    .size(SPACE_SM + SPACE_2XS)
                    .rounded_full()
                    .when_some(color, |d, color| d.bg(color))
            })
        };
        let current = options[selected].clone();
        let menu = open.then(|| {
            let rows = options.iter().enumerate().map(|(option, (name, color))| {
                let chosen = option == selected;
                let lit = option == highlighted;
                menu_row(
                    ("settings-menu-row", option),
                    name.clone(),
                    chosen,
                    lit,
                    theme,
                )
                .when(lit, |row| row.aria_active_descendant())
                .child(menu_check(chosen, theme))
                .children(swatch(*color))
                .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.settings_control_changed(id, Change::Choice(option), window, cx);
                }))
            });
            motion::enter(
                ("settings-menu", id as usize),
                Rise::Down,
                dropdown("settings-menu-list", label, width, theme)
                    .mt(SPACE_XS)
                    .max_h(MENU_MAX_HEIGHT)
                    .overflow_y_scroll()
                    .children(rows),
            )
        });
        let size = window.viewport_size();
        div()
            .child(
                focusable_control(div().id(("settings-menu-button", ix)), theme)
                    .role(gpui::Role::ComboBox)
                    .aria_label(format!("{label}: {}", current.0))
                    .aria_expanded(open)
                    .w(width)
                    .h(CONTROL_MD)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .bg(theme.control_track)
                    .text_size(TEXT_SM)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .hover(|s| s.bg(theme.selected_bg))
                    .active(|s| s.bg(theme.selected_bg).opacity(0.8))
                    .children(swatch(current.1))
                    .child(div().flex_1().min_w_0().truncate().child(current.0))
                    .child(icon(Icon::ChevronDown, ICON_MD, theme.text_faint))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if this.settings_page.menu == Some(id) {
                            this.settings_page.menu = None;
                            cx.notify();
                        } else {
                            this.open_settings_menu(id, selected, cx);
                        }
                        window.focus(&this.settings_page.focus, cx);
                    })),
            )
            .when(open, |d| {
                d.child(
                    deferred(
                        anchored().position(point(px(0.), px(0.))).child(
                            div()
                                .id("settings-menu-backdrop")
                                .occlude()
                                .w(size.width)
                                .h(size.height)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.settings_page.menu = None;
                                        cx.stop_propagation();
                                        cx.notify();
                                    }),
                                ),
                        ),
                    )
                    .with_priority(1),
                )
            })
            .children(menu.map(popup))
            .into_any_element()
    }
}

/// `from` to `to` at `t`, alpha too.
fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let (a, b) = (gpui::Rgba::from(from), gpui::Rgba::from(to));
    let t = t.clamp(0., 1.);
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    gpui::Rgba {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: lerp(a.a, b.a),
    }
    .into()
}

fn menu_options(names: &[&str]) -> Vec<(String, Option<Hsla>)> {
    names.iter().map(|name| (name.to_string(), None)).collect()
}

fn group_header(title: &'static str, theme: &Theme) -> AnyElement {
    super::section_caption(title, theme)
        .pt(SPACE_LG)
        .pb(SPACE_XS)
        .px(SPACE_MD)
        .into_any_element()
}

/// "Google · a@b.com · Mail, Calendar and Contacts allowed" (MB-058).
fn account_line(email: &str, backend: &crate::backend::Backend) -> String {
    let granted = super::status::granted(backend);
    let provider = (backend.source() == Source::Google).then_some("Google");
    provider
        .into_iter()
        .chain([email])
        .chain((!granted.is_empty()).then_some(granted.as_str()))
        .collect::<Vec<_>>()
        .join(" · ")
}

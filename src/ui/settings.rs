//! The settings page (⌘,). It replaces the panes beside the rail: the
//! sections, with a search over every setting, on the left, and the
//! section's rows on the right. A row's value is read from where the app
//! keeps it, so the shortcuts and menus that change the same things stay in
//! step.
//!
//! Keys: ↑/↓ (j/k) move through the sections or the rows, Tab and ⇧Tab go
//! between the two, Space/Enter toggles, ←/→ change a value, `/` or ⌘F
//! search, Esc clears the search, then goes back.

use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, FontWeight,
    HighlightStyle, InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled, StyledText, Subscription,
    Task, Window, WindowAppearance, actions,
    component::input::{Input, InputEvent, InputState},
    div, point,
    prelude::FluentBuilder as _,
    px,
};

use super::{HEADER_HEIGHT, avatar, binding_caps, empty_state, icon, keycap, window_drag_area};
use crate::ui::style::*;
use crate::{
    actions::{COMMANDS, Scope, SignOut, SyncNow},
    app::{Density, PigeonApp, shell},
    assets::Icon,
    backend::Source,
    data::Contact,
    native::{self, LoginItem},
    settings::{self, Appearance, Entry, Match, RemoteImages, Section, SettingId},
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

/// How long a setting jumped to from the search stays lit.
const FLASH: Duration = Duration::from_millis(1200);
const SECTIONS_WIDTH: Pixels = px(220.);
const CONTENT_MAX: Pixels = px(680.);

const APPEARANCES: &[&str] = &["System", "Light", "Dark"];
const SCOPES: &[&str] = &["Current view", "All mail"];
const VIEWS: &[&str] = &["Day", "Week", "Month"];
const IMAGES: &[&str] = &["Ask", "Always", "Never"];
// W5 (CAL-41).
const EVENT_LENGTHS: &[&str] = &["15 min", "30 min", "45 min", "1 hr", "1.5 hrs", "2 hrs"];
const EVENT_MINUTES: [u32; 6] = [15, 30, 45, 60, 90, 120];

pub struct SettingsPage {
    pub open: bool,
    section: Section,
    /// The row the keyboard is on; none while it's in the section list.
    row: Option<usize>,
    /// The highlighted search result.
    result: usize,
    /// Moved with the keys since the last click, so the ring shows.
    keyboard: bool,
    /// A setting jumped to, lit for a moment.
    flash: Option<Entry>,
    /// Scroll the cursor into view at the next render, once its rows exist.
    pending_scroll: Option<Reveal>,
    pub focus: FocusHandle,
    pub search: Entity<InputState>,
    scroll: ScrollHandle,
    /// Sign Out was pressed once: the row asks to confirm.
    confirm_sign_out: bool,
    _flash: Option<Task<()>>,
}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<PigeonApp>) -> (Self, Subscription) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        let events = cx.subscribe_in(&search, window, PigeonApp::on_settings_search);
        let page = Self {
            open: false,
            section: Section::Appearance,
            row: None,
            result: 0,
            keyboard: false,
            flash: None,
            pending_scroll: None,
            focus: cx.focus_handle(),
            search,
            scroll: ScrollHandle::new(),
            confirm_sign_out: false,
            _flash: None,
        };
        (page, events)
    }
}

/// What a row shows on its right.
enum Control {
    Switch(bool),
    Choice(usize, &'static [&'static str]),
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
    Reset,
}

/// Shortcuts are looked up in their app's contexts.
fn shortcut_bindings(ix: usize, window: &Window) -> Vec<KeyBinding> {
    let command = &COMMANDS[ix];
    let scope = match command.group.scope() {
        Scope::Calendar => Scope::Calendar,
        Scope::Mail | Scope::Global => Scope::Mail,
    };
    live_bindings(&*(command.action)(), scope, window)
}

/// Shortcuts without a binding are left out, as in the `?` sheet.
fn shown(entry: Entry, window: &Window) -> bool {
    match entry {
        Entry::Shortcut(ix) => !shortcut_bindings(ix, window).is_empty(),
        _ => true,
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
            self.close_overlays(window, cx);
            let page = &mut self.settings_page;
            page.open = true;
            page.row = None;
            page.keyboard = false;
            window.focus(&page.focus, cx);
            cx.notify();
        }
    }

    /// Sign Out from ⌘K or the menu (ACC-7): Settings › Accounts with the
    /// confirmation showing, as its own button asks.
    pub(crate) fn ask_sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings_page.open {
            self.close_overlays(window, cx);
            self.settings_page.open = true;
        }
        self.jump_to_setting(Entry::Setting(SettingId::SignOut), window, cx);
        self.settings_page.confirm_sign_out = true;
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

    fn settings_results(&self, window: &Window, cx: &App) -> Vec<Match> {
        settings::search(&self.settings_query(cx))
            .into_iter()
            .filter(|m| shown(m.entry, window))
            .collect()
    }

    fn section_rows(&self, section: Section, window: &Window) -> Vec<Entry> {
        settings::entries(section)
            .into_iter()
            .filter(|e| shown(*e, window))
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
        let results = self.settings_results(window, cx);
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
            entry => self
                .section_rows(section, window)
                .iter()
                .position(|e| *e == entry),
        };
        let page = &mut self.settings_page;
        page.section = section;
        page.row = row;
        page.keyboard = true;
        page.pending_scroll = row.is_some().then_some(Reveal::Top);
        if row.is_none() {
            page.scroll.set_offset(point(px(0.), px(0.)));
        }
        page.flash = row.is_some().then_some(entry);
        page._flash = row.is_some().then(|| {
            cx.spawn(async |this, cx| {
                cx.background_executor().timer(FLASH).await;
                this.update(cx, |this, cx| {
                    this.settings_page.flash = None;
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
        }
        page.row = None;
        page.confirm_sign_out = false;
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
        if !self.settings_query(cx).is_empty() {
            let len = self.settings_results(window, cx).len();
            self.settings_page.result = step(self.settings_page.result, len);
            self.settings_page.pending_scroll = Some(Reveal::Nearest);
        } else if let Some(row) = self.settings_page.row {
            let len = self.section_rows(self.settings_page.section, window).len();
            self.settings_page.row = Some(step(row, len));
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
            && !self
                .section_rows(self.settings_page.section, window)
                .is_empty()
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
    fn cursor_setting(&self, window: &Window) -> Option<SettingId> {
        let row = self.settings_page.row?;
        match self
            .section_rows(self.settings_page.section, window)
            .get(row)?
        {
            Entry::Setting(id) => Some(*id),
            _ => None,
        }
    }

    fn settings_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings_query(cx).is_empty() {
            return self.open_settings_result(window, cx);
        }
        if self.settings_page.row.is_none() {
            return self.settings_enter_rows(window, cx);
        }
        let Some(id) = self.cursor_setting(window) else {
            return;
        };
        let change = match self.control(id, window, cx) {
            Control::Switch(on) => Change::Switch(!on),
            Control::Choice(ix, options) => Change::Choice((ix + 1) % options.len()),
            Control::Stepper { .. } => Change::Reset,
            Control::Button { .. } => return self.press_setting(id, window, cx),
            Control::Account(_) | Control::Keys(_) | Control::Info(_) => return,
        };
        self.change_setting(id, change, window, cx);
    }

    /// Sync Now and Sign Out; Sign Out asks first, signed in to Google.
    fn press_setting(&mut self, id: SettingId, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            SettingId::SyncNow => window.dispatch_action(Box::new(SyncNow), cx),
            SettingId::TestNotification => crate::notify::send_test(cx),
            SettingId::SignOut => {
                let google = self.backend.read(cx).source() == Source::Google;
                if google && !self.settings_page.confirm_sign_out {
                    self.settings_page.confirm_sign_out = true;
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
        let Some(id) = self.cursor_setting(window) else {
            if !up {
                self.settings_leave_rows(window, cx);
            }
            return;
        };
        let change = match self.control(id, window, cx) {
            Control::Switch(_) => Change::Switch(up),
            Control::Choice(ix, options) if up => Change::Choice((ix + 1).min(options.len() - 1)),
            Control::Choice(ix, _) => Change::Choice(ix.saturating_sub(1)),
            Control::Stepper { .. } => Change::Step(if up { 1 } else { -1 }),
            Control::Account(_) | Control::Keys(_) | Control::Info(_) | Control::Button { .. } => {
                return;
            }
        };
        self.change_setting(id, change, window, cx);
    }

    fn settings_close(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.settings_page.confirm_sign_out) {
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
                match self.settings.appearance {
                    Appearance::System => 0,
                    Appearance::Light => 1,
                    Appearance::Dark => 2,
                },
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
            SettingId::ReduceMotion => Control::Switch(self.settings.reduce_motion),
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
            SettingId::TimeFormat => Control::Switch(calendar.uses_24_hour()),
            SettingId::EventLength => {
                let minutes = calendar.event_minutes();
                let ix = (0..EVENT_MINUTES.len())
                    .min_by_key(|&i| EVENT_MINUTES[i].abs_diff(minutes))
                    .unwrap_or(3);
                Control::Choice(ix, EVENT_LENGTHS)
            }
            SettingId::TimeZone => Control::Info(calendar.time_zone_note()),
            SettingId::Account => Control::Account(self.data.me()),
            SettingId::Access => Control::Info(super::status::scopes(self.backend.read(cx))),
            SettingId::LastSync => {
                Control::Info(super::status::last_sync(self.backend.read(cx), now_ms()))
            }
            SettingId::SyncNow => Control::Button {
                label: "Sync Now",
                danger: false,
            },
            SettingId::SignOut => Control::Button {
                label: "Sign Out",
                danger: true,
            },
            // Sprint 2 W1.
            SettingId::Notifications => Control::Switch(settings::current(cx).notifications),
            SettingId::TestNotification => Control::Button {
                label: "Send",
                danger: false,
            },
            SettingId::OpenAtLogin => match native::login_item() {
                LoginItem::Unavailable(why) => Control::Info(why.into()),
                LoginItem::On | LoginItem::NeedsApproval => Control::Switch(true),
                LoginItem::Off => Control::Switch(false),
            },
            SettingId::LoginItemStatus => Control::Info(shell::login_item_status()),
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
        match id {
            SettingId::Appearance => {
                let appearance = [Appearance::System, Appearance::Light, Appearance::Dark][choice];
                self.set_appearance(appearance, window, cx);
            }
            SettingId::TextSize => match change {
                Change::Step(delta) => {
                    self.set_text_scale(self.text_scale + delta as f32 * TEXT_SCALE_STEP, cx)
                }
                _ => self.set_text_scale(1., cx),
            },
            SettingId::ReduceMotion => self.set_reduce_motion(on, cx),
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
                // Kept on `saved` itself, where the reader reads it (ATT-1).
                self.saved.remote_images =
                    [RemoteImages::Ask, RemoteImages::Always, RemoteImages::Never][choice];
                self.saved.notifications = settings::current(cx).notifications;
                settings::remember(self.saved, cx);
                let map = self.saved.to_map();
                self.backend.update(cx, |b, cx| b.save_settings(map, cx));
            }
            SettingId::Notifications => shell::set_notifications(on, cx),
            SettingId::OpenAtLogin => shell::set_open_at_login(on, cx),
            SettingId::TimeFormat => self.calendar.update(cx, |c, cx| {
                c.prefs.time_24h = Some(on);
                c.apply_display();
                cx.notify();
            }),
            SettingId::EventLength => self.calendar.update(cx, |c, cx| {
                c.prefs.event_minutes = Some(EVENT_MINUTES[choice]);
                c.apply_display();
                cx.notify();
            }),
            SettingId::TimeZone
            | SettingId::Account
            | SettingId::Access
            | SettingId::LastSync
            | SettingId::SyncNow
            | SettingId::SignOut
            | SettingId::TestNotification
            | SettingId::LoginItemStatus => {}
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
        cx.set_window_appearance(match appearance {
            Appearance::System => None,
            Appearance::Light => Some(WindowAppearance::Light),
            Appearance::Dark => Some(WindowAppearance::Dark),
        });
        self.sync_theme(window, cx);
    }

    /// The theme for the user's choice, or for the window's appearance.
    pub(crate) fn sync_theme(&self, window: &mut Window, cx: &mut App) {
        let appearance = match self.settings.appearance {
            Appearance::System => window.appearance(),
            Appearance::Light => WindowAppearance::Light,
            Appearance::Dark => WindowAppearance::Dark,
        };
        Theme::apply(appearance, window, cx);
    }

    /// On: reduce motion whatever the system says. Off: follow it again.
    pub(crate) fn set_reduce_motion(&mut self, on: bool, cx: &mut Context<Self>) {
        if on {
            self.system_reduce_motion = cx.reduce_motion();
            cx.set_reduce_motion(true);
        } else {
            // Back to what Base last wrote, so it takes the flag over again.
            cx.set_reduce_motion(self.system_reduce_motion);
            gpui::base::apply_system_reduce_motion(cx);
        }
        self.settings.reduce_motion = on;
    }

    // Rendering.

    pub(crate) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let query = self.settings_query(cx);
        let searching = !query.is_empty();
        let results = if searching {
            self.settings_results(window, cx)
        } else {
            Vec::new()
        };
        let focused = self.settings_page.focus.is_focused(window);
        let ring = focused && self.settings_page.keyboard;
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
            let rows = self.section_rows(section, window);
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
            .child(self.render_settings_sections(&results, searching, ring, &theme, cx))
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
                            .h(HEADER_HEIGHT)
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
                                    .child(keycap("Esc", &theme))
                                    .child(if searching { "to clear" } else { "to go back" }),
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
                            this.child(
                                div()
                                    .id("settings-rows")
                                    .role(gpui::Role::List)
                                    .aria_label(title.clone())
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.settings_page.scroll)
                                    .px(SPACE_2XL)
                                    .pt(SPACE_LG)
                                    .pb(SPACE_2XL)
                                    .flex()
                                    .flex_col()
                                    // Rows are the scroll view's own children, so
                                    // `scroll_to_item` can find them by index.
                                    .children(children.into_iter().map(|child| {
                                        div()
                                            .flex_none()
                                            .max_w(CONTENT_MAX)
                                            .flex()
                                            .flex_col()
                                            .child(child)
                                    })),
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
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                .aria_label(section.title())
                .aria_selected(active)
                .h(px(30.))
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .text_size(TEXT_BASE)
                .cursor_pointer()
                .map(|this| {
                    if active {
                        this.bg(theme.nav_active_bg)
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
                    px(16.),
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
                    .h(HEADER_HEIGHT)
                    .flex_none()
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .text_size(TEXT_LG)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child(
                div()
                    .key_context(SETTINGS_SEARCH)
                    .mx(SPACE_MD)
                    .mb(SPACE_SM)
                    .h(px(30.))
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .bg(theme.search_bg)
                    .text_size(TEXT_BASE)
                    .child(icon(Icon::Search, px(13.), theme.text_faint))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.settings_page.search)
                                .appearance(false)
                                .px(px(0.)),
                        ),
                    )
                    .when(!searching, |this| this.child(keycap("/", theme))),
            )
            .child(
                div()
                    .id("settings-sections")
                    .role(gpui::Role::TabList)
                    .aria_label("Settings sections")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(SPACE_MD)
                    .pb(SPACE_MD)
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .children(items),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_settings_row(
        &self,
        entry: Entry,
        found: Option<&Match>,
        cursor: bool,
        ring: bool,
        ix: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let highlight = HighlightStyle {
            color: Some(theme.match_text),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        };
        let styled = |text: &'static str, ranges: Option<&Vec<std::ops::Range<usize>>>| {
            let ranges: Vec<_> = ranges
                .map(|r| r.iter().map(|r| (r.clone(), highlight)).collect())
                .unwrap_or_default();
            StyledText::new(text).with_highlights(ranges)
        };
        let flash = self.settings_page.flash == Some(entry);
        let in_results = found.is_some();
        let control = match entry {
            Entry::Setting(id) => Some(self.control(id, window, cx)),
            Entry::Shortcut(command) => Some(Control::Keys(shortcut_bindings(command, window))),
            Entry::Section(_) => None,
        };
        let compact = matches!(entry, Entry::Shortcut(_));
        let label = entry.label();
        let description = match (&control, found) {
            // The account's own row says whose it is; info rows say it.
            (Some(Control::Account(_) | Control::Info(_)), None) => "",
            _ => entry.description(),
        };
        let confirming = entry == Entry::Setting(SettingId::SignOut)
            && self.settings_page.confirm_sign_out
            && !in_results;

        let leading = match (&control, entry) {
            (Some(Control::Account(me)), _) => Some(
                avatar(
                    me.initials(),
                    theme.avatar_color(&me.email),
                    px(32.),
                    &theme,
                )
                .into_any_element(),
            ),
            (_, Entry::Section(section)) => {
                Some(icon(section.icon(), px(16.), theme.icon).into_any_element())
            }
            _ => None,
        };
        let (title, subtitle): (AnyElement, Option<AnyElement>) = match &control {
            Some(Control::Account(me)) if !in_results => (
                div().child(me.name.clone()).into_any_element(),
                Some(
                    div()
                        .child(me.email.clone())
                        .child(super::status::granted(self.backend.read(cx)))
                        .into_any_element(),
                ),
            ),
            Some(Control::Info(text)) if !in_results => (
                div().child(label).into_any_element(),
                Some(div().child(text.clone()).into_any_element()),
            ),
            _ if confirming => (
                div().child("Sign out?").into_any_element(),
                Some(
                    div()
                        .text_color(theme.danger)
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
                    switch(("settings-switch", ix), on, label, &theme)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.change_setting(id, Change::Switch(!on), window, cx);
                        }))
                        .into_any_element(),
                )
            }
            Some(Control::Choice(selected, options)) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
                };
                Some(
                    segmented(ix, label, options, selected, &theme, |option| {
                        cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.change_setting(id, Change::Choice(option), window, cx);
                        })
                    })
                    .into_any_element(),
                )
            }
            Some(Control::Stepper {
                value,
                can_down,
                can_up,
            }) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
                };
                let button = |name: &'static str,
                              glyph: Icon,
                              label: &'static str,
                              enabled,
                              delta| {
                    div()
                        .id((name, ix))
                        .role(gpui::Role::Button)
                        .aria_label(label)
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(RADIUS_SM)
                        .bg(theme.control_track)
                        .when(enabled, |this| {
                            this.cursor_pointer()
                                .hover(|s| s.bg(theme.selected_bg))
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.change_setting(id, Change::Step(delta), window, cx);
                                }))
                        })
                        .child(icon(
                            glyph,
                            px(14.),
                            if enabled {
                                theme.icon
                            } else {
                                theme.text_faint
                            },
                        ))
                };
                Some(
                    div()
                        .id(("settings-stepper", ix))
                        .role(gpui::Role::SpinButton)
                        .aria_label(label)
                        .aria_value(value.clone())
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child(button(
                            "settings-smaller",
                            Icon::Minus,
                            "Smaller text",
                            can_down,
                            -1,
                        ))
                        .child(
                            div()
                                .min_w(px(44.))
                                .flex()
                                .justify_center()
                                .text_size(TEXT_SM)
                                .font_weight(FontWeight::MEDIUM)
                                .font_features(tabular_figures())
                                .child(value),
                        )
                        .child(button(
                            "settings-bigger",
                            Icon::Plus,
                            "Bigger text",
                            can_up,
                            1,
                        ))
                        .into_any_element(),
                )
            }
            Some(Control::Keys(bindings)) => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_XS)
                    .children(bindings.iter().enumerate().flat_map(|(ix, binding)| {
                        let or = (ix > 0).then(|| {
                            div()
                                .text_size(TEXT_XS)
                                .text_color(theme.text_faint)
                                .child("or")
                                .into_any_element()
                        });
                        or.into_iter()
                            .chain([binding_caps(binding, &theme).into_any_element()])
                    }))
                    .into_any_element(),
            ),
            Some(Control::Info(_)) => None,
            Some(Control::Button { label, danger }) => {
                let Entry::Setting(id) = entry else {
                    unreachable!()
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
                            button(("settings-cancel", ix), "Cancel", Tone::Plain, &theme)
                                .on_click(press("settings-cancel")),
                        )
                        .child(
                            button(("settings-confirm", ix), label, Tone::Confirm, &theme)
                                .on_click(press("settings-confirm")),
                        )
                        .into_any_element()
                } else {
                    let tone = if danger { Tone::Danger } else { Tone::Plain };
                    button(("settings-button", ix), label, tone, &theme)
                        .on_click(press("settings-button"))
                        .into_any_element()
                })
            }
            Some(Control::Account(_)) => Some(super::status::status_line(
                self.backend.read(cx).status(),
                chrono::Utc::now().timestamp_millis(),
                &theme,
            )),
            None => Some(icon(Icon::ChevronRight, px(14.), theme.text_faint).into_any_element()),
        };

        div()
            .id(("settings-row", ix))
            .role(gpui::Role::ListItem)
            .aria_label(match entry {
                Entry::Section(_) => format!("Open {label}"),
                _ => label.to_string(),
            })
            .aria_selected(cursor)
            .when(!description.is_empty(), |this| {
                this.aria_description(description)
            })
            .min_h(px(if compact { 36. } else { 56. }))
            .px(SPACE_MD)
            .py(if compact { SPACE_XS } else { SPACE_SM })
            .flex()
            .items_center()
            .gap(SPACE_LG)
            .rounded(RADIUS_MD)
            .map(|this| {
                if flash {
                    this.bg(theme.selected_focused_bg)
                } else if cursor {
                    this.bg(theme.hover_bg)
                } else {
                    this.hover(|s| s.bg(theme.hover_bg))
                }
            })
            .when(cursor && ring, |this| this.shadow(focus_ring(&theme)))
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
                    window.focus(&this.settings_page.focus, cx);
                }
                this.settings_page.keyboard = false;
                cx.notify();
            }))
            .into_any_element()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Plain,
    /// Red text: it asks first.
    Danger,
    /// Filled red: the confirmation.
    Confirm,
}

fn button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    tone: Tone,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    focusable_control(div().id(id), theme)
        .role(gpui::Role::Button)
        .aria_label(label)
        .h(HIT_TARGET)
        .px(SPACE_MD)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .text_size(TEXT_SM)
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .map(|this| match tone {
            Tone::Plain => this
                .bg(theme.control_track)
                .text_color(theme.text)
                .hover(|s| s.bg(theme.selected_bg)),
            Tone::Danger => this
                .bg(theme.control_track)
                .text_color(theme.danger)
                .hover(|s| s.bg(theme.selected_bg)),
            Tone::Confirm => this
                .bg(theme.danger)
                .text_color(theme.accent_text)
                .hover(|s| s.opacity(0.9)),
        })
        .child(label)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn group_header(title: &'static str, theme: &Theme) -> AnyElement {
    div()
        .pt(SPACE_LG)
        .pb(SPACE_XS)
        .px(SPACE_MD)
        .text_size(TEXT_XS)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.text_faint)
        .child(title.to_uppercase())
        .into_any_element()
}

fn switch(
    id: impl Into<gpui::ElementId>,
    on: bool,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::Switch)
        .aria_label(label)
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .w(px(32.))
        .h(px(18.))
        .p(SPACE_2XS)
        .flex()
        .items_center()
        .when(on, |this| this.justify_end())
        .rounded_full()
        .cursor_pointer()
        .bg(if on {
            theme.accent
        } else {
            theme.control_track
        })
        .child(
            div()
                .size(px(14.))
                .rounded_full()
                .bg(theme.control_knob)
                .shadow(super::popover_shadow(theme)),
        )
}

fn segmented<L>(
    ix: usize,
    label: &'static str,
    options: &'static [&'static str],
    selected: usize,
    theme: &Theme,
    mut on_select: impl FnMut(usize) -> L,
) -> gpui::Stateful<gpui::Div>
where
    L: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    div()
        .id(("settings-choice", ix))
        .role(gpui::Role::RadioGroup)
        .aria_label(label)
        .flex()
        .p(SPACE_2XS)
        .gap(SPACE_2XS)
        .rounded(RADIUS_MD)
        .bg(theme.control_track)
        .children(options.iter().enumerate().map(|(option, name)| {
            let chosen = option == selected;
            div()
                .id(("settings-option", ix * 8 + option))
                .role(gpui::Role::RadioButton)
                .aria_label(*name)
                .aria_toggled(if chosen {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .h(px(24.))
                .px(SPACE_MD)
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .text_size(TEXT_SM)
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .map(|this| {
                    if chosen {
                        this.bg(theme.segment_bg)
                            .text_color(theme.text)
                            .shadow(super::popover_shadow(theme))
                    } else {
                        this.text_color(theme.text_muted)
                            .hover(|s| s.text_color(theme.text))
                    }
                })
                .child(*name)
                .on_click(on_select(option))
        }))
}

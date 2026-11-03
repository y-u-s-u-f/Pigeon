//! A right-click menu drawn by the app, in its own popover style, with
//! icons, shortcuts, headers and submenus. Built like `NativeMenu` so call
//! sites move over by swapping the type:
//!
//! ```ignore
//! ContextMenu::new()
//!     .item(Item::new("Reply", Box::new(Reply)).icon(Icon::Reply))
//!     .separator()
//!     .item(Item::new("Delete", Box::new(Delete)).icon(Icon::Trash).destructive())
//!     .on_close(|_, cx| ..)
//!     .show(position, window, cx);
//! ```
//!
//! It shows in any window whose root draws `context_menu::layer`; elsewhere
//! `show` falls back to the native menu. While open it takes the keyboard
//! (↑ ↓ → ← ↩ Space ⎋, Home, End and type-to-select) and every click,
//! without taking focus (only VoiceOver's, so it reads the menu); a chosen
//! item's action goes to whatever had focus when it opened. An item's
//! shortcut chooses it; any other modified key closes the menu and goes on
//! to the app.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    Action, AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle, FontWeight,
    Global, Hsla, InteractiveElement as _, IntoElement, KeyBinding, Keystroke, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Rems, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Toggled, Window, WindowId, canvas, component::native_menu::NativeMenu, deferred, div, point,
    prelude::FluentBuilder as _, px,
};

use super::controls::{menu_check, popover_surface};
use super::motion::{
    MENU_AIM_GRACE, MENU_DRAG_CHOOSE, MENU_HOLD, MENU_SUBMENU_DELAY, MENU_TRAVEL, presence,
};
use super::style::*;
use super::{binding_caps, binding_text, dot_marker, icon};
use crate::{assets::Icon, theme::Theme};

/// How wide a menu gets: room for a label and a shortcut, and a cap so a
/// long label truncates instead.
const MENU_MIN_W: Rems = type_size(200.);
const MENU_MAX_W: Rems = type_size(340.);
/// Item rows; a keycap fits with room around it.
const MENU_ROW: Rems = CONTROL_MD;
/// Kept clear between a menu and the window's edge.
const EDGE: Pixels = SPACE_SM;
/// Over every other deferred popover.
const PRIORITY: usize = 10;

/// What a row shows before its label.
#[derive(Clone, Copy)]
pub enum Leading {
    Icon(Icon),
    /// A label's or calendar's color.
    Dot(Hsla),
}

/// One item: an action, or a submenu.
pub struct Item {
    label: SharedString,
    action: Option<Box<dyn Action>>,
    submenu: Option<Vec<Entry>>,
    leading: Option<Leading>,
    disabled: bool,
    /// `Some` for an item with a check column: checked or not.
    checked: Option<bool>,
    destructive: bool,
    binding: Option<KeyBinding>,
}

impl Item {
    pub fn new(label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        Self {
            label: label.into(),
            action: Some(action),
            submenu: None,
            leading: None,
            disabled: false,
            checked: None,
            destructive: false,
            binding: None,
        }
    }

    pub fn submenu(label: impl Into<SharedString>, menu: ContextMenu) -> Self {
        Self {
            label: label.into(),
            action: None,
            submenu: Some(menu.items),
            leading: None,
            disabled: false,
            checked: None,
            destructive: false,
            binding: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.leading = Some(Leading::Icon(icon));
        self
    }

    pub fn dot(mut self, color: Hsla) -> Self {
        self.leading = Some(Leading::Dot(color));
        self
    }

    #[allow(dead_code)] // Until a call site dims one.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// A check mark when `checked`; the column shows either way.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Red: Delete and the like.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    fn selectable(&self) -> bool {
        !self.disabled && (self.action.is_some() || self.submenu.is_some())
    }
}

enum Entry {
    Item(Box<Item>),
    Separator,
    Header(SharedString),
}

impl Entry {
    fn item(&self) -> Option<&Item> {
        match self {
            Entry::Item(item) => Some(&**item),
            _ => None,
        }
    }
}

type OnClose = Box<dyn FnOnce(&mut Window, &mut App)>;

/// A menu to show at the pointer. See the module docs.
#[derive(Default)]
pub struct ContextMenu {
    items: Vec<Entry>,
    label: Option<SharedString>,
    on_close: Option<OnClose>,
}

impl ContextMenu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn item(mut self, item: Item) -> Self {
        self.items.push(Entry::Item(Box::new(item)));
        self
    }

    pub fn separator(mut self) -> Self {
        // Never two in a row, nor one first.
        if matches!(
            self.items.last(),
            Some(Entry::Item(_)) | Some(Entry::Header(_))
        ) {
            self.items.push(Entry::Separator);
        }
        self
    }

    /// A caption over the items after it, such as "3 Conversations".
    pub fn header(mut self, text: impl Into<SharedString>) -> Self {
        self.items.push(Entry::Header(text.into()));
        self
    }

    /// What VoiceOver calls the menu; "Context menu" by default.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Called once the menu has closed, chosen or dismissed, after the
    /// chosen action.
    pub fn on_close(mut self, f: impl FnOnce(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Box::new(f));
        self
    }

    /// The same items as a native menu: headers as dimmed items, without
    /// icons. For call sites that keep the system's menu.
    pub fn into_native(self) -> NativeMenu {
        native(self.items)
    }

    /// Opens the menu at `position` (window coordinates), closing any other
    /// open in this window.
    pub fn show(mut self, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        if self.items.is_empty() {
            return;
        }
        if !cx.default_global::<Menus>().hosts.contains(&id) {
            let on_close = self.on_close.take();
            self.into_native().show(position, window, cx);
            if let Some(on_close) = on_close {
                window.defer(cx, on_close);
            }
            return;
        }
        dismiss(window, cx);
        let restore = window.focused(cx);
        resolve_bindings(&mut self.items, restore.as_ref(), window);
        let serial = {
            let menus = cx.global_mut::<Menus>();
            menus.serial += 1;
            menus.serial
        };
        // Focus stays where it is, as with a native menu: the keys come
        // through the interceptor. VoiceOver follows the highlight only
        // inside focus, so for it the menu takes focus until it closes.
        let take_focus = window.is_a11y_active();
        let restore = restore.filter(|_| take_focus);
        let menu = cx.new(|cx| OpenMenu::new(self, position, restore, serial, window, cx));
        if take_focus {
            let focus = menu.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        cx.global_mut::<Menus>().open.insert(id, menu);
        window.refresh();
    }
}

/// `NativeMenu`'s builders, so the call sites still on it move over by
/// swapping the type.
#[allow(dead_code)]
impl ContextMenu {
    pub fn menu(self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.item(Item::new(label, action))
    }

    pub fn menu_with_disabled(
        self,
        label: impl Into<SharedString>,
        disabled: bool,
        action: Box<dyn Action>,
    ) -> Self {
        self.item(Item::new(label, action).disabled(disabled))
    }

    pub fn menu_with_check(
        self,
        label: impl Into<SharedString>,
        checked: bool,
        action: Box<dyn Action>,
    ) -> Self {
        self.item(Item::new(label, action).checked(checked))
    }

    pub fn menu_with_icon(
        self,
        label: impl Into<SharedString>,
        icon: Icon,
        action: Box<dyn Action>,
    ) -> Self {
        self.item(Item::new(label, action).icon(icon))
    }

    pub fn submenu(self, label: impl Into<SharedString>, menu: ContextMenu) -> Self {
        self.item(Item::submenu(label, menu))
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn native(items: Vec<Entry>) -> NativeMenu {
    let mut menu = NativeMenu::new();
    let mut items = items.into_iter().peekable();
    while let Some(entry) = items.next() {
        menu = match entry {
            Entry::Separator => menu.separator(),
            // A dimmed item, set off from the items under it.
            Entry::Header(text) => {
                let menu = menu.menu_with_disabled(text, true, Box::new(gpui::NoAction));
                match items.peek() {
                    Some(Entry::Item(_)) => menu.separator(),
                    _ => menu,
                }
            }
            Entry::Item(item) => match (item.action, item.submenu) {
                (_, Some(sub)) => menu.submenu(item.label, native(sub)),
                (Some(action), None) if item.disabled => {
                    menu.menu_with_disabled(item.label, true, action)
                }
                (Some(action), None) => {
                    menu.menu_with_check(item.label, item.checked == Some(true), action)
                }
                (None, None) => menu,
            },
        };
    }
    menu
}

/// Each item's shortcut where the menu's action will land.
fn resolve_bindings(items: &mut [Entry], focus: Option<&FocusHandle>, window: &Window) {
    for entry in items {
        let Entry::Item(item) = entry else { continue };
        if let Some(sub) = &mut item.submenu {
            resolve_bindings(sub, focus, window);
        }
        if let Some(action) = &item.action {
            item.binding = match focus {
                Some(focus) => window.highest_precedence_binding_for_action_in(&**action, focus),
                None => window.highest_precedence_binding_for_action(&**action),
            };
        }
    }
}

/// The open menu of each window, and which windows draw one.
#[derive(Default)]
struct Menus {
    hosts: HashSet<WindowId>,
    open: HashMap<WindowId, Entity<OpenMenu>>,
    serial: usize,
    /// Forgets windows as they close.
    _closed: Option<Subscription>,
}

impl Global for Menus {}

/// The open menu, if any, over everything in this window. A window's root
/// draws it last; drawing it is also what lets `show` use it here.
pub fn layer(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let id = window.window_handle().window_id();
    if cx.default_global::<Menus>()._closed.is_none() {
        let closed = cx.on_window_closed(|cx, id| {
            if cx.has_global::<Menus>() {
                let menus = cx.global_mut::<Menus>();
                menus.hosts.remove(&id);
                menus.open.remove(&id);
            }
        });
        cx.global_mut::<Menus>()._closed = Some(closed);
    }
    let menus = cx.global_mut::<Menus>();
    menus.hosts.insert(id);
    let menu = menus.open.get(&id)?.clone();
    Some(
        deferred(div().absolute().inset_0().child(menu))
            .with_priority(PRIORITY)
            .into_any_element(),
    )
}

/// Whether a menu is open in this window, so other keystroke interceptors
/// can leave its keys alone.
pub fn is_open(window: &Window, cx: &App) -> bool {
    let id = window.window_handle().window_id();
    cx.try_global::<Menus>()
        .is_some_and(|menus| menus.open.contains_key(&id))
}

/// Closes this window's open menu without choosing anything.
pub fn dismiss(window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    let open = cx
        .try_global::<Menus>()
        .and_then(|menus| menus.open.get(&id).cloned());
    if let Some(menu) = open {
        menu.update(cx, |menu, cx| menu.close(None, window, cx));
    }
}

/// One shown panel: the root menu or an open submenu.
struct Level {
    /// Keys its entrance, fresh each time it opens.
    serial: usize,
    /// The row of the level above that opened it.
    parent_row: usize,
    highlighted: Option<usize>,
    scroll: ScrollHandle,
    /// Measured as drawn, for placing it from the next frame on.
    size: Rc<Cell<Option<gpui::Size<Pixels>>>>,
    rows: Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>,
    /// Where it was drawn last, in window coordinates.
    placed: Option<Bounds<Pixels>>,
    /// Which way it opened from its anchor: (left, up).
    flipped: (bool, bool),
}

impl Level {
    fn new(serial: usize, parent_row: usize, highlighted: Option<usize>, len: usize) -> Self {
        Self {
            serial,
            parent_row,
            highlighted,
            scroll: ScrollHandle::new(),
            size: Rc::default(),
            rows: Rc::new(RefCell::new(vec![None; len])),
            placed: None,
            flipped: (false, false),
        }
    }
}

struct OpenMenu {
    items: Vec<Entry>,
    label: SharedString,
    origin: Point<Pixels>,
    levels: Vec<Level>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    on_close: Option<OnClose>,
    window: WindowId,
    serial: usize,
    /// Submenus opened so far.
    opened: usize,
    pointer: Option<Point<Pixels>>,
    /// A press began on the menu, so its release can choose.
    pressed: bool,
    /// The button that opened it may still be down: where the pointer and
    /// the time were then.
    held: Option<(Point<Pixels>, Instant)>,
    timer: Option<Task<()>>,
    closed: bool,
    _keys: Subscription,
    _activation: Subscription,
}

impl OpenMenu {
    fn new(
        menu: ContextMenu,
        origin: Point<Pixels>,
        restore: Option<FocusHandle>,
        serial: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = window.window_handle().window_id();
        let this = cx.weak_entity();
        // Every key goes to the menu while it's open, as with a native one.
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle().window_id() != id {
                return;
            }
            let Some(menu) = this.upgrade() else { return };
            if menu.update(cx, |menu, cx| menu.key(&event.keystroke, window, cx)) {
                cx.stop_propagation();
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.close(None, window, cx);
            }
        });
        let len = menu.items.len();
        Self {
            items: menu.items,
            label: menu.label.unwrap_or_else(|| "Context menu".into()),
            origin,
            levels: vec![Level::new(0, 0, None, len)],
            focus: cx.focus_handle(),
            restore,
            on_close: menu.on_close,
            window: id,
            serial,
            opened: 0,
            pointer: None,
            pressed: false,
            held: Some((window.mouse_position(), Instant::now())),
            timer: None,
            closed: false,
            _keys: keys,
            _activation: activation,
        }
    }

    fn items(&self, level: usize) -> &[Entry] {
        let mut items = &self.items[..];
        for lv in &self.levels[1..=level] {
            match items.get(lv.parent_row).and_then(Entry::item) {
                Some(Item {
                    submenu: Some(sub), ..
                }) => items = sub,
                _ => return &[],
            }
        }
        items
    }

    fn selectable(&self, level: usize) -> Vec<bool> {
        self.items(level)
            .iter()
            .map(|e| e.item().is_some_and(Item::selectable))
            .collect()
    }

    fn submenu_len(&self, level: usize, row: usize) -> Option<usize> {
        let item = self.items(level).get(row)?.item()?;
        if item.disabled {
            return None;
        }
        item.submenu.as_ref().map(Vec::len)
    }

    /// The deepest level with a highlighted row: where keys act.
    fn key_level(&self) -> usize {
        self.levels
            .iter()
            .rposition(|l| l.highlighted.is_some())
            .unwrap_or(0)
    }

    fn highlight(&mut self, level: usize, row: Option<usize>) {
        self.levels.truncate(level + 1);
        self.levels[level].highlighted = row;
        if let Some(row) = row {
            self.levels[level].scroll.scroll_to_item(row);
        }
    }

    fn open_submenu(&mut self, level: usize, row: usize, select_first: bool) {
        let Some(len) = self.submenu_len(level, row) else {
            return;
        };
        self.timer = None;
        self.levels.truncate(level + 1);
        self.levels[level].highlighted = Some(row);
        self.opened += 1;
        self.levels.push(Level::new(self.opened, row, None, len));
        if select_first {
            let first = step(&self.selectable(level + 1), None, true);
            self.levels[level + 1].highlighted = first;
        }
    }

    fn activate(&mut self, level: usize, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items(level).get(row).and_then(Entry::item) else {
            return;
        };
        if !item.selectable() {
            return;
        }
        if item.submenu.is_some() {
            self.open_submenu(level, row, true);
            cx.notify();
            return;
        }
        let action = item.action.as_ref().map(|a| a.boxed_clone());
        self.close(action, window, cx);
    }

    fn close(
        &mut self,
        action: Option<Box<dyn Action>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.timer = None;
        let me = cx.entity_id();
        let menus = cx.global_mut::<Menus>();
        if menus
            .open
            .get(&self.window)
            .is_some_and(|m| m.entity_id() == me)
        {
            menus.open.remove(&self.window);
        }
        // Only set when the menu took focus.
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        if let Some(action) = action {
            window.dispatch_action(action, cx);
        }
        // After the action, which is deferred too.
        if let Some(on_close) = self.on_close.take() {
            window.defer(cx, on_close);
        }
        window.refresh();
    }

    /// A key while open; false to let it through. Plain navigation keys move
    /// the highlight; an item's shortcut chooses it; any other modified key
    /// closes the menu and goes on to the app, as with a native menu.
    fn key(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.closed {
            return false;
        }
        let kind = classify_key(&keystroke.key, keystroke.modifiers);
        if kind != KeyKind::Navigate {
            if let Some(action) = shortcut_action(&self.items, keystroke) {
                self.close(Some(action), window, cx);
                return true;
            }
            if kind == KeyKind::Modified {
                self.close(None, window, cx);
                return false;
            }
        }
        let level = self.key_level();
        let current = self.levels[level].highlighted;
        match keystroke.key.as_str() {
            "escape" => {
                if self.levels.len() > 1 {
                    self.levels.pop();
                } else {
                    self.close(None, window, cx);
                    return true;
                }
            }
            "up" | "down" => {
                let next = step(&self.selectable(level), current, keystroke.key == "down");
                self.highlight(level, next.or(current));
            }
            "home" | "end" | "pageup" | "pagedown" => {
                let forward = matches!(keystroke.key.as_str(), "home" | "pageup");
                let next = step(&self.selectable(level), None, forward);
                self.highlight(level, next);
            }
            "right" => {
                if let Some(row) = current {
                    self.open_submenu(level, row, true);
                }
            }
            "left" => {
                if self.levels.len() > 1 {
                    self.levels.pop();
                }
            }
            "enter" | "space" => match current {
                Some(row) => self.activate(level, row, window, cx),
                None => {
                    let first = step(&self.selectable(level), None, true);
                    self.highlight(level, first);
                }
            },
            _ => {
                let typed = keystroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| keystroke.key.clone());
                let labels: Vec<Option<&str>> = self
                    .items(level)
                    .iter()
                    .map(|e| e.item().filter(|i| i.selectable()).map(|i| &*i.label))
                    .collect();
                if let Some(row) = type_select(&labels, current, &typed) {
                    self.highlight(level, Some(row));
                }
            }
        }
        self.timer = None;
        cx.notify();
        true
    }

    /// The level and row under `at`, deepest first: a row of no use
    /// (separator, header, disabled) is `None` within its level.
    fn hit(&self, at: Point<Pixels>) -> Option<(usize, Option<usize>)> {
        (0..self.levels.len()).rev().find_map(|level| {
            let panel = self.levels[level].placed?;
            if !panel.contains(&at) {
                return None;
            }
            let selectable = self.selectable(level);
            let rows = self.levels[level].rows.borrow();
            let row = rows
                .iter()
                .position(|b| b.is_some_and(|b| b.contains(&at)))
                .filter(|&row| selectable.get(row) == Some(&true));
            Some((level, row))
        })
    }

    fn pointer_moved(&mut self, at: Point<Pixels>, aim: bool, cx: &mut Context<Self>) {
        let before = self.pointer.replace(at);
        let Some((level, row)) = self.hit(at) else {
            // Off the menus: the deepest loses its highlight, unless it opened
            // a submenu.
            let deepest = self.levels.len() - 1;
            if self.levels[deepest].highlighted.is_some() {
                self.levels[deepest].highlighted = None;
                self.timer = None;
                cx.notify();
            }
            return;
        };
        if let Some(sub) = self.levels.get(level + 1) {
            if row == Some(sub.parent_row) {
                // Back on the row that opened it.
                self.timer = None;
                if self.levels[level + 1].highlighted.take().is_some()
                    || self.levels.len() > level + 2
                {
                    self.levels.truncate(level + 2);
                    cx.notify();
                }
                return;
            }
            // Heading across other rows for the open submenu: they wait.
            if aim
                && let (Some(from), Some(panel)) = (before, sub.placed)
                && heading_for(from, at, panel)
            {
                self.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(MENU_AIM_GRACE).await;
                    this.update(cx, |this, cx| {
                        if let Some(at) = this.pointer {
                            this.pointer_moved(at, false, cx);
                        }
                    })
                    .ok();
                }));
                return;
            }
        }
        if self.levels.len() == level + 1 && self.levels[level].highlighted == row {
            return;
        }
        self.levels.truncate(level + 1);
        self.levels[level].highlighted = row;
        self.timer = None;
        if let Some(row) = row
            && self.submenu_len(level, row).is_some()
        {
            self.timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(MENU_SUBMENU_DELAY).await;
                this.update(cx, |this, cx| {
                    if this.levels.len() == level + 1 && this.levels[level].highlighted == Some(row)
                    {
                        this.open_submenu(level, row, false);
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.held = None;
        if self.hit(event.position).is_some() {
            self.pressed = true;
        } else {
            // Like macOS: the click closes the menu and nothing else.
            self.close(None, window, cx);
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        // The opening button let go over a row after a drag or a hold
        // chooses it, as on macOS; a plain click leaves the menu open.
        let dragged = self.held.take().is_some_and(|(at, since)| {
            release_chooses((event.position - at).magnitude() as f32, since.elapsed())
        });
        if !std::mem::take(&mut self.pressed) && !dragged {
            return;
        }
        if let Some((level, Some(row))) = self.hit(event.position) {
            if self.submenu_len(level, row).is_some() {
                self.open_submenu(level, row, false);
                cx.notify();
            } else {
                self.activate(level, row, window, cx);
            }
        }
    }

    /// Where level `level` goes, or `None` until it (and its anchor) has
    /// been measured.
    fn place(
        &self,
        level: usize,
        viewport: gpui::Size<Pixels>,
    ) -> Option<(Bounds<Pixels>, (bool, bool))> {
        let size = self.levels[level].size.get()?;
        if level == 0 {
            let (origin, flipped) = place_at(self.origin, size, viewport, EDGE);
            return Some((Bounds { origin, size }, flipped));
        }
        let parent = &self.levels[level - 1];
        let row = parent
            .rows
            .borrow()
            .get(self.levels[level].parent_row)
            .copied()
            .flatten()?;
        let (origin, left) = place_beside(row, parent.placed?, size, viewport, EDGE);
        Some((Bounds { origin, size }, (left, false)))
    }

    fn render_level(
        &mut self,
        level: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewport = window.viewport_size();
        let placed = self.place(level, viewport);
        if let Some((bounds, flipped)) = placed {
            self.levels[level].placed = Some(bounds);
            self.levels[level].flipped = flipped;
        } else {
            self.levels[level].placed = None;
            window.request_animation_frame();
        }
        let id = format!("context-menu-{}-{}", self.serial, self.levels[level].serial);
        let p = presence(SharedString::from(id), placed.is_some(), window, cx).clamp(0., 1.);
        // Unplaced, it's only being measured.
        let p = if placed.is_some() { p } else { 0. };
        let theme = Theme::of(cx);
        let lv = &self.levels[level];
        let items = self.items(level);
        let (origin, (left, up)) = match placed {
            Some((bounds, flipped)) => (bounds.origin, flipped),
            None => (self.origin, (false, false)),
        };
        // From the cursor's side: a submenu from its parent, the root from
        // the corner at the cursor.
        let travel = MENU_TRAVEL * (1. - p);
        let dx = if left { travel } else { -travel };
        let dy = match (level, up) {
            (0, true) => travel,
            (0, false) => -travel,
            _ => px(0.),
        };
        let checks = items
            .iter()
            .any(|e| e.item().is_some_and(|i| i.checked.is_some()));
        let leading = items
            .iter()
            .any(|e| e.item().is_some_and(|i| i.leading.is_some()));
        let open_row = self.levels.get(level + 1).map(|l| l.parent_row);
        let key_level = level == self.key_level();
        let positions =
            set_positions(&items.iter().map(|e| e.item().is_some()).collect::<Vec<_>>());
        let set_size = positions.last().copied().unwrap_or(0);
        let this = cx.weak_entity();
        let label = if level == 0 {
            self.label.clone()
        } else {
            self.items(level - 1)
                .get(lv.parent_row)
                .and_then(Entry::item)
                .map(|i| i.label.clone())
                .unwrap_or_default()
        };
        let rows = items.iter().enumerate().map(|(ix, entry)| {
            let bounds = lv.rows.clone();
            let measure = canvas(
                move |b, _, _| {
                    if let Some(slot) = bounds.borrow_mut().get_mut(ix) {
                        *slot = Some(b);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full();
            match entry {
                Entry::Separator => div()
                    .id(("separator", ix))
                    .relative()
                    .py(SPACE_XS)
                    .px(SPACE_SM)
                    .child(div().h(px(1.)).bg(theme.border))
                    .child(measure)
                    .into_any_element(),
                Entry::Header(text) => div()
                    .id(("header", ix))
                    .role(gpui::Role::Heading)
                    .aria_label(text.clone())
                    .relative()
                    .h(CONTROL_SM)
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .text_size(TEXT_XS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_muted)
                    .child(div().truncate().child(text.clone()))
                    .child(measure)
                    .into_any_element(),
                Entry::Item(item) => {
                    let on = lv.highlighted == Some(ix);
                    let this = this.clone();
                    let color = if item.disabled {
                        theme.text_disabled
                    } else if item.destructive {
                        theme.text_danger
                    } else {
                        theme.text
                    };
                    let glyph_color = if item.disabled || item.destructive {
                        color
                    } else {
                        theme.icon
                    };
                    div()
                        .id(("item", ix))
                        .role(if item.checked.is_some() {
                            gpui::Role::MenuItemCheckBox
                        } else {
                            gpui::Role::MenuItem
                        })
                        .aria_label(item.label.clone())
                        .aria_selected(on)
                        // One claim a frame: the row where keys act.
                        .when(on && key_level, |el| el.aria_active_descendant())
                        .aria_position_in_set(positions[ix])
                        .aria_size_of_set(set_size)
                        // VoiceOver's press; the mouse goes through the
                        // backdrop, so this never fires with it.
                        .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                            this.update(cx, |menu, cx| menu.activate(level, ix, window, cx))
                                .ok();
                        })
                        .when_some(item.checked, |el, checked| {
                            el.aria_toggled(if checked {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                        })
                        .when(item.submenu.is_some(), |el| {
                            el.aria_expanded(open_row == Some(ix))
                        })
                        .when(item.disabled, |el| el.aria_description("Dimmed"))
                        .when_some(item.binding.as_ref(), |el, b| {
                            el.aria_keyshortcuts(binding_text(b))
                        })
                        .relative()
                        .h(MENU_ROW)
                        .px(SPACE_SM)
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .rounded(RADIUS_SM)
                        .text_size(TEXT_BASE)
                        .text_color(color)
                        .when(on, |el| el.bg(theme.selected_bg))
                        .when(checks, |el| {
                            el.ml(-SPACE_XS)
                                .child(menu_check(item.checked == Some(true), theme))
                        })
                        .when(leading, |el| {
                            el.child(
                                div()
                                    .flex_none()
                                    .size(ICON_LG)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .map(|d| match item.leading {
                                        Some(Leading::Icon(glyph)) => {
                                            d.child(icon(glyph, ICON_MD, glyph_color))
                                        }
                                        Some(Leading::Dot(c)) => d.child(dot_marker(c)),
                                        None => d,
                                    }),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(item.label.clone()),
                        )
                        .map(|el| {
                            if item.submenu.is_some() {
                                el.child(icon(Icon::ChevronRight, ICON_SM, theme.text_faint))
                            } else if let Some(binding) = &item.binding {
                                el.child(
                                    div()
                                        .flex_none()
                                        .pl(SPACE_MD)
                                        .when(item.disabled, |d| d.opacity(0.5))
                                        .child(binding_caps(binding, theme)),
                                )
                            } else {
                                el
                            }
                        })
                        .child(measure)
                        .into_any_element()
                }
            }
        });
        let size = lv.size.clone();
        div()
            .id(("context-menu", level))
            .absolute()
            .left(origin.x + dx)
            .top(origin.y + dy)
            .opacity(p)
            .child(
                popover_surface(theme)
                    .id("surface")
                    .role(gpui::Role::Menu)
                    .aria_label(label)
                    .min_w(MENU_MIN_W)
                    .max_w(MENU_MAX_W)
                    .max_h(viewport.height - EDGE * 2.)
                    .overflow_y_scroll()
                    .track_scroll(&lv.scroll)
                    .p(SPACE_XS)
                    .flex()
                    .flex_col()
                    .line_height(LINE_UI)
                    .cursor_default()
                    .children(rows),
            )
            .child(
                canvas(move |b, _, _| size.set(Some(b.size)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .into_any_element()
    }
}

impl Render for OpenMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels: Vec<AnyElement> = (0..self.levels.len())
            .map(|level| self.render_level(level, window, cx))
            .collect();
        div()
            .id("context-menu-backdrop")
            // Focus sits around every panel, so VoiceOver follows the
            // highlight into submenus too.
            .role(gpui::Role::GenericContainer)
            .track_focus(&self.focus)
            .absolute()
            .inset_0()
            .occlude()
            .on_any_mouse_down(cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.pointer_moved(event.position, true, cx)
            }))
            .children(panels)
    }
}

/// Each row's place among the items (`is_item`), from 1; a separator or
/// header has the place of the item before it. The last is the count.
fn set_positions(is_item: &[bool]) -> Vec<usize> {
    is_item
        .iter()
        .scan(0, |n, &item| {
            *n += usize::from(item);
            Some(*n)
        })
        .collect()
}

/// Whether releasing the button that opened the menu, `moved` px from where
/// it was pressed `held` ago, chooses the row under it.
fn release_chooses(moved: f32, held: Duration) -> bool {
    moved > MENU_DRAG_CHOOSE || held > MENU_HOLD
}

/// What a key does while a menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyKind {
    /// Moves the highlight, opens, chooses or closes: plain arrows, ↩,
    /// Space, Home, End, Page Up/Down, and ⎋ with anything held.
    Navigate,
    /// A character, maybe shifted: an item's shortcut, or type-to-select.
    Typed,
    /// Held ⌘, ⌃, ⌥ or fn, or ⇧ on a navigation key: an item's shortcut,
    /// or else for the app.
    Modified,
}

fn classify_key(key: &str, m: gpui::Modifiers) -> KeyKind {
    let navigation = matches!(
        key,
        "up" | "down"
            | "left"
            | "right"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "enter"
            | "space"
    );
    if key == "escape" {
        KeyKind::Navigate
    } else if m.platform || m.control || m.alt || m.function || (navigation && m.shift) {
        KeyKind::Modified
    } else if navigation {
        KeyKind::Navigate
    } else {
        KeyKind::Typed
    }
}

/// The action of the item, at any depth, whose shortcut is `keystroke`.
fn shortcut_action(items: &[Entry], keystroke: &Keystroke) -> Option<Box<dyn Action>> {
    items.iter().filter_map(Entry::item).find_map(|item| {
        if !item.selectable() {
            return None;
        }
        if let Some(sub) = &item.submenu {
            return shortcut_action(sub, keystroke);
        }
        let binding = item.binding.as_ref()?;
        (binding.match_keystrokes(std::slice::from_ref(keystroke)) == Some(false))
            .then(|| item.action.as_ref().map(|a| a.boxed_clone()))
            .flatten()
    })
}

/// The next row `selectable` from `from` (or from the end, with `None`)
/// going down when `forward`, wrapping around.
fn step(selectable: &[bool], from: Option<usize>, forward: bool) -> Option<usize> {
    let len = selectable.len();
    if len == 0 {
        return None;
    }
    (1..=len)
        .map(|offset| match (from, forward) {
            (Some(from), true) => (from + offset) % len,
            (Some(from), false) => (from + len - offset % len) % len,
            (None, true) => offset - 1,
            (None, false) => len - offset,
        })
        .find(|&ix| selectable[ix])
}

/// The next row after `from` whose label starts with `typed`, any case,
/// wrapping around; `None` labels can't be chosen.
fn type_select(labels: &[Option<&str>], from: Option<usize>, typed: &str) -> Option<usize> {
    let typed = typed.to_lowercase();
    if typed.trim().is_empty() || typed.chars().count() != 1 {
        return None;
    }
    let len = labels.len();
    let start = from.map_or(0, |f| f + 1);
    (0..len)
        .map(|o| (start + o) % len)
        .find(|&ix| labels[ix].is_some_and(|label| label.to_lowercase().starts_with(&typed)))
}

/// A menu of `size` opening at `cursor`: below and to the right, flipped to
/// the other side on an axis without room, then kept `edge` inside the
/// window. Also which way it flipped: (left, up).
fn place_at(
    cursor: Point<Pixels>,
    size: gpui::Size<Pixels>,
    viewport: gpui::Size<Pixels>,
    edge: Pixels,
) -> (Point<Pixels>, (bool, bool)) {
    let axis = |at: Pixels, len: Pixels, room: Pixels| {
        let flip = at + len > room - edge && at - len >= edge;
        let start = if flip { at - len } else { at };
        (clamp(start, len, room, edge), flip)
    };
    let (x, left) = axis(cursor.x, size.width, viewport.width);
    let (y, up) = axis(cursor.y, size.height, viewport.height);
    (point(x, y), (left, up))
}

/// A submenu of `size` beside `row` of the `parent` panel: to its right,
/// or its left without room, its first row level with `row`. Also whether
/// it went left.
fn place_beside(
    row: Bounds<Pixels>,
    parent: Bounds<Pixels>,
    size: gpui::Size<Pixels>,
    viewport: gpui::Size<Pixels>,
    edge: Pixels,
) -> (Point<Pixels>, bool) {
    // Overlapping the parent's padding, as macOS does.
    let right = parent.right() - SPACE_XS;
    let left = parent.left() + SPACE_XS - size.width;
    let fits_right = right + size.width <= viewport.width - edge;
    let go_left = !fits_right && (left >= edge || parent.left() > viewport.width - parent.right());
    let x = if go_left { left } else { right };
    let y = row.top() - SPACE_XS - px(1.);
    (
        point(
            clamp(x, size.width, viewport.width, edge),
            clamp(y, size.height, viewport.height, edge),
        ),
        go_left,
    )
}

/// `start` moved so `len` from it stays `edge` inside `0..room`; the
/// start edge wins when it can't fit.
fn clamp(start: Pixels, len: Pixels, room: Pixels, edge: Pixels) -> Pixels {
    start.min(room - edge - len).max(edge)
}

/// Whether the pointer going `from` → `to` is heading for `panel`: `to` is
/// inside the triangle from `from` to the panel's near edge.
fn heading_for(from: Point<Pixels>, to: Point<Pixels>, panel: Bounds<Pixels>) -> bool {
    if from == to {
        return false;
    }
    let x = if panel.left() >= from.x {
        panel.left()
    } else {
        panel.right()
    };
    in_triangle(
        to,
        from,
        point(x, panel.top() - SPACE_SM),
        point(x, panel.bottom() + SPACE_SM),
    )
}

fn in_triangle(p: Point<Pixels>, a: Point<Pixels>, b: Point<Pixels>, c: Point<Pixels>) -> bool {
    let cross = |o: Point<Pixels>, u: Point<Pixels>, v: Point<Pixels>| {
        (u.x - o.x).as_f32() * (v.y - o.y).as_f32() - (u.y - o.y).as_f32() * (v.x - o.x).as_f32()
    };
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let negative = d1 < 0. || d2 < 0. || d3 < 0.;
    let positive = d1 > 0. || d2 > 0. || d3 > 0.;
    !(negative && positive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    #[test]
    fn arrows_skip_what_cant_be_chosen_and_wrap() {
        // Header, item, separator, disabled, item.
        let rows = [false, true, false, false, true];
        assert_eq!(step(&rows, None, true), Some(1));
        assert_eq!(step(&rows, None, false), Some(4));
        assert_eq!(step(&rows, Some(1), true), Some(4));
        assert_eq!(step(&rows, Some(4), true), Some(1));
        assert_eq!(step(&rows, Some(1), false), Some(4));
        assert_eq!(step(&rows, Some(4), false), Some(1));
        assert_eq!(step(&[false, false], None, true), None);
        assert_eq!(step(&[], Some(0), true), None);
        // Alone, it stays.
        assert_eq!(step(&[false, true], Some(1), true), Some(1));
    }

    #[test]
    fn only_plain_navigation_keys_stay_in_the_menu() {
        use gpui::Modifiers;
        let none = Modifiers::default();
        let shift = Modifiers::shift();
        let cmd = Modifiers::command();
        assert_eq!(classify_key("down", none), KeyKind::Navigate);
        assert_eq!(classify_key("enter", none), KeyKind::Navigate);
        assert_eq!(classify_key("escape", cmd), KeyKind::Navigate);
        assert_eq!(classify_key("down", shift), KeyKind::Modified);
        assert_eq!(classify_key("enter", cmd), KeyKind::Modified);
        assert_eq!(classify_key("k", cmd), KeyKind::Modified);
        assert_eq!(
            classify_key("u", Modifiers::command_shift()),
            KeyKind::Modified
        );
        assert_eq!(classify_key("r", Modifiers::alt()), KeyKind::Modified);
        assert_eq!(classify_key("r", none), KeyKind::Typed);
        assert_eq!(classify_key("r", shift), KeyKind::Typed);
    }

    #[test]
    fn an_items_shortcut_finds_its_action_at_any_depth() {
        let bound = |label: &str, keys: &str| {
            let mut item = Item::new(label.to_string(), Box::new(gpui::NoAction));
            item.binding = Some(KeyBinding::new(keys, gpui::NoAction, None));
            item
        };
        let menu = ContextMenu::new()
            .item(bound("Reply", "r"))
            .item(bound("Dimmed", "cmd-d").disabled(true))
            .item(Item::submenu(
                "Mark",
                ContextMenu::new().item(bound("Unread", "shift-cmd-u")),
            ));
        let press = |s: &str| shortcut_action(&menu.items, &Keystroke::parse(s).unwrap());
        assert!(press("r").is_some());
        assert!(press("shift-cmd-u").is_some());
        assert!(press("cmd-d").is_none());
        assert!(press("cmd-k").is_none());
    }

    #[test]
    fn set_positions_count_only_items() {
        // Header, item, separator, item, item.
        let rows = set_positions(&[false, true, false, true, true]);
        assert_eq!(rows, vec![0, 1, 1, 2, 3]);
        assert_eq!(rows.last(), Some(&3));
        assert!(set_positions(&[]).is_empty());
    }

    #[test]
    fn the_opening_release_chooses_after_a_drag_or_a_hold() {
        // A click in place, quick: the menu stays.
        assert!(!release_chooses(0., Duration::from_millis(80)));
        assert!(!release_chooses(3., Duration::from_millis(200)));
        // Dragged onto a row.
        assert!(release_chooses(12., Duration::from_millis(80)));
        // Held, then let go.
        assert!(release_chooses(0., Duration::from_millis(400)));
    }

    #[test]
    fn typing_jumps_to_the_next_match() {
        let labels = [
            Some("Reply"),
            Some("Reply All"),
            None,
            Some("Archive"),
            Some("Report"),
        ];
        assert_eq!(type_select(&labels, None, "r"), Some(0));
        assert_eq!(type_select(&labels, Some(0), "R"), Some(1));
        assert_eq!(type_select(&labels, Some(1), "r"), Some(4));
        assert_eq!(type_select(&labels, Some(4), "r"), Some(0));
        assert_eq!(type_select(&labels, Some(0), "a"), Some(3));
        assert_eq!(type_select(&labels, None, "z"), None);
        // Not chosen: a disabled "Delete".
        assert_eq!(type_select(&[None, Some("Star")], None, "d"), None);
        assert_eq!(type_select(&labels, None, " "), None);
        assert_eq!(type_select(&labels, None, "up"), None);
    }

    #[test]
    fn opens_below_right_flips_and_stays_inside() {
        let window = size(px(800.), px(600.));
        let menu = size(px(200.), px(300.));
        let at = |x, y| place_at(point(px(x), px(y)), menu, window, px(8.));
        assert_eq!(at(100., 100.), (point(px(100.), px(100.)), (false, false)));
        // No room right or below: it opens up and to the left.
        assert_eq!(at(700., 500.), (point(px(500.), px(200.)), (true, true)));
        // No room either way: kept inside, without flipping.
        let tall = size(px(200.), px(560.));
        let (origin, flipped) = place_at(point(px(100.), px(300.)), tall, window, px(8.));
        assert_eq!(origin, point(px(100.), px(32.)));
        assert_eq!(flipped, (false, false));
        // Taller than the window: from the top edge.
        let huge = size(px(200.), px(900.));
        assert_eq!(
            place_at(point(px(0.), px(300.)), huge, window, px(8.)).0.y,
            px(8.)
        );
    }

    #[test]
    fn submenus_open_beside_their_row() {
        let window = size(px(800.), px(600.));
        let sub = size(px(200.), px(100.));
        let parent = Bounds::new(point(px(100.), px(100.)), size(px(200.), px(300.)));
        let row = Bounds::new(point(px(104.), px(150.)), size(px(192.), px(28.)));
        let (origin, left) = place_beside(row, parent, sub, window, px(8.));
        assert!(!left);
        assert_eq!(origin, point(px(296.), px(145.)));
        // At the right edge: to the left of the parent.
        let parent = Bounds::new(point(px(500.), px(100.)), size(px(200.), px(300.)));
        let row = Bounds::new(point(px(504.), px(550.)), size(px(192.), px(28.)));
        let (origin, left) = place_beside(row, parent, sub, window, px(8.));
        assert!(left);
        assert_eq!(origin, point(px(304.), px(492.)));
    }

    #[test]
    fn aiming_for_a_submenu() {
        let sub = Bounds::new(point(px(300.), px(100.)), size(px(200.), px(200.)));
        let from = point(px(200.), px(120.));
        assert!(heading_for(from, point(px(220.), px(140.)), sub));
        assert!(!heading_for(from, point(px(190.), px(140.)), sub));
        assert!(!heading_for(from, point(px(205.), px(180.)), sub));
        assert!(!heading_for(from, from, sub));
    }
}

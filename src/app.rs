//! The root view. It owns all app state; the panes in `ui/` render from it.

mod attachments;
mod backend_events;
mod compose;
mod contact_card;
mod draft_window;
mod labels;
mod mail;
pub mod menu_bar;
mod panes;
mod profile;
pub mod shell;
pub(crate) mod toast;
pub mod today;
mod undo;
pub use labels::LabelCommand;
pub use mail::{Density, SearchSuggestion};
pub use toast::{Toast, ToastAction, ToastKind};
pub use undo::{Retime, UndoAction};

use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, AppContext as _, Bounds, Context, DragMoveEvent, Entity, FocusHandle,
    Focusable as _, InteractiveElement, IntoElement, KeyContext, ListAlignment, ListOffset,
    ListState, MouseButton, ParentElement, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window,
    component::input::{self, InputEvent, InputState},
    div, point,
    prelude::FluentBuilder as _,
    px,
};

use crate::ui::style::*;
use crate::{
    actions::*,
    assets::Icon,
    backend::{Backend, MailOp, OpId, Source, auth::CALENDAR_SCOPE, gmail},
    contacts::{ContactBook, SharedContactBook},
    data::{Folder, MailData, Thread},
    search::{FocusResults, Query},
    settings::{CalendarViewSetting, Saved, SavedNav, Settings},
    theme::Theme,
    ui::{
        self, PaneResize,
        calendar::{CalendarEvent, CalendarView, Retimed, ViewKind},
        command_palette::{CommandPalette, Leading, PaletteCommand, PaletteEvent, PaletteItem},
        composer::Composer,
        settings::SettingsPage,
    },
};

/// Wide enough to hold the traffic lights above its icons.
pub const RAIL_WIDTH: Pixels = px(80.);
const SIDEBAR_WIDTH: (Pixels, Pixels) = (px(180.), px(360.));
const SIDEBAR_DEFAULT: Pixels = px(236.);
/// How many recent commands head the palette (ME-062).
const RECENT_SHOWN: usize = 5;
/// How far past its minimum the sidebar's divider is dragged to hide it.
const SIDEBAR_COLLAPSE: Pixels = px(60.);
const LIST_WIDTH: (Pixels, Pixels) = (px(300.), px(640.));
const LIST_DEFAULT: Pixels = px(400.);
/// How narrow the list gets when the user keeps the sidebar open in a narrow
/// window.
const LIST_SQUEEZED: Pixels = px(220.);
pub const READER_MIN: Pixels = px(360.);
/// ↑ and ↓ in the reader.
const SCROLL_STEP: Pixels = px(64.);
/// Where the palette and the contact card sit below the window's top.
const OVERLAY_TOP: Pixels = px(110.);
/// What a page in the reader leaves of the last one (MB-045).
const PAGE_OVERLAP: Pixels = px(40.);

/// The widths the mail panes get in a window, from the user's widths: the
/// list gives way first (down to its minimum), then the sidebar hides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneLayout {
    /// Zero while the sidebar is hidden.
    pub sidebar: Pixels,
    pub list: Pixels,
}

/// Which app fills the window, beside the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Mail,
    Calendar,
}

/// What the sidebar has selected; the thread list shows its threads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Folder(Folder),
    Starred,
    /// Index into `MailData::labels`.
    Label(usize),
    /// W4b: an Inbox category but Primary, which the Inbox itself shows
    /// (FOL-11).
    Category(crate::backend::categories::Category),
}

/// Whether a view shows the thread. W4b: the Inbox shows Primary, each
/// other category its own (FOL-11).
pub fn nav_holds(nav: Nav, thread: &Thread, labels: &[crate::data::Label]) -> bool {
    use crate::backend::categories::{Category, in_category};
    match nav {
        Nav::Folder(Folder::Sent) => thread.in_sent(),
        Nav::Folder(Folder::Inbox) => {
            thread.folder == Folder::Inbox && in_category(thread, Category::Primary)
        }
        Nav::Folder(folder) => thread.folder == folder,
        Nav::Starred => thread.starred && !thread.is_hidden(),
        Nav::Label(ix) => !thread.is_hidden() && thread.labels.contains(&labels[ix].id),
        Nav::Category(category) => thread.folder == Folder::Inbox && in_category(thread, category),
    }
}

/// One row of the thread list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// Index into `MailData::threads`.
    Thread(usize),
    /// The collapsible "Archived" header under the Inbox.
    ArchivedHeader { count: usize },
    /// W9: "Load older" at the end of a view, or more of Gmail's results
    /// at the end of a search (LST-7, SRC-7).
    LoadOlder,
    /// A conversation that just left the view, collapsing (MD-017): its
    /// key in `PigeonApp::leaving_rows`. Never selected, picked or counted.
    Leaving(u64),
}

/// A conversation's row collapsing after it left the view (MD-017).
pub(crate) struct LeavingRow {
    /// Its key in `Row::Leaving`.
    pub key: u64,
    pub id: String,
    /// Its height as last drawn.
    pub height: Pixels,
    /// How far it has gone, 0 to 1.
    pub glide: ui::motion::Glide,
}

/// Why the rows are changing, for how the change moves (MD-017, MD-024).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RowCause {
    /// The user's action: rows that leave collapse, new ones just show.
    #[default]
    User,
    /// A sync or Gmail's search answer: new rows also fade in.
    Sync,
    /// Undo or Redo: rows that come back grow into place.
    Undo,
    /// Rows the user revealed or hid (the Archived section): no motion.
    Quiet,
}

/// At most this many rows animate leaving at once; more just go.
const LEAVING_MAX: usize = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ComposeKind {
    #[default]
    New,
    Reply,
    ReplyAll,
    Forward,
}

pub struct PigeonApp {
    pub mode: Mode,
    /// Where the data comes from and changes go (`backend`).
    pub backend: Entity<Backend>,
    /// The calendar keeps its own state, so switching to Mail and back
    /// loses nothing on either side.
    pub calendar: Entity<CalendarView>,
    pub data: MailData,
    pub nav: Nav,
    pub rows: Vec<Row>,
    /// Rows on their way out (MD-017).
    pub(crate) leaving_rows: Vec<LeavingRow>,
    next_leaving: u64,
    /// Conversations Undo brought back, growing into place (MD-017).
    pub(crate) growing_rows: HashMap<String, ui::motion::Glide>,
    /// Conversations a sync or Gmail's search added, fading in (MD-024),
    /// and when they came.
    pub(crate) fresh_rows: (HashSet<String>, Option<std::time::Instant>),
    row_cause: RowCause,
    /// Index into `MailData::threads`.
    pub selected: Option<usize>,
    /// Older messages of the selected thread the user expanded. The newest is
    /// always expanded.
    pub expanded_messages: HashSet<usize>,
    /// The message last opened with a click, whose body fades in (MD-020).
    pub(crate) clicked_open: Option<usize>,
    pub archived_expanded: bool,
    /// The user's choice; the sidebar also hides by itself while the window
    /// is too narrow for it (see `pane_layout`).
    pub sidebar_open: bool,
    /// Shown by the user while the window is too narrow for it.
    sidebar_forced: bool,
    pub sidebar_width: Pixels,
    pub list_width: Pixels,
    /// The divider being dragged, for its highlight.
    resizing: Option<PaneResize>,
    /// The panes' widths as drawn this frame: `pane_layout`, but gliding
    /// after a divider's double-click (MD-015).
    pub(crate) shown: PaneLayout,
    /// The divider whose pane glides back to its default width.
    gliding: Option<PaneResize>,
    /// How far the sidebar is shown, 0 to 1, as drawn this frame (MD-012).
    pub(crate) sidebar_progress: f32,
    /// The user showed or hid the sidebar and it's still on its way; a
    /// window resize alone moves it at once (MD-014).
    sidebar_sliding: bool,
    /// Every view's count, worked out once per frame for the rail, the
    /// sidebar and the list (MC-037).
    pub(crate) counts: ui::sidebar::NavCounts,
    /// The panes as views of their own (MC-036).
    panes: panes::Panes,
    /// An overlay on its way out (MD-007).
    leaving_overlay: Option<LeavingOverlay>,
    /// ⌘= / ⌘-: scales all text through the window's rem size.
    pub text_scale: f32,
    pub search_open: bool,
    pub list_state: ListState,
    pub reader_scroll: ScrollHandle,
    /// A keyboard scroll of the reader on its way (MD-018); a wheel or a
    /// new conversation drops it.
    pub(crate) reader_glide: Option<ui::motion::Glide>,
    pub html_cache: crate::html::HtmlCache,
    /// Remote images let load and fetched (W10, ATT-1).
    pub images: ui::html_view::LoadedImages,
    /// The reading pane has keyboard focus (after `o`): arrows scroll it and
    /// escape returns to the list.
    pub reader_focused: bool,
    /// Focus of the panes; see `actions.rs` for the key contexts.
    pub(crate) focus_handle: FocusHandle,
    palette: Option<(Entity<CommandPalette>, Subscription)>,
    pub help_open: bool,
    /// W6: the contact card (CON-5), an overlay like the palette.
    contact_card: Option<(Entity<ui::contact_card::ContactCardView>, Subscription)>,
    pub help_focus: FocusHandle,
    /// The open draft, with its event subscription.
    pub composer: Option<(Entity<Composer>, Subscription)>,
    /// The reply card that left, by the conversation it answered, drawn
    /// while it fades out (MD-019); and how far the card is shown.
    reply_ghost: Option<(Entity<Composer>, String)>,
    pub(crate) reply_card: f32,
    /// Drafts in windows of their own (MB-047).
    draft_windows: Vec<draft_window::DraftWindow>,
    /// Composers made in a draft window, which Undo makes again here.
    windowed: HashSet<gpui::EntityId>,
    contact_book: SharedContactBook,
    pub search_input: Entity<InputState>,
    pub search_query: Query,
    /// The message `n` and `p` move between, in the selected thread.
    pub focused_message: Option<usize>,
    pub toast: Option<Toast>,
    /// The last toast, fading out (MD-008).
    toast_ghost: Option<toast::ToastGhost>,
    /// Conversations starred just now, by id, and when: their stars pop
    /// (MD-027).
    just_starred: (Vec<String>, Option<std::time::Instant>),
    /// The session's undo and redo (MA-071).
    pub undo_stack: undo::UndoStack,
    /// Sends waiting to go, for "Message sent" once they have (W6).
    sends: Vec<OpId>,
    /// The outbox's queued sends, by local message id, for the list's
    /// clock (MA-069). Read from the store only when the outbox changes.
    pub(crate) queued_sends: HashSet<String>,
    /// Google's sign-in had expired at the last status (MB-054).
    auth_expired: bool,
    // Mail lists and search; see `app/mail.rs`.
    /// Threads picked for a bulk action, by id. While any are, the reader
    /// shows the bulk actions instead of a conversation.
    pub checked: HashSet<String>,
    /// Where a shift-extended selection started, by thread id.
    select_anchor: Option<String>,
    /// The thread (by id) a row's context menu is open on, when it isn't one
    /// of the picked threads: the menu's actions apply to it.
    pub context_target: Option<String>,
    pub density: Density,
    /// Search all mail rather than the current view (the scope chip).
    pub search_all: bool,
    pub recent_searches: Vec<String>,
    /// The highlighted search suggestion, once moved to with the arrows.
    pub search_highlight: Option<usize>,
    /// Palette commands by last use, most recent first, by action name;
    /// saved (ME-062).
    recent_commands: Vec<String>,
    /// The message whose whole text ⌘A selected, until a click clears it.
    pub select_all_text: Option<usize>,
    pub help_filter: Entity<InputState>,
    /// A reveal waiting for the list's next layout.
    reveal_pending: bool,
    /// The preferences with no other home; the settings page shows these
    /// and the state above.
    pub settings: Settings,
    pub settings_page: SettingsPage,
    /// The settings as last handed to the backend to save.
    pub(crate) saved: Saved,
    /// Gmail's unread counts (`ui::sidebar`).
    pub server_counts: ui::sidebar::ServerCounts,
    /// W9: Gmail's half of the search, and whether views page (LST-7).
    pub server_search: crate::search::ServerSearch,
    /// W2 (RDR-8): the source shown in place of the conversation.
    pub original: Option<ui::reader::Original>,
    /// W4b: folded labels, hidden ones shown, a name being typed.
    pub labels_ui: labels::LabelsUi,
    /// Threads held without their bodies, by id (MA-072): read from the
    /// store when one is opened.
    heads: HashSet<String>,
    /// Each mail view's selection and scroll, as last left (MB-071).
    nav_memory: Vec<(Nav, NavMemory)>,
    /// The window's frame, saved with the settings (MA-034).
    window_frame: Option<crate::settings::WindowFrame>,
    /// The window's title as last set (MB-011).
    window_title: SharedString,
    /// The ⌘K and ? tip was used or dismissed (MA-094).
    seen_tip: bool,
    /// Threads a reply draft answers, by id (ME-011); kept by
    /// `refresh_rows`.
    pub draft_replies: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

/// Where a mail view was left (MB-071).
#[derive(Clone)]
struct NavMemory {
    selected: Option<String>,
    scroll: ListOffset,
}

/// A closed overlay, drawn while it fades out. Inert: focus has moved on,
/// its events go nowhere and its backdrop takes no clicks (MD-007).
enum LeavingOverlay {
    Palette(Entity<CommandPalette>),
    Help,
    Card(Entity<ui::contact_card::ContactCardView>),
}

impl PigeonApp {
    /// The app on `backend`'s data, with the settings `saved` applied.
    pub fn new(
        backend: Entity<Backend>,
        saved: Saved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Without the bodies: the first frame doesn't wait on them (MA-072).
        let (data, calendar, contacts) = {
            let b = backend.read(cx);
            (b.load_mail_heads(), b.load_calendar(), b.contacts())
        };
        let heads = data.threads.iter().map(|t| t.id.clone()).collect();
        if let Some(started) = crate::STARTED.get().copied() {
            window.on_next_frame(move |_, _| {
                crate::logging::info(&format!(
                    "window: first frame {} ms after launch",
                    started.elapsed().as_millis()
                ));
            });
        }
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.sync_theme(window, cx);
            cx.notify();
        });
        // Reduce Motion is read at startup; this catches a change made while
        // Pigeon was in the background (MB-072: the system's alone). So do
        // the login item and the mail handler (MC-038).
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                gpui::base::apply_system_reduce_motion(cx);
                shell::refresh_system_items();
            }
            // Sync on focus; poll less in the background (SYN-2).
            let active = window.is_window_active();
            this.backend
                .update(cx, |b, cx| b.set_window_active(active, cx));
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search — try from:, is:unread, in:all")
        });
        let search_events = cx.subscribe_in(&search_input, window, Self::on_search_event);
        let help_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter shortcuts"));
        // Only the shortcuts sheet, in the overlay layer, reads the filter.
        let help_events = cx.subscribe(&help_filter, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.panes.layer.update(cx, |_, cx| cx.notify());
            }
        });
        let (settings_page, settings_events) = SettingsPage::new(window, cx);
        let contact_book = ContactBook::shared(contacts);
        let calendar = cx.new(|cx| {
            CalendarView::new(calendar, contact_book.clone(), backend.clone(), window, cx)
        });
        // What the backend brings fades in (MD-024).
        let backend_events = cx.subscribe(&backend, |this, backend, event, cx| {
            this.with_row_cause(RowCause::Sync, |this| {
                this.on_backend_event(backend, event, cx)
            })
        });
        // Settings are saved whenever they change, here or in the calendar.
        let settings_changed = cx.observe_self(|this, cx| this.save_settings(cx));
        // MC-036: the panes draw from this state, so any change redraws them.
        let panes = panes::Panes::new(
            Self::sidebar_pane,
            Self::list_pane,
            Self::reader_pane,
            Self::layer_pane,
            cx,
        );
        let panes_follow = cx.observe_self(|this, cx| this.panes.notify(cx));
        let calendar_changed = cx.observe_in(&calendar, window, |this, _, window, cx| {
            this.save_settings(cx);
            this.refresh_menus(window, cx);
            // The list's inbox-zero line names the next event, and the
            // calendar's minute tick moves the panes' relative times
            // ("Synced 4 min ago").
            this.panes.notify(cx);
        });
        // MA-034: the frame is saved as it changes, debounced by the backend.
        let bounds = cx.observe_window_bounds(window, |this, window, cx| {
            this.window_frame = Some(shell::window_frame(window, cx));
            this.save_settings(cx);
        });
        let quit = cx.on_app_quit(|this, cx| {
            this.save_settings(cx);
            this.backend.update(cx, |b, _| b.flush_settings());
            async {}
        });
        // Escape takes an error toast away before anything else (ME-056).
        let window_id = window.window_handle().window_id();
        let app = cx.weak_entity();
        let escape = cx.intercept_keystrokes(move |event, window, cx| {
            let k = &event.keystroke;
            if k.key != "escape"
                || k.modifiers.modified()
                || window.window_handle().window_id() != window_id
            {
                return;
            }
            if app
                .update(cx, |this, cx| this.dismiss_error_on_escape(cx))
                .unwrap_or(false)
            {
                cx.stop_propagation();
            }
        });
        let calendar_events =
            cx.subscribe_in(
                &calendar,
                window,
                |this, _, event, window, cx| match event {
                    CalendarEvent::Undoable { message, removal } => {
                        let undo = UndoAction::DeleteEvent(removal.clone());
                        this.show_undoable(message.clone(), removal.undo_title(), undo, cx);
                    }
                    CalendarEvent::Discarded { message, discarded } => {
                        let undo = UndoAction::DiscardEvent(discarded.clone());
                        this.show_undoable(message.clone(), "Discard Event", undo, cx);
                    }
                    // W6 (CAL-22, CON-5).
                    CalendarEvent::Refused(why) => this.show_error_toast(why.clone(), cx),
                    CalendarEvent::ShowContact(contact, position) => {
                        this.show_contact(contact.clone(), *position, window, cx)
                    }
                },
            );
        // A drag or panel edit of an event's time (MA-071).
        let retimed = cx.subscribe(&calendar, |this, _, event: &Retimed, cx| {
            this.push_undo(event.title, UndoAction::Retime(event.retime.clone()), cx);
        });
        let window_frame = Some(shell::window_frame(window, cx));

        let mut this = Self {
            mode: Mode::Mail,
            backend,
            calendar,
            data,
            nav: Nav::Folder(Folder::Inbox),
            rows: Vec::new(),
            leaving_rows: Vec::new(),
            next_leaving: 0,
            growing_rows: HashMap::new(),
            fresh_rows: (HashSet::new(), None),
            row_cause: RowCause::User,
            selected: None,
            expanded_messages: HashSet::new(),
            clicked_open: None,
            archived_expanded: false,
            sidebar_open: true,
            sidebar_forced: false,
            sidebar_width: SIDEBAR_DEFAULT,
            list_width: LIST_DEFAULT,
            resizing: None,
            shown: PaneLayout {
                sidebar: px(0.),
                list: LIST_DEFAULT,
            },
            gliding: None,
            sidebar_progress: 0.,
            sidebar_sliding: false,
            counts: Default::default(),
            panes,
            leaving_overlay: None,
            text_scale: 1.,
            search_open: false,
            list_state: ListState::new(0, ListAlignment::Top, px(400.)),
            reader_scroll: ScrollHandle::new(),
            reader_glide: None,
            html_cache: Default::default(),
            images: Default::default(),
            reader_focused: false,
            focus_handle,
            palette: None,
            help_open: false,
            contact_card: None,
            help_focus: cx.focus_handle(),
            composer: None,
            reply_ghost: None,
            reply_card: 0.,
            draft_windows: Vec::new(),
            windowed: HashSet::new(),
            contact_book,
            search_input,
            search_query: Query::default(),
            focused_message: None,
            toast: None,
            toast_ghost: None,
            just_starred: (Vec::new(), None),
            undo_stack: Default::default(),
            sends: Vec::new(),
            queued_sends: HashSet::new(),
            auth_expired: false,
            checked: HashSet::new(),
            select_anchor: None,
            context_target: None,
            density: Density::Comfortable,
            search_all: false,
            recent_searches: Vec::new(),
            search_highlight: None,
            recent_commands: saved.recent_commands.clone(),
            select_all_text: None,
            help_filter,
            reveal_pending: false,
            settings: Settings::default(),
            settings_page,
            saved: saved.clone(),
            server_counts: Default::default(),
            server_search: Default::default(),
            original: None,
            labels_ui: Default::default(),
            heads,
            nav_memory: Vec::new(),
            window_frame,
            window_title: "Pigeon".into(),
            seen_tip: saved.seen_tip,
            draft_replies: HashSet::new(),
            _subscriptions: vec![
                settings_events,
                appearance,
                activation,
                search_events,
                calendar_events,
                help_events,
                backend_events,
                settings_changed,
                panes_follow,
                calendar_changed,
                quit,
                escape,
                bounds,
                retimed,
            ],
        };
        this.labels_ui.folded = saved.collapsed_labels.iter().cloned().collect();
        // MA-034: back in Mail or Calendar, on the view last left; nothing
        // is selected, so nothing is marked read or opened.
        if let Some(nav) = this.nav_from_saved(&saved.nav) {
            this.nav = nav;
        }
        if saved.calendar_open {
            this.mode = Mode::Calendar;
        }
        this.apply_settings(saved, window, cx);
        this.server_search.available = this.backend.read(cx).can_search();
        this.refresh_rows(false);
        this.refresh_server_counts(cx);
        this.queued_sends = this.backend.read(cx).queued_sends();
        window.focus(&this.pane_focus(cx), cx);
        profile::start(this.backend.read(cx).is_demo(), window, cx);
        this
    }

    /// A saved view, if it's still there.
    fn nav_from_saved(&self, saved: &SavedNav) -> Option<Nav> {
        Some(match saved {
            SavedNav::Folder(folder) => Nav::Folder(*folder),
            SavedNav::Starred => Nav::Starred,
            SavedNav::Label(id) => Nav::Label(self.data.labels.iter().position(|l| &l.id == id)?),
            SavedNav::Category(category) => Nav::Category(*category),
        })
    }

    fn saved_nav(&self) -> SavedNav {
        match self.nav {
            Nav::Folder(folder) => SavedNav::Folder(folder),
            Nav::Starred => SavedNav::Starred,
            Nav::Label(ix) => self
                .data
                .labels
                .get(ix)
                .map_or_else(SavedNav::default, |l| SavedNav::Label(l.id.clone())),
            Nav::Category(category) => SavedNav::Category(category),
        }
    }

    /// Put saved settings into effect: at startup, before the first frame.
    fn apply_settings(&mut self, saved: Saved, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.auto_advance = saved.auto_advance;
        self.settings.search_all_mail = saved.search_all_mail;
        self.search_all = saved.search_all_mail;
        self.density = if saved.compact_list {
            Density::Compact
        } else {
            Density::Comfortable
        };
        self.set_text_scale(saved.text_scale, cx);
        self.sidebar_open = saved.sidebar_open;
        self.sidebar_width = px(saved.sidebar_width).clamp(SIDEBAR_WIDTH.0, SIDEBAR_WIDTH.1);
        self.list_width = px(saved.list_width).clamp(LIST_WIDTH.0, LIST_WIDTH.1);
        self.archived_expanded = saved.archived_expanded;
        self.set_appearance(saved.appearance, window, cx);
        self.calendar.update(cx, |c, _| {
            c.view = match saved.calendar_view {
                CalendarViewSetting::Day => ViewKind::Day,
                CalendarViewSetting::Week => ViewKind::Week,
                CalendarViewSetting::Month => ViewKind::Month,
            };
            c.show_week_numbers = saved.week_numbers;
            c.apply_saved(&saved);
        });
    }

    /// The settings as they are now, wherever they live: the window's
    /// here, the app-wide ones as last remembered (MC-046).
    fn current_settings(&self, cx: &gpui::App) -> Saved {
        let calendar = self.calendar.read(cx);
        Saved {
            appearance: self.settings.appearance,
            auto_advance: self.settings.auto_advance,
            search_all_mail: self.settings.search_all_mail,
            compact_list: self.density == Density::Compact,
            text_scale: self.text_scale,
            sidebar_open: self.sidebar_open,
            sidebar_width: f32::from(self.sidebar_width),
            list_width: f32::from(self.list_width),
            archived_expanded: self.archived_expanded,
            calendar_view: match calendar.view {
                ViewKind::Day => CalendarViewSetting::Day,
                ViewKind::Week => CalendarViewSetting::Week,
                ViewKind::Month => CalendarViewSetting::Month,
            },
            week_numbers: calendar.show_week_numbers,
            remote_images: self.saved.remote_images,
            calendar_date: Some(calendar.anchor),
            event_minutes: calendar.prefs.event_minutes,
            default_calendar: calendar.prefs.default_calendar.clone(),
            collapsed_labels: {
                let mut ids: Vec<String> = self.labels_ui.folded.iter().cloned().collect();
                ids.sort();
                ids
            },
            window: self.window_frame.clone(),
            calendar_open: self.mode == Mode::Calendar,
            nav: self.saved_nav(),
            seen_tip: self.seen_tip,
            recent_commands: self.recent_commands.clone(),
            // Notifications, time format, menu bar, glass, keymap, the
            // Dock badge and the rest.
            ..crate::settings::get(cx).clone()
        }
    }

    /// Hand the settings to the backend to save, when they changed (PER-3).
    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let now = self.current_settings(cx);
        if now != self.saved {
            let map = now.to_map();
            crate::settings::remember(&now, cx);
            self.saved = now;
            self.backend.update(cx, |b, cx| b.save_settings(map, cx));
        }
    }

    pub fn nav_contains(&self, nav: Nav, thread: &Thread) -> bool {
        nav_holds(nav, thread, &self.data.labels)
    }

    /// Recompute the list rows for the current nav. With `keep_scroll`, only
    /// the rows that changed are spliced so the list stays where it was.
    fn refresh_rows(&mut self, keep_scroll: bool) {
        // ME-011: the rows mark conversations a draft answers.
        self.draft_replies = self
            .data
            .threads
            .iter()
            .filter(|t| t.folder == Folder::Drafts)
            .filter_map(|t| t.origin.as_ref()?.reply_to.clone())
            .collect();
        let threads = &self.data.threads;
        let searching = self.is_searching();
        let in_view = |thread: &Thread| {
            if !searching {
                return self.nav_contains(self.nav, thread);
            }
            match self.search_query.scope {
                Some(scope) => Query::in_scope(scope, thread),
                None if self.search_all => Query::in_scope(crate::search::Scope::All, thread),
                // The Inbox shows its archived threads too, so its search
                // does; a Gmail-only one looks in the Inbox alone
                // (`Query::to_gmail_q`).
                None if self.nav == Nav::Folder(Folder::Inbox) => {
                    thread.folder == Folder::Inbox
                        || (!self.search_query.gmail_only() && thread.folder == Folder::Archive)
                }
                None => self.nav_contains(self.nav, thread),
            }
        };
        // W4b (SRC-9): Gmail alone answers some queries.
        let gmail_only = self.search_query.gmail_only();
        let mut rows: Vec<Row> = if searching {
            // W9: with what Gmail found (SRC-5).
            self.server_search
                .merge(threads, in_view, |t| {
                    !gmail_only && self.search_query.matches(t, &self.data)
                })
                .into_iter()
                .map(Row::Thread)
                .collect()
        } else {
            (0..threads.len())
                .filter(|&ix| in_view(&threads[ix]))
                .map(Row::Thread)
                .collect()
        };

        if self.nav == Nav::Folder(Folder::Inbox) && !searching {
            let archived = (0..threads.len()).filter(|&ix| threads[ix].folder == Folder::Archive);
            let count = archived.clone().count();
            if count > 0 {
                rows.push(Row::ArchivedHeader { count });
                if self.archived_expanded {
                    rows.extend(archived.map(Row::Thread));
                }
            }
        }
        if self.shows_load_older() {
            rows.push(Row::LoadOlder);
        }

        // A change while rows are still leaving takes them away at once.
        self.leaving_rows.clear();
        if keep_scroll {
            rows = self.rows_in_motion(rows);
        } else {
            self.growing_rows.clear();
        }
        if keep_scroll {
            let old = &self.rows;
            let prefix = old.iter().zip(&rows).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            // A list at its top stays there, so new mail shows above the
            // rest instead of the splice scrolling it out of view; the
            // selection, if it was showing, is kept on screen.
            let top = self.list_state.logical_scroll_top();
            let at_top = top.item_ix == 0 && top.offset_in_item == Pixels::ZERO;
            let selected_shown = self.selected_row().is_some_and(|row| {
                on_screen(
                    self.list_state.bounds_for_item(row),
                    self.list_state.viewport_bounds(),
                )
            });
            self.list_state
                .splice(prefix..old.len() - suffix, rows.len() - prefix - suffix);
            if at_top && prefix == 0 {
                self.list_state.scroll_to(ListOffset::default());
                self.reveal_pending |= selected_shown;
            }
        } else {
            self.list_state.reset(rows.len());
        }
        self.rows = rows;
    }

    /// `rows`, with the conversations that just left still in place as
    /// `Row::Leaving`, and the ones that just came marked to grow or fade
    /// in, as `row_cause` says (MD-017, MD-024).
    fn rows_in_motion(&mut self, mut rows: Vec<Row>) -> Vec<Row> {
        let threads = &self.data.threads;
        let id_of = |row: &Row| match row {
            Row::Thread(ix) => threads.get(*ix).map(|t| t.id.as_str()),
            _ => None,
        };
        let new_ids: HashSet<&str> = rows.iter().filter_map(id_of).collect();
        let old_ids: HashSet<&str> = self.rows.iter().filter_map(id_of).collect();
        if self.row_cause != RowCause::Quiet {
            // Where each one that left was: after the rows before it that
            // stayed.
            let mut at = 0;
            let mut leaving = Vec::new();
            for (old_ix, row) in self.rows.iter().enumerate() {
                match id_of(row) {
                    Some(id) if !new_ids.contains(id) => {
                        // Only one drawn before collapses; the rest just go.
                        if let Some(bounds) = self.list_state.bounds_for_item(old_ix) {
                            leaving.push((at, id.to_string(), bounds.size.height));
                        }
                    }
                    _ => {
                        if let Some(k) = rows[at..].iter().position(|r| r == row) {
                            at += k + 1;
                        }
                    }
                }
            }
            if leaving.len() <= LEAVING_MAX {
                for (shift, (at, id, height)) in leaving.into_iter().enumerate() {
                    let key = self.next_leaving;
                    self.next_leaving += 1;
                    rows.insert(at + shift, Row::Leaving(key));
                    self.leaving_rows.push(LeavingRow {
                        key,
                        id,
                        height,
                        glide: ui::motion::Glide::exit(),
                    });
                }
            }
        }
        let came: Vec<String> = rows
            .iter()
            .filter_map(id_of)
            .filter(|id| !old_ids.contains(id))
            .map(String::from)
            .collect();
        match self.row_cause {
            RowCause::Undo => {
                for id in came {
                    self.growing_rows.insert(id, ui::motion::Glide::pane());
                }
            }
            RowCause::Sync if !came.is_empty() => {
                self.fresh_rows = (came.into_iter().collect(), Some(std::time::Instant::now()));
            }
            _ => {}
        }
        rows
    }

    /// One frame of the rows leaving and growing, as the list draws: each
    /// is measured again, and a row that has left is taken out.
    fn step_rows(&mut self, window: &mut Window, cx: &gpui::App) {
        if self.leaving_rows.is_empty() && self.growing_rows.is_empty() {
            return;
        }
        // Under Reduce Motion they snap.
        let snap = cx.reduce_motion();
        let mut gone = HashSet::new();
        for row in &mut self.leaving_rows {
            if snap || row.glide.step().1 {
                gone.insert(row.key);
            }
        }
        self.leaving_rows.retain(|row| !gone.contains(&row.key));
        let mut grown = Vec::new();
        for (id, glide) in &mut self.growing_rows {
            if snap || glide.step().1 {
                grown.push(id.clone());
            }
        }
        for ix in (0..self.rows.len()).rev() {
            let id = match self.rows[ix] {
                Row::Leaving(key) if gone.contains(&key) => {
                    self.rows.remove(ix);
                    self.list_state.splice(ix..ix + 1, 0);
                    continue;
                }
                Row::Leaving(_) => None,
                Row::Thread(t) => self.data.threads.get(t).map(|t| t.id.as_str()),
                _ => continue,
            };
            if id.is_none_or(|id| self.growing_rows.contains_key(id)) {
                self.list_state.remeasure_items(ix..ix + 1);
            }
        }
        for id in grown {
            self.growing_rows.remove(&id);
        }
        if !self.leaving_rows.is_empty() || !self.growing_rows.is_empty() {
            window.request_animation_frame();
        }
    }

    /// Run `f` with the rows' changes moving as `cause` says.
    fn with_row_cause<R>(&mut self, cause: RowCause, f: impl FnOnce(&mut Self) -> R) -> R {
        let before = std::mem::replace(&mut self.row_cause, cause);
        let result = f(self);
        self.row_cause = before;
        result
    }

    pub fn is_searching(&self) -> bool {
        self.search_open && !self.search_query.is_empty()
    }

    pub fn set_nav(&mut self, nav: Nav, cx: &mut Context<Self>) {
        if self.nav != nav {
            self.stash_composer(cx);
            self.remember_nav();
            self.nav = nav;
            self.labels_ui.hidden_while_open = None;
            self.selected = None;
            self.clear_checked();
            self.reader_focused = false;
            self.search_gmail(cx);
            self.refresh_rows(false);
            self.restore_nav();
        }
        cx.notify();
    }

    /// MB-071: keep where the view was left, to come back to.
    fn remember_nav(&mut self) {
        let memory = NavMemory {
            selected: self.selected.map(|ix| self.data.threads[ix].id.clone()),
            scroll: self.list_state.logical_scroll_top(),
        };
        self.nav_memory.retain(|(nav, _)| *nav != self.nav);
        self.nav_memory.push((self.nav, memory));
    }

    /// Back on a view: its scroll and selection as left. The selection is
    /// only shown again: not marked read, and a draft doesn't open.
    fn restore_nav(&mut self) {
        let Some((_, memory)) = self.nav_memory.iter().find(|(nav, _)| *nav == self.nav) else {
            return;
        };
        let memory = memory.clone();
        if self.is_searching() {
            return;
        }
        self.list_state.scroll_to(memory.scroll);
        self.selected = memory
            .selected
            .and_then(|id| self.data.thread_ix(&id))
            .filter(|&ix| {
                self.data.threads[ix].folder != Folder::Drafts
                    && self.rows.contains(&Row::Thread(ix))
            });
        self.expanded_messages.clear();
        self.clicked_open = None;
        self.focused_message = None;
        self.reader_glide = None;
        self.reader_scroll.set_offset(point(px(0.), px(0.)));
    }

    pub fn select_thread(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Leaving a draft saves it, which can reorder the threads.
        let id = self.data.threads[ix].id.clone();
        let already_open = self
            .composer
            .as_ref()
            .is_some_and(|(c, _)| c.read(cx).draft_id.as_deref() == Some(&id));
        if !already_open {
            self.stash_composer(cx);
        }
        let Some(ix) = self.data.thread_ix(&id) else {
            return;
        };
        self.ensure_bodies(ix, cx);
        if self.selected != Some(ix) {
            self.expanded_messages.clear();
            self.clicked_open = None;
            self.focused_message = None;
            self.select_all_text = None;
            self.reader_glide = None;
            self.reader_scroll.set_offset(point(px(0.), px(0.)));
            self.expand_unread(ix);
        }
        self.selected = Some(ix);
        if std::mem::take(&mut self.data.threads[ix].unread) {
            // So reopening it before the next sync doesn't expand them again.
            for m in &mut self.data.threads[ix].messages {
                m.label_ids.retain(|l| l != gmail::UNREAD);
            }
            let threads = vec![id.clone()];
            self.backend.update(cx, |b, cx| {
                b.mail(
                    MailOp::Modify {
                        threads,
                        add: Vec::new(),
                        remove: vec![gmail::UNREAD.into()],
                    },
                    cx,
                )
            });
        }
        self.reveal_selected();
        // Bodies come on demand, once the selection settles (RDR-6).
        self.backend.update(cx, |b, cx| b.fetch_body(&id, cx));
        // A draft in its own window stays there, brought forward (MB-047).
        if self.data.threads[ix].folder == Folder::Drafts
            && !already_open
            && !self.raise_draft_window(&id, cx)
        {
            self.open_draft(ix, false, window, cx);
        }
        cx.notify();
    }

    /// MA-072: a thread held without its bodies gets the stored ones, in
    /// place. Whether it was held so: then `fetch_body` asks Gmail for any
    /// the store lacks.
    fn ensure_bodies(&mut self, ix: usize, cx: &mut Context<Self>) -> bool {
        let id = self.data.threads[ix].id.clone();
        if !self.heads.remove(&id) {
            return false;
        }
        if let Some(thread) = self.backend.read(cx).load_threads(&[id]).pop() {
            self.data.threads[ix] = thread;
        }
        true
    }

    /// ME-008: a newly opened conversation shows its unread messages
    /// expanded, the first of them focused.
    fn expand_unread(&mut self, ix: usize) {
        let messages = &self.data.threads[ix].messages;
        let last = messages.len().saturating_sub(1);
        let unread: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(i, m)| *i != last && m.label_ids.iter().any(|l| l == gmail::UNREAD))
            .map(|(i, _)| i)
            .collect();
        self.focused_message = unread.first().copied();
        self.expanded_messages.extend(unread);
    }

    /// A click on a message's header: it opens, fading in (MD-020), or
    /// closes at once.
    pub fn toggle_message(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.expanded_messages.remove(&ix) {
            self.expanded_messages.insert(ix);
            self.clicked_open = Some(ix);
        }
        cx.notify();
    }

    fn selected_row(&self) -> Option<usize> {
        let selected = Row::Thread(self.selected?);
        self.rows.iter().position(|r| *r == selected)
    }

    /// The thread row after `row`, or before it when there is none after.
    fn neighbor_thread(&self, row: usize, forward: bool) -> Option<usize> {
        let thread_at = |r: &Row| match r {
            Row::Thread(ix) => Some(*ix),
            _ => None,
        };
        if forward {
            self.rows[row + 1..].iter().find_map(thread_at)
        } else {
            self.rows[..row].iter().rev().find_map(thread_at)
        }
    }

    fn select_adjacent(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let next = match self.selected_row() {
            Some(row) => self.neighbor_thread(row, forward),
            None => self.rows.iter().find_map(|r| match r {
                Row::Thread(ix) => Some(*ix),
                _ => None,
            }),
        };
        match next {
            // Moving through a selection leaves the threads unread.
            Some(ix) if !self.checked.is_empty() => {
                self.selected = Some(ix);
                self.reveal_selected();
                cx.notify();
            }
            Some(ix) => self.select_thread(ix, window, cx),
            // W9: `j` past the last thread reaches "Load older".
            None if forward && self.rows.last() == Some(&Row::LoadOlder) => {
                self.activate_load_older(cx)
            }
            None => {}
        }
    }

    /// Home / End: the first or last conversation in the list.
    fn select_edge(&mut self, last: bool, window: &mut Window, cx: &mut Context<Self>) {
        let thread_at = |r: &Row| match r {
            Row::Thread(ix) => Some(*ix),
            _ => None,
        };
        let edge = if last {
            self.rows.iter().rev().find_map(thread_at)
        } else {
            self.rows.iter().find_map(thread_at)
        };
        if let Some(ix) = edge {
            self.move_selection_to(ix, window, cx);
        }
    }

    /// Select `ix` from the keyboard; moving through picked threads leaves
    /// them unread.
    fn move_selection_to(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.checked.is_empty() {
            return self.select_thread(ix, window, cx);
        }
        self.selected = Some(ix);
        self.reveal_selected();
        cx.notify();
    }

    /// MB-045: Space and Page Down in the reader, by a screenful less a
    /// little, so the last lines stay in view.
    fn page_reader(&mut self, forward: bool, cx: &mut Context<Self>) {
        let height = self.reader_scroll.bounds().size.height;
        let step = (height - PAGE_OVERLAP).max(PAGE_OVERLAP);
        self.scroll_reader(if forward { step } else { -step }, cx);
    }

    /// The list pages itself (`app/mail.rs`).
    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.reader_focused {
            return cx.propagate();
        }
        self.page_reader(true, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.reader_focused {
            return cx.propagate();
        }
        self.page_reader(false, cx);
    }

    fn scroll_to_top(&mut self, _: &ScrollToTop, window: &mut Window, cx: &mut Context<Self>) {
        if !self.reader_focused {
            return self.select_edge(false, window, cx);
        }
        self.reader_glide = None;
        self.reader_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn scroll_to_bottom(
        &mut self,
        _: &ScrollToBottom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.reader_focused {
            return self.select_edge(true, window, cx);
        }
        let max = self.reader_scroll.max_offset().y;
        self.reader_glide = None;
        self.reader_scroll.set_offset(point(px(0.), -max));
        cx.notify();
    }

    /// The thread that takes the selection when the selected one leaves the
    /// list: the next one, or the previous when there is none after it.
    fn next_after_selected(&self) -> Option<usize> {
        let row = self.selected_row()?;
        self.neighbor_thread(row, true)
            .or_else(|| self.neighbor_thread(row, false))
    }

    // Action handlers.

    /// The panes' widths in a window `width` wide. The sidebar hides while
    /// it would squeeze the list under its minimum, unless the user showed it
    /// anyway; then the list squeezes further. The reader takes the rest,
    /// never less than `READER_MIN`.
    pub fn pane_layout(&self, width: Pixels) -> PaneLayout {
        let room = width - RAIL_WIDTH - READER_MIN;
        let sidebar = if self.sidebar_open && (self.sidebar_fits(width) || self.sidebar_forced) {
            self.sidebar_width
                .min(room - LIST_SQUEEZED)
                .max(SIDEBAR_WIDTH.0)
        } else {
            px(0.)
        };
        let list = self.list_width.min(room - sidebar).max(LIST_SQUEEZED);
        PaneLayout { sidebar, list }
    }

    fn sidebar_fits(&self, width: Pixels) -> bool {
        width - RAIL_WIDTH - READER_MIN - self.sidebar_width >= LIST_WIDTH.0
    }

    pub(crate) fn toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self.pane_layout(window.viewport_size().width).sidebar > px(0.);
        if shown {
            self.sidebar_open = false;
            self.sidebar_forced = false;
        } else {
            self.sidebar_open = true;
            self.sidebar_forced = !self.sidebar_fits(window.viewport_size().width);
        }
        self.sidebar_sliding = true;
        cx.notify();
    }

    pub(crate) fn set_text_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        let (min, max) = ui::style::TEXT_SCALE_RANGE;
        // Rounded, so repeated steps land back on 1.
        self.text_scale = ((scale * 10.).round() / 10.).clamp(min, max);
        cx.notify();
    }

    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.set_text_scale(self.text_scale + ui::style::TEXT_SCALE_STEP, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.set_text_scale(self.text_scale - ui::style::TEXT_SCALE_STEP, cx);
    }

    fn reset_zoom(&mut self, _: &ResetZoom, _: &mut Window, cx: &mut Context<Self>) {
        self.set_text_scale(1., cx);
    }

    // Keyboard access to the buttons and rows: Tab moves between them,
    // escape goes back to the panes.

    fn focus_next_control(
        &mut self,
        _: &FocusNextControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus_next(cx);
    }

    fn focus_prev_control(
        &mut self,
        _: &FocusPrevControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus_prev(cx);
    }

    fn focus_panes(&mut self, _: &FocusPanes, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.pane_focus(cx), cx);
        cx.notify();
    }

    // The Edit menu (MA-070): a focused text field's own, else Pigeon's.

    fn edit_undo(&mut self, _: &EditUndo, window: &mut Window, cx: &mut Context<Self>) {
        if text_field_focused(window) {
            return window.dispatch_action(Box::new(input::Undo), cx);
        }
        self.with_row_cause(RowCause::Undo, |this| this.undo_last(window, cx));
    }

    fn edit_redo(&mut self, _: &EditRedo, window: &mut Window, cx: &mut Context<Self>) {
        if text_field_focused(window) {
            return window.dispatch_action(Box::new(input::Redo), cx);
        }
        self.with_row_cause(RowCause::Undo, |this| this.redo_last(window, cx));
    }

    /// The field's text, else the selected text (the base root copies it),
    /// else the source or the contact card's address.
    fn edit_copy(&mut self, _: &EditCopy, window: &mut Window, cx: &mut Context<Self>) {
        if text_field_focused(window) || gpui::base::TextSelection::has_selection(window, cx) {
            window.dispatch_action(Box::new(input::Copy), cx);
        } else if window.is_action_available(&CopyOriginal, cx) {
            window.dispatch_action(Box::new(CopyOriginal), cx);
        } else if self.contact_card.is_some() {
            window.dispatch_action(Box::new(ui::contact_card::CopyAddress), cx);
        }
    }

    /// The field's text, the message's text in the reader, else every
    /// conversation in the list (MA-078).
    fn edit_select_all(&mut self, _: &EditSelectAll, window: &mut Window, cx: &mut Context<Self>) {
        if text_field_focused(window) {
            return window.dispatch_action(Box::new(input::SelectAll), cx);
        }
        if self.mode != Mode::Mail || self.settings_page.open || self.overlay_open() {
            return;
        }
        if self.reader_focused {
            self.select_message_text(&SelectMessageText, window, cx);
        } else {
            self.select_all_threads(cx);
        }
    }

    fn toggle_full_screen(
        &mut self,
        _: &ToggleFullScreen,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }

    /// MB-006: the menu bar follows the app's state: checkmarks, Show/Hide
    /// and the undo's title. Rebuilt only when that changes.
    fn refresh_menus(&self, window: &Window, cx: &mut Context<Self>) {
        let targets = self.targets();
        let all = |f: fn(&Thread) -> bool| {
            !targets.is_empty() && targets.iter().all(|&ix| f(&self.data.threads[ix]))
        };
        let calendar = self.calendar.read(cx);
        let state = crate::MenuState {
            calendar: self.mode == Mode::Calendar,
            calendar_view: calendar.view,
            sidebar: self.pane_layout(window.viewport_size().width).sidebar > px(0.),
            compact_list: self.density == Density::Compact,
            week_numbers: calendar.show_week_numbers,
            starred: all(|t| t.starred),
            important: all(|t| t.important),
            full_screen: window.is_fullscreen(),
            signed_in: self.backend.read(cx).source() == Source::Google,
            undo: self
                .undo_stack
                .undo_title()
                .map(|t| format!("Undo {t}").into()),
            redo: self
                .undo_stack
                .redo_title()
                .map(|t| format!("Redo {t}").into()),
        };
        crate::set_menu_state(state, cx);
    }

    /// The palette, the shortcuts sheet or the contact card.
    pub(crate) fn overlay_open(&self) -> bool {
        self.palette.is_some() || self.help_open || self.contact_card.is_some()
    }

    /// MB-008: the standard About panel; the unbundled dev build, which
    /// has no Info.plist for it, says the version instead.
    fn about(&mut self, _: &About, _: &mut Window, cx: &mut Context<Self>) {
        let bundled = std::env::current_exe()
            .is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"));
        if bundled {
            crate::native::show_about();
        } else {
            let version = env!("CARGO_PKG_VERSION");
            self.show_toast(
                ToastKind::Info,
                format!("Pigeon {version} · mail and calendar"),
                None,
                cx,
            );
        }
    }

    // The account and the backend. Demo data has no server, so these only
    // say so there.

    fn sync_now(&mut self, _: &SyncNow, _: &mut Window, cx: &mut Context<Self>) {
        let demo = self.backend.read(cx).is_demo();
        self.backend.update(cx, |b, cx| b.sync_now(cx));
        if demo {
            self.show_toast(ToastKind::Info, "Demo data is always up to date", None, cx);
        }
    }

    fn sign_in(&mut self, _: &SignIn, _: &mut Window, cx: &mut Context<Self>) {
        // ME-050: demo data picked on the sign-in screen goes back to it.
        if self.backend.read(cx).demo_by_choice() {
            self.backend.update(cx, |b, cx| {
                b.leave_demo(cx);
                b.sign_in(cx);
            });
            return;
        }
        let b = self.backend.read(cx);
        let message = match b.source() {
            Source::Demo => {
                Some("Pigeon was opened with demo data. Quit and open it again to sign in.")
            }
            Source::Google if !b.needs_sign_in() => Some("Already signed in"),
            _ => None,
        };
        let signed_in = b.source() == Source::Google;
        match message {
            Some(message) => self.show_toast(ToastKind::Info, message, None, cx),
            None => {
                self.backend.update(cx, |b, cx| b.sign_in(cx));
                // The sign-in screen isn't showing to say so.
                if signed_in {
                    self.show_toast(
                        ToastKind::Info,
                        "Finish signing in in your browser",
                        None,
                        cx,
                    );
                }
            }
        }
    }

    fn sign_out(&mut self, _: &SignOut, window: &mut Window, cx: &mut Context<Self>) {
        if self.backend.read(cx).source() == Source::Google {
            return self.ask_sign_out(window, cx);
        }
        if self.backend.read(cx).demo_by_choice() {
            return self.backend.update(cx, |b, cx| b.leave_demo(cx));
        }
        let demo = self.backend.read(cx).is_demo();
        self.backend.update(cx, |b, cx| b.sign_out(cx));
        if demo {
            self.show_toast(
                ToastKind::Info,
                "Demo data has no account to sign out of",
                None,
                cx,
            );
        }
    }

    fn use_demo_data(&mut self, _: &UseDemoData, _: &mut Window, cx: &mut Context<Self>) {
        match self.backend.read(cx).source() {
            Source::Demo => self.show_toast(ToastKind::Info, "Already showing demo data", None, cx),
            Source::Google => self.show_toast(
                ToastKind::Info,
                "Sign out first to use the demo data",
                None,
                cx,
            ),
            Source::SignedOut => self.backend.update(cx, |b, cx| b.use_demo(cx)),
        }
    }

    fn retry_loading_message(
        &mut self,
        _: &RetryLoadingMessage,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected.map(|ix| self.data.threads[ix].id.clone()) else {
            return;
        };
        self.backend.update(cx, |b, cx| b.fetch_body(&id, cx));
    }

    /// W10 (ATT-1): load the selected conversation's remote images.
    fn load_images(&mut self, _: &LoadImages, _: &mut Window, cx: &mut Context<Self>) {
        if self.backend.read(cx).is_demo() {
            self.show_toast(
                ToastKind::Info,
                "Remote images don't load on the placeholder data",
                None,
                cx,
            );
            return;
        }
        let Some(ix) = self.selected else {
            return;
        };
        let ids: Vec<String> = self.data.threads[ix]
            .messages
            .iter()
            .map(|m| m.id.clone())
            .collect();
        for id in ids {
            self.allow_images(&id, cx);
        }
    }

    /// Settings › Images (ATT-1).
    pub(crate) fn remote_images_setting(&self) -> crate::settings::RemoteImages {
        self.saved.remote_images
    }

    fn load_older(&mut self, _: &LoadOlderConversations, _: &mut Window, cx: &mut Context<Self>) {
        let demo = self.backend.read(cx).is_demo();
        self.activate_load_older(cx);
        if demo {
            self.show_toast(ToastKind::Info, "That's all the demo mail", None, cx);
        }
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.before_close(cx);
        window.remove_window();
    }

    fn minimize(&mut self, _: &Minimize, window: &mut Window, _: &mut Context<Self>) {
        window.minimize_window();
    }

    fn zoom_window(&mut self, _: &ZoomWindow, window: &mut Window, _: &mut Context<Self>) {
        window.zoom_window();
    }

    /// Opens the search bar, or returns to it, and focuses it.
    fn toggle_search(&mut self, _: &ToggleSearch, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.open {
            return self.focus_settings_search(window, cx);
        }
        // MA-081: Find in Calendar searches the events, in the palette.
        if self.mode == Mode::Calendar {
            let items = self.event_items(false, cx);
            return self.show_palette(
                items,
                "",
                "Search events…".into(),
                "No matching events".into(),
                window,
                cx,
            );
        }
        self.search_open = true;
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close_search(&mut self, _: &CloseSearch, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_open {
            return cx.propagate();
        }
        self.search_open = false;
        self.search_all = self.settings.search_all_mail;
        self.search_query = Query::default();
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search_gmail(cx);
        self.refresh_rows(false);
        self.reveal_selected();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn on_search_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.search_query = Query::parse(&input.read(cx).value());
                self.search_highlight = None;
                self.refresh_rows(false);
                self.search_gmail(cx);
                cx.notify();
            }
            InputEvent::PressEnter { .. } if !self.accept_search_suggestion(window, cx) => {
                self.remember_search(cx);
                self.focus_results(&FocusResults, window, cx);
            }
            _ => {}
        }
    }

    /// From the search bar into its results.
    fn focus_results(&mut self, _: &FocusResults, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        self.reader_focused = false;
        if self.selected_row().is_none() {
            self.selected = None;
            self.select_adjacent(true, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_archived_section(
        &mut self,
        _: &ToggleArchivedSection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.archived_expanded = !self.archived_expanded;
        self.with_row_cause(RowCause::Quiet, |this| this.refresh_rows(true));
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNextThread, window: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(true, window, cx);
    }

    fn select_prev(&mut self, _: &SelectPrevThread, window: &mut Window, cx: &mut Context<Self>) {
        self.select_adjacent(false, window, cx);
    }

    fn select_first(&mut self, _: &SelectFirstThread, window: &mut Window, cx: &mut Context<Self>) {
        self.select_edge(false, window, cx);
    }

    fn select_last(&mut self, _: &SelectLastThread, window: &mut Window, cx: &mut Context<Self>) {
        self.select_edge(true, window, cx);
    }

    fn archive(&mut self, _: &Archive, window: &mut Window, cx: &mut Context<Self>) {
        self.archive_threads(self.targets(), window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_threads(self.targets(), window, cx);
    }

    fn toggle_star(&mut self, _: &ToggleStar, _: &mut Window, cx: &mut Context<Self>) {
        self.star_threads(self.targets(), cx);
    }

    fn mark_unread(&mut self, _: &MarkUnread, _: &mut Window, cx: &mut Context<Self>) {
        self.set_unread(self.targets(), true, cx);
    }

    fn mark_read(&mut self, _: &MarkRead, _: &mut Window, cx: &mut Context<Self>) {
        self.set_unread(self.targets(), false, cx);
    }

    /// Put a change to the threads into effect: newest first again, with the
    /// selection kept on the same thread.
    fn update_threads<R>(&mut self, f: impl FnOnce(&mut MailData) -> R) -> R {
        let selected = self.selected.map(|ix| self.data.threads[ix].id.clone());
        let old_ids: Vec<String> = self.data.threads.iter().map(|t| t.id.clone()).collect();
        let result = f(&mut self.data);
        self.data.sort_threads();
        self.selected = selected.and_then(|id| self.data.thread_ix(&id));
        // Point the old rows at where their threads are now, so a new or
        // re-sorted thread only splices its own row and the list stays put.
        let new_ix: HashMap<&str, usize> = self
            .data
            .threads
            .iter()
            .enumerate()
            .map(|(ix, t)| (t.id.as_str(), ix))
            .collect();
        for row in &mut self.rows {
            if let Row::Thread(ix) = row {
                *ix = new_ix
                    .get(old_ids[*ix].as_str())
                    .copied()
                    .unwrap_or(usize::MAX);
            }
        }
        self.refresh_rows(true);
        result
    }

    // Keyboard focus between the list and the reader.

    fn open_thread(&mut self, _: &OpenThread, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((composer, _)) = &self.composer {
            let composer = composer.clone();
            self.reader_focused = self.composer_inline(cx);
            composer.update(cx, |c, cx| c.focus_first(window, cx));
            return;
        }
        if self.selected.is_none() {
            self.select_adjacent(true, window, cx);
        }
        self.reader_focused = self.selected.is_some();
        cx.notify();
    }

    /// `n` / `p`: move through the messages of the open conversation,
    /// expanding the one moved to.
    fn step_message(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(thread) = self.selected.map(|ix| &self.data.threads[ix]) else {
            return;
        };
        let last = thread.messages.len() - 1;
        let target = match (self.focused_message, forward) {
            (None, true) => last,
            (None, false) => last.saturating_sub(1),
            (Some(ix), true) => (ix + 1).min(last),
            (Some(ix), false) => ix.saturating_sub(1),
        };
        self.focused_message = Some(target);
        let opened = target != last && self.expanded_messages.insert(target);
        // Glides to a message drawn as it is; one that opens or was folded
        // away is only placed once laid out, so the scroll waits for that.
        let item = ui::reader::MESSAGES_START + target;
        match self.reader_scroll_target(item).filter(|_| !opened) {
            Some(y) => self.glide_reader(y, cx),
            None => {
                self.reader_glide = None;
                self.reader_scroll.scroll_to_item(item);
            }
        }
        cx.notify();
    }

    /// The reader's offset that shows child `ix` whole, or its top when it's
    /// taller than the reader, moving the least: as `scroll_to_item` does.
    /// None while it hasn't been laid out with a height.
    fn reader_scroll_target(&self, ix: usize) -> Option<Pixels> {
        let item = self.reader_scroll.bounds_for_item(ix)?;
        if item.size.height <= px(0.) {
            return None;
        }
        let view = self.reader_scroll.bounds();
        let y = self
            .reader_glide
            .map_or(self.reader_scroll.offset().y, |g| px(g.target()));
        Some(
            if item.size.height > view.size.height || item.top() + y < view.top() {
                view.top() - item.top()
            } else if item.bottom() + y > view.bottom() {
                view.bottom() - item.bottom()
            } else {
                y
            },
        )
    }

    /// ⌘A in the reader: all of the focused message's text, or the last one's.
    fn select_message_text(
        &mut self,
        _: &SelectMessageText,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.selected.map(|ix| &self.data.threads[ix]) else {
            return;
        };
        gpui::base::TextSelection::clear(window, cx);
        self.select_all_text = Some(
            self.focused_message
                .unwrap_or(thread.messages.len().saturating_sub(1)),
        );
        cx.notify();
    }

    fn next_message(&mut self, _: &NextMessage, _: &mut Window, cx: &mut Context<Self>) {
        self.step_message(true, cx);
    }

    fn prev_message(&mut self, _: &PrevMessage, _: &mut Window, cx: &mut Context<Self>) {
        self.step_message(false, cx);
    }

    fn back_to_list(&mut self, _: &BackToList, _: &mut Window, cx: &mut Context<Self>) {
        // A download under way goes first, on its own Esc.
        if self.cancel_attachment_fetches(cx) {
            return;
        }
        self.reader_focused = false;
        cx.notify();
    }

    fn scroll_reader(&mut self, delta: Pixels, cx: &mut Context<Self>) {
        // From where a glide under way is going, so held keys add up.
        let from = self
            .reader_glide
            .map_or(self.reader_scroll.offset().y, |g| px(g.target()));
        self.glide_reader(from - delta, cx);
    }

    /// Scroll the reader to `y` (an offset, 0 or less): gliding, unless
    /// Reduce Motion is on (MD-018). Only the reader draws again.
    fn glide_reader(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let max = self.reader_scroll.max_offset().y;
        let y = y.clamp(-max, px(0.));
        if cx.reduce_motion() {
            self.reader_glide = None;
            self.reader_scroll
                .set_offset(point(self.reader_scroll.offset().x, y));
        } else {
            match &mut self.reader_glide {
                Some(glide) => glide.retarget(f32::from(y)),
                None => {
                    let from = f32::from(self.reader_scroll.offset().y);
                    self.reader_glide = Some(ui::motion::Glide::new(from, f32::from(y)));
                }
            }
        }
        self.panes.reader.update(cx, |_, cx| cx.notify());
    }

    /// One frame of the reader's glide, as the reader draws.
    fn step_reader_glide(&mut self, window: &mut Window) {
        let Some(glide) = &mut self.reader_glide else {
            return;
        };
        let (y, done) = glide.step();
        let x = self.reader_scroll.offset().x;
        self.reader_scroll.set_offset(point(x, px(y)));
        if done {
            self.reader_glide = None;
        } else {
            window.request_animation_frame();
        }
    }

    fn scroll_reader_down(&mut self, _: &ScrollReaderDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_reader(SCROLL_STEP, cx);
    }

    fn scroll_reader_up(&mut self, _: &ScrollReaderUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_reader(-SCROLL_STEP, cx);
    }

    fn mailbox_key_context(&self) -> KeyContext {
        let mut context = KeyContext::default();
        context.add(MAILBOX);
        if self.reader_focused {
            context.add(READER);
        }
        context
    }

    // Switching between Mail and Calendar.

    pub fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let leaving_settings = std::mem::replace(&mut self.settings_page.open, false);
        if leaving_settings {
            self.clear_settings_search(window, cx);
        }
        if self.mode != mode || leaving_settings {
            self.mode = mode;
            self.palette = None;
            self.help_open = false;
            window.focus(&self.pane_focus(cx), cx);
        }
        cx.notify();
    }

    /// The focus of the panes of the current app.
    pub(crate) fn pane_focus(&self, cx: &gpui::App) -> FocusHandle {
        if self.settings_page.open {
            return self.settings_page.focus.clone();
        }
        // The sign-in screen tracks the panes' handle.
        if self.backend.read(cx).source() == Source::SignedOut {
            return self.focus_handle.clone();
        }
        match self.mode {
            Mode::Mail => self.focus_handle.clone(),
            Mode::Calendar => self.calendar.read(cx).focus_handle.clone(),
        }
    }

    fn go_to_mail(&mut self, _: &GoToMail, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Mail, window, cx);
    }

    fn go_to_calendar(&mut self, _: &GoToCalendar, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Calendar, window, cx);
    }

    fn scope(&self) -> Scope {
        match self.mode {
            Mode::Mail => Scope::Mail,
            Mode::Calendar => Scope::Calendar,
        }
    }

    // Go to.

    fn go_to_inbox(&mut self, _: &GoToInbox, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Nav::Folder(Folder::Inbox), window, cx);
    }

    fn go_to_starred(&mut self, _: &GoToStarred, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Nav::Starred, window, cx);
    }

    fn go_to_sent(&mut self, _: &GoToSent, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Nav::Folder(Folder::Sent), window, cx);
    }

    fn go_to_drafts(&mut self, _: &GoToDrafts, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Nav::Folder(Folder::Drafts), window, cx);
    }

    fn go_to_archive(&mut self, _: &GoToArchive, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Nav::Folder(Folder::Archive), window, cx);
    }

    fn go_to_spam(&mut self, _: &GoToSpam, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Nav::Folder(Folder::Spam), window, cx);
    }

    fn go_to_trash(&mut self, _: &GoToTrash, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to(Nav::Folder(Folder::Trash), window, cx);
    }

    /// A Go menu item, from Calendar too.
    fn go_to(&mut self, nav: Nav, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Mail, window, cx);
        self.set_nav(nav, cx);
    }

    /// Gmail's `g l`: the palette, narrowed to labels.
    fn go_to_label(&mut self, _: &GoToLabel, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette("label: ", window, cx);
    }

    /// Select a thread from anywhere, switching to its folder if the current
    /// view doesn't show it.
    fn reveal_thread(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.rows.contains(&Row::Thread(ix)) {
            self.set_nav(Nav::Folder(self.data.threads[ix].folder), cx);
        }
        self.select_thread(ix, window, cx);
    }

    // Overlays: the command palette and the shortcuts sheet. While one is
    // open it holds focus; closing it hands focus back to the panes.

    fn toggle_command_palette(
        &mut self,
        _: &ToggleCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seen_tip = true;
        if self.palette.is_some() {
            self.close_overlays(window, cx);
        } else {
            self.open_palette("", window, cx);
        }
    }

    fn open_palette(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (items, recent) = self.palette_items(window, cx);
        let noun = match self.mode {
            Mode::Mail => "mail",
            Mode::Calendar => "events",
        };
        self.show_palette(
            items,
            query,
            format!("Type a command or search {noun}…"),
            format!("No matching commands or {noun}"),
            window,
            cx,
        );
        // ME-062: the commands used last head the list, under "Recent".
        if let Some((palette, _)) = &self.palette {
            palette.update(cx, |p, cx| p.set_recent(recent, cx));
        }
    }

    /// The palette with `items`: every command, or a list to pick from
    /// (`v`, `l`).
    pub(crate) fn show_palette(
        &mut self,
        items: Vec<PaletteItem>,
        query: &str,
        placeholder: String,
        empty_text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.help_open = false;
        let palette = cx.new(|cx| {
            CommandPalette::new(
                items,
                query,
                placeholder.into(),
                empty_text.into(),
                window,
                cx,
            )
        });
        let events = cx.subscribe_in(&palette, window, Self::on_palette_event);
        window.focus(&palette.focus_handle(cx), cx);
        self.palette = Some((palette, events));
        cx.notify();
    }

    /// Every palette row, and how many of the first are recent commands.
    fn palette_items(&self, window: &Window, cx: &gpui::App) -> (Vec<PaletteItem>, usize) {
        let scope = self.scope();
        // Once, not per command (MC-047).
        let contexts = lookup_contexts(scope, self.reader_focused);
        let binding = |action: &dyn gpui::Action| {
            contexts.iter().find_map(|context| {
                window.highest_precedence_binding_for_action_in_context(action, context.clone())
            })
        };
        let mut available = COMMANDS
            .iter()
            .filter(|c| {
                !c.help_only
                    && (c.scope() == Scope::Global || c.scope() == scope)
                    && self.command_available(c.when, cx)
            })
            .collect::<Vec<_>>();
        // The few used last first; the rest keep their order.
        let recent_rank = |c: &Command| {
            let name = (c.action_name)();
            self.recent_commands
                .iter()
                .take(RECENT_SHOWN)
                .position(|n| n == name)
        };
        available.sort_by_key(|c| recent_rank(c).unwrap_or(usize::MAX));
        let recent = available
            .iter()
            .filter(|c| recent_rank(c).is_some())
            .count();
        let commands = available.into_iter().map(|c| PaletteItem {
            command: PaletteCommand::Action(c.action),
            title: self.command_title(c, window, cx),
            subtitle: None,
            leading: Leading::Icon(c.icon),
            binding: binding(&*(c.action)()),
            search_only: false,
        });
        if self.mode == Mode::Calendar {
            let calendar = self.calendar.read(cx);
            let calendars = calendar
                .data
                .calendars
                .iter()
                .enumerate()
                .map(|(ix, c)| PaletteItem {
                    command: PaletteCommand::ToggleCalendar(ix),
                    title: format!(
                        "{} Calendar: {}",
                        if c.visible { "Hide" } else { "Show" },
                        c.name
                    )
                    .into(),
                    subtitle: None,
                    leading: Leading::Dot(c.color),
                    binding: None,
                    search_only: false,
                });
            let events = self.event_items(true, cx);
            return (commands.chain(calendars).chain(events).collect(), recent);
        }
        let labels = self
            .data
            .labels
            .iter()
            .enumerate()
            .map(|(ix, label)| PaletteItem {
                command: PaletteCommand::Nav(Nav::Label(ix)),
                title: format!("Go to Label: {}", label.name).into(),
                subtitle: None,
                leading: Leading::Dot(label.color),
                binding: None,
                search_only: false,
            });
        let threads = self.data.threads.iter().enumerate().map(|(ix, thread)| {
            let from = thread
                .messages
                .iter()
                .rev()
                .map(|m| &m.from)
                .find(|c| !self.data.is_me(c))
                .unwrap_or(&thread.last_message().from);
            // Same-subject conversations tell apart by date (ME-078).
            let date = crate::data::list_date(&thread.last_message().date);
            PaletteItem {
                command: PaletteCommand::Thread(ix),
                title: thread.subject.clone().into(),
                subtitle: Some(format!("{} · {date}", from.display_name()).into()),
                leading: Leading::Icon(Icon::Mail),
                binding: None,
                search_only: true,
            }
        });
        (commands.chain(labels).chain(threads).collect(), recent)
    }

    /// The calendar's events as palette rows; `search_only` rows show only
    /// once something is typed.
    fn event_items(&self, search_only: bool, cx: &gpui::App) -> Vec<PaletteItem> {
        let calendar = self.calendar.read(cx);
        let today = crate::ui::calendar::today();
        calendar
            .data
            .events
            .iter()
            .enumerate()
            .map(|(ix, event)| PaletteItem {
                command: PaletteCommand::Event(ix),
                title: event.display_title().to_string().into(),
                subtitle: Some(match &event.recurrence {
                    Some(r) => r.label().into(),
                    None => crate::calendar::time::date_label(event.start.date(), today).into(),
                }),
                leading: Leading::Dot(calendar.data.color_of(event)),
                binding: None,
                search_only,
            })
            .collect()
    }

    /// What running `command` will do: "Undo Archive", "Unstar" (ME-055).
    fn command_title(&self, command: &Command, window: &Window, cx: &gpui::App) -> SharedString {
        use gpui::Action as _;
        let name = (command.action)().name();
        if name == Undo.name() || name == EditUndo.name() {
            if let Some(title) = self.undo_stack.undo_title() {
                return format!("Undo {title}").into();
            }
        } else if name == EditRedo.name()
            && let Some(title) = self.undo_stack.redo_title()
        {
            return format!("Redo {title}").into();
        }
        command
            .title(|toggle| self.toggle_on(toggle, window, cx))
            .into()
    }

    fn on_palette_event(
        &mut self,
        _: &Entity<CommandPalette>,
        event: &PaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, PaletteEvent::Dismiss) {
            self.close_overlays(window, cx);
        } else {
            self.close_overlays_at_once(window, cx);
        }
        // W5: quick add (CAL-39).
        if let PaletteEvent::QuickAdd(text) = event {
            self.set_mode(Mode::Calendar, window, cx);
            return self
                .calendar
                .update(cx, |c, cx| c.quick_add(text, window, cx));
        }
        // W4b: a label's name, typed.
        if let PaletteEvent::Text(text) = event {
            return self.label_name_typed(text, window, cx);
        }
        let PaletteEvent::Run(command) = event else {
            return;
        };
        // A Mail or Calendar command run from Settings goes back to its app
        // first; the action waits for the panes to be drawn.
        if self.settings_page.open {
            let global = match *command {
                PaletteCommand::Action(action) => {
                    let name = action().name();
                    COMMANDS
                        .iter()
                        .find(|c| (c.action_name)() == name)
                        .is_none_or(|c| c.scope() == Scope::Global)
                }
                _ => false,
            };
            if !global {
                self.set_mode(self.mode, window, cx);
                if let PaletteCommand::Action(action) = *command {
                    self.remember_command(action().name());
                    cx.on_next_frame(window, move |_, window, cx| {
                        window.dispatch_action(action(), cx)
                    });
                    return;
                }
            }
        }
        match *command {
            // Dispatched from the panes, which now have focus, exactly as if
            // its shortcut had been pressed there.
            PaletteCommand::Action(action) => {
                let action = action();
                self.remember_command(action.name());
                window.dispatch_action(action, cx)
            }
            PaletteCommand::MoveTo(folder) => self.move_to(folder, window, cx),
            PaletteCommand::Label(command) => self.run_label_command(command, window, cx),
            PaletteCommand::ToggleLabel(ix) => self.toggle_label(ix, cx),
            PaletteCommand::Nav(nav) => self.set_nav(nav, cx),
            PaletteCommand::Thread(ix) => self.reveal_thread(ix, window, cx),
            PaletteCommand::ToggleCalendar(ix) => {
                self.calendar.update(cx, |c, cx| c.toggle_calendar(ix, cx))
            }
            PaletteCommand::Event(ix) => self
                .calendar
                .update(cx, |c, cx| c.reveal_event(ix, window, cx)),
        }
    }

    /// Most recent first, saved, at most `RECENT_COMMANDS` (ME-062).
    fn remember_command(&mut self, name: &str) {
        self.recent_commands.retain(|n| n != name);
        self.recent_commands.insert(0, name.to_string());
        self.recent_commands
            .truncate(crate::settings::RECENT_COMMANDS);
    }

    fn toggle_shortcuts_help(
        &mut self,
        _: &ToggleShortcutsHelp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seen_tip = true;
        if self.help_open {
            self.close_overlays(window, cx);
        } else {
            self.palette = None;
            self.help_open = true;
            // Cleared here, not on close, so the sheet fades out as it was.
            self.help_filter.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            cx.notify();
        }
    }

    /// Esc, a click outside or the overlay's own shortcut: it fades out
    /// (MD-007).
    pub(crate) fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let leaving = if let Some((palette, _)) = &self.palette {
            Some(LeavingOverlay::Palette(palette.clone()))
        } else if self.help_open {
            Some(LeavingOverlay::Help)
        } else {
            self.contact_card
                .as_ref()
                .filter(|(card, _)| !card.read(cx).is_anchored())
                .map(|(card, _)| LeavingOverlay::Card(card.clone()))
        };
        self.close_overlays_at_once(window, cx);
        self.leaving_overlay = leaving;
    }

    /// A command ran from the palette: it goes at once, so its result
    /// shows with nothing on top (MD-Q3).
    fn close_overlays_at_once(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.contact_card = None;
        self.help_open = false;
        self.leaving_overlay = None;
        window.focus(&self.pane_focus(cx), cx);
        cx.notify();
    }

    /// The palette, the shortcuts sheet or the contact card over a dimmed
    /// window. One presence spring for all three, so swapping one for
    /// another doesn't play it again (MD-007).
    fn render_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        // MB-069: hung at the click, the card is a popover with no backdrop.
        let anchored = self
            .contact_card
            .as_ref()
            .filter(|(card, _)| card.read(cx).is_anchored())
            .map(|(card, _)| card.clone());
        let open = self.overlay_open() && anchored.is_none();
        let p = ui::motion::presence("overlay", open, window, cx);
        if open || p == 0. {
            self.leaving_overlay = None;
        }
        if let Some(card) = anchored {
            // MD-011: it enters like the other popovers.
            return Some(
                div()
                    .absolute()
                    .child(ui::motion::enter(
                        "contact-card-in",
                        ui::motion::Rise::Down,
                        div().child(card),
                    ))
                    .into_any_element(),
            );
        }
        let (content, help) = if let Some((palette, _)) = &self.palette {
            (div().child(palette.clone()).into_any_element(), false)
        } else if self.help_open {
            (
                self.render_shortcuts_help(window, cx).into_any_element(),
                true,
            )
        } else if let Some((card, _)) = &self.contact_card {
            (div().child(card.clone()).into_any_element(), false)
        } else {
            match self.leaving_overlay.as_ref()? {
                LeavingOverlay::Palette(palette) => {
                    (div().child(palette.clone()).into_any_element(), false)
                }
                LeavingOverlay::Help => (
                    self.render_shortcuts_help(window, cx).into_any_element(),
                    true,
                ),
                LeavingOverlay::Card(card) => (div().child(card.clone()).into_any_element(), false),
            }
        };
        let theme = Theme::of(cx);
        let rise = ui::motion::moving(ui::motion::Rise::Down, cx);
        Some(
            div()
                .id("overlay-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .map(|this| {
                    if help {
                        this.justify_center().p(SPACE_2XL)
                    } else {
                        this.pt(OVERLAY_TOP).px(SPACE_2XL)
                    }
                })
                .bg(theme.backdrop.opacity(p.clamp(0., 1.)))
                .when(open, |this| {
                    this.occlude().on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_overlays(window, cx)),
                    )
                })
                .child(ui::motion::card_motion(
                    div()
                        .relative()
                        .child(content)
                        .when(!open, |this| this.child(ui::motion::inert_cover())),
                    p,
                    rise,
                ))
                .into_any_element(),
        )
    }

    /// MA-072: a selection made without `select_thread` (undo, a view
    /// shown again) gets its bodies too.
    fn load_selected_bodies(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else {
            return;
        };
        if self.ensure_bodies(ix, cx) {
            let id = self.data.threads[ix].id.clone();
            self.backend.update(cx, |b, cx| b.fetch_body(&id, cx));
        }
    }

    /// MB-011: "Inbox — Pigeon", "Calendar — Pigeon", for Mission Control,
    /// the Window menu and VoiceOver.
    fn sync_window_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view: SharedString = if self.backend.read(cx).source() == Source::SignedOut {
            "".into()
        } else if self.settings_page.open {
            "Settings".into()
        } else if self.mode == Mode::Calendar {
            "Calendar".into()
        } else {
            self.nav_title()
        };
        let title: SharedString = if view.is_empty() {
            "Pigeon".into()
        } else {
            format!("{view} — Pigeon").into()
        };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
    }

    /// MA-094: the one first-run tip, in Mail, until ⌘K or ? is used or
    /// it's dismissed. Out of the toast's way, at the bottom right.
    fn render_tip(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        // Not over a draft's or the event panel's buttons, which sit there.
        if self.seen_tip
            || self.toast.is_some()
            || self.overlay_open()
            || self.mode != Mode::Mail
            || self.composer.is_some()
        {
            return None;
        }
        let theme = Theme::of(cx);
        let hint = |id: &'static str, text: &'static str, keys: &str| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .child(text)
                .child(ui::keys_caps(keys, theme))
        };
        Some(
            div()
                .absolute()
                .bottom(SPACE_XL)
                .right(SPACE_XL)
                .child(ui::motion::enter(
                    "tip-enter",
                    ui::motion::Rise::Up,
                    ui::controls::popover_surface(theme)
                        .id("first-run-tip")
                        .occlude()
                        .role(gpui::Role::Note)
                        .aria_label(
                            "Tip: Command K lists every command, question mark shows the shortcuts",
                        )
                        .py(SPACE_XS)
                        .pl(SPACE_LG)
                        .pr(SPACE_XS)
                        .flex()
                        .items_center()
                        .gap(SPACE_LG)
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .child(hint("tip-palette", "Every command", "cmd-k"))
                        .child(hint("tip-shortcuts", "Shortcuts", "?"))
                        .child(
                            ui::icon_button_small("tip-dismiss", Icon::X, "Dismiss Tip", theme)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.seen_tip = true;
                                    cx.notify();
                                })),
                        ),
                ))
                .into_any_element(),
        )
    }

    fn resize_pane(
        &mut self,
        event: &DragMoveEvent<PaneResize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = *event.drag(cx);
        self.resizing = Some(kind);
        self.gliding = None;
        let width = window.viewport_size().width;
        let x = event.event.position.x - RAIL_WIDTH;
        let (min, max) = self.pane_range(kind, width);
        let before = match kind {
            PaneResize::Sidebar => self.sidebar_width,
            PaneResize::List => self.list_width,
        };
        match kind {
            // Dragged well past its minimum, the sidebar hides, keeping its
            // width for next time (MB-075).
            PaneResize::Sidebar if x < SIDEBAR_WIDTH.0 - SIDEBAR_COLLAPSE => {
                self.sidebar_open = false;
                self.sidebar_forced = false;
            }
            PaneResize::Sidebar => self.sidebar_width = x.min(max).max(min),
            PaneResize::List => {
                let sidebar = self.pane_layout(width).sidebar;
                self.list_width = (x - sidebar).min(max).max(min);
            }
        }
        let (after, default) = match kind {
            PaneResize::Sidebar => (self.sidebar_width, SIDEBAR_DEFAULT),
            PaneResize::List => (self.list_width, LIST_DEFAULT),
        };
        // MA-074: a tap as the pane reaches a limit or its default width.
        if crosses_mark(before, after, &[min, max, default]) {
            crate::native::haptic_alignment();
        }
        // The cursor stays a resize cursor off the divider too, pointing
        // the way the pane can still go (MA-093).
        cx.set_active_drag_cursor_style(self.divider_cursor(kind, width), window);
        cx.notify();
    }

    /// How narrow and wide a divider can make its pane: the reader keeps its
    /// minimum, and the list its own beside a sidebar.
    fn pane_range(&self, kind: PaneResize, width: Pixels) -> (Pixels, Pixels) {
        let room = width - RAIL_WIDTH - READER_MIN;
        match kind {
            PaneResize::Sidebar => (SIDEBAR_WIDTH.0, (room - LIST_WIDTH.0).min(SIDEBAR_WIDTH.1)),
            PaneResize::List => (
                LIST_WIDTH.0,
                (room - self.pane_layout(width).sidebar).min(LIST_WIDTH.1),
            ),
        }
    }

    /// ↔, or ← / → at a limit (MA-093). The sidebar at its minimum can
    /// still be dragged left, to hide it.
    fn divider_cursor(&self, kind: PaneResize, width: Pixels) -> gpui::CursorStyle {
        let (min, max) = self.pane_range(kind, width);
        let layout = self.pane_layout(width);
        let shown = match kind {
            PaneResize::Sidebar => layout.sidebar,
            PaneResize::List => layout.list,
        };
        let at_min = kind == PaneResize::List && shown <= min;
        match (at_min, shown >= max) {
            (true, false) => gpui::CursorStyle::ResizeRight,
            (false, true) => gpui::CursorStyle::ResizeLeft,
            _ => gpui::CursorStyle::ResizeLeftRight,
        }
    }

    /// Double-clicking a divider puts its pane back to the default width,
    /// gliding there (MD-015).
    pub fn reset_pane(&mut self, kind: PaneResize, cx: &mut Context<Self>) {
        match kind {
            PaneResize::Sidebar => self.sidebar_width = SIDEBAR_DEFAULT,
            PaneResize::List => self.list_width = LIST_DEFAULT,
        }
        self.gliding = Some(kind);
        cx.notify();
    }

    /// The panes' widths to draw: `layout` at once, or on a spring towards
    /// it after a reset (MD-015).
    fn glide_panes(
        &mut self,
        layout: PaneLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaneLayout {
        let policy = |kind| {
            ui::motion::SPRING_PANE
                .with_epsilon(0.5)
                .with_travel(self.gliding == Some(kind))
        };
        let sidebar = gpui::base::spring(
            "sidebar-width",
            layout.sidebar,
            policy(PaneResize::Sidebar),
            window,
            cx,
        );
        let list = gpui::base::spring(
            "list-width",
            layout.list,
            policy(PaneResize::List),
            window,
            cx,
        );
        let shown = PaneLayout { sidebar, list };
        if shown == layout {
            self.gliding = None;
        }
        shown
    }
}

impl Render for PigeonApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profile::frame_start();
        let root = self.render_root(window, cx);
        profile::render_end();
        root
    }
}

impl PigeonApp {
    fn render_root(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Whatever had focus went away (a closed draft, say): back to the panes.
        if window.focused(cx).is_none() {
            window.focus(&self.pane_focus(cx), cx);
        }
        window.set_rem_size(px(ui::style::BASE_REM * self.text_scale));
        self.load_selected_bodies(cx);
        self.sync_window_title(window, cx);
        self.schedule_reveal(window, cx);
        let theme = Theme::of(cx).clone();
        self.refresh_menus(window, cx);

        let width = window.viewport_size().width;
        if self.sidebar_fits(width) {
            // Wide enough again: hiding by itself works as before next time.
            self.sidebar_forced = false;
        }
        if !cx.has_active_drag() {
            self.resizing = None;
        }
        let layout = self.pane_layout(width);
        self.shown = self.glide_panes(layout, window, cx);
        // The sidebar slides open and shut when the user says so, and
        // follows a window resize at once (MD-013, MD-014). Its content
        // keeps its width and is clipped, so nothing reflows while it moves.
        let target = if layout.sidebar > px(0.) { 1. } else { 0. };
        self.sidebar_progress = gpui::base::spring(
            "sidebar-shown",
            target,
            ui::motion::SPRING_PANE.with_travel(self.sidebar_sliding),
            window,
            cx,
        );
        if self.sidebar_progress == target {
            self.sidebar_sliding = false;
        }
        let sidebar_open = self.sidebar_progress.clamp(0., 1.);
        let sidebar_shown = self.sidebar_content_width() * sidebar_open;
        let settings_open = self.settings_page.open;
        let (signed_out, calendar_granted, sign_in, status) = {
            let b = self.backend.read(cx);
            (
                b.source() == Source::SignedOut,
                b.has_scope(CALENDAR_SCOPE),
                b.sign_in_state().clone(),
                b.status().clone(),
            )
        };
        // One pass over the threads for every count (MC-037).
        self.counts = self.nav_counts();
        let inbox_unread = self.counts.unread(Nav::Folder(Folder::Inbox));
        let panes = !settings_open && !signed_out;
        // MD-042: from the sign-in screen, the app fades in.
        let signed_in = ui::motion::presence("signed-in", !signed_out, window, cx).clamp(0., 1.);
        // A reused pane keeps the opacity it was painted with.
        let cache = signed_in == 1.;
        let entity = cx.entity();
        let reset = move |kind| {
            let entity = entity.clone();
            move |_: &mut Window, cx: &mut gpui::App| {
                entity.update(cx, |this, cx| this.reset_pane(kind, cx))
            }
        };

        // Handlers sit on the root, so actions reach them from the overlays too.
        div()
            .id("pigeon")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::toggle_command_palette))
            // W5: quick add (CAL-39).
            .on_action(cx.listener(|this, _: &NewEventFromText, window, cx| {
                this.open_palette("New event: ", window, cx)
            }))
            .on_action(cx.listener(Self::toggle_shortcuts_help))
            .on_action(cx.listener(Self::open_thread))
            .on_action(cx.listener(Self::back_to_list))
            .on_action(cx.listener(Self::scroll_reader_down))
            .on_action(cx.listener(Self::scroll_reader_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::mark_read))
            .on_action(cx.listener(Self::compose_new))
            .on_action(cx.listener(Self::go_to_inbox))
            .on_action(cx.listener(Self::go_to_starred))
            .on_action(cx.listener(Self::go_to_sent))
            .on_action(cx.listener(Self::go_to_drafts))
            .on_action(cx.listener(Self::go_to_archive))
            .on_action(cx.listener(Self::go_to_spam))
            .on_action(cx.listener(Self::go_to_trash))
            .on_action(cx.listener(Self::go_to_label))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_search))
            .on_action(cx.listener(Self::toggle_archived_section))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::archive))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::toggle_star))
            .on_action(cx.listener(Self::mark_unread))
            .on_action(cx.listener(Self::reply))
            .on_action(cx.listener(Self::reply_all))
            .on_action(cx.listener(Self::forward))
            .on_action(cx.listener(Self::send_message))
            .on_action(cx.listener(Self::close_draft))
            .on_action(cx.listener(Self::discard_draft))
            .on_action(cx.listener(Self::show_cc_bcc))
            .on_action(cx.listener(Self::open_draft_in_new_window))
            // W3.
            .on_action(cx.listener(Self::change_from))
            .on_action(cx.listener(Self::toggle_signature))
            .on_action(cx.listener(|this, action: &Undo, window, cx| {
                this.with_row_cause(RowCause::Undo, |this| this.undo(action, window, cx))
            }))
            .on_action(cx.listener(Self::edit_undo))
            .on_action(cx.listener(Self::edit_redo))
            .on_action(cx.listener(Self::edit_copy))
            .on_action(cx.listener(Self::edit_select_all))
            .on_action(cx.listener(Self::toggle_full_screen))
            .on_action(cx.listener(Self::save_draft_now))
            .on_action(cx.listener(Self::close_search))
            .on_action(cx.listener(Self::focus_results))
            .on_action(cx.listener(Self::next_message))
            .on_action(cx.listener(Self::select_message_text))
            .on_action(cx.listener(Self::prev_message))
            .on_action(cx.listener(Self::go_to_mail))
            .on_action(cx.listener(Self::go_to_calendar))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::focus_next_control))
            .on_action(cx.listener(Self::focus_prev_control))
            .on_action(cx.listener(Self::focus_panes))
            .on_action(cx.listener(Self::about))
            .on_action(cx.listener(Self::toggle_settings))
            .on_action(cx.listener(Self::open_settings_section))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::minimize))
            .on_action(cx.listener(Self::zoom_window))
            .on_action(cx.listener(Self::sync_now))
            .on_action(cx.listener(Self::sign_in))
            .on_action(cx.listener(Self::sign_out))
            .on_action(cx.listener(Self::use_demo_data))
            .on_action(cx.listener(Self::retry_loading_message))
            .on_action(cx.listener(Self::load_images))
            .on_action(cx.listener(Self::load_older))
            .on_action(cx.listener(Self::mark_all_read))
            .map(|this| self.on_mail_actions(this, cx))
            .map(|this| self.on_label_actions(this, cx))
            .map(|this| self.on_attachment_actions(this, cx))
            .map(|this| self.on_contact_actions(this, cx))
            .on_drag_move(cx.listener(Self::resize_pane))
            .relative()
            .size_full()
            .overflow_hidden()
            .line_height(LINE_UI)
            .text_color(theme.text)
            .when(settings_open, |this| {
                this.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_row()
                        .child(ui::rail::render(None, inbox_unread, &theme, window, cx))
                        .child(self.render_settings(window, cx)),
                )
            })
            .when(!settings_open && signed_out, |this| {
                this.child(self.render_onboarding(&sign_in, &self.focus_handle, cx))
            })
            .when(panes && self.mode == Mode::Calendar, |this| {
                this.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_row()
                        .opacity(signed_in)
                        .child(ui::rail::render_with_status(
                            Some(self.mode),
                            inbox_unread,
                            Some(&status),
                            &theme,
                            window,
                            cx,
                        ))
                        .map(|this| {
                            if calendar_granted {
                                this.child(self.calendar.clone())
                            } else {
                                this.child(ui::status::missing_scope(
                                    "Calendar",
                                    Icon::Calendar,
                                    &theme,
                                ))
                            }
                        }),
                )
            })
            .when(panes && self.mode == Mode::Mail, |this| {
                this.child(
                    div()
                        .key_context(self.mailbox_key_context())
                        .track_focus(&self.focus_handle)
                        .relative()
                        .size_full()
                        .flex()
                        .flex_row()
                        .opacity(signed_in)
                        // The sidebar has the status line; hidden, the rail
                        // shows it (MA-068).
                        .child(ui::rail::render_with_status(
                            Some(self.mode),
                            inbox_unread,
                            (layout.sidebar == px(0.)).then_some(&status),
                            &theme,
                            window,
                            cx,
                        ))
                        .when(sidebar_open > 0., |this| {
                            this.child(
                                div()
                                    .w(sidebar_shown)
                                    .h_full()
                                    .flex_none()
                                    .overflow_hidden()
                                    .child(panes::embed(
                                        &self.panes.sidebar,
                                        div().w(self.sidebar_content_width()).h_full().flex_none(),
                                        cache,
                                        window,
                                    )),
                            )
                        })
                        .child(panes::embed(
                            &self.panes.list,
                            div().w(self.shown.list).h_full().flex_none(),
                            cache,
                            window,
                        ))
                        .child(panes::embed(
                            &self.panes.reader,
                            div().flex_1().min_w_0().h_full(),
                            cache,
                            window,
                        ))
                        .when(layout.sidebar > px(0.), |this| {
                            this.child(ui::pane_divider(
                                PaneResize::Sidebar,
                                RAIL_WIDTH + sidebar_shown,
                                self.resizing == Some(PaneResize::Sidebar),
                                self.divider_cursor(PaneResize::Sidebar, width),
                                &theme,
                                reset(PaneResize::Sidebar),
                            ))
                        })
                        .child(ui::pane_divider(
                            PaneResize::List,
                            RAIL_WIDTH + sidebar_shown + self.shown.list,
                            self.resizing == Some(PaneResize::List),
                            self.divider_cursor(PaneResize::List, width),
                            &theme,
                            reset(PaneResize::List),
                        )),
                )
            })
            .child(self.panes.layer.clone())
            .when(profile::active(), |this| this.child(profile::marker()))
    }

    /// The sidebar's content width: as shown, or as it will show while it
    /// slides in.
    fn sidebar_content_width(&self) -> Pixels {
        if self.shown.sidebar > px(0.) {
            self.shown.sidebar
        } else {
            self.sidebar_width
        }
    }

    // The panes (`app/panes.rs`), each drawn as a view of its own.

    fn sidebar_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.sidebar_content_width();
        self.render_sidebar(width, &self.counts, window, cx)
            .into_any_element()
    }

    fn list_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.step_rows(window, cx);
        div()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.reader_focused = false;
                    if !this.search_input.focus_handle(cx).is_focused(window) {
                        window.focus(&this.focus_handle, cx);
                    }
                    cx.notify();
                }),
            )
            .child(self.render_thread_list(
                self.shown.list,
                self.sidebar_progress,
                &self.counts,
                window,
                cx,
            ))
            .into_any_element()
    }

    fn reader_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.reader_focused = this.selected.is_some();
                    cx.notify();
                }),
            )
            .child({
                self.step_reader_glide(window);
                self.reply_card_presence(window, cx);
                self.render_reader(window, cx)
            })
            .into_any_element()
    }

    /// Toast, first-run tip and overlays, over everything. Not cached: it
    /// draws whenever the app does, and its springs redraw it alone.
    fn layer_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let panes = !self.settings_page.open && self.backend.read(cx).source() != Source::SignedOut;
        let toast = self.render_toast(window, cx);
        let tip = if panes { self.render_tip(cx) } else { None };
        let overlay = self.render_overlay(window, cx);
        div()
            .absolute()
            .inset_0()
            .children(toast)
            .children(tip)
            .children(overlay)
            .into_any_element()
    }
}

/// Whether a text field has focus, so the Edit menu acts on its text.
fn text_field_focused(window: &Window) -> bool {
    window.context_stack().iter().any(|c| c.contains("Input"))
}

/// Whether a list row overlaps the viewport. `bounds_for_item` also gives
/// bounds for measured rows scrolled off below, so it alone can't tell.
fn on_screen(row: Option<Bounds<Pixels>>, viewport: Bounds<Pixels>) -> bool {
    row.is_some_and(|row| row.top() < viewport.bottom() && row.bottom() > viewport.top())
}

/// Whether a drag from `before` to `after` reached or passed one of
/// `marks`, having started off it.
fn crosses_mark(before: Pixels, after: Pixels, marks: &[Pixels]) -> bool {
    marks
        .iter()
        .any(|&m| (before < m && after >= m) || (before > m && after <= m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    #[test]
    fn divider_taps_once_on_reaching_a_mark() {
        let marks = [px(180.), px(236.), px(360.)];
        assert!(crosses_mark(px(200.), px(236.), &marks));
        assert!(crosses_mark(px(230.), px(240.), &marks));
        assert!(crosses_mark(px(350.), px(360.), &marks));
        // Held at a limit, or leaving it.
        assert!(!crosses_mark(px(360.), px(360.), &marks));
        assert!(!crosses_mark(px(360.), px(350.), &marks));
        assert!(!crosses_mark(px(200.), px(210.), &marks));
    }

    fn rect(top: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(top)), size(px(100.), px(height)))
    }

    #[test]
    fn on_screen_only_when_overlapping_viewport() {
        let viewport = rect(0., 500.);
        assert!(on_screen(Some(rect(100., 40.)), viewport));
        assert!(on_screen(Some(rect(480., 40.)), viewport));
        // Measured earlier, now scrolled off below or above.
        assert!(!on_screen(Some(rect(900., 40.)), viewport));
        assert!(!on_screen(Some(rect(500., 40.)), viewport));
        assert!(!on_screen(Some(rect(-40., 40.)), viewport));
        assert!(!on_screen(None, viewport));
    }
}

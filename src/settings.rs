//! The settings page's table: every setting with the words it is found by,
//! and the search over them. The values live where the app already keeps
//! them (`PigeonApp`, the calendar, `Settings`); `ui/settings.rs` reads and
//! changes them there, so shortcuts and menus stay in step. `Saved` is all
//! of them at once, as the store keeps them (PER-3).

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::{
    actions::{COMMANDS, Group},
    assets::Icon,
};

/// Preferences that have no other home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub appearance: Appearance,
    /// After the selected conversation is archived, deleted or moved, open
    /// the next one; otherwise leave nothing selected.
    pub auto_advance: bool,
    /// Where a new search looks: all mail, or the current view.
    pub search_all_mail: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            auto_advance: true,
            search_all_mail: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    /// What windows are set to; none follows the system (MC-085).
    pub fn window_appearance(self) -> Option<gpui::WindowAppearance> {
        match self {
            Appearance::System => None,
            Appearance::Light => Some(gpui::WindowAppearance::Light),
            Appearance::Dark => Some(gpui::WindowAppearance::Dark),
        }
    }
}

/// What the Dock icon's badge counts (MB-055).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockBadge {
    Off,
    /// Unread conversations in Primary.
    #[default]
    Primary,
    /// Unread conversations in every Inbox category.
    AllInbox,
}

impl DockBadge {
    pub const ALL: [DockBadge; 3] = [DockBadge::Off, DockBadge::Primary, DockBadge::AllInbox];
}

/// The clock times show on, in mail and calendar alike. `System` follows
/// macOS (System Settings › General › Date & Time).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeFormat {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "12h")]
    Twelve,
    #[serde(rename = "24h")]
    TwentyFour,
}

impl TimeFormat {
    pub const ALL: [TimeFormat; 3] = [
        TimeFormat::System,
        TimeFormat::Twelve,
        TimeFormat::TwentyFour,
    ];

    /// With the system's choice for `System`.
    pub fn is_24_hour(self, system: impl FnOnce() -> bool) -> bool {
        match self {
            TimeFormat::System => system(),
            TimeFormat::Twelve => false,
            TimeFormat::TwentyFour => true,
        }
    }
}

/// Whether messages load their remote images (ATT-1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteImages {
    #[default]
    Ask,
    Always,
    Never,
}

/// Every setting the app keeps across launches, wherever it lives while
/// running. Saved to the store's `settings` table one key per field, so a
/// key added later just takes its default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub appearance: Appearance,
    // `reduce_motion`, saved before MB-072, is ignored: Pigeon follows the
    // system's Reduce Motion.
    pub auto_advance: bool,
    pub search_all_mail: bool,
    pub compact_list: bool,
    pub text_scale: f32,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub list_width: f32,
    pub archived_expanded: bool,
    /// "day", "week" or "month".
    pub calendar_view: CalendarViewSetting,
    pub week_numbers: bool,
    pub remote_images: RemoteImages,
    /// Sprint 2 W1: notify about new mail (NTF-1).
    pub notifications: bool,
    // W5.
    /// The day the calendar showed last, where it opens again (CAL-23).
    pub calendar_date: Option<chrono::NaiveDate>,
    /// 12- or 24-hour times, app-wide. Replaces `time_24h` (`from_map`).
    pub time_format: TimeFormat,
    /// A new event's length in minutes; `None` follows Google Calendar.
    pub event_minutes: Option<u32>,
    /// The id of the calendar new events go to; `None` is automatic.
    pub default_calendar: Option<String>,
    /// W7b: the menu bar item, its panel and quick join (NTF-4, NTF-5).
    pub menu_bar: MenuBarPrefs,
    /// W3 (MST-2): new drafts carry the From address's Gmail signature.
    pub signatures: bool,
    /// How solid the panes are, 0–100 (MA-010): 0, the default, is the
    /// theme's own glass tints; 100 is fully opaque, with an opaque window
    /// behind. Reduce Transparency forces 100. A "Transparency" slider shows
    /// `100 - glass`.
    pub glass: u8,
    /// Which ⌘ shortcuts the keymap uses (MA-038).
    pub keymap: KeymapPreset,
    /// Labels whose sublabels are folded away, by id (FOL-9, MC-048).
    pub collapsed_labels: Vec<String>,
    /// The main window's frame and display as last left (MA-034).
    pub window: Option<WindowFrame>,
    /// Mail or Calendar, and the mail view, as last left (MA-034).
    pub calendar_open: bool,
    pub nav: SavedNav,
    /// The ⌘K and ? tip was used or dismissed (MA-094).
    pub seen_tip: bool,
    /// Palette commands by last use, most recent first, by action name
    /// (ME-062); at most `RECENT_COMMANDS`.
    pub recent_commands: Vec<String>,
    /// The alert sound with single new-mail notices and event reminders
    /// (MA-073).
    pub notification_sound: bool,
    /// Post event reminders (MB-052); off still marks them fired.
    pub event_reminders: bool,
    /// What the Dock badge counts (MB-055).
    pub dock_badge: DockBadge,
    /// SYS-4: the MCP server and what agents may do.
    pub mcp: crate::mcp::McpPrefs,
    /// AI-1: Settings › AI.
    pub ai: crate::ai::AiPrefs,
}

/// How many recent palette commands are kept (ME-062).
pub const RECENT_COMMANDS: usize = 20;

/// The main window's frame (MA-034), relative to its display, which is
/// known by its uuid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub full_screen: bool,
    pub display: Option<String>,
}

/// The mail view to open on (MA-034); a label by id, so it survives
/// label changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedNav {
    Folder(crate::data::Folder),
    Starred,
    Label(String),
    Category(crate::backend::categories::Category),
    Snoozed,
    Scheduled,
    /// AI-4; the Inbox while Needs Reply is off.
    NeedsReply,
}

impl Default for SavedNav {
    fn default() -> Self {
        SavedNav::Folder(crate::data::Folder::Inbox)
    }
}

/// The keyboard shortcuts preset (MA-038, MA-089). Single-key shortcuts are
/// the same in both; only ⌘ shortcuts differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeymapPreset {
    #[default]
    Gmail,
    AppleMail,
}

impl KeymapPreset {
    pub const ALL: [KeymapPreset; 2] = [KeymapPreset::Gmail, KeymapPreset::AppleMail];

    pub fn label(self) -> &'static str {
        match self {
            KeymapPreset::Gmail => "Gmail",
            KeymapPreset::AppleMail => "Apple Mail",
        }
    }
}

/// W7b: Settings › Menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenuBarPrefs {
    pub icon: bool,
    /// The next meeting's title and countdown beside the icon.
    pub next_meeting: bool,
    pub panel_key: HotKey,
    pub join_key: HotKey,
}

impl Default for MenuBarPrefs {
    fn default() -> Self {
        Self {
            icon: true,
            next_meeting: true,
            panel_key: HotKey::CtrlOptCmdSpace,
            join_key: HotKey::CtrlOptCmdJ,
        }
    }
}

/// The global shortcuts on offer: a few that macOS doesn't use itself
/// (⌃⌥Space switches input sources, ⌥⌘Space opens a Finder search).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotKey {
    CtrlOptCmdSpace,
    CtrlOptCmdP,
    CtrlOptCmdJ,
    CtrlOptCmdM,
    Off,
}

impl HotKey {
    /// The panel's choices, in Settings order.
    pub const PANEL: [HotKey; 3] = [HotKey::CtrlOptCmdSpace, HotKey::CtrlOptCmdP, HotKey::Off];
    /// Quick join's.
    pub const JOIN: [HotKey; 3] = [HotKey::CtrlOptCmdJ, HotKey::CtrlOptCmdM, HotKey::Off];

    pub fn label(self) -> &'static str {
        match self {
            HotKey::CtrlOptCmdSpace => "⌃⌥⌘Space",
            HotKey::CtrlOptCmdP => "⌃⌥⌘P",
            HotKey::CtrlOptCmdJ => "⌃⌥⌘J",
            HotKey::CtrlOptCmdM => "⌃⌥⌘M",
            HotKey::Off => "Off",
        }
    }

    /// The macOS virtual key code, with ⌃⌥⌘; none when off.
    pub fn key_code(self) -> Option<u32> {
        match self {
            HotKey::CtrlOptCmdSpace => Some(49),
            HotKey::CtrlOptCmdP => Some(35),
            HotKey::CtrlOptCmdJ => Some(38),
            HotKey::CtrlOptCmdM => Some(46),
            HotKey::Off => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalendarViewSetting {
    Day,
    #[default]
    Week,
    Month,
}

impl Default for Saved {
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            appearance: settings.appearance,
            auto_advance: settings.auto_advance,
            search_all_mail: settings.search_all_mail,
            compact_list: false,
            text_scale: 1.,
            sidebar_open: true,
            sidebar_width: f32::from(crate::app::SIDEBAR_DEFAULT),
            list_width: f32::from(crate::app::LIST_DEFAULT),
            archived_expanded: false,
            calendar_view: CalendarViewSetting::Week,
            week_numbers: false,
            remote_images: RemoteImages::Ask,
            notifications: true,
            calendar_date: None,
            time_format: TimeFormat::System,
            event_minutes: None,
            default_calendar: None,
            menu_bar: MenuBarPrefs::default(),
            signatures: true,
            glass: 0,
            keymap: KeymapPreset::Gmail,
            collapsed_labels: Vec::new(),
            window: None,
            calendar_open: false,
            nav: SavedNav::default(),
            seen_tip: false,
            recent_commands: Vec::new(),
            notification_sound: true,
            event_reminders: true,
            dock_badge: DockBadge::Primary,
            mcp: crate::mcp::McpPrefs::default(),
            ai: crate::ai::AiPrefs::default(),
        }
    }
}

impl Saved {
    /// From the store's rows; unknown or broken keys take their defaults.
    pub fn from_map(mut map: serde_json::Map<String, serde_json::Value>) -> Self {
        // Before `time_format`: a calendar switch, `None` following Google.
        if let Some(old) = map.remove("time_24h")
            && !map.contains_key("time_format")
        {
            let format = match old.as_bool() {
                Some(true) => TimeFormat::TwentyFour,
                Some(false) => TimeFormat::Twelve,
                None => TimeFormat::System,
            };
            map.insert(
                "time_format".into(),
                serde_json::to_value(format).expect("serializes"),
            );
        }
        let mut saved = serde_json::to_value(Saved::default()).expect("settings serialize");
        if let Some(fields) = saved.as_object_mut() {
            for (key, value) in map {
                // One bad value mustn't lose the others.
                if let Some(field) = fields.get_mut(&key) {
                    let old = std::mem::replace(field, value);
                    let fits =
                        serde_json::from_value::<Saved>(serde_json::Value::Object(fields.clone()))
                            .is_ok();
                    if !fits {
                        fields.insert(key, old);
                    }
                }
            }
        }
        serde_json::from_value(saved).unwrap_or_default()
    }

    pub fn to_map(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        }
    }
}

/// Sprint 2 W1 (NTF-3): the settings belong to the app, not the window, so
/// a window opened after the last one closed starts from them. The window
/// hands every change here (`remember`) as it saves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppSettings(pub Saved);

impl gpui::Global for AppSettings {}

/// The settings as last saved, wherever they were changed.
pub fn current(cx: &gpui::App) -> Saved {
    get(cx).clone()
}

/// The same, borrowed, to read a field without cloning them all (MC-046).
pub fn get(cx: &gpui::App) -> &Saved {
    static DEFAULT: std::sync::LazyLock<Saved> = std::sync::LazyLock::new(Saved::default);
    cx.try_global::<AppSettings>()
        .map(|s| &s.0)
        .unwrap_or(&DEFAULT)
}

pub fn remember(saved: &Saved, cx: &mut gpui::App) {
    cx.set_global(AppSettings(saved.clone()));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Appearance,
    Mail,
    Calendar,
    Notifications,
    MenuBar,
    Ai,
    Mcp,
    Accounts,
    Shortcuts,
}

impl Section {
    /// General first (ME-067).
    pub const ALL: [Section; 10] = [
        Section::General,
        Section::Appearance,
        Section::Mail,
        Section::Calendar,
        Section::Notifications,
        Section::MenuBar,
        Section::Ai,
        Section::Mcp,
        Section::Accounts,
        Section::Shortcuts,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Mail => "Mail",
            Section::Calendar => "Calendar",
            Section::Notifications => "Notifications",
            Section::MenuBar => "Menu Bar",
            Section::Ai => "AI",
            Section::Mcp => "MCP",
            Section::General => "General",
            Section::Accounts => "Accounts",
            Section::Shortcuts => "Keyboard Shortcuts",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Section::Appearance => Icon::SunMoon,
            Section::Mail => Icon::Mail,
            Section::Calendar => Icon::Calendar,
            Section::Notifications => Icon::Bell,
            Section::MenuBar => Icon::PanelTop,
            Section::Ai => Icon::MessagesSquare,
            Section::Mcp => Icon::Code,
            Section::General => Icon::Settings,
            Section::Accounts => Icon::User,
            Section::Shortcuts => Icon::Keyboard,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Section::Appearance => &["theme", "look", "display"],
            Section::Mail => &["inbox", "email", "list"],
            Section::Calendar => &["events", "schedule"],
            Section::Notifications => &["alerts", "banners", "badge", "notify"],
            Section::MenuBar => &["status bar", "status item", "tray", "menubar", "panel"],
            Section::Ai => &[
                "ai",
                "llm",
                "model",
                "assistant",
                "summary",
                "summaries",
                "jev",
            ],
            Section::Mcp => &["agents", "model context protocol", "claude", "integrations"],
            Section::General => &["startup", "launch", "app"],
            Section::Accounts => &["profile", "user", "sign in", "login"],
            Section::Shortcuts => &["keys", "keymap", "hotkeys", "keybindings", "shortcuts"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingId {
    Appearance,
    TextSize,
    CompactList,
    Sidebar,
    ArchivedInInbox,
    AutoAdvance,
    SearchScope,
    CalendarView,
    WeekNumbers,
    RemoteImages,
    TimeFormat,
    EventLength,
    DefaultCalendar,
    TimeZone,
    Account,
    Access,
    LastSync,
    SyncNow,
    SignOut,
    // Sprint 2 W1.
    Notifications,
    TestNotification,
    OpenAtLogin,
    LoginItemStatus,
    // Sprint 2 W7b.
    MenuBarIcon,
    MenuBarNextMeeting,
    MenuBarPanelKey,
    MenuBarJoinKey,
    MenuBarShortcutStatus,
    // Sprint 2 W3.
    Signatures,
    DefaultMailApp,
    // Polish W1.
    Transparency,
    ShortcutPreset,
    // Polish W3.
    NotificationSound,
    EventReminders,
    DockBadge,
    NotificationPermission,
    // SYS-4.
    McpServer,
    McpAddress,
    McpToken,
    McpReadMail,
    McpOrganizeMail,
    McpWriteDrafts,
    McpSendMail,
    McpReadCalendar,
    McpEditCalendar,
    McpConfig,
    McpActivity,
    // AI-1.
    Ai,
    AiWriting,
    AiJev,
    AiListSummaries,
    AiCleanView,
    AiNeedsReply,
    AiNotifyFor,
    AiActivity,
}

pub struct SettingMeta {
    pub id: SettingId,
    pub section: Section,
    pub label: &'static str,
    pub description: &'static str,
    /// Other words it's found by, lowercase.
    keywords: &'static [&'static str],
}

macro_rules! settings {
    ($($section:ident: $id:ident, $label:literal, $description:literal, [$($kw:literal),*];)*) => {
        &[$(SettingMeta {
            id: SettingId::$id,
            section: Section::$section,
            label: $label,
            description: $description,
            keywords: &[$($kw),*],
        }),*]
    };
}

/// Every setting, in page order.
pub const SETTINGS: &[SettingMeta] = settings! {
    Appearance: Appearance, "Appearance", "Follow the system, or always use light or dark.",
        ["theme", "dark", "light", "mode", "color", "colour", "night"];
    Appearance: Transparency, "Transparency", "How much the desktop shows through the window. Reduce Transparency or Increase Contrast in System Settings makes it opaque.",
        ["glass", "opaque", "opacity", "blur", "translucent", "see-through", "accessibility", "solid"];
    Appearance: TextSize, "Text size", "Make all text bigger or smaller. Also ⌘= and ⌘-.",
        ["font", "zoom", "scale", "bigger", "smaller", "larger", "accessibility"];
    Mail: CompactList, "Compact list", "One line per conversation in the list.",
        ["density", "dense", "comfortable", "condensed", "rows", "spacing"];
    Mail: Sidebar, "Show sidebar", "Mailboxes and labels beside the list.",
        ["folders", "labels", "panel", "hide"];
    Mail: ArchivedInInbox, "Expand Archived in Inbox", "Show archived conversations at the end of the Inbox.",
        ["archive", "collapse", "section"];
    Mail: AutoAdvance, "Auto-advance", "After Archive, Delete or Move, open the next conversation. Off clears the selection.",
        ["next", "advance", "after"];
    Mail: SearchScope, "Search scope", "Where a new search looks. A search with in: overrides it.",
        ["find", "default", "all mail", "everywhere", "current view"];
    Mail: RemoteImages, "Load remote images", "Ask shows a Load Images bar on messages with images from the web.",
        ["pictures", "images", "privacy", "tracking", "pixels", "remote content"];
    Calendar: CalendarView, "View", "Day, week or month. Also D, W and M in the calendar.",
        ["day", "week", "month", "default", "layout"];
    Calendar: WeekNumbers, "Week numbers", "Show the week of the year beside each week.",
        ["iso", "week of year"];
    Calendar: EventLength, "Default event length", "How long a new event is. Follows Google Calendar until changed here.",
        ["duration", "new event", "minutes", "meeting length"];
    Calendar: DefaultCalendar, "Default calendar", "Where new events go. Automatic uses your primary calendar. A calendar you can't add events to isn't listed.",
        ["new event", "which calendar", "primary", "add events", "create", "default"];
    Calendar: TimeZone, "Time zone", "Times show in this Mac's time zone.",
        ["timezone", "zone", "utc", "gmt", "travel"];
    Accounts: Account, "Account", "Who this mail and calendar are for.",
        ["email", "address", "name", "signed in", "status"];
    Accounts: Access, "Access", "What Google allowed Pigeon to use.",
        ["scopes", "permissions", "granted", "consent"];
    Accounts: LastSync, "Last sync", "When Pigeon last checked Google for changes.",
        ["synced", "updated", "offline", "status"];
    Accounts: SyncNow, "Sync now", "Check Gmail and Google Calendar for changes.",
        ["refresh", "reload", "fetch", "update"];
    Accounts: SignOut, "Sign out", "Removes this account and its cached mail and calendar from this Mac.",
        ["log out", "logout", "sign off", "remove account", "disconnect"];
    Notifications: Notifications, "New mail notifications", "A notification for each new message in the Inbox; more than three at once become one.",
        ["alerts", "banners", "notify", "new mail"];
    Notifications: NotificationPermission, "macOS permission", "Whether macOS lets Pigeon show notifications.",
        ["permission", "allow", "system settings", "blocked", "denied", "off"];
    Notifications: NotificationSound, "Play sound", "The alert sound with a new message and an event reminder. Several messages at once arrive quietly.",
        ["sound", "audio", "chime", "alert", "mute", "silent", "quiet"];
    Notifications: EventReminders, "Event reminders", "A notification when an event's reminder is due.",
        ["reminder", "alerts", "calendar", "meeting", "event", "notify"];
    Notifications: DockBadge, "Dock badge", "The unread count on Pigeon's Dock icon: Primary, or every Inbox category.",
        ["badge", "dock", "count", "unread", "icon", "number"];
    Notifications: TestNotification, "Send test notification", "Shows one now. The first time, macOS asks whether Pigeon may notify you.",
        ["try", "permission", "allow", "test"];
    General: TimeFormat, "Time format", "1 PM or 13:00, in mail and calendar. System follows macOS's Date & Time settings.",
        ["24-hour", "24 hour", "12-hour", "12 hour", "clock", "am", "pm", "military", "time"];
    General: OpenAtLogin, "Open at login", "Start Pigeon when you log in to this Mac.",
        ["startup", "launch", "login item", "start", "boot"];
    General: LoginItemStatus, "Login item", "What macOS says about Pigeon's login item.",
        ["startup", "approval", "system settings", "status"];
    MenuBar: MenuBarIcon, "Show menu bar icon", "Pigeon's bird in the menu bar, with a dot for new mail in Primary. Click it for today and recent mail.",
        ["status item", "icon", "tray", "bird", "dot"];
    MenuBar: MenuBarNextMeeting, "Show next meeting in menu bar", "The next meeting's title and countdown beside the icon, from three hours before.",
        ["countdown", "meeting", "title", "join", "event"];
    MenuBar: MenuBarPanelKey, "Open the menu bar panel", "A shortcut that works in any app.",
        ["global", "hotkey", "shortcut", "panel"];
    MenuBar: MenuBarJoinKey, "Join next meeting", "Opens the video call of the meeting under way or starting within 10 minutes, from any app.",
        ["quick join", "global", "hotkey", "shortcut", "video", "meet", "zoom"];
    MenuBar: MenuBarShortcutStatus, "Shortcuts", "Whether macOS gave Pigeon the shortcuts.",
        ["conflict", "taken", "registered", "status", "hotkey"];
    Mail: Signatures, "Signatures", "New messages get the From address's signature from Gmail's settings, where it's edited. Also ⌘⇧E in a draft.",
        ["signature", "sign off", "footer", "gmail", "alias"];
    General: DefaultMailApp, "Default mail app", "Open email links from other apps in Pigeon.",
        ["mailto", "default", "email app", "mail client", "links", "handler"];
    Shortcuts: ShortcutPreset, "Shortcut preset", "Gmail's or Apple Mail's ⌘ shortcuts. Single-key shortcuts are the same in both.",
        ["apple mail", "gmail", "preset", "keymap", "command", "bindings"];
    Mcp: McpServer, "MCP server", "Lets AI agents on this Mac use your mail and calendar while Pigeon is open, as far as the switches below allow.",
        ["agent", "claude", "codex", "cursor", "model context protocol", "server", "automation"];
    Mcp: McpAddress, "Server address", "The URL agents connect to. Only apps on this Mac can reach it.",
        ["url", "port", "localhost", "endpoint", "http"];
    Mcp: McpToken, "Access token", "Agents send it with every request. A new one disconnects them until they have it.",
        ["bearer", "key", "password", "secret", "auth", "authorization", "regenerate"];
    Mcp: McpReadMail, "Read mail", "Search conversations, read them and list labels.",
        ["search", "read", "email", "inbox", "labels"];
    Mcp: McpOrganizeMail, "Organize mail", "Archive, trash, star, mark read or unread, and add or remove labels.",
        ["archive", "trash", "delete", "star", "label", "unread"];
    Mcp: McpWriteDrafts, "Write drafts", "Save new messages and replies to Drafts.",
        ["draft", "compose", "write", "reply"];
    Mcp: McpSendMail, "Send mail", "Send messages and drafts, with no confirmation.",
        ["send", "email", "reply", "compose"];
    Mcp: McpReadCalendar, "Read calendar", "List events and read them.",
        ["events", "schedule", "agenda", "meetings"];
    Mcp: McpEditCalendar, "Edit calendar", "Add, change and delete events, and answer invitations, with no confirmation.",
        ["events", "create", "delete", "invite", "rsvp", "meeting"];
    Mcp: McpConfig, "Add to your agent", "Paste this into your agent's MCP settings. Copy includes the token.",
        ["config", "json", "setup", "connect", "install", "claude", "cursor", "codex"];
    Mcp: McpActivity, "Activity", "The last 50 tool calls since Pigeon opened.",
        ["log", "history", "calls", "audit", "requests"];
    Ai: Ai, "AI", "Summaries, Needs Reply, Clean View, Ask Pigeon and more, from the models set in .env. Nothing is sent while it's off.",
        ["ai", "llm", "consent", "privacy", "turn on", "enable", "assistant"];
    Ai: AiWriting, "Writing model", "Writes summaries, replies and answers. Set in .env; a change there takes effect on restart.",
        ["llm", "openai", "endpoint", "host", "server", "model", "test connection", "key"];
    Ai: AiJev, "Jev", "Makes quick yes-or-no and pick-one decisions, through OpenRouter. Set in .env; a change there takes effect on restart.",
        ["openrouter", "typesafe", "classifier", "endpoint", "host", "model", "test connection", "key"];
    Ai: AiListSummaries, "List summaries", "One line on what each Inbox conversation is about now, in place of its snippet.",
        ["summary", "snippet", "line", "inbox", "list"];
    Ai: AiCleanView, "Clean View for newsletters", "Newsletters open as plain, readable text. ⇧V shows the original.",
        ["newsletter", "reader", "reading view", "clean", "original", "simplify"];
    Ai: AiNeedsReply, "Needs Reply", "Marks Inbox conversations that wait for your answer, with quick replies to start from.",
        ["reply", "answer", "follow up", "marker", "quick reply", "todo"];
    Ai: AiNotifyFor, "Notify for", "Important Only leaves out newsletters and new mail Jev finds unimportant. The Dock still counts all unread mail.",
        ["notifications", "important", "filter", "alerts", "quiet"];
    Ai: AiActivity, "Activity", "The last 100 AI calls since Pigeon opened. Never the text sent.",
        ["log", "history", "calls", "audit", "requests", "tokens"];
};

pub fn meta(id: SettingId) -> &'static SettingMeta {
    SETTINGS
        .iter()
        .find(|m| m.id == id)
        .expect("every setting is in the table")
}

/// The order of the shortcut groups on the page.
const SHORTCUT_GROUPS: [Group; 10] = [
    Group::App,
    Group::Navigation,
    Group::GoTo,
    Group::Conversation,
    Group::Labels,
    Group::Selection,
    Group::Compose,
    Group::CalendarNavigation,
    Group::CalendarView,
    Group::CalendarEvents,
];

/// A group's title with its app, since both apps have a "Navigation".
pub fn group_label(group: Group) -> &'static str {
    match group {
        Group::App => "Application",
        Group::Navigation => "Mail · Navigation",
        Group::GoTo => "Mail · Go to",
        Group::Conversation => "Mail · Conversation",
        Group::Selection => "Mail · Selection",
        Group::Compose => "Mail · Compose",
        Group::Labels => "Mail · Labels",
        Group::CalendarNavigation => "Calendar · Navigation",
        Group::CalendarView => "Calendar · View",
        Group::CalendarEvents => "Calendar · Events",
    }
}

/// A row of the page, or a search result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A whole section, found by its name.
    Section(Section),
    Setting(SettingId),
    /// Index into `COMMANDS`: a read-only shortcut.
    Shortcut(usize),
}

impl Entry {
    pub fn section(self) -> Section {
        match self {
            Entry::Section(section) => section,
            Entry::Setting(id) => meta(id).section,
            Entry::Shortcut(_) => Section::Shortcuts,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Entry::Section(section) => section.title(),
            Entry::Setting(id) => meta(id).label,
            Entry::Shortcut(ix) => COMMANDS[ix].name,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Entry::Section(_) => "",
            Entry::Setting(id) => meta(id).description,
            Entry::Shortcut(ix) => group_label(COMMANDS[ix].group),
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Entry::Section(section) => section.keywords(),
            Entry::Setting(id) => meta(id).keywords,
            Entry::Shortcut(_) => &[],
        }
    }
}

/// The rows of a section, in order: its settings, then for Shortcuts
/// every command.
pub fn entries(section: Section) -> Vec<Entry> {
    let settings = SETTINGS
        .iter()
        .filter(|m| m.section == section)
        .map(|m| Entry::Setting(m.id));
    if section != Section::Shortcuts {
        return settings.collect();
    }
    settings
        .chain(SHORTCUT_GROUPS.iter().flat_map(|&group| {
            COMMANDS
                .iter()
                .enumerate()
                .filter(move |(_, c)| c.group == group)
                .map(|(ix, _)| Entry::Shortcut(ix))
        }))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub entry: Entry,
    /// Byte ranges of the query's words in the label and the description,
    /// sorted and apart.
    pub label: Vec<Range<usize>>,
    pub description: Vec<Range<usize>>,
}

/// Every entry whose label, description or keywords hold each word of
/// `query`, ignoring case: by section, a section's own entry first, then
/// its rows in page order. An empty query finds nothing.
pub fn search(query: &str) -> Vec<Match> {
    let query = query.to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    Section::ALL
        .iter()
        .flat_map(|&section| std::iter::once(Entry::Section(section)).chain(entries(section)))
        .filter_map(|entry| matches(entry, &words))
        .collect()
}

fn matches(entry: Entry, words: &[&str]) -> Option<Match> {
    let label = entry.label().to_lowercase();
    let description = entry.description().to_lowercase();
    let found = words.iter().all(|word| {
        label.contains(word)
            || description.contains(word)
            || entry.keywords().iter().any(|k| k.contains(word))
    });
    found.then(|| Match {
        entry,
        label: ranges(entry.label(), &label, words),
        description: ranges(entry.description(), &description, words),
    })
}

/// Where the words are in `text`, from its lowercase form. None when
/// lowercasing changed its length, so the offsets wouldn't line up.
fn ranges(text: &str, lower: &str, words: &[&str]) -> Vec<Range<usize>> {
    if text.len() != lower.len() {
        return Vec::new();
    }
    let mut found: Vec<Range<usize>> = words
        .iter()
        .flat_map(|word| {
            lower
                .match_indices(word)
                .map(|(start, w)| start..start + w.len())
        })
        .collect();
    found.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in found {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(query: &str) -> Vec<Entry> {
        search(query).into_iter().map(|m| m.entry).collect()
    }

    fn settings_found(query: &str) -> Vec<SettingId> {
        found(query)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Setting(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn saved_settings_round_trip() {
        let saved = Saved {
            appearance: Appearance::Dark,
            text_scale: 1.2,
            calendar_view: CalendarViewSetting::Month,
            remote_images: RemoteImages::Never,
            notifications: false,
            calendar_date: chrono::NaiveDate::from_ymd_opt(2026, 11, 3),
            time_format: TimeFormat::TwentyFour,
            event_minutes: Some(30),
            default_calendar: Some("team@group.calendar.google.com".into()),
            glass: 60,
            keymap: KeymapPreset::AppleMail,
            collapsed_labels: vec!["Label_1".into()],
            window: Some(WindowFrame {
                x: 40.,
                y: 30.,
                width: 1200.,
                height: 800.,
                full_screen: false,
                display: Some("uuid".into()),
            }),
            calendar_open: true,
            nav: SavedNav::Label("Label_2".into()),
            seen_tip: true,
            recent_commands: vec!["pigeon::Archive".into()],
            notification_sound: false,
            event_reminders: false,
            dock_badge: DockBadge::AllInbox,
            ai: crate::ai::AiPrefs {
                enabled: true,
                note_ack: Some("gmail|ai.example.com|openrouter.ai".into()),
                list_summaries: false,
                clean_view: true,
                needs_reply: false,
                notify_for: crate::ai::NotifyFor::Important,
            },
            ..Saved::default()
        };
        assert_eq!(Saved::from_map(saved.to_map()), saved);
        // Saved before MB-072: ignored.
        let mut map = saved.to_map();
        map.insert("reduce_motion".into(), true.into());
        assert_eq!(Saved::from_map(map), saved);
        // Unknown keys go, bad values take their default, the rest stay.
        let mut map = saved.to_map();
        map.insert("gone".into(), true.into());
        map.insert("week_numbers".into(), "yes".into());
        assert_eq!(Saved::from_map(map), saved);
        // Saved before the setting: automatic.
        let mut map = saved.to_map();
        map.remove("default_calendar");
        assert_eq!(Saved::from_map(map).default_calendar, None);
        assert_eq!(Saved::from_map(Default::default()), Saved::default());
    }

    #[test]
    fn ai_prefs_round_trip_and_default_off() {
        let mut saved = Saved {
            nav: SavedNav::NeedsReply,
            ..Saved::default()
        };
        saved.ai.enabled = true;
        saved.ai.note_ack = Some("demo|-|openrouter.ai".into());
        saved.ai.clean_view = false;
        let restored = Saved::from_map(saved.to_map());
        assert_eq!(restored.ai, saved.ai);
        assert_eq!(restored.nav, SavedNav::NeedsReply);
        // Saved before AI-1: off, with nothing confirmed.
        let mut map = saved.to_map();
        map.remove("ai");
        let old = Saved::from_map(map);
        assert!(!old.ai.enabled && old.ai.note_ack.is_none());
        // A broken value keeps the default rather than losing the rest.
        let mut map = saved.to_map();
        map.insert("ai".into(), "on".into());
        assert_eq!(Saved::from_map(map).ai, crate::ai::AiPrefs::default());
    }

    #[test]
    fn ai_words_find_the_ai_section_not_mcp() {
        for word in ["ai", "llm"] {
            let results = found(word);
            assert!(results.contains(&Entry::Section(Section::Ai)), "{word}");
            assert!(!results.contains(&Entry::Section(Section::Mcp)), "{word}");
        }
        assert!(settings_found("test connection").contains(&SettingId::AiWriting));
    }

    #[test]
    fn notifications_default_on_for_settings_saved_before_them() {
        let mut map = Saved::default().to_map();
        map.remove("notifications");
        assert!(Saved::from_map(map).notifications);
    }

    #[test]
    fn the_old_24_hour_switch_becomes_the_time_format() {
        let old = |value: serde_json::Value| {
            let mut map = Saved::default().to_map();
            map.remove("time_format");
            map.insert("time_24h".into(), value);
            Saved::from_map(map).time_format
        };
        assert_eq!(old(true.into()), TimeFormat::TwentyFour);
        assert_eq!(old(false.into()), TimeFormat::Twelve);
        assert_eq!(old(serde_json::Value::Null), TimeFormat::System);
        // Once saved, the new key wins over a stale old one.
        let mut map = Saved::default().to_map();
        map.insert("time_format".into(), "12h".into());
        map.insert("time_24h".into(), true.into());
        assert_eq!(Saved::from_map(map).time_format, TimeFormat::Twelve);
        assert!(TimeFormat::System.is_24_hour(|| true));
        assert!(!TimeFormat::Twelve.is_24_hour(|| true));
        assert!(TimeFormat::TwentyFour.is_24_hour(|| false));
    }

    #[test]
    fn time_format_is_found_by_its_names() {
        assert_eq!(settings_found("24-hour"), vec![SettingId::TimeFormat]);
        assert_eq!(settings_found("time format"), vec![SettingId::TimeFormat]);
        assert!(settings_found("12 hour").contains(&SettingId::TimeFormat));
    }

    #[test]
    fn empty_query_finds_nothing() {
        assert!(search("").is_empty());
        assert!(search("   ").is_empty());
    }

    #[test]
    fn keywords_find_settings() {
        assert_eq!(settings_found("dark"), vec![SettingId::Appearance]);
        assert_eq!(settings_found("font"), vec![SettingId::TextSize]);
        assert_eq!(settings_found("density"), vec![SettingId::CompactList]);
        assert_eq!(settings_found("badge"), vec![SettingId::DockBadge]);
    }

    #[test]
    fn ignores_case_and_needs_every_word() {
        assert_eq!(settings_found("DARK"), vec![SettingId::Appearance]);
        assert_eq!(settings_found("week num"), vec![SettingId::WeekNumbers]);
        assert!(found("dark zzz").is_empty());
        assert!(found("qwxz").is_empty());
    }

    #[test]
    fn finds_sections_and_shortcuts() {
        let results = found("keyboard");
        assert_eq!(results.first(), Some(&Entry::Section(Section::Shortcuts)));
        let archive = found("archive");
        assert!(archive.contains(&Entry::Setting(SettingId::ArchivedInInbox)));
        assert!(
            archive
                .iter()
                .any(|e| matches!(e, Entry::Shortcut(ix) if COMMANDS[*ix].name == "Archive"))
        );
    }

    #[test]
    fn results_keep_section_order() {
        let order: Vec<usize> = found("e")
            .iter()
            .map(|e| Section::ALL.iter().position(|s| *s == e.section()).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn highlights_words_in_label_and_description() {
        let m = &search("text")[0];
        assert_eq!(m.entry, Entry::Setting(SettingId::TextSize));
        assert_eq!(m.label, vec![0..4]);
        assert_eq!(
            ranges("Week numbers", "week numbers", &["wee", "eek"]),
            vec![0..4]
        );
        // Found by a keyword only: nothing to highlight.
        let dark = search("density");
        assert!(dark[0].label.is_empty() && dark[0].description.is_empty());
    }

    #[test]
    fn every_setting_has_a_row() {
        for m in SETTINGS {
            assert!(entries(m.section).contains(&Entry::Setting(m.id)));
        }
        let shortcuts = entries(Section::Shortcuts);
        assert_eq!(shortcuts[0], Entry::Setting(SettingId::ShortcutPreset));
        assert_eq!(shortcuts.len(), COMMANDS.len() + 1);
    }
}

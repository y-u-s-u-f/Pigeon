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
    /// Reduce motion whatever the system says; off follows the system.
    pub reduce_motion: bool,
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
            reduce_motion: false,
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub appearance: Appearance,
    pub reduce_motion: bool,
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
    /// 24-hour times; `None` follows Google Calendar (CAL-41).
    pub time_24h: Option<bool>,
    /// A new event's length in minutes; `None` follows Google Calendar.
    pub event_minutes: Option<u32>,
    /// W7b: the menu bar item, its panel and quick join (NTF-4, NTF-5).
    pub menu_bar: MenuBarPrefs,
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
            reduce_motion: settings.reduce_motion,
            auto_advance: settings.auto_advance,
            search_all_mail: settings.search_all_mail,
            compact_list: false,
            text_scale: 1.,
            sidebar_open: true,
            // `app::SIDEBAR_DEFAULT` and `LIST_DEFAULT`.
            sidebar_width: 236.,
            list_width: 400.,
            archived_expanded: false,
            calendar_view: CalendarViewSetting::Week,
            week_numbers: false,
            remote_images: RemoteImages::Ask,
            notifications: true,
            calendar_date: None,
            time_24h: None,
            event_minutes: None,
            menu_bar: MenuBarPrefs::default(),
        }
    }
}

impl Saved {
    /// From the store's rows; unknown or broken keys take their defaults.
    pub fn from_map(map: serde_json::Map<String, serde_json::Value>) -> Self {
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

    pub fn to_map(self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        }
    }
}

/// Sprint 2 W1 (NTF-3): the settings belong to the app, not the window, so
/// a window opened after the last one closed starts from them. The window
/// hands every change here (`remember`) as it saves it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AppSettings(pub Saved);

impl gpui::Global for AppSettings {}

/// The settings as last saved, wherever they were changed.
pub fn current(cx: &gpui::App) -> Saved {
    cx.try_global::<AppSettings>()
        .map(|s| s.0)
        .unwrap_or_default()
}

pub fn remember(saved: Saved, cx: &mut gpui::App) {
    cx.set_global(AppSettings(saved));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Mail,
    Calendar,
    Notifications,
    MenuBar,
    General,
    Accounts,
    Shortcuts,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Appearance,
        Section::Mail,
        Section::Calendar,
        Section::Notifications,
        Section::MenuBar,
        Section::General,
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
    ReduceMotion,
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
    Appearance: TextSize, "Text size", "Make all text bigger or smaller. Also ⌘= and ⌘-.",
        ["font", "zoom", "scale", "bigger", "smaller", "larger", "accessibility"];
    Appearance: ReduceMotion, "Reduce motion", "Turn animations off. Off follows the system setting.",
        ["animation", "animations", "transitions", "accessibility", "movement"];
    Mail: CompactList, "Compact list", "One line per conversation in the list.",
        ["density", "dense", "comfortable", "condensed", "rows", "spacing"];
    Mail: Sidebar, "Show sidebar", "Mailboxes and labels beside the list. Also ⌘\\.",
        ["folders", "labels", "panel", "hide"];
    Mail: ArchivedInInbox, "Expand Archived in Inbox", "Show archived conversations at the end of the Inbox.",
        ["archive", "collapse", "section"];
    Mail: AutoAdvance, "Auto-advance", "After Archive, Delete or Move, open the next conversation. Off clears the selection.",
        ["next", "advance", "after"];
    Mail: SearchScope, "Search scope", "Where a new search looks, unless it says in:.",
        ["find", "default", "all mail", "everywhere", "current view"];
    Mail: RemoteImages, "Load remote images", "Ask shows a Load Images bar on messages with images from the web.",
        ["pictures", "images", "privacy", "tracking", "pixels", "remote content"];
    Calendar: CalendarView, "View", "Day, week or month. Also D, W and M in the calendar.",
        ["day", "week", "month", "default", "layout"];
    Calendar: WeekNumbers, "Week numbers", "Show the week of the year beside each week.",
        ["iso", "week of year"];
    Calendar: TimeFormat, "24-hour time", "Show 13:00 rather than 1 PM. Follows Google Calendar until changed here.",
        ["clock", "12-hour", "am", "pm", "military", "format"];
    Calendar: EventLength, "Default event length", "How long a new event is. Follows Google Calendar until changed here.",
        ["duration", "new event", "minutes", "meeting length"];
    Calendar: TimeZone, "Time zone", "Times show in this Mac's time zone.",
        ["timezone", "zone", "utc", "gmt", "travel"];
    Accounts: Account, "Account", "Who this mail and calendar are for.",
        ["email", "address", "name", "signed in", "status"];
    Accounts: Access, "Access", "What Google allowed Pigeon to use.",
        ["scopes", "permissions", "granted", "consent"];
    Accounts: LastSync, "Last sync", "When Pigeon last checked Google for changes.",
        ["synced", "updated", "offline", "status"];
    Accounts: SyncNow, "Sync now", "Check Gmail and Google Calendar for changes. Also ⌘R.",
        ["refresh", "reload", "fetch", "update"];
    Accounts: SignOut, "Sign out", "Removes this account and its cached mail and calendar from this Mac.",
        ["log out", "logout", "sign off", "remove account", "disconnect"];
    Notifications: Notifications, "New mail notifications", "A notification for each new message in the Inbox; more than three at once become one.",
        ["alerts", "banners", "notify", "new mail"];
    Notifications: TestNotification, "Send test notification", "Shows one now. The first time, macOS asks whether Pigeon may notify you.",
        ["try", "permission", "allow", "test"];
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
};

pub fn meta(id: SettingId) -> &'static SettingMeta {
    SETTINGS
        .iter()
        .find(|m| m.id == id)
        .expect("every setting is in the table")
}

/// The order of the shortcut groups on the page.
const SHORTCUT_GROUPS: [Group; 11] = [
    Group::App,
    Group::Navigation,
    Group::GoTo,
    Group::Conversation,
    Group::Labels,
    Group::Selection,
    Group::Compose,
    Group::Invite,
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
        Group::Invite => "Mail · Invitations",
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

/// The rows of a section, in order.
pub fn entries(section: Section) -> Vec<Entry> {
    if section == Section::Shortcuts {
        return SHORTCUT_GROUPS
            .iter()
            .flat_map(|&group| {
                COMMANDS
                    .iter()
                    .enumerate()
                    .filter(move |(_, c)| c.group == group)
                    .map(|(ix, _)| Entry::Shortcut(ix))
            })
            .collect();
    }
    SETTINGS
        .iter()
        .filter(|m| m.section == section)
        .map(|m| Entry::Setting(m.id))
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
            time_24h: Some(true),
            event_minutes: Some(30),
            ..Saved::default()
        };
        assert_eq!(Saved::from_map(saved.to_map()), saved);
        // Unknown keys go, bad values take their default, the rest stay.
        let mut map = saved.to_map();
        map.insert("gone".into(), true.into());
        map.insert("week_numbers".into(), "yes".into());
        assert_eq!(Saved::from_map(map), saved);
        assert_eq!(Saved::from_map(Default::default()), Saved::default());
    }

    #[test]
    fn notifications_default_on_for_settings_saved_before_them() {
        let mut map = Saved::default().to_map();
        map.remove("notifications");
        assert!(Saved::from_map(map).notifications);
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
        assert!(settings_found("animation").contains(&SettingId::ReduceMotion));
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
        assert_eq!(entries(Section::Shortcuts).len(), COMMANDS.len());
    }
}

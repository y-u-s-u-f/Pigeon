//! The settings page's table: every setting with the words it is found by,
//! and the search over them. The values live where the app already keeps
//! them (`PigeonApp`, the calendar, `Settings`); `ui/settings.rs` reads and
//! changes them there, so shortcuts and menus stay in step.

use std::ops::Range;

use crate::{
    actions::{COMMANDS, Group},
    assets::Icon,
};

/// Preferences that have no other home. In memory, like the rest of the
/// app's state (see PER-3 in `docs/ROADMAP.md`).
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Mail,
    Calendar,
    Accounts,
    Shortcuts,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Appearance,
        Section::Mail,
        Section::Calendar,
        Section::Accounts,
        Section::Shortcuts,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Mail => "Mail",
            Section::Calendar => "Calendar",
            Section::Accounts => "Accounts",
            Section::Shortcuts => "Keyboard Shortcuts",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Section::Appearance => Icon::SunMoon,
            Section::Mail => Icon::Mail,
            Section::Calendar => Icon::Calendar,
            Section::Accounts => Icon::User,
            Section::Shortcuts => Icon::Keyboard,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Section::Appearance => &["theme", "look", "display"],
            Section::Mail => &["inbox", "email", "list"],
            Section::Calendar => &["events", "schedule"],
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
    Account,
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
    Calendar: CalendarView, "View", "Day, week or month. Also D, W and M in the calendar.",
        ["day", "week", "month", "default", "layout"];
    Calendar: WeekNumbers, "Week numbers", "Show the week of the year beside each week.",
        ["iso", "week of year"];
    Accounts: Account, "Account", "Who this mail and calendar are for.",
        ["email", "address", "name", "signed in"];
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

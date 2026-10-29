//! The command palette: every command, folder and label, plus matching
//! threads once there is a query, fuzzy-filtered as you type.

use std::{collections::HashMap, sync::OnceLock};

use gpui::{
    Action, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    HighlightStyle, Hsla, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement,
    Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled, StyledText,
    Subscription, UniformListScrollHandle, Window, actions,
    component::{
        input::{Input, InputEvent, InputState},
        scroll::ScrollableElement as _,
    },
    div,
    prelude::FluentBuilder as _,
    px, uniform_list,
};
use nucleo_matcher::{
    Config, Matcher, Utf32String,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

use super::{binding_caps, binding_text, controls::sheet_surface, icon, keycap, keys_caps};
use crate::ui::style::*;
use crate::{
    actions::{COMMANDS, PALETTE},
    app::{LabelCommand, Nav},
    assets::Icon,
    calendar::quick_add,
    data::Folder,
    theme::Theme,
};

actions!(command_palette, [SelectNext, SelectPrev, Confirm, Dismiss]);

const WIDTH: f32 = 620.;
/// The result area is always this many rows tall, so the panel keeps its
/// size as the results change.
const VISIBLE_ROWS: usize = 8;
/// Thread results are capped so a broad query stays readable.
const MAX_THREADS: usize = 30;
/// The widest a title gets before its subtitle.
const TITLE_MAX: gpui::Rems = type_size(380.);

#[derive(Clone, Copy)]
pub enum PaletteCommand {
    Action(fn() -> Box<dyn Action>),
    Nav(Nav),
    /// Index into `MailData::threads`.
    Thread(usize),
    /// Index into `CalendarData::calendars`.
    ToggleCalendar(usize),
    /// Index into `CalendarData::events`.
    Event(usize),
    /// Move the selected threads (`v`).
    MoveTo(Folder),
    /// Add or remove a label, by index, on the selected threads (`l`).
    ToggleLabel(usize),
    /// W4b: managing a label (FOL-6, FOL-10).
    Label(LabelCommand),
}

pub enum Leading {
    Icon(Icon),
    /// A label's or calendar's dot, indented by its depth in the tree (FOL-9).
    Nested(Hsla, usize),
}

impl Leading {
    pub fn dot(color: Hsla) -> Self {
        Leading::Nested(color, 0)
    }
}

pub struct PaletteItem {
    pub command: PaletteCommand,
    pub title: SharedString,
    /// Also searched; shown dimmed after the title.
    pub subtitle: Option<SharedString>,
    pub leading: Leading,
    pub binding: Option<KeyBinding>,
    /// Only offered once there is a query (threads).
    pub search_only: bool,
}

struct Entry {
    item: PaletteItem,
    /// Title, subtitle and the command's other name, separated by spaces.
    haystack: Utf32String,
    title_chars: usize,
}

/// ME-062: the palette's groups, in their order without a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    Recent,
    Commands,
    Labels,
    Calendars,
    Conversations,
    Events,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Recent => "Recent",
            Section::Commands => "Commands",
            Section::Labels => "Labels",
            Section::Calendars => "Calendars",
            Section::Conversations => "Conversations",
            Section::Events => "Events",
        }
    }

    fn of(command: PaletteCommand) -> Self {
        match command {
            PaletteCommand::Nav(Nav::Label(_)) => Section::Labels,
            PaletteCommand::ToggleCalendar(_) => Section::Calendars,
            PaletteCommand::Thread(_) => Section::Conversations,
            PaletteCommand::Event(_) => Section::Events,
            PaletteCommand::Action(_)
            | PaletteCommand::Nav(_)
            | PaletteCommand::MoveTo(_)
            | PaletteCommand::ToggleLabel(_)
            | PaletteCommand::Label(_) => Section::Commands,
        }
    }
}

/// One row of the results.
enum Row {
    /// A section's title; arrows skip it.
    Header(Section),
    /// Quick add (CAL-39), or the prompt's "Create label “…”".
    Quick,
    Entry {
        entry: usize,
        /// Matched char indices into the haystack, sorted.
        indices: Vec<u32>,
    },
}

/// The name each command has in the shortcuts sheet, by action name, for
/// commands whose palette title follows state ("Unstar" is found by "star").
fn static_names() -> &'static HashMap<&'static str, &'static str> {
    static NAMES: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        COMMANDS
            .iter()
            .map(|c| ((c.action_name)(), c.name))
            .collect()
    })
}

pub enum PaletteEvent {
    Run(PaletteCommand),
    Dismiss,
    // W5: "New event: Lunch with Sam Fri 1pm", the text after the colon.
    QuickAdd(String),
    /// W4b: the text typed into a prompt (`CommandPalette::prompt`).
    Text(String),
}

pub struct CommandPalette {
    input: Entity<InputState>,
    entries: Vec<Entry>,
    rows: Vec<Row>,
    selected: usize,
    scroll: UniformListScrollHandle,
    matcher: Matcher,
    /// Shown when nothing matches.
    empty_text: SharedString,
    /// The palette of every command offers quick add and has sections;
    /// lists to pick from don't.
    offers_quick_add: bool,
    /// The quick add row for the query: the text, its title and when.
    quick: Option<(String, SharedString, SharedString)>,
    /// W4b: a prompt for a name: what Enter does with it, and a hint.
    prompt: Option<(SharedString, SharedString)>,
    /// How many of the first entries were used last, shown under "Recent"
    /// while there is no query (ME-062).
    recent: usize,
    /// CAL-39: writable calendars as (id, name), for Quick Add's tags.
    #[allow(dead_code)]
    calendars: Vec<(String, String)>,
    _input_events: Subscription,
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl CommandPalette {
    pub fn new(
        items: Vec<PaletteItem>,
        query: &str,
        placeholder: SharedString,
        empty_text: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(placeholder);
            // Unlike `default_value`, leaves the cursor after the text.
            state.insert(query.to_string(), window, cx);
            state
        });
        let input_events = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.update_matches(cx);
            }
        });
        let offers_quick_add = items
            .iter()
            .any(|i| matches!(i.command, PaletteCommand::Action(_)));
        let entries = items
            .into_iter()
            .map(|item| {
                let mut haystack = item.title.to_string();
                if let Some(subtitle) = &item.subtitle {
                    haystack.push(' ');
                    haystack.push_str(subtitle);
                }
                // "Star / Unstar" finds "Unstar" by either word.
                if let PaletteCommand::Action(action) = item.command
                    && let Some(name) = static_names().get(action().name())
                    && *name != item.title.as_ref()
                {
                    haystack.push(' ');
                    haystack.push_str(name);
                }
                Entry {
                    title_chars: item.title.chars().count(),
                    haystack: haystack.into(),
                    item,
                }
            })
            .collect();

        let mut this = Self {
            input,
            entries,
            rows: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            matcher: Matcher::new(Config::DEFAULT),
            empty_text,
            offers_quick_add,
            quick: None,
            prompt: None,
            recent: 0,
            calendars: Vec::new(),
            _input_events: input_events,
        };
        this.update_matches(cx);
        this
    }

    /// ME-062: the first `recent` items are the commands used last, listed
    /// under "Recent" while there is no query.
    pub fn set_recent(&mut self, recent: usize, cx: &mut Context<Self>) {
        self.recent = recent;
        self.update_matches(cx);
    }

    pub fn set_calendars(&mut self, calendars: Vec<(String, String)>) {
        self.calendars = calendars;
    }

    /// W4b: a field for a name rather than a list: Enter sends the text
    /// (`PaletteEvent::Text`), shown as "`verb` “text”" with `hint`.
    pub fn prompt(
        initial: &str,
        placeholder: SharedString,
        verb: SharedString,
        hint: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::new(
            Vec::new(),
            initial,
            placeholder,
            "Type a name".into(),
            window,
            cx,
        );
        this.prompt = Some((verb, hint));
        this.update_matches(cx);
        this
    }

    fn update_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value();
        // Commands match fuzzily; threads only on whole substrings, since a
        // subsequence scattered over a long subject is noise.
        let pattern = |kind| Pattern::new(&query, CaseMatching::Ignore, Normalization::Smart, kind);
        let (fuzzy, substring) = (pattern(AtomKind::Fuzzy), pattern(AtomKind::Substring));

        self.rows.clear();
        self.quick = quick_add::command_text(&query)
            .filter(|text| self.offers_quick_add && !text.is_empty())
            .map(|text| {
                let event = quick_add::parse(
                    text,
                    chrono::Local::now().naive_local(),
                    quick_add::default_minutes(),
                );
                let title = if event.title.is_empty() {
                    "New event".to_string()
                } else {
                    format!("New event: {}", event.title)
                };
                let today = chrono::Local::now().date_naive();
                // The place it read, so a wrong guess shows before Enter.
                let when = match &event.location {
                    Some(place) => format!("{} · {place}", event.when_label(today)),
                    None => event.when_label(today),
                };
                (text.to_string(), title.into(), when.into())
            });
        if let Some((verb, hint)) = &self.prompt {
            let text = query.trim();
            self.quick = (!text.is_empty()).then(|| {
                (
                    text.to_string(),
                    format!("{verb} “{text}”").into(),
                    hint.clone(),
                )
            });
        }
        if self.quick.is_some() {
            self.rows.push(Row::Quick);
        }

        // Kept entries with their section, in order.
        let mut kept: Vec<(Section, usize)> = Vec::new();
        if fuzzy.atoms.is_empty() {
            kept.extend(
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| !e.item.search_only)
                    .map(|(ix, e)| {
                        let recent =
                            ix < self.recent && matches!(e.item.command, PaletteCommand::Action(_));
                        let section = if recent {
                            Section::Recent
                        } else {
                            Section::of(e.item.command)
                        };
                        (section, ix)
                    }),
            );
            // Recents lead; the rest keep palette order within a section.
            kept.sort_by_key(|(section, _)| *section);
        } else {
            // MC-039: rank by score alone; match positions only for the
            // rows kept.
            let mut scored: Vec<(u32, Section, usize)> = Vec::new();
            for (ix, entry) in self.entries.iter().enumerate() {
                let pattern = if entry.item.search_only {
                    &substring
                } else {
                    &fuzzy
                };
                if let Some(score) = pattern.score(entry.haystack.slice(..), &mut self.matcher) {
                    scored.push((score, Section::of(entry.item.command), ix));
                }
            }
            // Stable, so ties keep palette order: commands before threads.
            scored.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));
            let mut threads = 0;
            scored.retain(|&(_, _, ix)| {
                if self.entries[ix].item.search_only {
                    threads += 1;
                    return threads <= MAX_THREADS;
                }
                true
            });
            // Sections in the order of their best match; within one, by
            // score (ME-062).
            let mut order: Vec<Section> = Vec::new();
            for &(_, section, _) in &scored {
                if !order.contains(&section) {
                    order.push(section);
                }
            }
            kept.extend(scored.iter().map(|&(_, section, ix)| (section, ix)));
            kept.sort_by_key(|(section, _)| order.iter().position(|s| s == section));
        }

        // Pick lists and prompts have no sections.
        let sections = self.offers_quick_add;
        let mut last = None;
        for (section, entry) in kept {
            if sections && last != Some(section) {
                self.rows.push(Row::Header(section));
                last = Some(section);
            }
            let mut indices = Vec::new();
            if !fuzzy.atoms.is_empty() {
                let e = &self.entries[entry];
                let pattern = if e.item.search_only {
                    &substring
                } else {
                    &fuzzy
                };
                pattern.indices(e.haystack.slice(..), &mut self.matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();
            }
            self.rows.push(Row::Entry { entry, indices });
        }

        self.selected = self.first_selectable().unwrap_or(0);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn selectable(&self, ix: usize) -> bool {
        self.rows
            .get(ix)
            .is_some_and(|r| !matches!(r, Row::Header(_)))
    }

    fn first_selectable(&self) -> Option<usize> {
        (0..self.rows.len()).find(|&ix| self.selectable(ix))
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix != self.selected && self.selectable(ix) {
            self.selected = ix;
            // The first row's header comes into view with it.
            let first = self.first_selectable() == Some(ix);
            self.scroll
                .scroll_to_item(if first { 0 } else { ix }, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    /// The next selectable row `step` away, wrapping, skipping headers.
    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let n = self.rows.len();
        let next = (1..=n)
            .map(|d| {
                if forward {
                    (self.selected + d) % n
                } else {
                    (self.selected + n - d % n) % n
                }
            })
            .find(|&ix| self.selectable(ix));
        if let Some(ix) = next {
            self.select(ix, cx);
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.run(self.selected, cx);
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismiss);
    }

    fn run(&mut self, ix: usize, cx: &mut Context<Self>) {
        match self.rows.get(ix) {
            Some(Row::Quick) => {
                if let Some((text, _, _)) = &self.quick {
                    cx.emit(if self.prompt.is_some() {
                        PaletteEvent::Text(text.clone())
                    } else {
                        PaletteEvent::QuickAdd(text.clone())
                    });
                }
            }
            Some(Row::Entry { entry, .. }) => {
                cx.emit(PaletteEvent::Run(self.entries[*entry].item.command))
            }
            Some(Row::Header(_)) | None => {}
        }
    }

    /// ME-076: what Enter does with the selected row.
    fn enter_label(&self) -> Option<SharedString> {
        Some(match self.rows.get(self.selected)? {
            Row::Header(_) => return None,
            Row::Quick => match &self.prompt {
                Some((verb, _)) => verb
                    .split(' ')
                    .next()
                    .unwrap_or("Create")
                    .to_string()
                    .into(),
                None => "Create".into(),
            },
            Row::Entry { entry, .. } => match self.entries[*entry].item.command {
                PaletteCommand::Nav(_) | PaletteCommand::Thread(_) | PaletteCommand::Event(_) => {
                    "Open".into()
                }
                _ => "Run".into(),
            },
        })
    }

    fn render_row(&self, ix: usize, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (entry, indices) = match &self.rows[ix] {
            Row::Quick => return self.render_quick_row(ix, theme, cx),
            Row::Header(section) => {
                return div()
                    .id(ix)
                    .role(gpui::Role::Heading)
                    .aria_level(2)
                    .aria_label(section.title())
                    .h(ROW_PALETTE)
                    .px(SPACE_SM + SPACE_MD)
                    .pb(SPACE_XS)
                    .flex()
                    .items_end()
                    .text_size(TEXT_XS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_faint)
                    .child(section.title())
                    .into_any_element();
            }
            Row::Entry { entry, indices } => (&self.entries[*entry], indices),
        };
        let item = &entry.item;
        let selected = ix == self.selected;
        // ME-079: color only, so titles don't shift as matches change.
        let highlight = HighlightStyle {
            color: Some(theme.match_text),
            ..Default::default()
        };

        let title_hl = indices
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| i < entry.title_chars);
        let subtitle_hl = indices
            .iter()
            .map(|&i| i as usize)
            .filter_map(|i| i.checked_sub(entry.title_chars + 1));
        let label: SharedString = match &item.subtitle {
            Some(subtitle) => format!("{}, {subtitle}", item.title).into(),
            None => item.title.clone(),
        };

        let leading = match item.leading {
            Leading::Icon(glyph) => leading_icon(glyph, selected, theme).into_any_element(),
            Leading::Nested(color, depth) => super::dot_marker(color)
                .ml(SPACE_LG * depth as f32)
                .into_any_element(),
        };
        palette_row(
            ix,
            selected,
            label,
            leading,
            highlighted(&item.title, title_hl, highlight),
            item.subtitle
                .as_ref()
                .map(|subtitle| highlighted(subtitle, subtitle_hl, highlight)),
            item.binding.as_ref(),
            theme,
            cx,
        )
    }

    /// "New event: Lunch with Sam", "Fri, Oct 30 · 1 – 2 PM" (CAL-39).
    fn render_quick_row(
        &self,
        ix: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (title, when) = self
            .quick
            .as_ref()
            .map(|(_, title, when)| (title.clone(), when.clone()))
            .unwrap_or_default();
        let selected = ix == self.selected;
        palette_row(
            ix,
            selected,
            format!("{title}, {when}").into(),
            leading_icon(Icon::Plus, selected, theme).into_any_element(),
            title,
            Some(when),
            None,
            theme,
            cx,
        )
    }
}

fn leading_icon(glyph: Icon, selected: bool, theme: &Theme) -> gpui::Svg {
    icon(
        glyph,
        ICON_LG,
        if selected {
            theme.icon_active
        } else {
            theme.icon
        },
    )
}

/// One option row: leading icon or dot, title, dimmed subtitle, shortcut.
#[allow(clippy::too_many_arguments)]
fn palette_row(
    ix: usize,
    selected: bool,
    label: SharedString,
    leading: gpui::AnyElement,
    title: impl IntoElement,
    subtitle: Option<impl IntoElement>,
    binding: Option<&KeyBinding>,
    theme: &Theme,
    cx: &mut Context<CommandPalette>,
) -> gpui::AnyElement {
    div()
        .id(ix)
        .role(gpui::Role::ListBoxOption)
        .aria_label(label)
        .when_some(binding, |this, b| this.aria_keyshortcuts(binding_text(b)))
        .aria_selected(selected)
        .when(selected, |this| this.aria_active_descendant())
        .h(ROW_PALETTE)
        .px(SPACE_SM)
        .child(
            div()
                .size_full()
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_MD)
                .rounded(RADIUS_MD)
                .when(selected, |this| this.bg(theme.selected_bg))
                .child(leading)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_baseline()
                        .gap(SPACE_SM)
                        .overflow_hidden()
                        .child(
                            div()
                                .min_w_0()
                                .max_w(TITLE_MAX)
                                .truncate()
                                .text_size(TEXT_BASE)
                                .text_color(theme.text)
                                .child(title),
                        )
                        .children(subtitle.map(|subtitle| {
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_faint)
                                .child(subtitle)
                        })),
                )
                .children(binding.map(|b| binding_caps(b, theme))),
        )
        .on_mouse_move(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
        .on_click(cx.listener(move |this, _, _, cx| this.run(ix, cx)))
        .into_any_element()
}

/// `text` with the chars at `indices` (sorted) styled as matches.
fn highlighted(
    text: &SharedString,
    indices: impl Iterator<Item = usize>,
    style: HighlightStyle,
) -> StyledText {
    let mut indices = indices.peekable();
    let mut ranges = Vec::new();
    for (char_ix, (byte_ix, ch)) in text.char_indices().enumerate() {
        if indices.peek() == Some(&char_ix) {
            indices.next();
            ranges.push((byte_ix..byte_ix + ch.len_utf8(), style));
        }
    }
    StyledText::new(text.clone()).with_highlights(ranges)
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let rows = self.rows.len();
        // In rems like the rows, so the list scales with the text. A prompt
        // has one row at most (ME-077).
        let visible = if self.prompt.is_some() {
            1
        } else {
            VISIBLE_ROWS
        };
        let list_height = gpui::rems(ROW_PALETTE.0 * visible as f32);

        let results = if rows == 0 {
            div()
                .h(list_height)
                .flex()
                .items_center()
                .justify_center()
                .text_size(TEXT_BASE)
                .text_color(theme.text_faint)
                .child(self.empty_text.clone())
                .into_any_element()
        } else {
            let row_theme = theme.clone();
            div()
                .id("palette-results")
                .role(gpui::Role::ListBox)
                .aria_label("Results")
                .relative()
                .h(list_height)
                .child(
                    uniform_list(
                        "palette-list",
                        rows,
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|ix| this.render_row(ix, &row_theme, cx).into_any_element())
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                )
                .vertical_scrollbar(&self.scroll)
                .into_any_element()
        };

        // ME-076: a count only for a query, of results alone.
        let query = !self.input.read(cx).value().trim().is_empty();
        let count = self
            .rows
            .iter()
            .filter(|r| matches!(r, Row::Entry { .. }))
            .count();
        let count = (query && self.prompt.is_none())
            .then(|| super::status::plural(count, "result", "results"));

        let hint = |label: SharedString, caps: gpui::Div| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .child(label)
                .child(caps)
        };

        sheet_surface(&theme)
            .id("command-palette")
            .key_context(PALETTE)
            .role(gpui::Role::Dialog)
            .aria_label("Command palette")
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            // Keep clicks inside from reaching the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .min_h(CONTROL_LG)
                    .py(SPACE_MD)
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_MD)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(icon(Icon::Search, ICON_XL, theme.text_faint))
                    .child(
                        div()
                            .flex_1()
                            .text_size(TEXT_LG)
                            .child(Input::new(&self.input).appearance(false)),
                    ),
            )
            .child(div().py(SPACE_SM).child(results))
            .child(
                div()
                    .py(SPACE_SM)
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_LG)
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .when(self.prompt.is_none(), |this| {
                        this.child(hint(
                            "Navigate".into(),
                            div()
                                .flex()
                                .gap(SPACE_XS)
                                .child(keycap("↑", &theme))
                                .child(keycap("↓", &theme)),
                        ))
                    })
                    .children(
                        self.enter_label()
                            .map(|label| hint(label, keys_caps("enter", &theme))),
                    )
                    .child(hint("Close".into(), keys_caps("escape", &theme)))
                    .child(div().flex_1())
                    .children(count),
            )
    }
}

//! The command palette: every command, folder and label, plus matching
//! threads once there is a query, fuzzy-filtered as you type.

use gpui::{
    Action, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    HighlightStyle, Hsla, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement,
    Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled, StyledText,
    Subscription, UniformListScrollHandle, Window, actions,
    component::input::{Input, InputEvent, InputState},
    div,
    prelude::FluentBuilder as _,
    px, uniform_list,
};
use nucleo_matcher::{
    Config, Matcher, Utf32String,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

use super::{binding_caps, icon, keycap, overlay_shadow};
use crate::ui::style::*;
use crate::{
    actions::PALETTE, app::Nav, assets::Icon, calendar::quick_add, data::Folder, theme::Theme,
};

actions!(command_palette, [SelectNext, SelectPrev, Confirm, Dismiss]);

const WIDTH: f32 = 620.;
const ROW_HEIGHT: f32 = 38.;
/// The result area is always this many rows tall, so the panel keeps its
/// size as the results change.
const VISIBLE_ROWS: usize = 8;
/// Thread results are capped so a broad query stays readable.
const MAX_THREADS: usize = 30;
/// `Match::entry` of the quick add row (CAL-39), which isn't an entry.
const QUICK: usize = usize::MAX;

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
}

pub enum Leading {
    Icon(Icon),
    Dot(Hsla),
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
    /// Title and subtitle, separated by a space.
    haystack: Utf32String,
    title_chars: usize,
}

struct Match {
    entry: usize,
    /// Matched char indices into the haystack, sorted.
    indices: Vec<u32>,
}

pub enum PaletteEvent {
    Run(PaletteCommand),
    Dismiss,
    // W5: "New event: Lunch with Sam Fri 1pm", the text after the colon.
    QuickAdd(String),
}

pub struct CommandPalette {
    input: Entity<InputState>,
    entries: Vec<Entry>,
    matches: Vec<Match>,
    selected: usize,
    scroll: UniformListScrollHandle,
    matcher: Matcher,
    /// Shown when nothing matches.
    empty_text: SharedString,
    /// The palette of every command offers quick add; lists to pick from
    /// don't.
    offers_quick_add: bool,
    /// The quick add row for the query: the text, its title and when.
    quick: Option<(String, SharedString, SharedString)>,
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
            matches: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            matcher: Matcher::new(Config::DEFAULT),
            empty_text,
            offers_quick_add,
            quick: None,
            _input_events: input_events,
        };
        this.update_matches(cx);
        this
    }

    fn update_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value();
        // Commands match fuzzily; threads only on whole substrings, since a
        // subsequence scattered over a long subject is noise.
        let pattern = |kind| Pattern::new(&query, CaseMatching::Ignore, Normalization::Smart, kind);
        let (fuzzy, substring) = (pattern(AtomKind::Fuzzy), pattern(AtomKind::Substring));

        self.matches.clear();
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
                (
                    text.to_string(),
                    title.into(),
                    event.when_label(today).into(),
                )
            });
        if self.quick.is_some() {
            self.matches.push(Match {
                entry: QUICK,
                indices: Vec::new(),
            });
        }
        if fuzzy.atoms.is_empty() {
            self.matches.extend(
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| !e.item.search_only)
                    .map(|(entry, _)| Match {
                        entry,
                        indices: Vec::new(),
                    }),
            );
        } else {
            let mut scored = Vec::new();
            for (ix, entry) in self.entries.iter().enumerate() {
                let pattern = if entry.item.search_only {
                    &substring
                } else {
                    &fuzzy
                };
                let mut indices = Vec::new();
                if let Some(score) =
                    pattern.indices(entry.haystack.slice(..), &mut self.matcher, &mut indices)
                {
                    indices.sort_unstable();
                    indices.dedup();
                    scored.push((score, Match { entry: ix, indices }));
                }
            }
            // Stable, so ties keep palette order: commands before threads.
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            let mut threads = 0;
            for (_, m) in scored {
                if self.entries[m.entry].item.search_only {
                    threads += 1;
                    if threads > MAX_THREADS {
                        continue;
                    }
                }
                self.matches.push(m);
            }
        }

        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix != self.selected {
            self.selected = ix;
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.select((self.selected + 1) % self.matches.len(), cx);
        }
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(last) = self.matches.len().checked_sub(1) {
            self.select(self.selected.checked_sub(1).unwrap_or(last), cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.run(self.selected, cx);
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismiss);
    }

    fn run(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.matches.get(ix) {
            if m.entry == QUICK {
                if let Some((text, _, _)) = &self.quick {
                    cx.emit(PaletteEvent::QuickAdd(text.clone()));
                }
                return;
            }
            cx.emit(PaletteEvent::Run(self.entries[m.entry].item.command));
        }
    }

    fn render_row(&self, ix: usize, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let m = &self.matches[ix];
        if m.entry == QUICK {
            return self.render_quick_row(ix, theme, cx).into_any_element();
        }
        let entry = &self.entries[m.entry];
        let item = &entry.item;
        let selected = ix == self.selected;
        let highlight = HighlightStyle {
            color: Some(theme.match_text),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        };

        let title_hl = m
            .indices
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| i < entry.title_chars);
        let subtitle_hl = m
            .indices
            .iter()
            .map(|&i| i as usize)
            .filter_map(|i| i.checked_sub(entry.title_chars + 1));

        div()
            .id(ix)
            .role(gpui::Role::ListBoxOption)
            .aria_label(item.title.clone())
            .aria_selected(selected)
            .h(px(ROW_HEIGHT))
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
                    .child(match item.leading {
                        Leading::Icon(glyph) => icon(
                            glyph,
                            px(16.),
                            if selected {
                                theme.icon_active
                            } else {
                                theme.icon
                            },
                        )
                        .into_any_element(),
                        Leading::Dot(color) => div()
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(div().size(px(8.)).rounded_full().bg(color))
                            .into_any_element(),
                    })
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
                                    .flex_none()
                                    .max_w(px(380.))
                                    .truncate()
                                    .text_size(TEXT_BASE)
                                    .text_color(theme.text)
                                    .child(highlighted(&item.title, title_hl, highlight)),
                            )
                            .children(item.subtitle.as_ref().map(|subtitle| {
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(TEXT_SM)
                                    .text_color(theme.text_faint)
                                    .child(highlighted(subtitle, subtitle_hl, highlight))
                            })),
                    )
                    .children(item.binding.as_ref().map(|b| binding_caps(b, theme))),
            )
            .on_mouse_move(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
            .on_click(cx.listener(move |this, _, _, cx| this.run(ix, cx)))
            .into_any_element()
    }

    /// "New event: Lunch with Sam", "Fri, Oct 30 · 1 – 2 PM" (CAL-39).
    fn render_quick_row(
        &self,
        ix: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (title, when) = self
            .quick
            .as_ref()
            .map(|(_, title, when)| (title.clone(), when.clone()))
            .unwrap_or_default();
        let selected = ix == self.selected;
        div()
            .id(ix)
            .role(gpui::Role::ListBoxOption)
            .aria_label(SharedString::from(format!("{title}, {when}")))
            .aria_selected(selected)
            .h(px(ROW_HEIGHT))
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
                    .child(icon(
                        Icon::Plus,
                        px(16.),
                        if selected {
                            theme.icon_active
                        } else {
                            theme.icon
                        },
                    ))
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
                                    .flex_none()
                                    .max_w(px(380.))
                                    .truncate()
                                    .text_size(TEXT_BASE)
                                    .text_color(theme.text)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(TEXT_SM)
                                    .text_color(theme.text_faint)
                                    .child(when),
                            ),
                    ),
            )
            .on_mouse_move(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
            .on_click(cx.listener(move |this, _, _, cx| this.run(ix, cx)))
    }
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
        let rows = self.matches.len();
        let list_height = px(ROW_HEIGHT * VISIBLE_ROWS as f32);

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
            uniform_list(
                "palette-results",
                rows,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|ix| this.render_row(ix, &row_theme, cx).into_any_element())
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .h(list_height)
            .into_any_element()
        };

        let hint = |keys: &'static [&'static str], label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .children(keys.iter().map(|k| keycap(*k, &theme)))
                .child(div().ml(SPACE_2XS).child(label))
        };

        div()
            .key_context(PALETTE)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(RADIUS_LG)
            .bg(theme.overlay_bg)
            .border_1()
            .border_color(theme.overlay_border)
            .shadow(overlay_shadow(&theme))
            // Keep clicks inside from reaching the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(54.))
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_MD)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(icon(Icon::Search, px(17.), theme.text_faint))
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
                    .h(px(34.))
                    .px(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_LG)
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(hint(&["↑", "↓"], "Navigate"))
                    .child(hint(&["↵"], "Run"))
                    .child(hint(&["Esc"], "Close"))
                    .child(div().flex_1())
                    .child(format!("{rows} result{}", if rows == 1 { "" } else { "s" })),
            )
    }
}

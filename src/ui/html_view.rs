//! Renders a parsed email body (`crate::html`) as native GPUI elements.

use std::rc::Rc;

use gpui::{
    AnyElement, Div, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveElement as _,
    InteractiveText, IntoElement, ParentElement, SharedString, StatefulInteractiveElement as _,
    StrikethroughStyle, Styled, StyledText, UnderlineStyle, base::TextSelection,
    component::tooltip::Tooltip, div, prelude::FluentBuilder as _, px, relative,
};

use super::selectable::{OnClear, Selectable};
use crate::ui::style::*;
use crate::{
    html::{Align, Block, Document, Inline, Style, Table, TextKind},
    theme::Theme,
};

const MONO: &str = "Menlo";
const BLOCK_GAP: gpui::Pixels = SPACE_MD;

pub fn render(doc: &Document, theme: &Theme) -> impl IntoElement {
    render_filtered(doc, theme, None, |_| true)
}

/// Makes a rendered document's text selectable.
pub struct Selection {
    /// Reading order of the document's first text (see `Selectable::order`).
    pub order: u64,
    /// Selects all of it (⌘A) until a click clears it, which calls this.
    pub all: Option<OnClear>,
}

/// The document's top-level blocks that `keep` accepts.
pub fn render_filtered(
    doc: &Document,
    theme: &Theme,
    selection: Option<Selection>,
    keep: impl Fn(&Block) -> bool,
) -> impl IntoElement {
    let mut renderer = Renderer {
        theme,
        next_id: 0,
        list_depth: 0,
        selection,
        next_text: 0,
    };
    let blocks = doc
        .blocks
        .iter()
        .filter(|b| keep(b))
        .enumerate()
        .map(|(ix, b)| renderer.block(b, ix == 0))
        .collect::<Vec<_>>();
    div().flex().flex_col().gap(BLOCK_GAP).children(blocks)
}

struct Renderer<'a> {
    theme: &'a Theme,
    /// Ids for the interactive (linked) texts; unique within the message.
    next_id: usize,
    list_depth: usize,
    selection: Option<Selection>,
    /// Counts the selectable texts, for their ids and reading order.
    next_text: u64,
}

impl Renderer<'_> {
    fn blocks(&mut self, blocks: &[Block], gap: gpui::Pixels) -> Div {
        div().flex().flex_col().gap(gap).children(
            blocks
                .iter()
                .enumerate()
                .map(|(ix, b)| self.block(b, ix == 0)),
        )
    }

    fn block(&mut self, block: &Block, first: bool) -> AnyElement {
        let theme = self.theme;
        match block {
            Block::Text { text, kind, align } => {
                let el = div().map(|d| aligned(d, *align));
                match kind {
                    TextKind::Paragraph => el,
                    TextKind::Heading(level) => el
                        .when(!first, |d| d.pt(SPACE_SM))
                        .text_size(match level {
                            1 => TEXT_XL,
                            2 => TEXT_LG,
                            _ => TEXT_MD,
                        })
                        .line_height(relative(1.3))
                        .font_weight(FontWeight::SEMIBOLD),
                }
                .child(self.text(text))
                .into_any_element()
            }
            Block::Pre(text) => div()
                .px(SPACE_MD)
                .py(SPACE_MD)
                .rounded(RADIUS_MD)
                .bg(theme.chip_bg)
                .font_family(MONO)
                .text_size(TEXT_SM)
                .line_height(relative(1.5))
                .child(self.text(text))
                .into_any_element(),
            Block::List { ordered, items } => {
                self.list_depth += 1;
                let bullet = if self.list_depth % 2 == 1 {
                    "•"
                } else {
                    "◦"
                };
                let list =
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_XS)
                        .children(items.iter().enumerate().map(|(ix, item)| {
                            let marker = if *ordered {
                                format!("{}.", ix + 1)
                            } else {
                                bullet.to_string()
                            };
                            div()
                                .flex()
                                .gap(SPACE_SM)
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(18.))
                                        .text_right()
                                        .text_color(theme.text_muted)
                                        .child(marker),
                                )
                                .child(div().flex_1().min_w_0().child(self.blocks(item, SPACE_XS)))
                        }));
                self.list_depth -= 1;
                list.into_any_element()
            }
            Block::Quote(blocks) => div()
                .pl(SPACE_MD)
                .border_l_2()
                .border_color(theme.text_faint.alpha(0.45))
                .text_color(theme.text_muted)
                .child(self.blocks(blocks, BLOCK_GAP))
                .into_any_element(),
            Block::Table(table) => self.table(table),
            // Remote images aren't loaded (see BACKEND_REQUIREMENTS RDR-4).
            Block::Image { alt } => div()
                .max_w_full()
                .flex()
                .items_center()
                .justify_center()
                .gap(SPACE_SM)
                .min_h(px(44.))
                .px(SPACE_MD)
                .py(SPACE_SM)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .bg(theme.chip_bg)
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
                .child(super::icon(
                    crate::assets::Icon::Image,
                    px(14.),
                    theme.text_faint,
                ))
                .child(if alt.is_empty() {
                    SharedString::from("Image")
                } else {
                    alt.clone()
                })
                .into_any_element(),
            Block::Button {
                label,
                url,
                color,
                align,
            } => {
                let fill = color
                    .filter(|c| contrast(*c, theme.text) >= 1.5 || !theme.is_dark)
                    .map(|c| Hsla { a: 1., ..c })
                    .unwrap_or(theme.accent);
                let text = if contrast(fill, gpui::white()) >= contrast(fill, gpui::black()) {
                    gpui::white()
                } else {
                    gpui::black()
                };
                let url = url.clone();
                let tip = url.clone();
                let id = self.next_id;
                self.next_id += 1;
                div()
                    .flex()
                    .map(|d| match align {
                        Align::Left => d,
                        Align::Center => d.justify_center(),
                        Align::Right => d.justify_end(),
                    })
                    .child(
                        div()
                            .id(("html-button", id))
                            .h(px(36.))
                            .px(SPACE_LG)
                            .flex()
                            .items_center()
                            .rounded(RADIUS_MD)
                            .bg(fill)
                            .text_color(text)
                            .text_size(TEXT_BASE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.9))
                            .active(|s| s.opacity(0.8))
                            .child(label.clone())
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .into_any_element()
            }
            Block::Rule => div()
                .h(px(1.))
                .my(SPACE_XS)
                .bg(theme.border)
                .into_any_element(),
        }
    }

    fn table(&mut self, table: &Table) -> AnyElement {
        let theme = self.theme;
        let rows = table.rows.iter().enumerate().map(|(row_ix, row)| {
            let header_row = row.iter().all(|cell| cell.header);
            let cells = table
                .column_weights
                .iter()
                .enumerate()
                .map(|(col, &weight)| {
                    let cell = div()
                        .flex_basis(px(0.))
                        .flex_grow(weight)
                        .min_w_0()
                        .px(SPACE_MD)
                        .py(SPACE_SM);
                    match row.get(col) {
                        Some(c) => cell
                            .map(|d| aligned(d, c.align))
                            // The email's fill, where the text stays readable on it.
                            .when_some(
                                c.background.filter(|bg| contrast(*bg, theme.text) >= 4.5),
                                |d, bg| d.bg(bg),
                            )
                            .when(c.header, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .child(self.blocks(&c.blocks, SPACE_XS)),
                        None => cell,
                    }
                });
            div()
                .flex()
                .when(row_ix > 0, |d| d.border_t_1())
                .border_color(theme.border)
                .when(header_row, |d| {
                    d.bg(theme.chip_bg).text_color(theme.text_muted)
                })
                .children(cells)
        });
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .text_size(TEXT_BASE)
            .line_height(relative(1.45))
            .children(rows)
            .into_any_element()
    }

    fn text(&mut self, inline: &Inline) -> AnyElement {
        let highlights = inline
            .spans
            .iter()
            .map(|(range, style)| (range.clone(), self.highlight(style)))
            .collect::<Vec<_>>();
        let monospace = inline
            .spans
            .iter()
            .filter(|(_, style)| style.code)
            .map(|(range, _)| (range.clone(), SharedString::from(MONO)))
            .collect::<Vec<_>>();
        let text = StyledText::new(inline.text.clone())
            .with_highlights(highlights)
            .with_font_family_overrides(monospace);
        let wrap = self.links(inline);
        let Some(selection) = &self.selection else {
            return wrap(text);
        };
        let n = self.next_text;
        self.next_text += 1;
        Selectable::new(
            ("html-selectable", n as usize),
            inline.text.clone(),
            text,
            wrap,
        )
        .order(selection.order + n)
        .color(self.theme.text_selection)
        .select_all(selection.all.clone())
        .into_any_element()
    }

    /// Makes the text's links clickable, unless the click ended a selection.
    fn links(&mut self, inline: &Inline) -> impl FnOnce(StyledText) -> AnyElement + use<> {
        let links: Option<Rc<[_]>> =
            (!inline.links.is_empty()).then(|| inline.links.clone().into());
        let id = self.next_id;
        if links.is_some() {
            self.next_id += 1;
        }
        move |text| {
            let Some(links) = links else {
                return text.into_any_element();
            };
            let ranges = links.iter().map(|(range, _)| range.clone()).collect();
            InteractiveText::new(("html-text", id), text)
                .on_click(ranges, {
                    let links = links.clone();
                    move |ix, window, cx| {
                        if TextSelection::selected_text(window, cx).is_empty() {
                            cx.open_url(&links[ix].1)
                        }
                    }
                })
                .tooltip(move |ix, window, cx| {
                    let (_, url) = links.iter().find(|(range, _)| range.contains(&ix))?;
                    Some(Tooltip::new(url.clone()).build(window, cx))
                })
                .into_any_element()
        }
    }

    fn highlight(&self, style: &Style) -> HighlightStyle {
        let theme = self.theme;
        let color = style
            .color
            .map(|c| readable(c, theme.is_dark))
            .or(style.link.is_some().then_some(theme.accent));
        let underline = style.underline || style.link.is_some();
        HighlightStyle {
            color,
            font_weight: style.bold.then_some(FontWeight::SEMIBOLD),
            font_style: style.italic.then_some(FontStyle::Italic),
            background_color: style.code.then_some(theme.chip_bg),
            underline: underline.then(|| UnderlineStyle {
                thickness: px(1.),
                color: style
                    .link
                    .is_some()
                    .then(|| color.unwrap_or(theme.accent).alpha(0.4)),
                wavy: false,
            }),
            strikethrough: style.strike.then(|| StrikethroughStyle {
                thickness: px(1.),
                color: None,
            }),
            fade_out: None,
        }
    }
}

fn aligned(div: Div, align: Align) -> Div {
    match align {
        Align::Left => div,
        Align::Center => div.text_center(),
        Align::Right => div.text_right(),
    }
}

/// `color` with its lightness moved just far enough to reach WCAG AA contrast
/// on the reading pane, keeping its hue. A mid gray stays a muted gray in both
/// modes, and dark text from a light-mode email turns light in dark mode.
fn readable(color: Hsla, dark: bool) -> Hsla {
    let background = if dark { 0.012 } else { 1.0 };
    let passes = |c: Hsla| {
        let (a, b) = (luminance(c), background);
        (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5
    };
    let mut c = Hsla { a: 1., ..color };
    if passes(c) {
        return c;
    }
    // Mirror the lightness first, so relative emphasis survives the switch.
    c.l = 1. - c.l;
    if passes(c) {
        return c;
    }
    let (mut failing, mut passing) = (c.l, if dark { 1. } else { 0. });
    for _ in 0..12 {
        c.l = (failing + passing) / 2.;
        if passes(c) {
            passing = c.l;
        } else {
            failing = c.l;
        }
    }
    c.l = passing;
    c
}

/// WCAG contrast ratio between two opaque colors.
fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn luminance(color: Hsla) -> f32 {
    let rgb = color.to_rgb();
    let channel = |v: f32| {
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_colors() {
        let gray: Hsla = gpui::rgb(0x666666).into();
        // Fine on white, too dark on the dark pane.
        assert_eq!(readable(gray, false).l, gray.l);
        assert!(readable(gray, true).l > 0.55);
        let white: Hsla = gpui::rgb(0xffffff).into();
        assert!(readable(white, false).l < 0.5);
    }
}

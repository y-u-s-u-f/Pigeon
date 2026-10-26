//! Renders a parsed email body (`crate::html`) as native GPUI elements.

use std::rc::Rc;

use gpui::{
    AnyElement, Div, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText, IntoElement,
    ParentElement, SharedString, StrikethroughStyle, Styled, StyledText, UnderlineStyle,
    component::tooltip::Tooltip, div, prelude::FluentBuilder as _, px, relative,
};

use crate::ui::style::*;
use crate::{
    html::{Align, Block, Document, Inline, Style, Table, TextKind},
    theme::Theme,
};

const MONO: &str = "Menlo";
const BLOCK_GAP: gpui::Pixels = SPACE_MD;

pub fn render(doc: &Document, theme: &Theme) -> impl IntoElement {
    let mut renderer = Renderer {
        theme,
        next_id: 0,
        list_depth: 0,
    };
    renderer.blocks(&doc.blocks, BLOCK_GAP)
}

struct Renderer<'a> {
    theme: &'a Theme,
    /// Ids for the interactive (linked) texts; unique within the message.
    next_id: usize,
    list_depth: usize,
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
            Block::Image { alt } => div()
                .max_w_full()
                .flex()
                .items_center()
                .justify_center()
                .min_h(px(44.))
                .px(SPACE_MD)
                .py(SPACE_SM)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .bg(theme.chip_bg)
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
                .child(if alt.is_empty() {
                    SharedString::from("Image")
                } else {
                    alt.clone()
                })
                .into_any_element(),
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
        if inline.links.is_empty() {
            return text.into_any_element();
        }

        let links: Rc<[_]> = inline.links.clone().into();
        let ranges = links.iter().map(|(range, _)| range.clone()).collect();
        let id = self.next_id;
        self.next_id += 1;
        InteractiveText::new(("html-text", id), text)
            .on_click(ranges, {
                let links = links.clone();
                move |ix, _, cx| cx.open_url(&links[ix].1)
            })
            .tooltip(move |ix, window, cx| {
                let (_, url) = links.iter().find(|(range, _)| range.contains(&ix))?;
                Some(Tooltip::new(url.clone()).build(window, cx))
            })
            .into_any_element()
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

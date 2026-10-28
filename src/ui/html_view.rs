//! Renders a parsed email body (`crate::html`) as native GPUI elements.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Div, ElementId, FontStyle, FontWeight,
    HighlightStyle, Hsla, InteractiveElement as _, InteractiveText, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, StrikethroughStyle, Styled, StyledImage as _,
    StyledText, UnderlineStyle, Window, base::TextSelection, div, img, prelude::FluentBuilder as _,
    px,
};

use super::selectable::{OnClear, Selectable};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    html::{Align, Block, Document, Inline, Style, Table, TextKind, is_cid},
    theme::{self, Theme, contrast},
};

const BLOCK_GAP: gpui::Pixels = SPACE_MD;

/// Open a link from a message; from its menu (MB-010).
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct OpenLink(pub SharedString);

/// Put text (a link, an address) on the clipboard; from a menu.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct CopyText(pub SharedString);

/// A link's context menu (MB-010): Open and Copy, or the address of a
/// `mailto:`. Not while text is selected, which has its own.
pub fn link_menu(
    url: &SharedString,
    position: gpui::Point<gpui::Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let copy = match url.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => {
            let address = url[7..].split('?').next().unwrap_or_default();
            ("Copy Address", SharedString::from(address.to_string()))
        }
        _ => ("Copy Link", url.clone()),
    };
    gpui::component::native_menu::NativeMenu::new()
        .menu("Open Link", Box::new(OpenLink(url.clone())))
        .menu(copy.0, Box::new(CopyText(copy.1)))
        .show(position, window, cx);
}

pub fn render(doc: &Document, theme: &Theme) -> impl IntoElement {
    render_with(doc, theme, None, None, None)
}

/// Remote images (ATT-1): the messages whose images the user let load, and
/// the images fetched so far, by address. Inline (`cid:`) images too
/// (ATT-2), which need no consent.
#[derive(Default)]
pub struct LoadedImages {
    /// Message ids.
    pub allowed: HashSet<String>,
    /// Addresses asked for, loaded or not, so each goes out once.
    pub requested: HashSet<SharedString>,
    pub ready: HashMap<SharedString, ReadyImage>,
    /// Addresses asked for that came back with nothing to show (MA-075).
    pub failed: HashSet<SharedString>,
}

/// The images a document may show: inline ones always, remote ones once
/// the user let them load.
#[derive(Clone, Copy)]
pub struct ShowImages<'a> {
    pub loaded: &'a LoadedImages,
    pub remote: bool,
}

pub struct ReadyImage {
    pub image: Arc<gpui::Image>,
    /// Its size in pixels.
    pub width: f32,
    pub height: f32,
}

/// Makes a rendered document's text selectable.
pub struct Selection {
    /// Reading order of the document's first text (see `Selectable::order`).
    pub order: u64,
    /// Selects all of it (⌘A) until a click clears it, which calls this.
    pub all: Option<OnClear>,
    /// The link ⌥Tab is on, by its place in `html::links` (MA-003).
    pub focused_link: Option<usize>,
}

pub type OnToggle = Rc<dyn Fn(&mut Window, &mut App)>;

/// The trailing quoted history (ME-003): the blocks from `start` on, shown
/// behind a "•••" button that `on_toggle` opens and closes.
pub struct Fold {
    pub start: usize,
    pub open: bool,
    pub id: ElementId,
    pub on_toggle: OnToggle,
}

/// The document, with selectable text, images and a folded quote when
/// given.
pub fn render_with(
    doc: &Document,
    theme: &Theme,
    selection: Option<Selection>,
    images: Option<ShowImages>,
    fold: Option<Fold>,
) -> impl IntoElement {
    let mut renderer = Renderer {
        theme,
        next_id: 0,
        list_depth: 0,
        focused_link: selection.as_ref().and_then(|s| s.focused_link),
        selection,
        next_text: 0,
        next_link: 0,
        images,
    };
    let start = fold
        .as_ref()
        .map_or(doc.blocks.len(), |f| f.start.min(doc.blocks.len()));
    let (shown, quoted) = doc.blocks.split_at(start);
    let mut blocks = shown
        .iter()
        .enumerate()
        .map(|(ix, b)| renderer.block(b, ix == 0))
        .collect::<Vec<_>>();
    if let Some(fold) = fold
        && !quoted.is_empty()
    {
        blocks.push(quote_toggle(&fold, theme));
        if fold.open {
            blocks.extend(quoted.iter().map(|b| renderer.block(b, false)));
        }
    }
    div().flex().flex_col().gap(BLOCK_GAP).children(blocks)
}

/// The "•••" that shows or hides a message's quoted history.
fn quote_toggle(fold: &Fold, theme: &Theme) -> AnyElement {
    let label = if fold.open {
        "Hide quoted text"
    } else {
        "Show quoted text"
    };
    let on_toggle = fold.on_toggle.clone();
    div()
        .flex()
        .child(
            focusable_control(div().id(fold.id.clone()), theme)
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_expanded(fold.open)
                .h(CONTROL_XS)
                .px(SPACE_SM)
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .bg(theme.chip_bg)
                .hover(|s| s.bg(theme.selected_bg))
                .active(|s| s.opacity(0.8))
                .text_size(TEXT_SM)
                .line_height(LINE_UI)
                .text_color(theme.text_muted)
                .child("•••")
                .tooltip(move |window, cx| super::tooltip(label).build(window, cx))
                .on_click(move |_, window, cx| on_toggle(window, cx)),
        )
        .into_any_element()
}

struct Renderer<'a> {
    theme: &'a Theme,
    /// Ids for the interactive (linked) texts; unique within the message.
    next_id: usize,
    list_depth: usize,
    selection: Option<Selection>,
    /// Counts the selectable texts, for their ids and reading order.
    next_text: u64,
    /// Counts the links, in `html::links` order.
    next_link: usize,
    focused_link: Option<usize>,
    /// The images loaded; the rest show as placeholders.
    images: Option<ShowImages<'a>>,
}

/// Why an image isn't showing (MA-075).
#[derive(Clone, Copy, PartialEq)]
enum Pending {
    /// Waiting for the user, or never loaded here.
    Blocked,
    Loading,
    Failed,
}

impl<'a> Renderer<'a> {
    /// The image to show for `src`, if it may show and has loaded.
    fn ready(&self, src: &str) -> Option<&'a ReadyImage> {
        let images = self.images?;
        if !images.remote && !is_cid(src) {
            return None;
        }
        images.loaded.ready.get(src)
    }

    fn pending(&self, src: &SharedString) -> Pending {
        let Some(images) = self.images else {
            return Pending::Blocked;
        };
        if images.loaded.failed.contains(src) {
            Pending::Failed
        } else if (images.remote || is_cid(src)) && images.loaded.requested.contains(src) {
            Pending::Loading
        } else {
            Pending::Blocked
        }
    }

    /// The next link's place, and whether ⌥Tab is on it.
    fn take_link(&mut self) -> bool {
        let n = self.next_link;
        self.next_link += 1;
        self.focused_link == Some(n)
    }

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
                    TextKind::Paragraph => el.child(self.text(text, None)),
                    TextKind::Heading(level) => el
                        .when(!first, |d| d.pt(SPACE_SM))
                        .text_size(match level {
                            1 => TEXT_TITLE2,
                            2 => TEXT_LG,
                            _ => TEXT_MD,
                        })
                        .line_height(LINE_UI)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.text(text, Some(*level as usize))),
                }
                .into_any_element()
            }
            Block::Pre(text) => div()
                .px(SPACE_MD)
                .py(SPACE_MD)
                .rounded(RADIUS_MD)
                .bg(theme.chip_bg)
                .font_family(MONO)
                .text_size(TEXT_SM)
                .line_height(LINE_READING)
                .child(self.text(text, None))
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
                                        .min_w(SPACE_LG + SPACE_XS)
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
            Block::Image {
                alt,
                src,
                width,
                height,
                link,
                align,
            } => {
                let focused = link.is_some() && self.take_link();
                let id = self.next_id;
                self.next_id += 1;
                let picture = match self.ready(src) {
                    Some(ready) => {
                        let shown_width = width.unwrap_or(ready.width).max(1.);
                        let ratio = match (width, height) {
                            (Some(w), Some(h)) if *h > 0. => w / h,
                            _ => ready.width / ready.height.max(1.),
                        };
                        div()
                            .id(("html-image", id))
                            .w_full()
                            .max_w(px(shown_width))
                            .aspect_ratio(ratio)
                            // Bright banners would glow on the dark pane (MA-033).
                            .when(theme.is_dark, |d| d.opacity(0.9))
                            .child(
                                img(ready.image.clone())
                                    .size_full()
                                    .object_fit(gpui::ObjectFit::Contain),
                            )
                    }
                    None => self.placeholder(id, src, alt, *width, *height),
                };
                let picture = match link {
                    Some(url) => {
                        let tip = url.clone();
                        let url = url.clone();
                        picture
                            .role(gpui::Role::Link)
                            .aria_label(match alt.as_ref().filter(|a| !a.is_empty()) {
                                Some(alt) => alt.clone(),
                                None => url.clone(),
                            })
                            .rounded(RADIUS_SM)
                            .cursor_pointer()
                            .when(focused, |d| d.shadow(focus_ring(theme)))
                            .tooltip(move |window, cx| {
                                super::tooltip(tip.clone()).build(window, cx)
                            })
                            .on_any_mouse_down(link_mouse_down(url.clone()))
                            .on_click(move |_, _, cx| open_link(&url, cx))
                    }
                    // Decorative: an empty `alt` says there's nothing to read (MA-044).
                    None if alt.as_ref().is_some_and(|a| a.is_empty()) => picture,
                    None => picture
                        .role(gpui::Role::Image)
                        .aria_label(alt.clone().unwrap_or_else(|| "Image".into())),
                };
                div()
                    .flex()
                    .map(|d| match align {
                        Align::Left => d,
                        Align::Center => d.justify_center(),
                        Align::Right => d.justify_end(),
                    })
                    .child(picture)
                    .into_any_element()
            }
            Block::Button {
                label,
                url,
                color,
                align,
            } => {
                let focused = self.take_link();
                let fill = color
                    .filter(|c| contrast(*c, theme.text) >= 1.5 || !theme.is_dark)
                    .map(|c| Hsla { a: 1., ..c })
                    .unwrap_or(theme.accent);
                let text = theme::on_fill(fill);
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
                            .role(gpui::Role::Link)
                            .aria_label(label.clone())
                            .min_h(CONTROL_LG)
                            .py(SPACE_SM)
                            .px(SPACE_LG)
                            .flex()
                            .items_center()
                            .rounded(RADIUS_MD)
                            .bg(fill)
                            .text_color(text)
                            .text_size(TEXT_BASE)
                            .line_height(LINE_UI)
                            .font_weight(FontWeight::SEMIBOLD)
                            .hover(|s| s.opacity(0.9))
                            .active(|s| s.opacity(0.8))
                            .cursor_pointer()
                            .when(focused, |d| d.shadow(focus_ring(theme)))
                            .child(label.clone())
                            .tooltip(move |window, cx| {
                                super::tooltip(tip.clone()).build(window, cx)
                            })
                            .on_any_mouse_down(link_mouse_down(url.clone()))
                            .on_click(move |_, _, cx| open_link(&url, cx)),
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

    /// An image not showing: sized like the image when its size is known,
    /// so text doesn't jump when it arrives (MA-075).
    fn placeholder(
        &self,
        id: usize,
        src: &SharedString,
        alt: &Option<SharedString>,
        width: Option<f32>,
        height: Option<f32>,
    ) -> gpui::Stateful<Div> {
        let theme = self.theme;
        let pending = self.pending(src);
        let glyph = super::icon(
            if pending == Pending::Failed {
                Icon::OctagonAlert
            } else {
                Icon::Image
            },
            ICON_MD,
            theme.text_faint,
        );
        let glyph = if pending == Pending::Loading {
            div()
                .child(glyph)
                .with_animation(
                    ("html-image-loading", id),
                    Animation::new(std::time::Duration::from_millis(1200))
                        .repeat()
                        .with_easing(gpui::pulsating_between(0.3, 1.)),
                    |d, t| d.opacity(t),
                )
                .into_any_element()
        } else {
            glyph.into_any_element()
        };
        let text = match (pending, alt.as_ref().filter(|a| !a.is_empty())) {
            (Pending::Failed, Some(alt)) => format!("{alt} (didn't load)").into(),
            (Pending::Failed, None) => SharedString::from("Image didn't load"),
            (_, Some(alt)) => alt.clone(),
            (_, None) => SharedString::from("Image"),
        };
        div()
            .id(("html-image", id))
            .max_w_full()
            .map(|d| match (width, height) {
                (Some(w), Some(h)) if w > 0. && h > 0. => {
                    d.w_full().max_w(px(w)).aspect_ratio(w / h)
                }
                _ => d.min_h(CONTROL_LG).py(SPACE_SM),
            })
            .flex()
            .items_center()
            .justify_center()
            .gap(SPACE_SM)
            .px(SPACE_MD)
            .overflow_hidden()
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(theme.border)
            .bg(theme.chip_bg)
            .text_size(TEXT_SM)
            .text_color(theme.text_faint)
            .child(glyph)
            .child(div().min_w_0().truncate().child(text))
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
            .line_height(LINE_READING)
            .children(rows)
            .into_any_element()
    }

    /// A run of text; `heading` gives its level when it is one.
    fn text(&mut self, inline: &Inline, heading: Option<usize>) -> AnyElement {
        // The link ⌥Tab is on, if it's in this text.
        let first_link = self.next_link;
        self.next_link += inline.links.len();
        let focused = self
            .focused_link
            .and_then(|n| n.checked_sub(first_link))
            .and_then(|n| inline.links.get(n))
            .map(|(range, _)| range.clone());
        let highlights = inline
            .spans
            .iter()
            .map(|(range, style)| {
                let mut highlight = self.highlight(style);
                if focused
                    .as_ref()
                    .is_some_and(|f| f.start <= range.start && range.end <= f.end)
                {
                    highlight.background_color = Some(self.theme.selected_bg);
                }
                (range.clone(), highlight)
            })
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
        let selectable = Selectable::new(
            ("html-selectable", n as usize),
            inline.text.clone(),
            text,
            wrap,
        )
        .order(selection.order + n)
        .color(self.theme.text_selection)
        .select_all(selection.all.clone())
        .links(inline.links.clone());
        match heading {
            Some(level) => selectable.heading(level),
            None => selectable,
        }
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
            let layout = text.layout().clone();
            let menu_links = links.clone();
            let text = InteractiveText::new(("html-text", id), text)
                .on_click(ranges, {
                    let links = links.clone();
                    move |ix, window, cx| {
                        if TextSelection::selected_text(window, cx).is_empty() {
                            open_link(&links[ix].1, cx)
                        }
                    }
                })
                .tooltip(move |ix, window, cx| {
                    let (_, url) = links.iter().find(|(range, _)| range.contains(&ix))?;
                    Some(super::tooltip(url.clone()).build(window, cx))
                });
            div()
                .child(text)
                .on_any_mouse_down(move |event, window, cx| {
                    if !super::is_secondary(event)
                        || !TextSelection::selected_text(window, cx).is_empty()
                    {
                        return;
                    }
                    let Ok(ix) = layout.index_for_position(event.position) else {
                        return;
                    };
                    if let Some((_, url)) = menu_links.iter().find(|(range, _)| range.contains(&ix))
                    {
                        cx.stop_propagation();
                        link_menu(url, event.position, window, cx);
                    }
                })
                .into_any_element()
        }
    }

    fn highlight(&self, style: &Style) -> HighlightStyle {
        let theme = self.theme;
        let color = style
            .color
            .map(|c| {
                // Corrected once per background, not every frame (MC-050).
                let background = theme.solid(theme.reader_bg);
                match style.readable.get() {
                    Some((bg, shown)) if bg == background => shown,
                    _ => {
                        let shown = readable(c, background);
                        style.readable.set(Some((background, shown)));
                        shown
                    }
                }
            })
            .or(style.link.is_some().then_some(theme.link));
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
                    .then(|| color.unwrap_or(theme.link).alpha(0.4)),
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
/// on the reading pane (`background`, opaque), keeping its hue. A mid gray stays a muted gray in both
/// modes, and dark text from a light-mode email turns light in dark mode.
fn readable(color: Hsla, background: Hsla) -> Hsla {
    let c = Hsla { a: 1., ..color };
    if contrast(c, background) >= 4.5 {
        return c;
    }
    // Mirror the lightness first, so relative emphasis survives the switch.
    theme::readable(Hsla { l: 1. - c.l, ..c }, background, 4.5)
}

/// A button or linked image's right-click: its link's menu.
fn link_mouse_down(
    url: SharedString,
) -> impl Fn(&gpui::MouseDownEvent, &mut Window, &mut App) + 'static {
    move |event, window, cx| {
        if super::is_secondary(event) {
            cx.stop_propagation();
            link_menu(&url, event.position, window, cx);
        }
    }
}

/// W3 (LNK-3): `mailto:` opens Pigeon's composer; other links the browser.
pub fn open_link(url: &str, cx: &mut gpui::App) {
    use crate::app::shell::{self, Mailto, Show};
    match Mailto::parse(url) {
        Some(mailto) => shell::show(Show::Compose(Some(mailto)), cx),
        None => cx.open_url(url),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_colors() {
        let dark_pane: Hsla = gpui::rgb(0x18181c).into();
        let gray: Hsla = gpui::rgb(0x666666).into();
        // Fine on white, too dark on the dark pane.
        assert_eq!(readable(gray, gpui::white()).l, gray.l);
        assert!(readable(gray, dark_pane).l > 0.55);
        let white: Hsla = gpui::rgb(0xffffff).into();
        assert!(readable(white, gpui::white()).l < 0.5);
    }
}

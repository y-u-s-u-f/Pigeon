//! Email HTML, parsed once into a small tree of blocks with styled inline
//! runs. `ui::html_view` renders it. Nothing here runs scripts or loads URLs;
//! images keep their `src` for the reader to load when the user asks (ATT-1).

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash as _, Hasher as _},
    ops::Range,
    rc::Rc,
};

use gpui::{Hsla, Rgba, SharedString};
use html5ever::{Attribute, parse_document, tendril::TendrilSink as _};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

#[derive(Debug, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// The remote and inline (`cid:`) images it shows, each once, in
    /// order; found once at parse time (MC-049).
    pub remote_images: Vec<SharedString>,
    pub cid_images: Vec<SharedString>,
}

#[derive(Debug)]
pub enum Block {
    Text {
        text: Inline,
        kind: TextKind,
        align: Align,
    },
    /// Preformatted text: whitespace kept, set in a monospace font.
    Pre(Inline),
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Table(Table),
    Image {
        /// `None` when the `alt` attribute is missing; empty when the image
        /// is marked decorative.
        alt: Option<SharedString>,
        /// A safe `http(s)` address (see `is_safe_image_url`) or a `cid:`
        /// reference; empty otherwise.
        src: SharedString,
        /// The `width` and `height` attributes, in px.
        width: Option<f32>,
        height: Option<f32>,
        /// The link it sits in.
        link: Option<SharedString>,
        align: Align,
    },
    /// A link styled as a button: a call to action.
    Button {
        label: SharedString,
        url: SharedString,
        /// The email's own fill; the renderer keeps it when it's readable.
        color: Option<Hsla>,
        align: Align,
    },
    Rule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Paragraph,
    Heading(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug)]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
    /// Relative width of each column, from the longest text in it.
    pub column_weights: Vec<f32>,
}

#[derive(Debug)]
pub struct Cell {
    pub header: bool,
    pub align: Align,
    /// The cell's (or its row's) own background color.
    pub background: Option<Hsla>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Default)]
pub struct Inline {
    pub text: SharedString,
    /// Sorted and non-overlapping; text outside them is plain.
    pub spans: Vec<(Range<usize>, Style)>,
    /// Sorted ranges of `text` that are links, with their URLs.
    pub links: Vec<(Range<usize>, SharedString)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    /// The email's own color; the renderer adjusts it for contrast.
    pub color: Option<Hsla>,
    pub link: Option<SharedString>,
    /// The renderer's last contrast correction of `color`, as (background,
    /// result), so it runs once per theme rather than every frame (MC-050).
    pub readable: std::cell::Cell<Option<(Hsla, Hsla)>>,
}

/// Parsed bodies, filled the first time a message is shown, so each body is
/// parsed once. Keyed by message id and a cheap fingerprint of the HTML
/// (MC-049), so a body that arrives or changes gets a new entry. Bounded:
/// cleared when it holds more than `CACHE_CAP` documents.
#[derive(Default)]
pub struct HtmlCache {
    docs: RefCell<HashMap<CacheKey, Rc<Document>>>,
    /// Big bodies being parsed off the main thread (MC-054).
    parsing: RefCell<HashSet<CacheKey>>,
}

pub type CacheKey = (String, u64);

const CACHE_CAP: usize = 300;

/// Bodies at least this long are parsed in the background.
pub const BACKGROUND_PARSE: usize = 64 * 1024;

impl HtmlCache {
    pub fn key(id: &str, html: &str) -> CacheKey {
        // The length plus both ends: enough to tell a loaded body from
        // none, or an edited one, without hashing a whole newsletter.
        let mut hasher = DefaultHasher::new();
        html.len().hash(&mut hasher);
        let head = html.floor_char_boundary(html.len().min(256));
        let tail = html.ceil_char_boundary(html.len().saturating_sub(256));
        html[..head].hash(&mut hasher);
        html[tail..].hash(&mut hasher);
        (id.to_string(), hasher.finish())
    }

    /// The parsed body, parsing it here if it's small. A big one returns
    /// `None` and is parsed by `start_parse`'s caller.
    pub fn get(&self, id: &str, html: &str) -> Option<Rc<Document>> {
        let key = Self::key(id, html);
        if let Some(doc) = self.docs.borrow().get(&key) {
            return Some(doc.clone());
        }
        if html.len() >= BACKGROUND_PARSE {
            return None;
        }
        let doc = Rc::new(parse(html));
        self.insert(key, doc.clone());
        Some(doc)
    }

    /// Whether to start a background parse for `key`: true once.
    pub fn start_parse(&self, key: &CacheKey) -> bool {
        self.parsing.borrow_mut().insert(key.clone())
    }

    pub fn insert(&self, key: CacheKey, doc: Rc<Document>) {
        self.parsing.borrow_mut().remove(&key);
        let mut docs = self.docs.borrow_mut();
        if docs.len() >= CACHE_CAP {
            docs.clear();
        }
        docs.insert(key, doc);
    }
}

pub fn parse(html: &str) -> Document {
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let blocks = parse_blocks(&dom.document, &Ctx::default());
    Document {
        remote_images: images(&blocks, is_safe_image_url),
        cid_images: images(&blocks, is_cid),
        blocks,
    }
}

/// Where the trailing quoted history starts (ME-003): a last `Quote`, with
/// the "On … wrote:" line before it, or an Outlook "Original Message"
/// header and all after it. `None` when there is none, or when it would
/// leave nothing above it.
pub fn trailing_quote_start(doc: &Document) -> Option<usize> {
    let blocks = &doc.blocks;
    let is_text = |b: &Block, f: &dyn Fn(&str) -> bool| match b {
        Block::Text { text, .. } => f(text.text.trim()),
        _ => false,
    };
    let start = if let Some(ix) = blocks.iter().position(|b| {
        is_text(b, &|t| {
            t.starts_with("-----Original Message-----") || t.starts_with("———— Original Message")
        })
    }) {
        ix
    } else if matches!(blocks.last(), Some(Block::Quote(_))) {
        let quote = blocks.len() - 1;
        match quote.checked_sub(1) {
            Some(before) if is_text(&blocks[before], &is_attribution) => before,
            _ => quote,
        }
    } else {
        return None;
    };
    (start > 0).then_some(start)
}

/// "On Tue, Oct 27, 2026 at 9:12 AM Ana <ana@x.io> wrote:", and the like.
fn is_attribution(text: &str) -> bool {
    let last = text.lines().last().unwrap_or_default().trim_end();
    text.len() < 400
        && [
            "wrote:",
            "schrieb:",
            "a écrit :",
            "a écrit:",
            "escribió:",
            "scrisse:",
        ]
        .iter()
        .any(|end| last.ends_with(end))
}

/// The links of `blocks` in reading order, as the renderer draws them:
/// inline links, buttons and linked images (MA-003).
pub fn links(blocks: &[Block]) -> Vec<SharedString> {
    fn walk(blocks: &[Block], out: &mut Vec<SharedString>) {
        for block in blocks {
            match block {
                Block::Text { text, .. } | Block::Pre(text) => {
                    out.extend(text.links.iter().map(|(_, url)| url.clone()))
                }
                Block::List { items, .. } => items.iter().for_each(|item| walk(item, out)),
                Block::Quote(blocks) => walk(blocks, out),
                Block::Table(table) => table
                    .rows
                    .iter()
                    .flatten()
                    .for_each(|cell| walk(&cell.blocks, out)),
                Block::Image {
                    link: Some(url), ..
                } => out.push(url.clone()),
                Block::Button { url, .. } => out.push(url.clone()),
                Block::Image { .. } | Block::Rule => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

/// The document's text, one block per paragraph, for search and for editing a
/// draft as plain text.
pub fn plain_text(doc: &Document) -> String {
    fn push_blocks(blocks: &[Block], out: &mut String) {
        for block in blocks {
            match block {
                Block::Text { text, .. } | Block::Pre(text) => push_par(&text.text, out),
                Block::List { items, .. } => {
                    for item in items {
                        push_blocks(item, out);
                    }
                }
                Block::Quote(blocks) => push_blocks(blocks, out),
                Block::Table(table) => {
                    for cell in table.rows.iter().flatten() {
                        push_blocks(&cell.blocks, out);
                    }
                }
                Block::Image { alt, .. } => push_par(alt.as_deref().unwrap_or_default(), out),
                Block::Button { label, .. } => push_par(label, out),
                Block::Rule => {}
            }
        }
    }
    fn push_par(text: &str, out: &mut String) {
        let text = text.trim();
        if !text.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(text);
        }
    }
    let mut out = String::new();
    push_blocks(&doc.blocks, &mut out);
    out
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Plain text as HTML: blank lines separate paragraphs, single newlines break
/// lines.
pub fn from_plain_text(text: &str) -> String {
    text.split("\n\n")
        .map(|p| p.trim_matches('\n'))
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("<p>{}</p>", escape(p).replace('\n', "<br>")))
        .collect()
}

/// What an element passes down to its children.
#[derive(Clone, Debug, Default)]
struct Ctx {
    style: Style,
    align: Align,
    pre: bool,
    /// Set by `tr`, `td` and `th` for their cells.
    background: Option<Hsla>,
}

/// Block-level elements: they end the current paragraph and start a new one.
const BLOCK_TAGS: &[&str] = &[
    "html",
    "body",
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "center",
    "figure",
    "figcaption",
    "address",
    "dl",
    "dt",
    "dd",
    "details",
    "summary",
    "form",
    "fieldset",
    "li",
    "tr",
    "td",
    "th",
    "thead",
    "tbody",
    "tfoot",
    "caption",
];

/// Never rendered, and their text is never shown.
const SKIPPED_TAGS: &[&str] = &[
    "head", "title", "meta", "link", "script", "style", "template", "noscript", "iframe", "object",
    "embed", "svg", "canvas", "button", "input", "select", "textarea",
];

fn parse_blocks(node: &Handle, ctx: &Ctx) -> Vec<Block> {
    let mut parser = Parser::default();
    parser.walk_children(node, ctx);
    parser.flush(TextKind::Paragraph, ctx.align);
    parser.blocks
}

#[derive(Default)]
struct Parser {
    blocks: Vec<Block>,
    inline: InlineBuilder,
}

impl Parser {
    fn flush(&mut self, kind: TextKind, align: Align) {
        if let Some(text) = self.inline.take() {
            // Hebrew or Arabic with no alignment of its own reads from the
            // right (MA-042).
            let align = if align == Align::Left && starts_rtl(&text.text) {
                Align::Right
            } else {
                align
            };
            self.blocks.push(Block::Text { text, kind, align });
        }
    }

    fn walk_children(&mut self, node: &Handle, ctx: &Ctx) {
        for child in node.children.borrow().iter() {
            self.walk(child, ctx);
        }
    }

    fn walk(&mut self, node: &Handle, parent: &Ctx) {
        let (name, attrs) = match &node.data {
            NodeData::Text { contents } => {
                let text = contents.borrow();
                if parent.pre {
                    self.inline.push(&text, &parent.style);
                } else if parent.style.link.is_some() {
                    self.inline.push_collapsed(&text, &parent.style);
                } else {
                    self.push_autolinked(&text, &parent.style);
                }
                return;
            }
            NodeData::Document => return self.walk_children(node, parent),
            NodeData::Element { name, attrs, .. } => (name, attrs.borrow()),
            _ => return,
        };
        let tag = name.local.as_ref();
        if SKIPPED_TAGS.contains(&tag) {
            return;
        }
        let ctx = &element_ctx(tag, &attrs, parent);

        match tag {
            "br" => self.inline.push("\n", &ctx.style),
            "b" | "strong" => self.walk_styled(node, ctx, |s| s.bold = true),
            "i" | "em" | "cite" => self.walk_styled(node, ctx, |s| s.italic = true),
            "u" | "ins" => self.walk_styled(node, ctx, |s| s.underline = true),
            "s" | "strike" | "del" => self.walk_styled(node, ctx, |s| s.strike = true),
            "code" | "kbd" | "tt" | "samp" => self.walk_styled(node, ctx, |s| s.code = true),
            "a" => match attr(&attrs, "href").filter(|href| is_safe_url(href)) {
                Some(href) if is_button(&attrs) => {
                    let label = plain_text(&Document {
                        blocks: parse_blocks(node, ctx),
                        ..Default::default()
                    });
                    if label.is_empty() {
                        return;
                    }
                    self.flush(TextKind::Paragraph, parent.align);
                    let css = attr(&attrs, "style").unwrap_or_default();
                    self.blocks.push(Block::Button {
                        label: label.replace("\n\n", " ").into(),
                        url: href.trim().to_string().into(),
                        color: background(css, &attrs),
                        align: parent.align,
                    });
                }
                Some(href) => {
                    let href = SharedString::from(href.trim().to_string());
                    self.walk_styled(node, ctx, |s| s.link = Some(href));
                }
                None => self.walk_children(node, ctx),
            },
            "img" => {
                // Tracking pixels and spacers carry nothing worth a
                // placeholder, and are never loaded.
                let css = attr(&attrs, "style").unwrap_or_default();
                let size = |name| {
                    attr(&attrs, name)
                        .or_else(|| css_value(css, name))
                        .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
                };
                let (width, height) = (size("width"), size("height"));
                let hidden = css_value(css, "display").is_some_and(|v| v.contains("none"));
                if width.is_some_and(|v| v <= 2.) || height.is_some_and(|v| v <= 2.) || hidden {
                    return;
                }
                self.flush(TextKind::Paragraph, parent.align);
                let alt = attr(&attrs, "alt").map(|alt| SharedString::from(alt.trim().to_string()));
                let src = attr(&attrs, "src")
                    .map(str::trim)
                    .filter(|src| is_safe_image_url(src) || is_cid(src))
                    .unwrap_or_default();
                self.blocks.push(Block::Image {
                    alt,
                    src: src.to_string().into(),
                    width,
                    height,
                    link: ctx.style.link.clone(),
                    align: parent.align,
                });
            }
            "hr" => {
                self.flush(TextKind::Paragraph, parent.align);
                self.blocks.push(Block::Rule);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush(TextKind::Paragraph, parent.align);
                self.walk_children(node, ctx);
                let level = tag.as_bytes()[1] - b'0';
                self.flush(TextKind::Heading(level), ctx.align);
            }
            "ul" | "ol" => {
                self.flush(TextKind::Paragraph, parent.align);
                let mut items: Vec<Vec<Block>> = Vec::new();
                for child in node.children.borrow().iter() {
                    let is_li = matches!(&child.data,
                        NodeData::Element { name, .. } if name.local.as_ref() == "li");
                    let blocks = parse_blocks(child, ctx);
                    match items.last_mut() {
                        // Stray content between items belongs to the one before.
                        Some(last) if !is_li => last.extend(blocks),
                        _ if is_li || !blocks.is_empty() => items.push(blocks),
                        _ => {}
                    }
                }
                if !items.is_empty() {
                    self.blocks.push(Block::List {
                        ordered: tag == "ol",
                        items,
                    });
                }
            }
            "blockquote" => {
                self.flush(TextKind::Paragraph, parent.align);
                let blocks = parse_blocks(node, ctx);
                if !blocks.is_empty() {
                    self.blocks.push(Block::Quote(blocks));
                }
            }
            "pre" => {
                self.flush(TextKind::Paragraph, parent.align);
                let mut pre = Parser::default();
                pre.walk_children(
                    node,
                    &Ctx {
                        pre: true,
                        ..ctx.clone()
                    },
                );
                self.blocks.extend(pre.blocks);
                if let Some(text) = pre.inline.take() {
                    self.blocks.push(Block::Pre(text));
                }
            }
            "table" => {
                self.flush(TextKind::Paragraph, parent.align);
                self.table(node, ctx);
            }
            // Gmail's quoted history (ME-003). A forward's is its content,
            // and one around a `blockquote` is quoted already.
            "div" if has_class(&attrs, "gmail_quote") => {
                self.flush(TextKind::Paragraph, parent.align);
                let blocks = parse_blocks(node, ctx);
                let forward = matches!(blocks.first(), Some(Block::Text { text, .. })
                    if text.text.contains("Forwarded message"));
                if forward || blocks.iter().any(|b| matches!(b, Block::Quote(_))) {
                    self.blocks.extend(blocks);
                } else if !blocks.is_empty() {
                    self.blocks.push(Block::Quote(blocks));
                }
            }
            _ if BLOCK_TAGS.contains(&tag) => {
                self.flush(TextKind::Paragraph, parent.align);
                self.walk_children(node, ctx);
                self.flush(TextKind::Paragraph, ctx.align);
            }
            // Unknown tags are inline containers, so their text is kept.
            _ => self.walk_children(node, ctx),
        }
    }

    /// Text with its bare addresses ("example.com/path", "https://…") made
    /// into links.
    fn push_autolinked(&mut self, text: &str, style: &Style) {
        let mut rest = 0;
        for (range, url) in find_urls(text) {
            self.inline.push_collapsed(&text[rest..range.start], style);
            let linked = Style {
                link: Some(url.into()),
                ..style.clone()
            };
            self.inline.push_collapsed(&text[range.clone()], &linked);
            rest = range.end;
        }
        self.inline.push_collapsed(&text[rest..], style);
    }

    fn walk_styled(&mut self, node: &Handle, ctx: &Ctx, f: impl FnOnce(&mut Style)) {
        let mut ctx = ctx.clone();
        f(&mut ctx.style);
        self.walk_children(node, &ctx);
    }

    fn table(&mut self, node: &Handle, ctx: &Ctx) {
        let mut rows = Vec::new();
        collect_rows(node, ctx, &mut rows);

        // A one-column table is layout, as most newsletters use them: keep
        // only its content. A filled cell holding just a link is a button.
        if rows.iter().all(|row| row.len() <= 1) {
            for cell in rows.into_iter().flatten() {
                match (cell.background, &cell.blocks[..]) {
                    (Some(color), [Block::Text { text, align, .. }])
                        if text.links.len() == 1 && text.links[0].0 == (0..text.text.len()) =>
                    {
                        self.blocks.push(Block::Button {
                            label: text.text.clone(),
                            url: text.links[0].1.clone(),
                            color: Some(color),
                            align: *align,
                        })
                    }
                    _ => self.blocks.extend(cell.blocks),
                }
            }
            return;
        }

        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        // A header over a right-aligned column (amounts, prices) follows it.
        for col in 0..columns {
            let body = || {
                rows.iter()
                    .filter_map(|row| row.get(col))
                    .filter(|c| !c.header)
            };
            if body().next().is_some() && body().all(|c| c.align == Align::Right) {
                for row in &mut rows {
                    if let Some(cell) = row.get_mut(col).filter(|c| c.header) {
                        cell.align = Align::Right;
                    }
                }
            }
        }
        let column_weights = (0..columns)
            .map(|col| {
                let longest = rows
                    .iter()
                    .filter_map(|row| row.get(col))
                    .map(|cell| text_len(&cell.blocks))
                    .max()
                    .unwrap_or(0);
                longest.clamp(4, 40) as f32
            })
            .collect();
        self.blocks.push(Block::Table(Table {
            rows,
            column_weights,
        }));
    }
}

/// The rows of a table, looking through `thead`/`tbody`/`tfoot` but not into
/// nested tables, which become part of their cell.
fn collect_rows(node: &Handle, ctx: &Ctx, rows: &mut Vec<Vec<Cell>>) {
    for child in node.children.borrow().iter() {
        let NodeData::Element { name, attrs, .. } = &child.data else {
            continue;
        };
        match name.local.as_ref() {
            "thead" | "tbody" | "tfoot" => collect_rows(child, ctx, rows),
            "tr" => {
                let row_ctx = element_ctx("tr", &attrs.borrow(), ctx);
                let cells: Vec<Cell> = child
                    .children
                    .borrow()
                    .iter()
                    .filter_map(|cell| {
                        let NodeData::Element { name, attrs, .. } = &cell.data else {
                            return None;
                        };
                        let tag = name.local.as_ref();
                        if tag != "td" && tag != "th" {
                            return None;
                        }
                        let ctx = element_ctx(tag, &attrs.borrow(), &row_ctx);
                        Some(Cell {
                            header: tag == "th",
                            align: ctx.align,
                            background: ctx.background,
                            blocks: parse_blocks(cell, &ctx),
                        })
                    })
                    .collect();
                if !cells.is_empty() {
                    rows.push(cells);
                }
            }
            _ => {}
        }
    }
}

fn text_len(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            Block::Text { text, .. } | Block::Pre(text) => text.text.chars().count(),
            Block::List { items, .. } => items.iter().map(|item| text_len(item)).sum(),
            Block::Quote(blocks) => text_len(blocks),
            Block::Table(table) => table
                .rows
                .iter()
                .flatten()
                .map(|c| text_len(&c.blocks))
                .sum(),
            Block::Image { alt, .. } => alt.as_ref().map_or(0, |alt| alt.len()),
            Block::Button { label, .. } => label.len(),
            Block::Rule => 0,
        })
        .sum()
}

/// The context for an element's children: its inline `style`, `color` and
/// `align` attributes applied over its parent's.
fn element_ctx(tag: &str, attrs: &[Attribute], parent: &Ctx) -> Ctx {
    let mut ctx = parent.clone();
    let css = attr(attrs, "style").unwrap_or_default();
    let color =
        css_value(css, "color").or_else(|| (tag == "font").then(|| attr(attrs, "color"))?);
    if let Some(color) = color.and_then(parse_color) {
        ctx.style.color = Some(color);
    }
    if let Some(weight) = css_value(css, "font-weight") {
        ctx.style.bold = matches!(weight, "bold" | "bolder" | "600" | "700" | "800" | "900");
    }
    if css_value(css, "font-style") == Some("italic") {
        ctx.style.italic = true;
    }
    if css_value(css, "text-decoration").is_some_and(|v| v.contains("underline")) {
        ctx.style.underline = true;
    }
    if matches!(tag, "tr" | "td" | "th")
        && let Some(color) = background(css, attrs)
    {
        ctx.background = Some(color);
    }
    if attr(attrs, "dir").is_some_and(|d| d.eq_ignore_ascii_case("rtl")) {
        ctx.align = Align::Right;
    }
    let align = css_value(css, "text-align").or_else(|| attr(attrs, "align"));
    match align.map(str::to_ascii_lowercase).as_deref() {
        Some("right" | "end") => ctx.align = Align::Right,
        Some("center") => ctx.align = Align::Center,
        Some("left" | "start" | "justify") => ctx.align = Align::Left,
        _ => {}
    }
    ctx
}

fn attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name.local.as_ref() == name)
        .map(|a| a.value.as_ref())
}

/// Whether the first strongly directional letter is Hebrew or Arabic.
fn starts_rtl(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF))
}

fn has_class(attrs: &[Attribute], class: &str) -> bool {
    attr(attrs, "class").is_some_and(|c| c.split_whitespace().any(|c| c == class))
}

/// One declaration's value from an inline `style` attribute.
fn css_value<'a>(css: &'a str, property: &str) -> Option<&'a str> {
    css.split(';').rev().find_map(|decl| {
        let (key, value) = decl.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(property)
            .then(|| value.trim().trim_end_matches("!important").trim())
    })
}

/// An element's background color, from `bgcolor` or its inline style.
fn background(css: &str, attrs: &[Attribute]) -> Option<Hsla> {
    css_value(css, "background-color")
        .or_else(|| css_value(css, "background"))
        .or_else(|| attr(attrs, "bgcolor"))
        .and_then(parse_color)
}

/// A link dressed as a button: filled, or padded with a border, or named so.
fn is_button(attrs: &[Attribute]) -> bool {
    let css = attr(attrs, "style").unwrap_or_default();
    let class = attr(attrs, "class")
        .unwrap_or_default()
        .to_ascii_lowercase();
    background(css, attrs).is_some()
        || (css_value(css, "padding").is_some() && css_value(css, "border-radius").is_some())
        || class
            .split_whitespace()
            .any(|c| c.contains("button") || c.contains("btn") || c == "cta")
}

/// Web addresses written out in text, with the URL each opens. Takes
/// "https://…", "www.…" and lowercase "name.tld/path" for common TLDs;
/// trailing punctuation stays outside.
fn find_urls(text: &str) -> Vec<(Range<usize>, String)> {
    const TLDS: &[&str] = &[
        "com", "org", "net", "io", "dev", "app", "co", "me", "ai", "edu", "gov", "us", "uk", "de",
        "fr", "es", "ca",
    ];
    let mut found = Vec::new();
    let mut start = None;
    let ends = text
        .char_indices()
        .map(|(ix, c)| (ix, c.is_whitespace()))
        .chain([(text.len(), true)]);
    for (ix, space) in ends {
        match (space, start) {
            (false, None) => start = Some(ix),
            (true, Some(from)) => {
                start = None;
                let token = &text[from..ix];
                let lead = token.len() - token.trim_start_matches(['(', '<', '[', '"', '\'']).len();
                let token = token[lead..].trim_end_matches(|c: char| ".,;:!?)]}>'\"".contains(c));
                if token.is_empty() || token.contains('@') {
                    continue;
                }
                let lower = token.to_ascii_lowercase();
                let url = if lower.starts_with("https://") || lower.starts_with("http://") {
                    token.to_string()
                } else {
                    let host = token.split('/').next().unwrap_or_default();
                    let labels: Vec<&str> = host.split('.').collect();
                    let is_host = labels.len() >= 2
                        && labels.iter().all(|l| {
                            !l.is_empty()
                                && l.chars().all(|c| {
                                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
                                })
                        })
                        && TLDS.contains(labels.last().expect("two or more"));
                    if !is_host {
                        continue;
                    }
                    format!("https://{token}")
                };
                let begin = from + lead;
                found.push((begin..begin + token.len(), url));
            }
            _ => {}
        }
    }
    found
}

/// The images of `blocks` whose address passes `keep`, each once, in order.
fn images(blocks: &[Block], keep: fn(&str) -> bool) -> Vec<SharedString> {
    fn walk(blocks: &[Block], keep: fn(&str) -> bool, out: &mut Vec<SharedString>) {
        for block in blocks {
            match block {
                Block::Image { src, .. } if keep(src) && !out.contains(src) => {
                    out.push(src.clone())
                }
                Block::List { items, .. } => items.iter().for_each(|item| walk(item, keep, out)),
                Block::Quote(blocks) => walk(blocks, keep, out),
                Block::Table(table) => table
                    .rows
                    .iter()
                    .flatten()
                    .for_each(|cell| walk(&cell.blocks, keep, out)),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, keep, &mut out);
    out
}

/// Whether `src` is an inline image's address.
pub fn is_cid(src: &str) -> bool {
    src.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("cid:"))
}

/// An image address the reader may fetch when asked: `http(s)` to a named
/// public host. No credentials, no `localhost` or `.local` names and no IP
/// literals in loopback, private or link-local ranges (or IPv6 at all).
pub fn is_safe_image_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return false;
    };
    if url.len() > 4096 || url.chars().any(char::is_whitespace) {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') || authority.starts_with('[') {
        return false;
    }
    let host = authority.split(':').next().unwrap_or_default();
    let host = host.trim_end_matches('.');
    if host.is_empty()
        || !host.contains('.')
        || host == "localhost"
        || [".localhost", ".local", ".internal", ".home.arpa"]
            .iter()
            .any(|suffix| host.ends_with(suffix))
    {
        return false;
    }
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        let [a, b, ..] = ip.octets();
        return !(ip.is_loopback()
            || ip.is_private()
            || ip.is_link_local()
            || ip.is_unspecified()
            || ip.is_broadcast()
            || ip.is_multicast()
            || a == 0
            || (a == 100 && (64..128).contains(&b)));
    }
    // A host that is all digits and dots but not an address (`127.1`).
    !host.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// A link address safe to open or send: web, mail and phone only.
pub(crate) fn is_safe_url(url: &str) -> bool {
    let url = url.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:", "tel:"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
}

/// CSS hex, `rgb()` and the common named colors.
fn parse_color(value: &str) -> Option<Hsla> {
    let value = value.trim().to_ascii_lowercase();
    let hex = |v: u32| -> Hsla { gpui::rgb(v).into() };
    if let Some(digits) = value.strip_prefix('#') {
        let v = u32::from_str_radix(digits, 16).ok()?;
        return match digits.len() {
            3 => {
                let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
                Some(hex(((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11)))
            }
            6 => Some(hex(v)),
            _ => None,
        };
    }
    if let Some(args) = value
        .strip_prefix("rgb(")
        .or_else(|| value.strip_prefix("rgba("))
    {
        let mut parts = args
            .trim_end_matches(')')
            .split([',', ' ', '/'])
            .filter(|p| !p.is_empty())
            .map(|p| p.parse::<f32>().ok());
        let (r, g, b) = (parts.next()??, parts.next()??, parts.next()??);
        return Some(
            Rgba {
                r: r / 255.,
                g: g / 255.,
                b: b / 255.,
                a: 1.,
            }
            .into(),
        );
    }
    Some(hex(match value.as_str() {
        "black" => 0x000000,
        "white" => 0xffffff,
        "gray" | "grey" => 0x808080,
        "darkgray" | "darkgrey" => 0xa9a9a9,
        "silver" | "lightgray" | "lightgrey" => 0xc0c0c0,
        "red" => 0xff0000,
        "darkred" | "maroon" => 0x800000,
        "green" => 0x008000,
        "blue" => 0x0000ff,
        "navy" => 0x000080,
        "orange" => 0xffa500,
        "purple" => 0x800080,
        "teal" => 0x008080,
        _ => return None,
    }))
}

/// An `Inline` under construction.
#[derive(Default)]
struct InlineBuilder {
    text: String,
    spans: Vec<(Range<usize>, Style)>,
}

fn is_html_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

impl InlineBuilder {
    fn push(&mut self, text: &str, style: &Style) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        if *style == Style::default() {
            return;
        }
        match self.spans.last_mut() {
            Some((range, last)) if range.end == start && last == style => range.end = end,
            _ => self.spans.push((start..end, style.clone())),
        }
    }

    /// Push text with HTML's whitespace collapsing. Non-breaking spaces stay.
    fn push_collapsed(&mut self, text: &str, style: &Style) {
        let mut out = String::with_capacity(text.len());
        let mut after_space = self.text.is_empty() || self.text.ends_with(is_html_space);
        for c in text.chars() {
            if is_html_space(c) {
                if !after_space {
                    out.push(' ');
                    after_space = true;
                }
            } else {
                out.push(c);
                after_space = false;
            }
        }
        self.push(&out, style);
    }

    /// The finished text, or `None` when it has nothing visible.
    fn take(&mut self) -> Option<Inline> {
        let InlineBuilder {
            mut text,
            mut spans,
        } = std::mem::take(self);
        if text.trim().is_empty() {
            return None;
        }
        // A trailing space or `<br>` adds nothing at the end of a block.
        let len = text.trim_end_matches(is_html_space).len();
        text.truncate(len);
        spans.retain_mut(|(range, _)| {
            range.end = range.end.min(len);
            range.start < range.end
        });

        let mut links: Vec<(Range<usize>, SharedString)> = Vec::new();
        for (range, style) in &spans {
            let Some(url) = &style.link else { continue };
            match links.last_mut() {
                Some((last, last_url)) if last.end == range.start && last_url == url => {
                    last.end = range.end
                }
                _ => links.push((range.clone(), url.clone())),
            }
        }

        Some(Inline {
            text: text.into(),
            spans,
            links,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Text { text, .. } | Block::Pre(text) => text.text.to_string(),
                Block::List { items, .. } => format!("list({})", items.len()),
                Block::Quote(_) => "quote".into(),
                Block::Table(_) => "table".into(),
                Block::Image { .. } => "image".into(),
                Block::Button { label, .. } => format!("button({label})"),
                Block::Rule => "hr".into(),
            })
            .collect()
    }

    #[test]
    fn blocks_and_whitespace() {
        let doc = parse(
            "<head><style>p{}</style></head><p>Hi <b>there</b>,\n  friend<br></p>\
             <hr><ul><li>a</li><li>b</li></ul><blockquote>old</blockquote>\
             <pre>  x\n  y</pre><script>alert(1)</script>",
        );
        assert_eq!(
            texts(&doc.blocks),
            ["Hi there, friend", "hr", "list(2)", "quote", "  x\n  y"]
        );
        let Block::Text { text, .. } = &doc.blocks[0] else {
            panic!()
        };
        assert_eq!(text.spans.len(), 1);
        assert_eq!(&text.text[text.spans[0].0.clone()], "there");
    }

    #[test]
    fn links_and_colors() {
        let doc = parse(
            r#"<p style="color:#666">See <a href="https://x.io">the <b>docs</b></a>
               or <a href="javascript:evil()">this</a>.</p>"#,
        );
        let Block::Text { text, .. } = &doc.blocks[0] else {
            panic!()
        };
        assert_eq!(text.links.len(), 1);
        assert_eq!(&text.text[text.links[0].0.clone()], "the docs");
        assert!(text.spans.iter().all(|(_, s)| s.color.is_some()));
    }

    #[test]
    fn tables() {
        let doc = parse(
            "<table><tr><th>Item</th><th style='text-align:right'>Price</th></tr>\
             <tr><td>Pen</td><td align=right>$4</td></tr></table>\
             <table><tr><td><p>layout</p></td></tr></table>",
        );
        let Block::Table(table) = &doc.blocks[0] else {
            panic!()
        };
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1][1].align, Align::Right);
        assert_eq!(texts(&doc.blocks[1..]), ["layout"]);
    }

    #[test]
    fn bare_addresses_become_links() {
        let doc = parse(
            "<p>Check in at skylineair.com/checkin. Or https://x.io/a, \
             not MessageStore.com or me@x.com.</p>",
        );
        let Block::Text { text, .. } = &doc.blocks[0] else {
            panic!()
        };
        let linked: Vec<(&str, &str)> = text
            .links
            .iter()
            .map(|(r, url)| (&text.text[r.clone()], url.as_ref()))
            .collect();
        assert_eq!(
            linked,
            [
                ("skylineair.com/checkin", "https://skylineair.com/checkin"),
                ("https://x.io/a", "https://x.io/a")
            ]
        );
    }

    #[test]
    fn buttons_and_cell_backgrounds() {
        let doc = parse(
            r##"<p><a href="https://a.io" style="background:#0a7;padding:8px">Check in</a></p>
               <table><tr><td bgcolor="#1d5bd1"><a href="https://b.io">Pay now</a></td></tr></table>
               <table><tr><td style="background-color:#fee">a</td><td>b</td></tr></table>"##,
        );
        assert_eq!(
            texts(&doc.blocks[..2]),
            ["button(Check in)", "button(Pay now)"]
        );
        let Block::Table(table) = &doc.blocks[2] else {
            panic!()
        };
        assert!(table.rows[0][0].background.is_some());
        assert!(table.rows[0][1].background.is_none());
    }

    #[test]
    fn images_keep_their_src() {
        let doc = parse(
            r#"<a href="https://shop.io"><img src="https://cdn.shop.io/hero.png" alt="Sale"
                 width="600" height="300"></a>
               <img src="https://t.io/open.gif" width="1" height="1">
               <img src="https://t.io/p.gif" style="width:1px;height:1px">
               <img src="https://t.io/h.gif" style="display:none">
               <img src="cid:logo@x">
               <img src="javascript:alert(1)" alt="x">
               <img src="https://cdn.shop.io/hero.png">"#,
        );
        let images: Vec<_> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Image {
                    src, width, link, ..
                } => Some((src.to_string(), *width, link.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 4, "tracking pixels are dropped: {images:?}");
        assert_eq!(images[0].0, "https://cdn.shop.io/hero.png");
        assert_eq!(images[0].1, Some(600.));
        assert_eq!(images[0].2.as_deref(), Some("https://shop.io"));
        assert_eq!(images[1].0, "cid:logo@x");
        assert_eq!(images[2].0, "", "unsafe schemes lose their src");
        assert_eq!(doc.remote_images, ["https://cdn.shop.io/hero.png"]);
        assert_eq!(doc.cid_images, ["cid:logo@x"]);
    }

    #[test]
    fn image_url_rules() {
        for ok in [
            "https://cdn.example.com/a.png",
            "http://example.com/a.png?x=1",
            "https://8.8.8.8/a.png",
            "https://example.com:8443/a",
        ] {
            assert!(is_safe_image_url(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "data:image/png;base64,AAAA",
            "file:///etc/passwd",
            "cid:logo",
            "https://localhost/a.png",
            "https://printer.local/a.png",
            "https://127.0.0.1/a.png",
            "https://10.0.0.5/a.png",
            "https://192.168.1.1/a.png",
            "https://172.16.0.1/a.png",
            "https://169.254.169.254/latest",
            "https://100.64.0.1/a",
            "https://0.0.0.0/a",
            "https://127.1/a",
            "https://[::1]/a.png",
            "https://user:pw@example.com/a.png",
            "https://intranet/a.png",
            "https://exa mple.com/a.png",
        ] {
            assert!(!is_safe_image_url(bad), "{bad}");
        }
    }

    #[test]
    fn folds_trailing_quotes() {
        let doc = parse(
            r#"<p>Sounds good.</p><div class="gmail_quote"><div class="gmail_attr">On Mon, Oct 26, 2026 at 9:12 AM Ana &lt;ana@x.io&gt; wrote:<br></div>
               <blockquote class="gmail_quote"><p>Lunch?</p></blockquote></div>"#,
        );
        assert_eq!(texts(&doc.blocks)[2..], ["quote"]);
        assert_eq!(trailing_quote_start(&doc), Some(1));
        let bare = parse(r#"<p>Hi</p><div class="gmail_quote"><p>old</p></div>"#);
        assert_eq!(texts(&bare.blocks), ["Hi", "quote"]);
        assert_eq!(trailing_quote_start(&bare), Some(1));
        assert_eq!(
            trailing_quote_start(&parse("<blockquote>only</blockquote>")),
            None
        );
        let forward = parse(
            r#"<div class="gmail_quote">---------- Forwarded message ---------<br>From: Bob</div>"#,
        );
        assert_eq!(trailing_quote_start(&forward), None);
    }

    #[test]
    fn right_to_left_text_aligns_right() {
        let doc = parse("<p>שלום, מה שלומך?</p><p>Hello</p><div dir=rtl><p>abc</p></div>");
        let aligns: Vec<Align> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Text { align, .. } => *align,
                _ => panic!(),
            })
            .collect();
        assert_eq!(aligns, [Align::Right, Align::Left, Align::Right]);
    }

    #[test]
    fn missing_and_empty_alt_differ() {
        let doc = parse(r#"<img src="https://a.io/x.png" alt=""><img src="https://a.io/y.png">"#);
        let alts: Vec<_> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Image { alt, .. } => alt.clone(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(alts, [Some(SharedString::default()), None]);
    }

    #[test]
    fn parses_all_bundled_mail_quickly() {
        let data = crate::backend::demo::mail_data();
        let start = std::time::Instant::now();
        let mut count = 0;
        for message in data.threads.iter().flat_map(|t| &t.messages) {
            assert!(!parse(message.html()).blocks.is_empty());
            count += 1;
        }
        eprintln!("parsed {count} messages in {:?}", start.elapsed());
    }
}

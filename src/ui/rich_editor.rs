//! The composer's body: gpui-component's editor, with its markdown styled as
//! it's typed. The editor owns the caret, selection, IME, undo, clipboard and
//! scrolling; this only says how each line on screen looks (see
//! `markdown::scan_line`). Syntax markers stay visible, dimmed, so the caret
//! moves through exactly the characters that are there.

use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};

use gpui::{
    Context, FontStyle, FontWeight, HighlightStyle, Hsla, SharedString, StrikethroughStyle,
    UnderlineStyle, Window,
    base::input::{FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope},
    component::input::EditorState,
    px,
};

use crate::{
    markdown::{
        self, BOLD, BULLET, CODE, Flags, HEADING, ITALIC, LINK, MARKER, QUOTE, STRIKE, UNDERLINE,
    },
    theme::Theme,
};

/// The colors the styling uses, kept in step with the theme by the composer.
#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    marker: Hsla,
    accent: Hsla,
    muted: Hsla,
    code_bg: Hsla,
}

impl Palette {
    pub fn new(theme: &Theme) -> Self {
        Self {
            marker: theme.text_faint,
            accent: theme.accent,
            muted: theme.text_muted,
            code_bg: theme.chip_bg,
        }
    }

    fn style(&self, flags: Flags) -> HighlightStyle {
        let mut style = HighlightStyle::default();
        if flags & MARKER != 0 {
            style.color = Some(self.marker);
            return style;
        }
        if flags & BULLET != 0 {
            style.color = Some(self.muted);
        }
        // As the reader shows it.
        if flags & (BOLD | HEADING) != 0 {
            style.font_weight = Some(FontWeight::SEMIBOLD);
        }
        if flags & ITALIC != 0 {
            style.font_style = Some(FontStyle::Italic);
        }
        if flags & STRIKE != 0 {
            style.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            });
        }
        if flags & (UNDERLINE | LINK) != 0 {
            style.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            });
        }
        if flags & LINK != 0 {
            style.color = Some(self.accent);
        }
        if flags & QUOTE != 0 {
            style.color = Some(self.muted);
        }
        if flags & CODE != 0 {
            style.background_color = Some(self.code_bg);
        }
        style
    }
}

struct MarkdownHighlighter {
    text: Rope,
    palette: Rc<Cell<Palette>>,
    /// Reused between frames, so styling allocates only its result.
    buf: RefCell<String>,
    runs: RefCell<Vec<markdown::Run>>,
}

impl InputHighlighter for MarkdownHighlighter {
    fn language(&self) -> SharedString {
        "markdown".into()
    }

    /// Nothing to reparse: lines are styled on their own when shown.
    fn update(
        &mut self,
        _: Option<InputEdit>,
        text: &Rope,
        _: bool,
        _: &mut Window,
        _: &mut Context<EditorState>,
    ) {
        self.text = text.clone();
    }

    fn styles(
        &self,
        range: &Range<usize>,
        _: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        let palette = self.palette.get();
        let mut buf = self.buf.borrow_mut();
        buf.clear();
        for chunk in self.text.slice(range.clone()).chunks() {
            buf.push_str(chunk);
        }
        let mut runs = self.runs.borrow_mut();
        let mut out = Vec::new();
        let mut at = range.start;
        let mut line_start = range.start;
        for line in buf.split('\n') {
            runs.clear();
            markdown::scan_line(line, &mut runs, None);
            for (run, flags) in runs.iter() {
                let run = line_start + run.start..line_start + run.end;
                if at < run.start {
                    out.push((at..run.start, HighlightStyle::default()));
                }
                at = run.end;
                out.push((run, palette.style(*flags)));
            }
            line_start += line.len() + 1;
        }
        if at < range.end {
            out.push((at..range.end, HighlightStyle::default()));
        }
        out
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// An editor for prose: no gutter, folding, guides, pairing or code indent,
/// wrapping lines, styled as markdown.
pub fn state(
    placeholder: &'static str,
    value: String,
    palette: Rc<Cell<Palette>>,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) -> EditorState {
    let mut state = EditorState::new(window, cx)
        .language("markdown")
        .line_number(false)
        .folding(false)
        .indent_guides(false)
        .auto_close(false)
        .smart_indent(false)
        .soft_wrap(true)
        .searchable(false)
        .scroll_beyond_last_line(Some(0))
        .cursor_surrounding_lines(Some(0))
        .placeholder(placeholder)
        .default_value(value);
    state.set_highlighter_factory(
        Rc::new(move |_| {
            Some(Box::new(MarkdownHighlighter {
                text: Rope::new(),
                palette: palette.clone(),
                buf: RefCell::default(),
                runs: RefCell::default(),
            }) as Box<dyn InputHighlighter>)
        }),
        cx,
    );
    state
}

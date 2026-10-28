//! The composer's markdown: what each line's syntax is, for styling it as it
//! is typed; the edits behind the formatting toolbar and shortcuts; and the
//! HTML a message is sent and saved as.
//!
//! Styling is line by line with no state across lines, so only the lines on
//! screen are ever looked at and an edit never restyles the rest.

use std::ops::Range;

use crate::html::escape;

/// What a run of a line is, as bits.
pub type Flags = u16;
pub const BOLD: Flags = 1;
pub const ITALIC: Flags = 1 << 1;
pub const STRIKE: Flags = 1 << 2;
pub const UNDERLINE: Flags = 1 << 3;
pub const CODE: Flags = 1 << 4;
pub const LINK: Flags = 1 << 5;
/// Syntax: `**`, `# `, `> `, `](url)`. Shown dimmed.
pub const MARKER: Flags = 1 << 6;
pub const HEADING: Flags = 1 << 7;
pub const QUOTE: Flags = 1 << 8;
/// A list item's bullet or number.
pub const BULLET: Flags = 1 << 9;

/// A run of a line, by byte offsets into it.
pub type Run = (Range<usize>, Flags);
/// The opening and closing syntax of one span (`**` and `**`, `[` and
/// `](url)`), by byte offsets into the line. A heading's or quote's prefix
/// has an empty close.
pub type Pair = (Range<usize>, Range<usize>);

/// The styled runs of `line` (no newline), in order and not overlapping;
/// text in no run is plain. Pairs, when asked for, are the spans' syntax.
pub fn scan_line(line: &str, runs: &mut Vec<Run>, mut pairs: Option<&mut Vec<Pair>>) {
    let (body, flags) = block_prefix(line, runs, pairs.as_deref_mut());
    inline(line, body, flags, runs, &mut pairs);
}

/// The styles on at byte `at` of `line`, for the format bar: the inline
/// flags of the text there (up to its end, so the caret just past bold
/// text counts) and the line's kind.
pub fn styles_at(line: &str, at: usize) -> (Flags, Option<LineKind>) {
    let mut runs = Vec::new();
    scan_line(line, &mut runs, None);
    let flags = runs
        .iter()
        .filter(|(run, flags)| flags & MARKER == 0 && run.start < at && at <= run.end)
        .fold(0, |all, (_, flags)| all | flags);
    let kind = if quote_prefix(line).is_some() {
        Some(LineKind::Quote)
    } else {
        list_prefix(line).map(|l| match l.number {
            Some(_) => LineKind::Numbered,
            None => LineKind::Bullet,
        })
    };
    (flags & !(BULLET | QUOTE | HEADING), kind)
}

/// A list item's prefix: indentation length, where the marker ends (past its
/// space), and the number for an ordered item with its delimiter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ListPrefix {
    indent: usize,
    end: usize,
    number: Option<(u32, u8)>,
}

fn list_prefix(line: &str) -> Option<ListPrefix> {
    let b = line.as_bytes();
    let indent = b.iter().take_while(|&&c| c == b' ' || c == b'\t').count();
    let rest = &b[indent..];
    let (marker_len, number) = match rest.first()? {
        b'-' | b'*' | b'+' => (1, None),
        c if c.is_ascii_digit() => {
            let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
            let delim = *rest.get(digits)?;
            if digits > 9 || !matches!(delim, b'.' | b')') {
                return None;
            }
            let n = line[indent..indent + digits].parse().ok()?;
            (digits + 1, Some((n, delim)))
        }
        _ => return None,
    };
    (rest.get(marker_len) == Some(&b' ')).then_some(ListPrefix {
        indent,
        end: indent + marker_len + 1,
        number,
    })
}

/// The length of a quote's `>` markers and the space after them.
fn quote_prefix(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut i = b.iter().take_while(|&&c| c == b' ').count();
    if b.get(i) != Some(&b'>') {
        return None;
    }
    while b.get(i) == Some(&b'>') {
        i += 1;
        if b.get(i) == Some(&b' ') {
            i += 1;
        }
    }
    Some(i)
}

fn heading_prefix(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|&c| c == b'#').count();
    match line.as_bytes().get(hashes) {
        _ if hashes == 0 || hashes > 6 => None,
        Some(b' ') => Some(hashes + 1),
        None => Some(hashes),
        _ => None,
    }
}

/// `---`, `***` or `___`, spaces allowed between.
fn is_rule(line: &str) -> bool {
    let mut chars = line.chars().filter(|c| !c.is_whitespace());
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    let mut count = 0;
    chars.all(|c| {
        count += 1;
        c == first
    }) && count >= 2
}

/// Styles the line's block syntax; returns where its text starts and how it
/// is styled.
fn block_prefix(
    line: &str,
    runs: &mut Vec<Run>,
    pairs: Option<&mut Vec<Pair>>,
) -> (Range<usize>, Flags) {
    let len = line.len();
    if line.trim_start().starts_with("```") || is_rule(line) {
        runs.push((0..len, MARKER));
        return (len..len, 0);
    }
    if let Some(end) = heading_prefix(line) {
        runs.push((0..end, MARKER));
        if let Some(pairs) = pairs {
            pairs.push((0..end, len..len));
        }
        return (end..len, HEADING | BOLD);
    }
    if let Some(end) = quote_prefix(line) {
        runs.push((0..end, MARKER));
        if let Some(pairs) = pairs {
            pairs.push((0..end, len..len));
        }
        return (end..len, QUOTE);
    }
    if let Some(list) = list_prefix(line) {
        runs.push((list.indent..list.end, BULLET));
        return (list.end..len, 0);
    }
    (0..len, 0)
}

/// Kinds of inline span, for remembering which have no closer left.
const SPANS: usize = 7;

fn inline(
    line: &str,
    range: Range<usize>,
    flags: Flags,
    runs: &mut Vec<Run>,
    pairs: &mut Option<&mut Vec<Pair>>,
) {
    let b = line.as_bytes();
    let end = range.end;
    let mut i = range.start;
    let mut plain = i;
    // Per kind of span, the offset from which a closer was looked for and
    // not found: no opener after it can find one either.
    let mut no_closer = [usize::MAX; SPANS];
    let is_space = |at: usize| at >= end || b[at].is_ascii_whitespace();

    while i < end {
        let c = b[i];
        // (opener length, closer range, kind, flags added)
        let span: Option<(usize, Range<usize>, Flags)> = match c {
            b'\\' => {
                i = (i + 2).min(end);
                continue;
            }
            b'`' => {
                let n = run_len(b, i, end, b'`');
                find_run(b, i + n, end, b'`', n).map(|k| (n, k..k + n, CODE))
            }
            b'*' | b'_' | b'~' => {
                let n = run_len(b, i, end, c);
                let intraword = c == b'_' && i > 0 && b[i - 1].is_ascii_alphanumeric();
                if is_space(i + n) || intraword {
                    None
                } else if c == b'~' {
                    if n >= 2 {
                        find_closer(b, i + 2, end, c, 2, &mut no_closer[1], i)
                            .map(|k| (2, k, STRIKE))
                    } else {
                        None
                    }
                } else if n >= 2 {
                    let slot = if c == b'*' { 2 } else { 3 };
                    find_closer(b, i + 2, end, c, 2, &mut no_closer[slot], i).map(|k| (2, k, BOLD))
                } else {
                    let slot = if c == b'*' { 4 } else { 5 };
                    find_closer(b, i + 1, end, c, 1, &mut no_closer[slot], i)
                        .map(|k| (1, k, ITALIC))
                }
            }
            b'<' if line[i..end].starts_with("<u>") && no_closer[6] > i => {
                match line[i + 3..end].find("</u>") {
                    Some(k) => Some((3, i + 3 + k..i + 3 + k + 4, UNDERLINE)),
                    None => {
                        no_closer[6] = i;
                        None
                    }
                }
            }
            b'[' => {
                link_at(line, i, end).map(|(text_end, close_end)| (1, text_end..close_end, LINK))
            }
            _ => None,
        };
        let Some((open, close, added)) = span else {
            i += if c.is_ascii() {
                run_len(b, i, end, c).max(1)
            } else {
                1
            };
            continue;
        };
        if flags != 0 && plain < i {
            runs.push((plain..i, flags));
        }
        runs.push((i..i + open, MARKER));
        if let Some(pairs) = pairs.as_deref_mut() {
            pairs.push((i..i + open, close.clone()));
        }
        let inner = i + open..close.start;
        if added == CODE {
            if !inner.is_empty() {
                runs.push((inner, flags | CODE));
            }
        } else {
            inline(line, inner, flags | added, runs, pairs);
        }
        runs.push((close.clone(), MARKER));
        i = close.end;
        plain = i;
    }
    if flags != 0 && plain < end {
        runs.push((plain..end, flags));
    }
}

fn run_len(b: &[u8], at: usize, end: usize, c: u8) -> usize {
    b[at..end].iter().take_while(|&&x| x == c).count()
}

/// The next run of exactly `n` `c`s from `from`.
fn find_run(b: &[u8], from: usize, end: usize, c: u8, n: usize) -> Option<usize> {
    let mut k = from;
    while k < end {
        if b[k] == c {
            let m = run_len(b, k, end, c);
            if m == n {
                return Some(k);
            }
            k += m;
        } else {
            k += 1;
        }
    }
    None
}

/// The closer for an emphasis opener of `n` `c`s: a run not after a space,
/// and for one `c` not a run of exactly two (that's bold). A longer run
/// closes with its last `n`, so `***a***` is bold around italic.
fn find_closer(
    b: &[u8],
    from: usize,
    end: usize,
    c: u8,
    n: usize,
    no_closer: &mut usize,
    opener: usize,
) -> Option<Range<usize>> {
    if *no_closer <= opener {
        return None;
    }
    let mut k = from;
    while k < end {
        if b[k] != c {
            k += 1;
            continue;
        }
        let m = run_len(b, k, end, c);
        let after_text = k > from && !b[k - 1].is_ascii_whitespace();
        let intraword = c == b'_' && k + m < end && b[k + m].is_ascii_alphanumeric();
        let fits = if n == 1 { m != 2 } else { m >= 2 };
        if after_text && !intraword && fits {
            return Some(k + m - n..k + m);
        }
        k += m;
    }
    *no_closer = opener;
    None
}

/// `[text](url)` at `at`: where the text ends and the link ends.
fn link_at(line: &str, at: usize, end: usize) -> Option<(usize, usize)> {
    let text_end = at + 1 + line[at + 1..end].find(']')?;
    if text_end + 1 >= end || line.as_bytes()[text_end + 1] != b'(' {
        return None;
    }
    let close = text_end + 2 + line[text_end + 2..end].find(')')?;
    Some((text_end, close + 1))
}

// Editing.

/// A change to the body: replace `range` with `text`, then select
/// `selection` (offsets in the new text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
}

impl Edit {
    #[cfg(test)]
    pub fn apply(&self, s: &str) -> String {
        let mut out = s.to_string();
        out.replace_range(self.range.clone(), &self.text);
        out
    }

    /// Changes nothing; keeps the selection.
    fn none(selection: Range<usize>) -> Self {
        Self {
            range: selection.start..selection.start,
            text: String::new(),
            selection,
        }
    }
}

/// Several replacements, not overlapping, as one edit; `sel` follows them.
fn combine(s: &str, mut edits: Vec<(Range<usize>, String)>, sel: Range<usize>) -> Edit {
    if edits.is_empty() {
        return Edit::none(sel);
    }
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    let start = edits[0].0.start;
    let end = edits.iter().map(|(r, _)| r.end).max().unwrap_or(start);
    let mut text = String::new();
    let mut at = start;
    for (range, insert) in &edits {
        text.push_str(&s[at..range.start]);
        text.push_str(insert);
        at = range.end;
    }
    text.push_str(&s[at..end]);
    let shift = |pos: usize, counts: &dyn Fn(&Range<usize>) -> bool| {
        let mut pos = pos as isize;
        for (range, insert) in &edits {
            if counts(range) {
                pos += insert.len() as isize - range.len() as isize;
            }
        }
        pos.max(0) as usize
    };
    // An insertion at the selection's start goes before it; at its end, after.
    let new_start = shift(sel.start, &|r| r.end <= sel.start);
    let new_end = shift(sel.end, &|r| r.end <= sel.end && r.start < sel.end);
    Edit {
        range: start..end,
        text,
        selection: new_start..new_end.max(new_start),
    }
}

fn line_start(s: &str, pos: usize) -> usize {
    s[..pos].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(s: &str, pos: usize) -> usize {
    s[pos..].find('\n').map_or(s.len(), |i| pos + i)
}

/// The starts of the lines `sel` touches. A selection that ends at a line's
/// start leaves that line out.
fn touched_lines(s: &str, sel: &Range<usize>) -> Vec<usize> {
    let mut end = sel.end;
    if sel.end > sel.start && s[..sel.end].ends_with('\n') {
        end -= 1;
    }
    let mut starts = vec![line_start(s, sel.start)];
    let mut at = starts[0];
    while let Some(i) = s[at..end.max(at)].find('\n') {
        at += i + 1;
        starts.push(at);
    }
    starts
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// The word around the caret, if it's in or at the edge of one.
fn word_at(s: &str, pos: usize) -> Option<Range<usize>> {
    let start = s[..pos]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(pos, |(i, _)| i);
    let end = pos
        + s[pos..]
            .char_indices()
            .find(|&(_, c)| !is_word(c))
            .map_or(s.len() - pos, |(i, _)| i);
    (start < end).then_some(start..end)
}

/// Inline formatting a selection can be wrapped in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
}

impl Inline {
    /// The flag its runs carry.
    pub fn flag(self) -> Flags {
        match self {
            Inline::Bold => BOLD,
            Inline::Italic => ITALIC,
            Inline::Underline => UNDERLINE,
            Inline::Strike => STRIKE,
            Inline::Code => CODE,
        }
    }

    fn markers(self) -> (&'static str, &'static str) {
        match self {
            Inline::Bold => ("**", "**"),
            Inline::Italic => ("*", "*"),
            Inline::Underline => ("<u>", "</u>"),
            Inline::Strike => ("~~", "~~"),
            Inline::Code => ("`", "`"),
        }
    }
}

fn stars_before(s: &str, pos: usize) -> usize {
    s[..pos].bytes().rev().take_while(|&c| c == b'*').count()
}

fn stars_after(s: &str, pos: usize) -> usize {
    s[pos..].bytes().take_while(|&c| c == b'*').count()
}

/// Whether `seg` is already wrapped in `kind`'s markers, right outside it.
fn is_wrapped(s: &str, seg: &Range<usize>, kind: Inline) -> bool {
    let (l, r) = (stars_before(s, seg.start), stars_after(s, seg.end));
    match kind {
        Inline::Bold => l >= 2 && r >= 2,
        Inline::Italic => l % 2 == 1 && r % 2 == 1,
        _ => {
            let (open, close) = kind.markers();
            s[..seg.start].ends_with(open) && s[seg.end..].starts_with(close)
        }
    }
}

/// Where a line's text starts, past its list, quote or heading syntax.
fn content_start(line: &str) -> usize {
    heading_prefix(line)
        .or_else(|| quote_prefix(line))
        .or_else(|| list_prefix(line).map(|l| l.end))
        .unwrap_or(0)
}

/// Wraps the selection in `kind` (each line of it on its own), or unwraps it
/// when it's wrapped already. With no selection, the word at the caret;
/// with no word, an empty pair to type into.
pub fn toggle_inline(s: &str, sel: Range<usize>, kind: Inline) -> Edit {
    let (open, close) = kind.markers();
    let sel = if sel.is_empty() {
        word_at(s, sel.start).unwrap_or(sel)
    } else {
        sel
    };
    if sel.is_empty() {
        let p = sel.start;
        if s[..p].ends_with(open) && s[p..].starts_with(close) && is_wrapped(s, &sel, kind) {
            let at = p - open.len();
            return Edit {
                range: at..p + close.len(),
                text: String::new(),
                selection: at..at,
            };
        }
        return Edit {
            range: p..p,
            text: format!("{open}{close}"),
            selection: p + open.len()..p + open.len(),
        };
    }

    let segments: Vec<Range<usize>> = touched_lines(s, &sel)
        .into_iter()
        .filter_map(|ls| {
            let le = line_end(s, ls);
            let mut start = sel.start.max(ls).max(ls + content_start(&s[ls..le]));
            let mut end = sel.end.min(le);
            let text = &s[start..end.max(start)];
            start += text.len() - text.trim_start().len();
            end -= text.len() - text.trim_end().len();
            // The markers themselves may be selected too.
            loop {
                let inner = &s[start..end.max(start)];
                let star = matches!(kind, Inline::Bold | Inline::Italic);
                if star && inner.len() >= 2 && inner.starts_with('*') && inner.ends_with('*') {
                    start += 1;
                    end -= 1;
                } else if !star
                    && inner.len() >= open.len() + close.len()
                    && inner.starts_with(open)
                    && inner.ends_with(close)
                {
                    start += open.len();
                    end -= close.len();
                } else {
                    break;
                }
            }
            (start < end).then_some(start..end)
        })
        .collect();
    let Some(first) = segments.first() else {
        return Edit::none(sel);
    };
    let last = segments.last().unwrap_or(first);
    let selected = first.start..last.end;
    let all = segments.iter().all(|seg| is_wrapped(s, seg, kind));
    let mut edits = Vec::new();
    for seg in &segments {
        if all {
            edits.push((seg.start - open.len()..seg.start, String::new()));
            edits.push((seg.end..seg.end + close.len(), String::new()));
        } else if !is_wrapped(s, seg, kind) {
            edits.push((seg.start..seg.start, open.to_string()));
            edits.push((seg.end..seg.end, close.to_string()));
        }
    }
    combine(s, edits, selected)
}

/// Block formatting applied to whole lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Bullet,
    Numbered,
    Quote,
}

/// Makes the selected lines a bulleted or numbered list or a quote, or
/// stops when they all are already. A list of the other kind is switched.
pub fn toggle_lines(s: &str, sel: Range<usize>, kind: LineKind) -> Edit {
    let lines: Vec<(usize, &str)> = touched_lines(s, &sel)
        .into_iter()
        .map(|ls| (ls, &s[ls..line_end(s, ls)]))
        .collect();
    let content: Vec<_> = if lines.iter().any(|(_, l)| !l.trim().is_empty()) {
        lines
            .iter()
            .filter(|(_, l)| !l.trim().is_empty())
            .copied()
            .collect()
    } else {
        lines
    };
    let has = |line: &str| match kind {
        LineKind::Bullet => list_prefix(line).is_some_and(|l| l.number.is_none()),
        LineKind::Numbered => list_prefix(line).is_some_and(|l| l.number.is_some()),
        LineKind::Quote => quote_prefix(line).is_some(),
    };
    let all = content.iter().all(|(_, l)| has(l));
    let mut edits = Vec::new();
    for (n, (ls, line)) in content.iter().enumerate() {
        let list = list_prefix(line);
        if all {
            match (kind, list) {
                (LineKind::Quote, _) => {
                    edits.push((*ls..ls + quote_prefix(line).unwrap_or(0), String::new()))
                }
                (_, Some(list)) => edits.push((ls + list.indent..ls + list.end, String::new())),
                _ => {}
            }
            continue;
        }
        if has(line) {
            continue;
        }
        let marker = match kind {
            LineKind::Quote => {
                edits.push((*ls..*ls, "> ".into()));
                continue;
            }
            LineKind::Bullet => "- ".to_string(),
            LineKind::Numbered => format!("{}. ", n + 1),
        };
        match list {
            Some(list) => edits.push((ls + list.indent..ls + list.end, marker)),
            None => {
                let indent = line.len() - line.trim_start().len();
                edits.push((ls + indent..ls + indent, marker));
            }
        }
    }
    combine(s, edits, sel)
}

/// The prefix that continues the list or quote on the line at `ls`, and
/// where its text starts.
fn continuation(line: &str) -> Option<(usize, String)> {
    if let Some(list) = list_prefix(line) {
        let indent = &line[..list.indent];
        let task = line[list.end..]
            .starts_with("[ ] ")
            .then_some("[ ] ")
            .or_else(|| {
                (line[list.end..].starts_with("[x] ") || line[list.end..].starts_with("[X] "))
                    .then_some("[ ] ")
            })
            .unwrap_or("");
        let marker = match list.number {
            Some((n, delim)) => format!("{indent}{}{} {task}", n + 1, delim as char),
            None => format!("{indent}{} {task}", &line[list.indent..list.indent + 1]),
        };
        return Some((list.end + task.len(), marker));
    }
    quote_prefix(line).map(|end| (end, line[..end].to_string()))
}

/// Enter in a list item or quote: a new item, or on an empty item, the end
/// of the list (or one level out, when nested). None for an ordinary Enter.
pub fn enter(s: &str, sel: Range<usize>) -> Option<Edit> {
    if !sel.is_empty() {
        return None;
    }
    let p = sel.start;
    let ls = line_start(s, p);
    let le = line_end(s, p);
    let line = &s[ls..le];
    let (text_start, prefix) = continuation(line)?;
    if p - ls < text_start {
        return None;
    }
    if line[text_start..].trim().is_empty() {
        if let Some(list) = list_prefix(line).filter(|l| l.indent > 0) {
            return Some(outdent_line(ls, line, list, p));
        }
        return Some(Edit {
            range: ls..le,
            text: String::new(),
            selection: ls..ls,
        });
    }
    let text = format!("\n{prefix}");
    let at = p + text.len();
    Some(Edit {
        range: p..p,
        text,
        selection: at..at,
    })
}

const INDENT: &str = "    ";

fn outdent_line(ls: usize, line: &str, list: ListPrefix, p: usize) -> Edit {
    let remove = if line.starts_with('\t') {
        1
    } else {
        list.indent.min(INDENT.len())
    };
    combine(line, vec![(0..remove, String::new())], p - ls..p - ls).shifted(ls)
}

impl Edit {
    /// The same edit, made on text that starts `by` bytes into the whole.
    pub fn shifted(mut self, by: usize) -> Self {
        self.range = self.range.start + by..self.range.end + by;
        self.selection = self.selection.start + by..self.selection.end + by;
        self
    }
}

/// Tab and Shift-Tab in a list: the selected items one level in or out.
/// None outside a list, so Tab can move between fields there.
pub fn indent(s: &str, sel: Range<usize>, outdent: bool) -> Option<Edit> {
    let starts = touched_lines(s, &sel);
    list_prefix(&s[starts[0]..line_end(s, starts[0])])?;
    let mut edits = Vec::new();
    for ls in starts {
        let line = &s[ls..line_end(s, ls)];
        let Some(list) = list_prefix(line) else {
            continue;
        };
        if outdent {
            let remove = if line.starts_with('\t') {
                1
            } else {
                list.indent.min(INDENT.len())
            };
            if remove > 0 {
                edits.push((ls..ls + remove, String::new()));
            }
        } else {
            edits.push((ls..ls, INDENT.to_string()));
            // A nested numbered list counts from one.
            if let Some((n, _)) = list.number.filter(|&(n, _)| n != 1) {
                let digits = n.to_string().len();
                edits.push((ls + list.indent..ls + list.indent + digits, "1".into()));
            }
        }
    }
    Some(combine(s, edits, sel))
}

/// Removes the inline formatting the selection touches, with headings and
/// quotes; with no selection, all of it on the caret's line. Lists stay.
pub fn clear_formatting(s: &str, sel: Range<usize>) -> Edit {
    let mut edits = Vec::new();
    let mut runs = Vec::new();
    let mut pairs = Vec::new();
    for ls in touched_lines(s, &sel) {
        let line = &s[ls..line_end(s, ls)];
        runs.clear();
        pairs.clear();
        scan_line(line, &mut runs, Some(&mut pairs));
        for (open, close) in &pairs {
            let span = ls + open.start..ls + close.end.max(open.end);
            let touches = sel.is_empty() || (span.start < sel.end && span.end > sel.start);
            if touches {
                edits.push((ls + open.start..ls + open.end, String::new()));
                if !close.is_empty() {
                    edits.push((ls + close.start..ls + close.end, String::new()));
                }
            }
        }
    }
    combine(s, edits, sel)
}

/// Links the selection (or the word at the caret), with the URL selected to
/// type over; a selected URL becomes the link with its text selected. In a
/// link already, unlinks it.
pub fn toggle_link(s: &str, sel: Range<usize>) -> Edit {
    let ls = line_start(s, sel.start);
    let line = &s[ls..line_end(s, ls)];
    let mut runs = Vec::new();
    let mut pairs = Vec::new();
    scan_line(line, &mut runs, Some(&mut pairs));
    let link = pairs.iter().find(|(open, close)| {
        line[open.clone()].starts_with('[')
            && ls + open.start <= sel.start
            && sel.end <= ls + close.end
    });
    if let Some((open, close)) = link {
        let edits = vec![
            (ls + open.start..ls + open.end, String::new()),
            (ls + close.start..ls + close.end, String::new()),
        ];
        return combine(s, edits, sel);
    }

    let sel = if sel.is_empty() {
        word_at(s, sel.start).unwrap_or(sel)
    } else {
        sel.start..sel.end.min(line_end(s, sel.start))
    };
    let text = &s[sel.clone()];
    let at = sel.start;
    if text.is_empty() {
        let label = "link text";
        return Edit {
            range: sel,
            text: format!("[{label}](https://)"),
            selection: at + 1..at + 1 + label.len(),
        };
    }
    let is_url = ["http://", "https://", "mailto:", "www."]
        .iter()
        .any(|p| text.starts_with(p));
    if is_url {
        Edit {
            range: sel.clone(),
            text: format!("[{text}]({text})"),
            selection: at + 1..at + 1 + text.len(),
        }
    } else {
        let url_start = at + text.len() + 3;
        Edit {
            range: sel.clone(),
            text: format!("[{text}](https://)"),
            selection: url_start..url_start + "https://".len(),
        }
    }
}

// HTML.

/// The body as HTML. Line breaks are kept, as people mean them in mail.
/// Raw HTML is shown as text, except `<u>` and `</u>`.
pub fn to_html(md: &str) -> String {
    let Ok(root) = markdown::to_mdast(md, &markdown::ParseOptions::gfm()) else {
        return crate::html::from_plain_text(md);
    };
    let mut out = String::with_capacity(md.len() * 2);
    render(&root, false, &mut out);
    out
}

fn render_children(children: &[markdown::mdast::Node], tight: bool, out: &mut String) {
    for child in children {
        render(child, tight, out);
    }
}

fn render(node: &markdown::mdast::Node, tight: bool, out: &mut String) {
    use markdown::mdast::Node;
    let wrap = |out: &mut String, tag: &str, children: &[Node], tight: bool| {
        out.push_str(&format!("<{tag}>"));
        render_children(children, tight, out);
        out.push_str(&format!("</{tag}>"));
    };
    match node {
        Node::Root(n) => render_children(&n.children, false, out),
        Node::Paragraph(n) if tight => render_children(&n.children, false, out),
        Node::Paragraph(n) => wrap(out, "p", &n.children, false),
        Node::Heading(n) => wrap(
            out,
            &format!("h{}", n.depth.clamp(1, 6)),
            &n.children,
            false,
        ),
        Node::Text(n) => out.push_str(&escape(&n.value).replace('\n', "<br>")),
        Node::Break(_) => out.push_str("<br>"),
        Node::Strong(n) => wrap(out, "strong", &n.children, false),
        Node::Emphasis(n) => wrap(out, "em", &n.children, false),
        Node::Delete(n) => wrap(out, "s", &n.children, false),
        Node::InlineCode(n) => out.push_str(&format!("<code>{}</code>", escape(&n.value))),
        Node::Code(n) => out.push_str(&format!("<pre>{}</pre>", escape(&n.value))),
        Node::Html(n) => {
            let tag = n.value.trim().to_ascii_lowercase();
            if tag == "<u>" || tag == "</u>" {
                out.push_str(&tag);
            } else {
                out.push_str(&escape(&n.value));
            }
        }
        // Only web, mail and phone links; anything else is sent as its text.
        Node::Link(n) if crate::html::is_safe_url(&n.url) => {
            out.push_str(&format!("<a href=\"{}\">", escape(&n.url)));
            render_children(&n.children, false, out);
            out.push_str("</a>");
        }
        Node::Link(n) => render_children(&n.children, false, out),
        Node::Image(n) => out.push_str(&format!("<img alt=\"{}\">", escape(&n.alt))),
        Node::List(n) => {
            let tag = if n.ordered { "ol" } else { "ul" };
            match n.start.filter(|&s| n.ordered && s != 1) {
                Some(start) => out.push_str(&format!("<ol start=\"{start}\">")),
                None => out.push_str(&format!("<{tag}>")),
            }
            for item in &n.children {
                let item_tight = match item {
                    Node::ListItem(item) => !(n.spread || item.spread),
                    _ => !n.spread,
                };
                render(item, item_tight, out);
            }
            out.push_str(&format!("</{tag}>"));
        }
        Node::ListItem(n) => {
            out.push_str("<li>");
            match n.checked {
                Some(true) => out.push_str("☑ "),
                Some(false) => out.push_str("☐ "),
                None => {}
            }
            render_children(&n.children, tight, out);
            out.push_str("</li>");
        }
        Node::Blockquote(n) => wrap(out, "blockquote", &n.children, false),
        Node::ThematicBreak(_) => out.push_str("<hr>"),
        Node::Table(n) => {
            out.push_str("<table>");
            for (row_ix, row) in n.children.iter().enumerate() {
                out.push_str("<tr>");
                let cell_tag = if row_ix == 0 { "th" } else { "td" };
                for cell in row.children().into_iter().flatten() {
                    wrap(out, cell_tag, cell.children().map_or(&[][..], |c| c), false);
                }
                out.push_str("</tr>");
            }
            out.push_str("</table>");
        }
        other => {
            if let Some(children) = other.children() {
                render_children(children, tight, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(line: &str) -> Vec<(&str, Flags)> {
        let mut out = Vec::new();
        scan_line(line, &mut out, None);
        out.into_iter().map(|(r, f)| (&line[r], f)).collect()
    }

    #[test]
    fn styles_inline_syntax() {
        assert_eq!(
            runs("a **b** c"),
            [("**", MARKER), ("b", BOLD), ("**", MARKER)]
        );
        assert_eq!(
            runs("*i* ~~s~~ `c`"),
            [
                ("*", MARKER),
                ("i", ITALIC),
                ("*", MARKER),
                ("~~", MARKER),
                ("s", STRIKE),
                ("~~", MARKER),
                ("`", MARKER),
                ("c", CODE),
                ("`", MARKER),
            ]
        );
        assert_eq!(
            runs("***x***"),
            [
                ("**", MARKER),
                ("*", MARKER),
                ("x", BOLD | ITALIC),
                ("*", MARKER),
                ("**", MARKER),
            ]
        );
        assert_eq!(
            runs("[Pigeon](https://p.dev)"),
            [
                ("[", MARKER),
                ("Pigeon", LINK),
                ("](https://p.dev)", MARKER)
            ]
        );
        assert_eq!(
            runs("<u>u</u>"),
            [("<u>", MARKER), ("u", UNDERLINE), ("</u>", MARKER)]
        );
        assert!(runs("snake_case_name and 2 * 3 * 4").is_empty());
        assert!(runs("**unclosed").is_empty());
        assert!(runs(r"\*not\*").is_empty());
    }

    #[test]
    fn styles_block_syntax() {
        assert_eq!(runs("# Hi"), [("# ", MARKER), ("Hi", HEADING | BOLD)]);
        assert_eq!(runs("> q"), [("> ", MARKER), ("q", QUOTE)]);
        assert_eq!(runs("  - item"), [("- ", BULLET)]);
        assert_eq!(runs("12. item"), [("12. ", BULLET)]);
        assert_eq!(runs("---"), [("---", MARKER)]);
        assert!(runs("#hashtag").is_empty());
    }

    #[test]
    fn styling_a_long_body_is_fast() {
        let line = "Some **bold**, *italic*, `code` and a [link](https://example.com) here.";
        let body: Vec<String> = (0..5_000)
            .map(|i| match i % 4 {
                0 => format!("- {line}"),
                1 => format!("> {line}"),
                2 => format!("## {line}"),
                _ => line.to_string(),
            })
            .collect();
        let mut out = Vec::new();
        let started = std::time::Instant::now();
        for line in &body {
            out.clear();
            scan_line(line, &mut out, None);
        }
        let elapsed = started.elapsed();
        // The editor styles only the lines it lays out; even all 5,000 at
        // once must fit in a frame (an unoptimized build gets slack).
        let budget = if cfg!(debug_assertions) { 160 } else { 8 };
        assert!(elapsed.as_millis() < budget, "took {elapsed:?}");
    }

    #[test]
    fn pathological_lines_stay_linear() {
        let line = "**a ".repeat(5_000);
        let mut out = Vec::new();
        let started = std::time::Instant::now();
        scan_line(&line, &mut out, None);
        let budget = if cfg!(debug_assertions) { 100 } else { 10 };
        assert!(started.elapsed().as_millis() < budget);
    }

    fn toggle(s: &str, sel: Range<usize>, kind: Inline) -> (String, String) {
        let edit = toggle_inline(s, sel, kind);
        let out = edit.apply(s);
        let selected = out[edit.selection.clone()].to_string();
        (out, selected)
    }

    #[test]
    fn inline_toggles_on_and_off() {
        assert_eq!(
            toggle("say hi", 4..6, Inline::Bold),
            ("say **hi**".into(), "hi".into())
        );
        assert_eq!(
            toggle("say **hi**", 6..8, Inline::Bold),
            ("say hi".into(), "hi".into())
        );
        // Markers selected along with the text.
        assert_eq!(
            toggle("say **hi**", 4..10, Inline::Bold),
            ("say hi".into(), "hi".into())
        );
        // A bold word is not italic, and becomes both.
        assert_eq!(
            toggle("**hi**", 2..4, Inline::Italic),
            ("***hi***".into(), "hi".into())
        );
        assert_eq!(toggle("***hi***", 3..5, Inline::Italic).0, "**hi**");
        assert_eq!(toggle("***hi***", 3..5, Inline::Bold).0, "*hi*");
        assert_eq!(toggle("a b", 0..3, Inline::Underline).0, "<u>a b</u>");
        assert_eq!(toggle("<u>a b</u>", 3..6, Inline::Underline).0, "a b");
        assert_eq!(toggle("x", 0..1, Inline::Code).0, "`x`");
        assert_eq!(toggle("x", 0..1, Inline::Strike).0, "~~x~~");
        // Surrounding spaces stay outside.
        assert_eq!(toggle("a  b  c", 2..6, Inline::Bold).0, "a  **b**  c");
    }

    #[test]
    fn inline_toggle_at_the_caret() {
        // In a word: the word.
        assert_eq!(toggle("say hello", 6..6, Inline::Bold).0, "say **hello**");
        assert_eq!(toggle("say **hello**", 8..8, Inline::Bold).0, "say hello");
        // Between words: an empty pair to type into, toggled off again.
        let edit = toggle_inline("a ", 2..2, Inline::Bold);
        assert_eq!(edit.apply("a "), "a ****");
        assert_eq!(edit.selection, 4..4);
        assert_eq!(
            toggle_inline("a ****", 4..4, Inline::Bold).apply("a ****"),
            "a "
        );
    }

    #[test]
    fn inline_toggle_per_line() {
        let s = "- one\n- two";
        assert_eq!(
            toggle(s, 0..s.len(), Inline::Bold).0,
            "- **one**\n- **two**"
        );
        let s = "- **one**\n- **two**";
        assert_eq!(toggle(s, 0..s.len(), Inline::Bold).0, "- one\n- two");
        // Mixed: the plain one is bolded.
        let s = "**one**\ntwo";
        assert_eq!(toggle(s, 0..s.len(), Inline::Bold).0, "**one**\n**two**");
    }

    #[test]
    fn line_toggles() {
        let apply = |s: &str, sel, kind| toggle_lines(s, sel, kind).apply(s);
        assert_eq!(apply("a\nb", 0..3, LineKind::Bullet), "- a\n- b");
        assert_eq!(apply("- a\n- b", 0..7, LineKind::Bullet), "a\nb");
        assert_eq!(apply("- a\n- b", 0..7, LineKind::Numbered), "1. a\n2. b");
        assert_eq!(apply("a\n\nb", 0..4, LineKind::Numbered), "1. a\n\n2. b");
        assert_eq!(apply("a", 0..0, LineKind::Quote), "> a");
        assert_eq!(apply("> a", 3..3, LineKind::Quote), "a");
        assert_eq!(apply("", 0..0, LineKind::Bullet), "- ");
        // The caret keeps its place in the text.
        let edit = toggle_lines("hello", 3..3, LineKind::Bullet);
        assert_eq!(edit.selection, 5..5);
    }

    #[test]
    fn enter_continues_and_ends_lists() {
        let apply = |s: &str, p: usize| enter(s, p..p).map(|e| (e.apply(s), e.selection));
        assert_eq!(apply("- a", 3), Some(("- a\n- ".into(), 6..6)));
        assert_eq!(apply("9. a", 4), Some(("9. a\n10. ".into(), 9..9)));
        assert_eq!(apply("> a", 3), Some(("> a\n> ".into(), 6..6)));
        assert_eq!(apply("- [x] a", 7).unwrap().0, "- [x] a\n- [ ] ");
        // An empty item ends the list; nested, it moves out a level.
        assert_eq!(apply("- a\n- ", 6), Some(("- a\n".into(), 4..4)));
        assert_eq!(apply("    - ", 6).unwrap().0, "- ");
        // Splitting an item carries the rest of it into the new one.
        assert_eq!(apply("- ab", 3).unwrap().0, "- a\n- b");
        assert_eq!(apply("plain", 5), None);
        assert_eq!(apply("- a", 1), None);
    }

    #[test]
    fn tab_indents_list_items() {
        let s = "1. a\n2. b";
        let edit = indent(s, 8..8, false).unwrap();
        assert_eq!(edit.apply(s), "1. a\n    1. b");
        let s = "- a\n    - b";
        assert_eq!(indent(s, 10..10, true).unwrap().apply(s), "- a\n- b");
        assert_eq!(indent("text", 2..2, false), None);
    }

    #[test]
    fn clears_formatting() {
        let s = "# **a** *b* [c](https://x.y) - d";
        assert_eq!(clear_formatting(s, 0..0).apply(s), "a b c - d");
        let s = "**a** **b**";
        assert_eq!(clear_formatting(s, 8..9).apply(s), "**a** b");
        assert_eq!(clear_formatting("- **a**", 0..7).apply("- **a**"), "- a");
    }

    #[test]
    fn links() {
        let s = "see docs";
        let edit = toggle_link(s, 4..8);
        let out = edit.apply(s);
        assert_eq!(out, "see [docs](https://)");
        assert_eq!(&out[edit.selection], "https://");
        let s = "see [docs](https://x.y)";
        assert_eq!(toggle_link(s, 6..6).apply(s), "see docs");
        let s = "https://x.y";
        let edit = toggle_link(s, 0..s.len());
        assert_eq!(edit.apply(s), "[https://x.y](https://x.y)");
        assert_eq!(edit.selection, 1..12);
        let edit = toggle_link("", 0..0);
        assert_eq!(&edit.apply("")[edit.selection], "link text");
    }

    #[test]
    fn markdown_to_html() {
        assert_eq!(to_html("**a** *b*"), "<p><strong>a</strong> <em>b</em></p>");
        assert_eq!(to_html("line one\nline two"), "<p>line one<br>line two</p>");
        assert_eq!(
            to_html("~~x~~ `y` <u>z</u>"),
            "<p><s>x</s> <code>y</code> <u>z</u></p>"
        );
        assert_eq!(to_html("- a\n- b"), "<ul><li>a</li><li>b</li></ul>");
        assert_eq!(
            to_html("3. a\n4. b"),
            "<ol start=\"3\"><li>a</li><li>b</li></ol>"
        );
        assert_eq!(to_html("> q"), "<blockquote><p>q</p></blockquote>");
        assert_eq!(to_html("# H"), "<h1>H</h1>");
        assert_eq!(
            to_html("[a](https://x.y)"),
            "<p><a href=\"https://x.y\">a</a></p>"
        );
        // Raw HTML other than underline is text.
        assert_eq!(
            to_html("<b>x</b> & <"),
            "<p>&lt;b&gt;x&lt;/b&gt; &amp; &lt;</p>"
        );
    }

    #[test]
    fn html_renders_through_the_reader() {
        let doc = crate::html::parse(&to_html("Hi **there**\n\n- one\n- two"));
        assert_eq!(crate::html::plain_text(&doc), "Hi there\n\none\n\ntwo");
    }

    #[test]
    fn styles_at_the_caret() {
        let line = "- some **bold** text";
        let bold = line.find("bold").unwrap();
        assert_eq!(styles_at(line, bold + 2), (BOLD, Some(LineKind::Bullet)));
        assert_eq!(styles_at(line, bold + 4).0, BOLD, "just past it");
        assert_eq!(styles_at(line, 3), (0, Some(LineKind::Bullet)));
        assert_eq!(styles_at("> 1. *x*", 7), (ITALIC, Some(LineKind::Quote)));
        assert_eq!(styles_at("2. x", 4).1, Some(LineKind::Numbered));
    }

    #[test]
    fn rules_need_three_of_one_mark() {
        assert!(is_rule("---"));
        assert!(is_rule("* * *"));
        assert!(!is_rule("--"));
        assert!(!is_rule("-*-"));
        assert!(!is_rule("- item"));
    }

    #[test]
    fn unsafe_links_go_as_text() {
        assert_eq!(to_html("[a](javascript:alert(1))"), "<p>a</p>");
        assert_eq!(
            to_html("[call](tel:+123)"),
            "<p><a href=\"tel:+123\">call</a></p>"
        );
    }
}

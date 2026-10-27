//! A draft: recipients as chips with contact suggestions, subject and body.
//! It lives in the reading pane, alone for a new message or under the
//! conversation for a reply. Sending, closing and discarding are app actions
//! (see `PigeonApp`), so they work from the palette too; this view owns only
//! the editing.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable as _,
    FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, PathPromptOptions,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled, Subscription,
    Window, actions, anchored,
    base::{
        actions::{SelectDown, SelectUp},
        input::RopeExt as _,
    },
    component::{
        ActiveTheme as _,
        input::{
            BufferPoint, Editor, EditorState, Input, InputEvent, InputState, MoveDown, MoveUp,
        },
        scroll::ScrollableElement as _,
    },
    deferred, div,
    prelude::FluentBuilder as _,
    px, relative,
};

use super::{
    attachment_icon, avatar, html_view, icon, keycap,
    rich_editor::{self, Palette},
};
use crate::ui::style::*;
use crate::{
    actions::{COMPOSER, DiscardDraft, RECIPIENTS, SendMessage, ShowCcBcc},
    app::ComposeKind,
    assets::Icon,
    contacts::SharedContactBook,
    data::{Attachment, Contact, file_size},
    html::{self, Document},
    markdown::{self, Edit, Inline, LineKind},
    theme::Theme,
};

actions!(
    composer,
    [
        NextField,
        PrevField,
        SuggestNext,
        SuggestPrev,
        RemoveLastRecipient,
        DismissSuggestions,
        // Formatting the body (see `Format`).
        FormatBold,
        FormatItalic,
        FormatUnderline,
        FormatStrikethrough,
        FormatCode,
        InsertLink,
        BulletedList,
        NumberedList,
        QuoteLines,
        ClearFormatting,
        // Enter, Tab and Shift-Tab in the body: lists first, then as usual.
        BodyEnter,
        BodyIndent,
        BodyOutdent,
        // Page Up and Down in the body, by a page of the draft's scroll.
        BodyPageUp,
        BodyPageDown,
        BodySelectPageUp,
        BodySelectPageDown,
    ]
);

/// What a formatting button or shortcut does to the body's markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Inline(Inline),
    Lines(LineKind),
    Link,
    Clear,
}

impl Format {
    /// The toolbar, in order: icon, name, action.
    const TOOLBAR: [(Format, Icon, &'static str); 10] = [
        (Format::Inline(Inline::Bold), Icon::Bold, "Bold"),
        (Format::Inline(Inline::Italic), Icon::Italic, "Italic"),
        (
            Format::Inline(Inline::Underline),
            Icon::Underline,
            "Underline",
        ),
        (
            Format::Inline(Inline::Strike),
            Icon::Strikethrough,
            "Strikethrough",
        ),
        (Format::Link, Icon::Link, "Link"),
        (Format::Lines(LineKind::Bullet), Icon::List, "Bulleted list"),
        (
            Format::Lines(LineKind::Numbered),
            Icon::ListOrdered,
            "Numbered list",
        ),
        (Format::Lines(LineKind::Quote), Icon::TextQuote, "Quote"),
        (Format::Inline(Inline::Code), Icon::Code, "Code"),
        (Format::Clear, Icon::RemoveFormatting, "Clear formatting"),
    ];

    fn action(self) -> Box<dyn gpui::Action> {
        match self {
            Format::Inline(Inline::Bold) => Box::new(FormatBold),
            Format::Inline(Inline::Italic) => Box::new(FormatItalic),
            Format::Inline(Inline::Underline) => Box::new(FormatUnderline),
            Format::Inline(Inline::Strike) => Box::new(FormatStrikethrough),
            Format::Inline(Inline::Code) => Box::new(FormatCode),
            Format::Link => Box::new(InsertLink),
            Format::Lines(LineKind::Bullet) => Box::new(BulletedList),
            Format::Lines(LineKind::Numbered) => Box::new(NumberedList),
            Format::Lines(LineKind::Quote) => Box::new(QuoteLines),
            Format::Clear => Box::new(ClearFormatting),
        }
    }
}

/// Registers the formatting actions on `el`, each calling `f` with what it
/// does. The composer handles them while it has focus; the app, for the
/// palette, when it doesn't.
pub fn on_format_actions<E: InteractiveElement>(
    el: E,
    f: impl Fn(Format, &mut Window, &mut gpui::App) + Clone + 'static,
) -> E {
    macro_rules! on {
        ($el:expr, $($action:ident => $format:expr),*) => {
            $el$(.on_action({
                let f = f.clone();
                move |_: &$action, window, cx| f($format, window, cx)
            }))*
        };
    }
    on!(el,
        FormatBold => Format::Inline(Inline::Bold),
        FormatItalic => Format::Inline(Inline::Italic),
        FormatUnderline => Format::Inline(Inline::Underline),
        FormatStrikethrough => Format::Inline(Inline::Strike),
        FormatCode => Format::Inline(Inline::Code),
        InsertLink => Format::Link,
        BulletedList => Format::Lines(LineKind::Bullet),
        NumberedList => Format::Lines(LineKind::Numbered),
        QuoteLines => Format::Lines(LineKind::Quote),
        ClearFormatting => Format::Clear
    )
}

/// What a draft says, read out of the fields.
#[derive(Clone, Debug, Default)]
pub struct DraftContent {
    pub to: Vec<Contact>,
    pub cc: Vec<Contact>,
    pub bcc: Vec<Contact>,
    pub subject: String,
    pub body: String,
    /// Files attached, as name, size and type; the files themselves aren't
    /// read (see ATT-6 in `docs/ROADMAP.md`).
    pub attachments: Vec<Attachment>,
}

impl DraftContent {
    /// The body as HTML.
    pub fn body_html(&self) -> String {
        markdown::to_html(&self.body)
    }

    pub fn has_recipients(&self) -> bool {
        !(self.to.is_empty() && self.cc.is_empty() && self.bcc.is_empty())
    }
}

/// The forwarded message, quoted under the body.
pub struct Quoted {
    /// "From", "Date", "Subject", "To" lines.
    pub header: Vec<(String, String)>,
    pub html: String,
    doc: Rc<Document>,
}

impl Quoted {
    pub fn new(header: Vec<(String, String)>, html: String) -> Self {
        Self {
            doc: Rc::new(html::parse(&html)),
            header,
            html,
        }
    }
}

pub enum ComposerEvent {
    /// The body changed; `at_end` when the caret is at its end, so a growing
    /// draft can be kept in view.
    Edited { at_end: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    To,
    Cc,
    Bcc,
    Subject,
    Body,
}

struct Recipients {
    chips: Vec<Contact>,
    input: Entity<InputState>,
    /// Indices into the contact book, best first.
    suggestions: Vec<usize>,
    highlighted: usize,
    /// The user moved through the suggestions, so enter takes the highlighted
    /// one even over a typed address.
    navigated: bool,
}

pub struct Composer {
    pub kind: ComposeKind,
    /// The thread replied to, by id. Forwards start a new thread, so have none.
    pub reply_to: Option<String>,
    /// The Drafts thread this edits: replaced on save, removed on send.
    pub draft_id: Option<String>,
    pub quoted: Option<Quoted>,
    /// Under the conversation it answers, rather than alone in the pane.
    pub inline: bool,
    recipients: [Recipients; 3],
    show_cc_bcc: bool,
    subject: Entity<InputState>,
    /// Markdown, styled as it's typed (see `rich_editor`).
    body: Entity<EditorState>,
    /// The body's rows when last laid out, to grow it when they change.
    body_rows: usize,
    palette: Rc<Cell<Palette>>,
    attachments: Vec<Attachment>,
    /// The problem, and the field it's shown under.
    error: Option<(Field, SharedString)>,
    /// Scrolls the fields of a draft alone in the pane, above its send row.
    scroll: ScrollHandle,
    /// Scrolls a draft under its conversation: the conversation's.
    pub outer_scroll: Option<ScrollHandle>,
    /// The body's caret and length when last seen, to follow the caret.
    body_caret: (usize, usize),
    /// Frames left to bring the caret into view; layout can lag an edit.
    reveal_tries: u8,
    initial: DraftContent,
    book: SharedContactBook,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: ComposeKind,
        content: DraftContent,
        reply_to: Option<String>,
        draft_id: Option<String>,
        quoted: Option<Quoted>,
        inline: bool,
        book: SharedContactBook,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let recipients = [
            (Field::To, &content.to),
            (Field::Cc, &content.cc),
            (Field::Bcc, &content.bcc),
        ]
        .map(|(field, chips)| {
            let input = cx.new(|cx| InputState::new(window, cx));
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, _, event, window, cx| this.on_recipient_event(field, event, window, cx),
            ));
            Recipients {
                chips: chips.clone(),
                input,
                suggestions: Vec::new(),
                highlighted: 0,
                navigated: false,
            }
        });
        let subject = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Subject")
                .default_value(content.subject.clone())
        });
        subscriptions.push(
            cx.subscribe_in(&subject, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: false, ..
                } = event
                {
                    this.focus_field(Field::Body, window, cx);
                }
            }),
        );
        let palette = Rc::new(Cell::new(Palette::new(Theme::of(cx))));
        let body = cx.new(|cx| {
            let placeholder = match kind {
                ComposeKind::Forward => "Add a note…",
                _ => "Write your message…",
            };
            rich_editor::state(
                placeholder,
                content.body.clone(),
                palette.clone(),
                window,
                cx,
            )
        });
        // Sized to its rows, the body never scrolls itself: whatever moves the
        // caret, the draft's scroll follows it.
        subscriptions.push(cx.observe_in(&body, window, |this, body, window, cx| {
            let body = body.read(cx);
            let caret = (body.cursor(), body.text().len());
            if caret != this.body_caret {
                this.body_caret = caret;
                this.reveal_caret(window, cx);
            }
        }));
        subscriptions.push(cx.subscribe(&body, |this: &mut Self, body, event, cx| {
            if let InputEvent::Change = event {
                // The body's height follows its rows.
                let rows = body.read(cx).display_map().wrap_row_count();
                if rows != this.body_rows {
                    this.body_rows = rows;
                    cx.notify();
                }
                let body = body.read(cx);
                let at_end = body.cursor() >= body.text().len();
                // A growing draft alone in the pane keeps its end in view.
                if at_end && !this.inline && this.quoted.is_none() {
                    this.scroll.scroll_to_bottom();
                }
                cx.emit(ComposerEvent::Edited { at_end });
            }
        }));
        let body_caret = {
            let body = body.read(cx);
            (body.cursor(), body.text().len())
        };

        Self {
            kind,
            reply_to,
            draft_id,
            quoted,
            inline,
            show_cc_bcc: !content.cc.is_empty() || !content.bcc.is_empty(),
            recipients,
            subject,
            body,
            body_rows: 0,
            palette,
            attachments: content.attachments.clone(),
            error: None,
            scroll: ScrollHandle::new(),
            outer_scroll: None,
            body_caret,
            reveal_tries: 0,
            initial: content,
            book,
            _subscriptions: subscriptions,
        }
    }

    /// Whether the subject is a field of its own; replies keep the thread's.
    fn edits_subject(&self) -> bool {
        self.kind != ComposeKind::Reply && self.kind != ComposeKind::ReplyAll || !self.inline
    }

    /// The fields in tab order.
    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::To];
        if self.show_cc_bcc {
            fields.extend([Field::Cc, Field::Bcc]);
        }
        if self.edits_subject() {
            fields.push(Field::Subject);
        }
        fields.push(Field::Body);
        fields
    }

    fn focus_handle_of(&self, field: Field, cx: &gpui::App) -> FocusHandle {
        match field {
            Field::To => self.recipients[0].input.focus_handle(cx),
            Field::Cc => self.recipients[1].input.focus_handle(cx),
            Field::Bcc => self.recipients[2].input.focus_handle(cx),
            Field::Subject => self.subject.focus_handle(cx),
            Field::Body => self.body.focus_handle(cx),
        }
    }

    fn focused_field(&self, window: &Window, cx: &gpui::App) -> Option<Field> {
        self.fields()
            .into_iter()
            .find(|&f| self.focus_handle_of(f, cx).is_focused(window))
    }

    fn focus_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        match field {
            Field::To | Field::Cc | Field::Bcc => {
                let input = self.recipients[recipient_ix(field)].input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
            }
            Field::Subject => self.subject.update(cx, |s, cx| s.focus(window, cx)),
            Field::Body => self.body.update(cx, |s, cx| s.focus(window, cx)),
        }
        cx.notify();
    }

    /// Focus where writing starts: the first empty field, else the body.
    pub fn focus_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = if self.recipients[0].chips.is_empty() {
            Field::To
        } else if self.edits_subject() && self.subject.read(cx).value().is_empty() {
            Field::Subject
        } else {
            Field::Body
        };
        self.focus_field(field, window, cx);
    }

    pub fn focus_to(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_field(Field::To, window, cx);
    }

    pub fn show_cc_bcc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_cc_bcc = true;
        self.focus_field(Field::Cc, window, cx);
    }

    /// Show `error` under the To field.
    pub fn set_error(&mut self, error: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some((Field::To, error.into()));
        cx.notify();
    }

    /// Before sending: every recipient must be an address. Otherwise says
    /// which one isn't, under its field, and puts the caret there.
    pub fn validate(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        for (ix, field) in self.recipients.iter().enumerate() {
            let pending = field.input.read(cx).value();
            let bad = field
                .chips
                .iter()
                .map(|c| c.email.clone())
                .find(|email| parse_address(email).is_none())
                .or_else(|| {
                    pending
                        .split([',', ';'])
                        .map(str::trim)
                        .find(|part| {
                            !part.is_empty()
                                && parse_address(part).is_none()
                                && self.book.borrow().best(part).is_none()
                        })
                        .map(str::to_string)
                });
            if let Some(bad) = bad {
                let field = recipient_field(ix);
                self.error = Some((field, format!("“{bad}” isn’t an email address").into()));
                self.focus_field(field, window, cx);
                return false;
            }
        }
        true
    }

    /// What the draft says now. Addresses and names still being typed
    /// count as recipients, as if committed with a comma.
    pub fn content(&self, cx: &gpui::App) -> DraftContent {
        let recipients = |r: &Recipients| {
            let mut chips = r.chips.clone();
            let pending = r.input.read(cx).value();
            for part in pending.split([',', ';']).map(str::trim) {
                let contact = parse_address(part).or_else(|| {
                    (!part.is_empty() && !part.contains('@'))
                        .then(|| self.book.borrow().best(part).cloned())
                        .flatten()
                });
                chips.extend(contact);
            }
            chips
        };
        DraftContent {
            to: recipients(&self.recipients[0]),
            cc: recipients(&self.recipients[1]),
            bcc: recipients(&self.recipients[2]),
            subject: self.subject.read(cx).value().trim().to_string(),
            body: self.body.read(cx).value().trim_end().to_string(),
            attachments: self.attachments.clone(),
        }
    }

    /// Nothing changed since it opened. Text still in a recipient field
    /// counts as a change, even when it isn't an address yet.
    pub fn is_pristine(&self, cx: &gpui::App) -> bool {
        if self
            .recipients
            .iter()
            .any(|r| !r.input.read(cx).value().trim().is_empty())
        {
            return false;
        }
        let now = self.content(cx);
        let emails = |c: &[Contact]| c.iter().map(|c| c.email.to_lowercase()).collect::<Vec<_>>();
        now.attachments.len() == self.initial.attachments.len()
            && emails(&now.to) == emails(&self.initial.to)
            && emails(&now.cc) == emails(&self.initial.cc)
            && emails(&now.bcc) == emails(&self.initial.bcc)
            && now.subject == self.initial.subject.trim()
            && now.body == self.initial.body.trim_end()
    }

    /// Whether one of the draft's fields has focus.
    pub fn is_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.focused_field(window, cx).is_some()
    }

    /// The body as HTML, with the forwarded message quoted under it.
    pub fn html(&self, cx: &gpui::App) -> String {
        let mut out = self.content(cx).body_html();
        if let Some(quoted) = &self.quoted {
            out.push_str("<blockquote><p>---------- Forwarded message ----------");
            for (label, value) in &quoted.header {
                out.push_str(&format!("<br>{label}: {}", html::escape(value)));
            }
            out.push_str("</p>");
            out.push_str(&quoted.html);
            out.push_str("</blockquote>");
        }
        out
    }

    // Formatting.

    /// Applies `format` to the body's selection, and puts the caret back in
    /// the body.
    pub fn format(&mut self, format: Format, window: &mut Window, cx: &mut Context<Self>) {
        let edit = self.edit_lines(cx, |text, sel| {
            Some(match format {
                Format::Inline(kind) => markdown::toggle_inline(text, sel, kind),
                Format::Lines(kind) => markdown::toggle_lines(text, sel, kind),
                Format::Link => markdown::toggle_link(text, sel),
                Format::Clear => markdown::clear_formatting(text, sel),
            })
        });
        if let Some(edit) = edit {
            self.apply(edit, window, cx);
        }
        self.focus_field(Field::Body, window, cx);
    }

    /// `f`'s edit of the lines the selection is on, given just those lines,
    /// so a long body isn't copied for each one.
    fn edit_lines(
        &self,
        cx: &gpui::App,
        f: impl FnOnce(&str, Range<usize>) -> Option<Edit>,
    ) -> Option<Edit> {
        let body = self.body.read(cx);
        let text = body.text();
        let sel = body.selected_range();
        let start = text.line_start_offset(text.offset_to_point(sel.start).row);
        let end = text.line_end_offset(text.offset_to_point(sel.end).row);
        let lines = text.slice(start..end).to_string();
        f(&lines, sel.start - start..sel.end - start).map(|edit| edit.shifted(start))
    }

    fn apply(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<Self>) {
        self.body.update(cx, |body, cx| {
            if !(edit.range.is_empty() && edit.text.is_empty()) {
                body.set_selected_range(edit.range, cx);
                body.replace(edit.text, window, cx);
            }
            body.set_selected_range(edit.selection, cx);
        });
    }

    fn body_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.body.focus_handle(cx).is_focused(window)
    }

    /// Enter in a list or quote continues it; anywhere else it's the
    /// editor's own.
    fn body_enter(&mut self, _: &BodyEnter, window: &mut Window, cx: &mut Context<Self>) {
        let edit = self
            .body_focused(window, cx)
            .then(|| self.edit_lines(cx, markdown::enter))
            .flatten();
        match edit {
            Some(edit) => self.apply(edit, window, cx),
            None => cx.propagate(),
        }
    }

    /// Tab in a list indents it; elsewhere in the body it moves on, as in
    /// the other fields.
    fn body_indent(&mut self, _: &BodyIndent, window: &mut Window, cx: &mut Context<Self>) {
        self.body_tab(false, window, cx);
    }

    fn body_outdent(&mut self, _: &BodyOutdent, window: &mut Window, cx: &mut Context<Self>) {
        self.body_tab(true, window, cx);
    }

    fn body_tab(&mut self, outdent: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_focused(window, cx) {
            return cx.propagate();
        }
        match self.edit_lines(cx, |text, sel| markdown::indent(text, sel, outdent)) {
            Some(edit) => self.apply(edit, window, cx),
            None => self.move_focus(!outdent, window, cx),
        }
    }

    fn body_page_up(&mut self, _: &BodyPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.body_page(-1, false, window, cx);
    }

    fn body_page_down(&mut self, _: &BodyPageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.body_page(1, false, window, cx);
    }

    fn body_select_page_up(
        &mut self,
        _: &BodySelectPageUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.body_page(-1, true, window, cx);
    }

    fn body_select_page_down(
        &mut self,
        _: &BodySelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.body_page(1, true, window, cx);
    }

    /// The body is as tall as its text, so its own page is all of it: move
    /// by the rows that fit in the draft's scroll instead, less one to keep
    /// context. The caret-following scroll then turns the page.
    fn body_page(&mut self, dir: i8, select: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_focused(window, cx) {
            return cx.propagate();
        }
        let scroll = if self.inline {
            self.outer_scroll.as_ref()
        } else {
            Some(&self.scroll)
        };
        let (Some(scroll), Some(line_height)) = (scroll, self.body.read(cx).line_height()) else {
            return;
        };
        let rows = ((scroll.bounds().size.height / line_height) as usize)
            .saturating_sub(1)
            .max(1);
        let action: Box<dyn gpui::Action> = match (dir < 0, select) {
            (true, false) => Box::new(MoveUp),
            (false, false) => Box::new(MoveDown),
            (true, true) => Box::new(SelectUp),
            (false, true) => Box::new(SelectDown),
        };
        for _ in 0..rows {
            window.dispatch_action(action.boxed_clone(), cx);
        }
    }

    // Recipients.

    fn on_recipient_event(
        &mut self,
        field: Field,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = recipient_ix(field);
        match event {
            InputEvent::Change => {
                self.error = None;
                let value = self.recipients[ix].input.read(cx).value();
                if value.contains([',', ';']) {
                    // Everything before the last separator is finished.
                    let (done, rest) = value.rsplit_once([',', ';']).unwrap_or_default();
                    // What matches no one stays as text, with why (CON-2).
                    let mut unmatched: Vec<&str> = Vec::new();
                    for part in done.split([',', ';']).map(str::trim) {
                        match self.resolve(ix, part, false) {
                            Some(contact) => self.recipients[ix].chips.push(contact),
                            None if !part.is_empty() => unmatched.push(part),
                            None => {}
                        }
                    }
                    if let Some(first) = unmatched.first() {
                        let error = if first.contains('@') {
                            format!("“{first}” isn’t an email address")
                        } else {
                            format!("No contact named “{first}”")
                        };
                        self.error = Some((field, error.into()));
                    }
                    let rest = rest.trim_start();
                    if !rest.is_empty() {
                        unmatched.push(rest);
                    }
                    let rest = unmatched.join(", ");
                    let input = self.recipients[ix].input.clone();
                    input.update(cx, |input, cx| input.set_value(rest, window, cx));
                }
                self.update_suggestions(ix, cx);
            }
            InputEvent::PressEnter {
                secondary: false, ..
            } => {
                if !self.commit(ix, window, cx) {
                    self.move_focus(true, window, cx);
                }
            }
            InputEvent::Blur => {
                self.recipients[ix].suggestions.clear();
                cx.notify();
            }
            _ => {}
        }
    }

    fn update_suggestions(&mut self, ix: usize, cx: &mut Context<Self>) {
        let query = self.recipients[ix].input.read(cx).value();
        let recipients = &self.recipients;
        let taken = |c: &Contact| {
            recipients
                .iter()
                .flat_map(|r| &r.chips)
                .any(|chip| chip.email.eq_ignore_ascii_case(&c.email))
        };
        let suggestions = self.book.borrow().suggest(&query, taken);
        let field = &mut self.recipients[ix];
        field.suggestions = suggestions;
        field.highlighted = 0;
        field.navigated = false;
        cx.notify();
    }

    /// The contact `text` stands for: the highlighted suggestion, or the
    /// address as typed. A typed address wins over a fuzzy match unless the
    /// user picked a suggestion.
    fn resolve(&mut self, ix: usize, text: &str, use_highlight: bool) -> Option<Contact> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let field = &self.recipients[ix];
        let book = self.book.borrow();
        let exact = book.find(text).cloned();
        if use_highlight
            && (field.navigated || !text.contains('@'))
            && let Some(&contact_ix) = field.suggestions.get(field.highlighted)
        {
            return Some(book.contact(contact_ix).clone());
        }
        if !use_highlight && !text.contains('@') {
            // Committed with a comma: take the best match for the name.
            return book.best(text).cloned();
        }
        exact.or_else(|| parse_address(text))
    }

    /// Turn the typed text into a chip. False when there was nothing to commit.
    fn commit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.recipients[ix].input.read(cx).value();
        if text.trim().is_empty() {
            return false;
        }
        match self.resolve(ix, &text, true) {
            Some(contact) => {
                self.recipients[ix].chips.push(contact);
                let input = self.recipients[ix].input.clone();
                input.update(cx, |input, cx| input.set_value("", window, cx));
                self.update_suggestions(ix, cx);
            }
            None => {
                self.error = Some((
                    recipient_field(ix),
                    format!("“{}” isn’t an email address", text.trim()).into(),
                ));
                cx.notify();
            }
        }
        true
    }

    fn commit_suggestion(
        &mut self,
        ix: usize,
        suggestion: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.recipients[ix].highlighted = suggestion;
        self.recipients[ix].navigated = true;
        self.commit(ix, window, cx);
        self.focus_field(recipient_field(ix), window, cx);
    }

    fn remove_chip(&mut self, ix: usize, chip: usize, cx: &mut Context<Self>) {
        self.recipients[ix].chips.remove(chip);
        cx.notify();
    }

    fn focused_recipients(&self, window: &Window, cx: &gpui::App) -> Option<usize> {
        match self.focused_field(window, cx)? {
            f @ (Field::To | Field::Cc | Field::Bcc) => Some(recipient_ix(f)),
            _ => None,
        }
    }

    fn move_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let fields = self.fields();
        let current = self
            .focused_field(window, cx)
            .and_then(|f| fields.iter().position(|&x| x == f));
        let next = match (current, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(fields.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.focus_field(fields[next], window, cx);
    }

    // Key handlers.

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.focused_recipients(window, cx)
            && self.commit(ix, window, cx)
        {
            return;
        }
        self.move_focus(true, window, cx);
    }

    fn prev_field(&mut self, _: &PrevField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(false, window, cx);
    }

    fn suggest(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.focused_recipients(window, cx) else {
            return cx.propagate();
        };
        let field = &mut self.recipients[ix];
        let count = field.suggestions.len();
        if count == 0 {
            return cx.propagate();
        }
        field.highlighted =
            (field.highlighted as isize + delta).rem_euclid(count as isize) as usize;
        field.navigated = true;
        cx.notify();
    }

    fn suggest_next(&mut self, _: &SuggestNext, window: &mut Window, cx: &mut Context<Self>) {
        self.suggest(1, window, cx);
    }

    fn suggest_prev(&mut self, _: &SuggestPrev, window: &mut Window, cx: &mut Context<Self>) {
        self.suggest(-1, window, cx);
    }

    fn remove_last_recipient(
        &mut self,
        _: &RemoveLastRecipient,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.focused_recipients(window, cx) {
            Some(ix) if self.recipients[ix].input.read(cx).value().is_empty() => {
                self.recipients[ix].chips.pop();
                cx.notify();
            }
            _ => cx.propagate(),
        }
    }

    fn dismiss_suggestions(
        &mut self,
        _: &DismissSuggestions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.focused_recipients(window, cx) {
            Some(ix) if !self.recipients[ix].suggestions.is_empty() => {
                self.recipients[ix].suggestions.clear();
                cx.notify();
            }
            _ => cx.propagate(),
        }
    }

    // Rendering.

    fn render_recipients(
        &self,
        ix: usize,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let field = &self.recipients[ix];
        let label = ["To", "Cc", "Bcc"][ix];
        let focused = field.input.focus_handle(cx).is_focused(window);

        let chips = field.chips.iter().enumerate().map(|(chip_ix, contact)| {
            let invalid = parse_address(&contact.email).is_none();
            div()
                .id(("chip", ix * 1000 + chip_ix))
                .h(px(24.))
                .pl(SPACE_XS)
                .pr(SPACE_XS)
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .rounded_full()
                .bg(if invalid {
                    theme.danger.alpha(0.16)
                } else {
                    theme.chip_bg
                })
                .border_1()
                .border_color(if invalid { theme.danger } else { theme.border })
                .child(avatar(
                    contact.initials(),
                    theme.avatar_color(&contact.email),
                    px(18.),
                    theme,
                ))
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .font_weight(FontWeight::MEDIUM)
                        .child(contact.display_name().to_string()),
                )
                .child(
                    div()
                        .id(("chip-remove", ix * 1000 + chip_ix))
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .hover(|s| s.bg(theme.hover_bg))
                        .child(icon(Icon::X, px(11.), theme.text_faint))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.remove_chip(ix, chip_ix, cx)),
                        ),
                )
        });

        let suggestions = (focused && !field.suggestions.is_empty()).then(|| {
            let rows = field
                .suggestions
                .iter()
                .enumerate()
                .map(|(row, &contact_ix)| {
                    let contact = self.book.borrow().contact(contact_ix).clone();
                    let highlighted = row == field.highlighted;
                    div()
                        .id(("suggestion", row))
                        .h(px(40.))
                        .px(SPACE_SM)
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .rounded(RADIUS_MD)
                        .when(highlighted, |d| d.bg(theme.selected_bg))
                        .hover(|d| d.bg(theme.hover_bg))
                        .child(avatar(
                            contact.initials(),
                            theme.avatar_color(&contact.email),
                            px(26.),
                            theme,
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(TEXT_BASE)
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(contact.display_name().to_string()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(TEXT_SM)
                                        .text_color(theme.text_muted)
                                        .child(contact.email.clone()),
                                ),
                        )
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.commit_suggestion(ix, row, window, cx)
                        }))
                });
            // `anchored` is absolutely positioned: a zero-height box after
            // the input puts it under the input rather than over it.
            div().w_full().h(px(0.)).child(
                deferred(
                    anchored().snap_to_window_with_margin(px(8.)).child(
                        div()
                            .occlude()
                            .mt(SPACE_SM)
                            .w(px(320.))
                            .p(SPACE_XS)
                            .flex()
                            .flex_col()
                            .rounded(RADIUS_MD)
                            .bg(theme.overlay_bg)
                            .border_1()
                            .border_color(theme.overlay_border)
                            .shadow(super::popover_shadow(theme))
                            .children(rows),
                    ),
                )
                .with_priority(1),
            )
        });

        let error = self
            .error
            .as_ref()
            .filter(|(at, _)| *at == recipient_field(ix))
            .map(|(_, error)| {
                div()
                    .pl(px(60.))
                    .pb(SPACE_SM)
                    .flex()
                    .items_center()
                    .gap(SPACE_XS)
                    .text_size(TEXT_SM)
                    .text_color(theme.danger)
                    .child(icon(Icon::OctagonAlert, px(13.), theme.danger))
                    .child(error.clone())
            });

        let row = div()
            .min_h(px(40.))
            .py(SPACE_SM)
            .flex()
            .items_start()
            .gap(SPACE_SM)
            .child(field_label(label, theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(SPACE_XS)
                    .children(chips)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(140.))
                            .h(px(28.))
                            .flex()
                            .flex_col()
                            .child(
                                Input::new(&field.input)
                                    .appearance(false)
                                    .px(SPACE_2XS)
                                    .h(px(28.)),
                            )
                            .children(suggestions),
                    ),
            )
            .when(ix == 0 && !self.show_cc_bcc, |this| {
                this.child(
                    div()
                        .id("show-cc-bcc")
                        .flex_none()
                        .h(px(28.))
                        .px(SPACE_SM)
                        .flex()
                        .items_center()
                        .rounded(RADIUS_SM)
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                        .child("Cc Bcc")
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowCcBcc), cx)),
                )
            });
        div()
            .key_context(RECIPIENTS)
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme.border)
            .child(row)
            .children(error)
    }

    // Attachments.

    fn attach_paths(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        for path in paths {
            let Ok(meta) = std::fs::metadata(path) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.attachments
                .push(Attachment::from_file(name, meta.len()));
        }
        cx.notify();
    }

    /// The native open panel; the files picked are attached.
    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| this.attach_paths(&paths, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn render_attachments(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let chips = self.attachments.iter().enumerate().map(|(ix, attachment)| {
            div()
                .id(("attachment", ix))
                .h(px(32.))
                .pl(SPACE_SM)
                .pr(SPACE_XS)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .bg(theme.chip_bg)
                .child(attachment_icon(attachment, theme))
                .child(
                    div()
                        .max_w(px(220.))
                        .truncate()
                        .text_size(TEXT_SM)
                        .font_weight(FontWeight::MEDIUM)
                        .child(attachment.name.clone()),
                )
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_faint)
                        .child(file_size(attachment.size_bytes)),
                )
                .child(
                    div()
                        .id(("attachment-remove", ix))
                        .role(gpui::Role::Button)
                        .aria_label(format!("Remove {}", attachment.name))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(RADIUS_SM)
                        .hover(|s| s.bg(theme.hover_bg))
                        .child(icon(Icon::X, px(12.), theme.text_faint))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.attachments.remove(ix);
                            cx.notify();
                        })),
                )
        });
        Some(
            div()
                .mt(SPACE_MD)
                .flex()
                .flex_wrap()
                .gap(SPACE_SM)
                .children(chips),
        )
    }
}

impl Composer {
    /// Bring the body's caret into view over the next frames.
    fn reveal_caret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reveal_tries == 0 {
            cx.on_next_frame(window, Self::reveal_step);
        }
        self.reveal_tries = 4;
    }

    /// Scroll just enough to show the caret with a row to spare, then check
    /// again next frame, as an edit can still change the layout.
    fn reveal_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reveal_tries == 0 {
            return;
        }
        let scroll = if self.inline {
            self.outer_scroll.as_ref()
        } else {
            Some(&self.scroll)
        };
        let body = self.body.read(cx);
        let (Some(scroll), Some(line_height), Some(text)) =
            (scroll, body.line_height(), body.text_bounds())
        else {
            self.reveal_tries = 0;
            return;
        };
        let view = scroll.bounds();
        let point = body.text().offset_to_point(body.cursor());
        let row = body
            .display_map()
            .buffer_pos_to_display_pos(BufferPoint::new(point.row, point.column))
            .row;
        let top = text.top() + line_height * row as f32 - line_height;
        let bottom = top + line_height * 3.;
        let mut offset = scroll.offset();
        let delta = if view.size.height < line_height * 3. {
            px(0.)
        } else if top < view.top() {
            view.top() - top
        } else if bottom > view.bottom() {
            view.bottom() - bottom
        } else {
            px(0.)
        };
        if delta == px(0.) {
            self.reveal_tries = 0;
            return;
        }
        offset.y = (offset.y + delta).min(px(0.));
        scroll.set_offset(offset);
        self.reveal_tries -= 1;
        cx.on_next_frame(window, Self::reveal_step);
        cx.notify();
    }

    fn render_body(
        &self,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The editor scrolls on its own; sized to its rows, it grows like a
        // textarea and the draft scrolls instead.
        const PADDING_Y: f32 = 8.;
        let min_rows = if self.inline { 5 } else { 14 };
        let rows = self.body_rows.max(min_rows);
        let line_height = self
            .body
            .read(cx)
            .line_height()
            .unwrap_or_else(|| TEXT_MD.to_pixels(window.rem_size()) * 1.55);
        let height = line_height * rows as f32 + px(PADDING_Y * 2.);
        // Cancels the editor's own padding, so text lines up with the field
        // labels.
        div().pt(SPACE_SM).ml(px(-12.)).mr(px(-10.)).child(
            Editor::new(&self.body)
                .appearance(false)
                .aria_label("Message body")
                .h(height)
                .font_family(cx.theme().font_family.clone())
                .text_size(TEXT_MD)
                .line_height(relative(1.55))
                .text_color(theme.text),
        )
    }

    fn render_format_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("format-bar")
            .role(gpui::Role::Toolbar)
            .aria_label("Formatting")
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .children(
                Format::TOOLBAR
                    .iter()
                    .enumerate()
                    .map(|(ix, &(format, glyph, name))| {
                        let action = format.action();
                        div()
                            .id(("format", ix))
                            .role(gpui::Role::Button)
                            .aria_label(name)
                            .size(HIT_TARGET)
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS_SM)
                            .hover(|s| s.bg(theme.hover_bg))
                            .active(|s| s.bg(theme.selected_bg))
                            .child(icon(glyph, px(15.), theme.icon))
                            // Keeps the body's focus and selection.
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                window.prevent_default()
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.format(format, window, cx)
                            }))
                            .tooltip(move |window, cx| {
                                gpui::component::tooltip::Tooltip::new(name)
                                    .action(action.as_ref(), None)
                                    .build(window, cx)
                            })
                    }),
            )
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        self.palette.set(Palette::new(&theme));
        self.body_rows = self.body.read(cx).display_map().wrap_row_count();
        // How lines wrap is known once the body is laid out: grow then, if
        // they wrapped differently.
        cx.on_next_frame(window, |this, _, cx| {
            if this.body.read(cx).display_map().wrap_row_count() != this.body_rows {
                cx.notify();
            }
        });
        let standalone = !self.inline;

        let title = match (self.kind, self.draft_id.is_some()) {
            (_, true) => "Draft",
            (ComposeKind::New, _) => "New Message",
            (ComposeKind::Reply, _) => "Reply",
            (ComposeKind::ReplyAll, _) => "Reply All",
            (ComposeKind::Forward, _) => "Forward",
        };

        let quoted = self.quoted.as_ref().map(|quoted| {
            div()
                .mt(SPACE_LG)
                .pl(SPACE_LG)
                .border_l_2()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(SPACE_2XS)
                .text_size(TEXT_SM)
                .text_color(theme.text_muted)
                .child(
                    div()
                        .pb(SPACE_XS)
                        .text_color(theme.text_faint)
                        .child("Forwarded message"),
                )
                .children(quoted.header.iter().map(|(label, value)| {
                    div()
                        .flex()
                        .gap(SPACE_SM)
                        .child(
                            div()
                                .flex_none()
                                .text_color(theme.text_faint)
                                .child(label.clone()),
                        )
                        .child(div().min_w_0().truncate().child(value.clone()))
                }))
                .child(
                    div()
                        .mt(SPACE_MD)
                        .text_size(TEXT_BASE)
                        .line_height(relative(1.55))
                        .text_color(theme.text_muted)
                        .child(html_view::render(&quoted.doc, &theme)),
                )
        });

        let hint = |keys: &'static [&'static str], label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .children(keys.iter().map(|k| keycap(*k, &theme)))
                .child(div().ml(SPACE_2XS).child(label))
        };

        let send_row = div()
            .flex()
            .items_center()
            .gap(SPACE_MD)
            .child(
                div()
                    .id("send")
                    .role(gpui::Role::Button)
                    .aria_label("Send")
                    .h(px(32.))
                    .pl(SPACE_MD)
                    .pr(SPACE_LG)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .bg(theme.accent)
                    .text_color(theme.accent_text)
                    .text_size(TEXT_BASE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .hover(|s| s.opacity(0.9))
                    .active(|s| s.opacity(0.8))
                    .child(icon(Icon::Send, px(14.), theme.accent_text))
                    .child("Send")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(SendMessage), cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_MD)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(hint(&["⌘", "↵"], "Send"))
                    .child(hint(&["Esc"], "Save draft")),
            )
            .child(div().flex_1())
            .child(
                super::style::focusable_control(div().id("attach"), &theme)
                    .role(gpui::Role::Button)
                    .aria_label("Attach files")
                    .size(px(30.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_MD)
                    .hover(|s| s.bg(theme.hover_bg))
                    .active(|s| s.bg(theme.selected_bg))
                    .child(icon(Icon::Paperclip, px(16.), theme.icon))
                    .tooltip(|window, cx| {
                        gpui::component::tooltip::Tooltip::new("Attach files (or drop them here)")
                            .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))),
            )
            // Alone in the pane, the toolbar above has these.
            .when(!standalone, |this| {
                this.child(super::icon_button(
                    "discard",
                    Icon::Trash,
                    "Discard draft",
                    Box::new(DiscardDraft),
                    &theme,
                ))
            });

        let footer = div()
            .flex()
            .flex_col()
            .gap(SPACE_SM)
            .child(self.render_format_bar(&theme, cx))
            .child(send_row);

        let fields = div()
            .flex()
            .flex_col()
            .when(!standalone, |this| {
                this.child(
                    div()
                        .pb(SPACE_XS)
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .text_size(TEXT_SM)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_muted)
                        .child(icon(
                            match self.kind {
                                ComposeKind::ReplyAll => Icon::ReplyAll,
                                ComposeKind::Forward => Icon::Forward,
                                _ => Icon::Reply,
                            },
                            px(14.),
                            theme.icon,
                        ))
                        .child(title),
                )
            })
            .child(self.render_recipients(0, window, &theme, cx))
            .when(self.show_cc_bcc, |this| {
                this.child(self.render_recipients(1, window, &theme, cx))
                    .child(self.render_recipients(2, window, &theme, cx))
            })
            .when(self.edits_subject(), |this| {
                this.child(
                    div()
                        .h(px(42.))
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .border_b_1()
                        .border_color(theme.border)
                        .child(field_label("Subject", &theme))
                        .child(
                            div()
                                .flex_1()
                                .text_size(TEXT_MD)
                                .font_weight(FontWeight::MEDIUM)
                                .child(Input::new(&self.subject).appearance(false).px(px(0.))),
                        ),
                )
            })
            .child(self.render_body(window, &theme, cx))
            .children(self.render_attachments(&theme, cx))
            .children(quoted);

        let drop_tint = theme.accent.alpha(0.06);
        let drop_ring = theme.accent;
        div()
            .id("composer")
            .key_context(COMPOSER)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::prev_field))
            .on_action(cx.listener(Self::suggest_next))
            .on_action(cx.listener(Self::suggest_prev))
            .on_action(cx.listener(Self::remove_last_recipient))
            .on_action(cx.listener(Self::dismiss_suggestions))
            .on_action(cx.listener(Self::body_enter))
            .on_action(cx.listener(Self::body_indent))
            .on_action(cx.listener(Self::body_outdent))
            .on_action(cx.listener(Self::body_page_up))
            .on_action(cx.listener(Self::body_page_down))
            .on_action(cx.listener(Self::body_select_page_up))
            .on_action(cx.listener(Self::body_select_page_down))
            .map(|this| {
                let composer = cx.entity().downgrade();
                on_format_actions(this, move |format, window, cx| {
                    let Some(composer) = composer.upgrade() else {
                        return;
                    };
                    // Only in the body: elsewhere ⌘K is the palette, ⌘\ the
                    // sidebar.
                    if composer.read(cx).body_focused(window, cx) {
                        composer.update(cx, |c, cx| c.format(format, window, cx));
                    } else {
                        cx.propagate();
                    }
                })
            })
            // Files dropped anywhere on the draft are attached.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.attach_paths(paths.paths(), cx)
            }))
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.bg(drop_tint).border_color(drop_ring)
            })
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(gpui::transparent_black())
            .text_size(TEXT_BASE)
            .map(|this| {
                if standalone {
                    // The send row stays at the bottom of the pane while the
                    // fields scroll above it.
                    this.size_full()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .child(
                                    div()
                                        .id("compose-scroll")
                                        .size_full()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.scroll)
                                        .px(SPACE_XL)
                                        .pt(SPACE_2XS)
                                        .pb(SPACE_LG)
                                        .child(fields),
                                )
                                .vertical_scrollbar(&self.scroll),
                        )
                        .child(
                            footer
                                .flex_none()
                                .px(SPACE_XL)
                                .py(SPACE_MD)
                                .border_t_1()
                                .border_color(theme.border),
                        )
                } else {
                    this.flex()
                        .flex_col()
                        .child(fields)
                        .child(footer.mt(SPACE_LG))
                }
            })
    }
}

fn field_label(label: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(52.))
        .h(px(28.))
        .flex()
        .items_center()
        .text_size(TEXT_SM)
        .text_color(theme.text_faint)
        .child(label)
}

fn recipient_ix(field: Field) -> usize {
    match field {
        Field::To => 0,
        Field::Cc => 1,
        _ => 2,
    }
}

fn recipient_field(ix: usize) -> Field {
    [Field::To, Field::Cc, Field::Bcc][ix]
}

/// "Name <a@b.c>" or a bare address.
pub fn parse_address(text: &str) -> Option<Contact> {
    let text = text.trim();
    let (name, email) = match text.rsplit_once('<') {
        Some((name, rest)) => (name.trim().trim_matches('"'), rest.trim_end_matches('>')),
        None => ("", text),
    };
    let email = email.trim();
    let (local, domain) = email.split_once('@')?;
    (!local.is_empty() && domain.contains('.') && !email.contains(char::is_whitespace)).then(|| {
        Contact {
            name: name.to_string(),
            email: email.to_string(),
        }
    })
}

//! A draft: recipients as chips with contact suggestions, subject and body.
//! It lives in the reading pane, alone for a new message or under the
//! conversation for a reply. Sending, closing and discarding are app actions
//! (see `PigeonApp`), so they work from the palette too; this view owns only
//! the editing.

use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, actions, anchored,
    component::input::{Input, InputEvent, InputState, Textarea, TextareaState},
    deferred, div,
    prelude::FluentBuilder as _,
    px, relative,
};
use nucleo_matcher::{
    Config, Matcher, Utf32String,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

use super::{avatar, html_view, icon, keycap};
use crate::{
    actions::{COMPOSER, DiscardDraft, RECIPIENTS, SendMessage, ShowCcBcc},
    app::ComposeKind,
    assets::Icon,
    data::Contact,
    html::{self, Document},
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
    ]
);

const MAX_SUGGESTIONS: usize = 6;

/// Everyone the user could write to, with what suggestions match against.
pub struct ContactBook {
    contacts: Vec<Contact>,
    haystacks: Vec<Utf32String>,
}

impl ContactBook {
    pub fn new(contacts: Vec<Contact>) -> Self {
        let haystacks = contacts
            .iter()
            .map(|c| format!("{} {}", c.name, c.email).into())
            .collect();
        Self {
            contacts,
            haystacks,
        }
    }
}

/// What a draft says, read out of the fields.
#[derive(Clone, Debug, Default)]
pub struct DraftContent {
    pub to: Vec<Contact>,
    pub cc: Vec<Contact>,
    pub bcc: Vec<Contact>,
    pub subject: String,
    pub body: String,
}

impl DraftContent {
    pub fn has_recipients(&self) -> bool {
        !(self.to.is_empty() && self.cc.is_empty() && self.bcc.is_empty())
    }
}

/// The forwarded message, quoted under the body.
pub struct Quoted {
    /// "From", "Date", "Subject", "To" lines.
    pub header: Vec<(&'static str, String)>,
    pub html: String,
    doc: Rc<Document>,
}

impl Quoted {
    pub fn new(header: Vec<(&'static str, String)>, html: String) -> Self {
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
    /// The thread replied to or forwarded, by id.
    pub reply_to: Option<String>,
    /// The Drafts thread this edits: replaced on save, removed on send.
    pub draft_id: Option<String>,
    pub quoted: Option<Quoted>,
    recipients: [Recipients; 3],
    show_cc_bcc: bool,
    subject: Entity<InputState>,
    body: Entity<TextareaState>,
    error: Option<SharedString>,
    initial: DraftContent,
    book: Rc<ContactBook>,
    matcher: Matcher,
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
        book: Rc<ContactBook>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let standalone = reply_to.is_none() || draft_id.is_some();
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
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(if standalone { 14 } else { 5 }, 100_000)
                .placeholder(match kind {
                    ComposeKind::Forward => "Add a note…",
                    _ => "Write your message…",
                })
                .default_value(content.body.clone())
        });
        subscriptions.push(cx.subscribe(&body, |_, body, event, cx| {
            if let InputEvent::Change = event {
                let body = body.read(cx);
                let at_end = body.cursor() >= body.text().len();
                cx.emit(ComposerEvent::Edited { at_end });
            }
        }));

        Self {
            kind,
            reply_to,
            draft_id,
            quoted,
            show_cc_bcc: !content.cc.is_empty() || !content.bcc.is_empty(),
            recipients,
            subject,
            body,
            error: None,
            initial: content,
            book,
            matcher: Matcher::new(Config::DEFAULT),
            _subscriptions: subscriptions,
        }
    }

    /// Whether the subject is a field of its own; replies keep the thread's.
    fn edits_subject(&self) -> bool {
        self.kind != ComposeKind::Reply && self.kind != ComposeKind::ReplyAll
            || self.draft_id.is_some()
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

    pub fn set_error(&mut self, error: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(error.into());
        cx.notify();
    }

    /// What the draft says now. Addresses still being typed count as
    /// recipients.
    pub fn content(&self, cx: &gpui::App) -> DraftContent {
        let recipients = |r: &Recipients| {
            let mut chips = r.chips.clone();
            let pending = r.input.read(cx).value();
            for part in pending.split([',', ';']) {
                if let Some(contact) = parse_address(part) {
                    chips.push(contact);
                }
            }
            chips
        };
        DraftContent {
            to: recipients(&self.recipients[0]),
            cc: recipients(&self.recipients[1]),
            bcc: recipients(&self.recipients[2]),
            subject: self.subject.read(cx).value().trim().to_string(),
            body: self.body.read(cx).value().trim_end().to_string(),
        }
    }

    /// Nothing changed since it opened.
    pub fn is_pristine(&self, cx: &gpui::App) -> bool {
        let now = self.content(cx);
        let emails = |c: &[Contact]| c.iter().map(|c| c.email.to_lowercase()).collect::<Vec<_>>();
        emails(&now.to) == emails(&self.initial.to)
            && emails(&now.cc) == emails(&self.initial.cc)
            && emails(&now.bcc) == emails(&self.initial.bcc)
            && now.subject == self.initial.subject.trim()
            && now.body == self.initial.body.trim_end()
    }

    /// The body as HTML, with the forwarded message quoted under it.
    pub fn html(&self, cx: &gpui::App) -> String {
        let mut out = html::from_plain_text(&self.content(cx).body);
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
                    for part in done.split([',', ';']) {
                        if let Some(contact) = self.resolve(ix, part, false) {
                            self.recipients[ix].chips.push(contact);
                        }
                    }
                    let rest = rest.trim_start().to_string();
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
        let pattern = Pattern::new(
            query.trim(),
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let taken = |c: &Contact| {
            self.recipients
                .iter()
                .flat_map(|r| &r.chips)
                .any(|chip| chip.email.eq_ignore_ascii_case(&c.email))
        };
        let mut scored = Vec::new();
        if !pattern.atoms.is_empty() {
            for (contact_ix, haystack) in self.book.haystacks.iter().enumerate() {
                if let Some(score) = pattern.score(haystack.slice(..), &mut self.matcher)
                    && !taken(&self.book.contacts[contact_ix])
                {
                    scored.push((score, contact_ix));
                }
            }
        }
        // Stable, so ties keep the book's order: most frequent first.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        let field = &mut self.recipients[ix];
        field.suggestions = scored
            .into_iter()
            .take(MAX_SUGGESTIONS)
            .map(|(_, ix)| ix)
            .collect();
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
        let book = &self.book.contacts;
        let exact = book
            .iter()
            .find(|c| c.email.eq_ignore_ascii_case(text))
            .cloned();
        if use_highlight
            && (field.navigated || !text.contains('@'))
            && let Some(&contact_ix) = field.suggestions.get(field.highlighted)
        {
            return Some(book[contact_ix].clone());
        }
        if !use_highlight && !text.contains('@') {
            // Committed with a comma: take the best match for the name.
            let pattern = Pattern::new(
                text,
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
            );
            let best = self
                .book
                .haystacks
                .iter()
                .enumerate()
                .filter_map(|(i, h)| Some((pattern.score(h.slice(..), &mut self.matcher)?, i)))
                .max_by_key(|(score, i)| (*score, std::cmp::Reverse(*i)));
            return best.map(|(_, i)| book[i].clone());
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
            None => self.set_error(format!("“{}” isn’t an email address", text.trim()), cx),
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
            let invalid = !contact.email.contains('@');
            div()
                .id(("chip", ix * 1000 + chip_ix))
                .h(px(24.))
                .pl(px(3.))
                .pr(px(4.))
                .flex()
                .items_center()
                .gap(px(5.))
                .rounded_full()
                .bg(if invalid {
                    theme.danger.alpha(0.16)
                } else {
                    theme.chip_bg
                })
                .border_1()
                .border_color(theme.border)
                .child(avatar(
                    contact.initials(),
                    theme.avatar_color(&contact.email),
                    px(18.),
                    theme,
                ))
                .child(
                    div()
                        .text_size(px(12.5))
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
                    let contact = &self.book.contacts[contact_ix];
                    let highlighted = row == field.highlighted;
                    div()
                        .id(("suggestion", row))
                        .h(px(40.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(7.))
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
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(contact.display_name().to_string()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(11.5))
                                        .text_color(theme.text_muted)
                                        .child(contact.email.clone()),
                                ),
                        )
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.commit_suggestion(ix, row, window, cx)
                        }))
                });
            deferred(
                anchored().snap_to_window().child(
                    div()
                        .occlude()
                        .mt(px(4.))
                        .w(px(320.))
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .rounded(px(10.))
                        .bg(theme.overlay_bg)
                        .border_1()
                        .border_color(theme.overlay_border)
                        .shadow(super::overlay_shadow(theme))
                        .children(rows),
                ),
            )
            .with_priority(1)
        });

        div()
            .key_context(RECIPIENTS)
            .min_h(px(40.))
            .py(px(6.))
            .flex()
            .items_start()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme.border)
            .child(field_label(label, theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.))
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
                                    .px(px(2.))
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
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                        .child("Cc Bcc")
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowCcBcc), cx)),
                )
            })
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let standalone = self.reply_to.is_none() || self.draft_id.is_some();

        let title = match (self.kind, self.draft_id.is_some()) {
            (_, true) => "Draft",
            (ComposeKind::New, _) => "New Message",
            (ComposeKind::Reply, _) => "Reply",
            (ComposeKind::ReplyAll, _) => "Reply All",
            (ComposeKind::Forward, _) => "Forward",
        };

        let quoted = self.quoted.as_ref().map(|quoted| {
            div()
                .mt(px(16.))
                .pl(px(14.))
                .border_l_2()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(2.))
                .text_size(px(12.5))
                .text_color(theme.text_muted)
                .child(
                    div()
                        .pb(px(4.))
                        .text_color(theme.text_faint)
                        .child("Forwarded message"),
                )
                .children(quoted.header.iter().map(|(label, value)| {
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(div().flex_none().text_color(theme.text_faint).child(*label))
                        .child(div().min_w_0().truncate().child(value.clone()))
                }))
                .child(
                    div()
                        .mt(px(12.))
                        .text_size(px(13.5))
                        .line_height(relative(1.55))
                        .text_color(theme.text_muted)
                        .child(html_view::render(&quoted.doc, &theme)),
                )
        });

        let hint = |keys: &'static [&'static str], label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .children(keys.iter().map(|k| keycap(*k, &theme)))
                .child(div().ml(px(2.)).child(label))
        };

        let footer = div()
            .mt(px(14.))
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .id("send")
                    .h(px(32.))
                    .pl(px(12.))
                    .pr(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded(px(8.))
                    .bg(theme.accent)
                    .text_color(theme.accent_text)
                    .text_size(px(13.))
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
                    .gap(px(14.))
                    .text_size(px(11.5))
                    .text_color(theme.text_faint)
                    .child(hint(&["⌘", "↵"], "Send"))
                    .child(hint(&["Esc"], "Save draft")),
            )
            .child(div().flex_1())
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(theme.danger)
                    .child(error)
            }))
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

        div()
            .key_context(COMPOSER)
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::prev_field))
            .on_action(cx.listener(Self::suggest_next))
            .on_action(cx.listener(Self::suggest_prev))
            .on_action(cx.listener(Self::remove_last_recipient))
            .on_action(cx.listener(Self::dismiss_suggestions))
            .flex()
            .flex_col()
            .text_size(px(13.5))
            .when(!standalone, |this| {
                this.child(
                    div()
                        .pb(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
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
                        .gap(px(8.))
                        .border_b_1()
                        .border_color(theme.border)
                        .child(field_label("Subject", &theme))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(Input::new(&self.subject).appearance(false).px(px(0.))),
                        ),
                )
            })
            .child(
                // Cancels the textarea's own padding, so text lines up with
                // the field labels.
                div()
                    .pt(px(8.))
                    .mx(px(-10.))
                    .text_size(px(14.))
                    .line_height(relative(1.55))
                    .child(Textarea::new(&self.body).appearance(false)),
            )
            .children(quoted)
            .child(footer)
    }
}

fn field_label(label: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(52.))
        .h(px(28.))
        .flex()
        .items_center()
        .text_size(px(12.5))
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
fn parse_address(text: &str) -> Option<Contact> {
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

//! A draft: recipients as chips with contact suggestions, subject and body.
//! It lives in the reading pane, alone for a new message or under the
//! conversation for a reply. Sending, closing and discarding are app actions
//! (see `PigeonApp`), so they work from the palette too; this view owns only
//! the editing.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, ExternalPaths, FocusHandle,
    Focusable as _, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    PathPromptOptions, Pixels, Rems, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, actions,
    base::{
        actions::{SelectDown, SelectUp},
        input::RopeExt as _,
    },
    component::{
        ActiveTheme as _,
        input::{
            self, BufferPoint, Editor, EditorState, Input, InputEvent, InputState, MoveDown, MoveUp,
        },
        native_menu::NativeMenu,
        scroll::ScrollableElement as _,
    },
    div,
    prelude::FluentBuilder as _,
    px, rems,
};

use super::{
    attachment_icon, avatar, controls, html_view, icon, keys_caps,
    rich_editor::{self, Palette},
};
use crate::ui::style::*;
use crate::{
    actions::{COMPOSER, DiscardDraft, FROM_PICKER, RECIPIENTS, SendMessage, ShowCcBcc},
    app::ComposeKind,
    assets::Icon,
    contacts::SharedContactBook,
    data::{Attachment, Contact, SendAs, file_size},
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
        // W3: the From picker (MST-1).
        FromActivate,
        FromNext,
        FromPrev,
        FromDismiss,
        // A recipient chip's context menu (MB-024), on `menu_chip`.
        CopyChipAddress,
        ShowChipContact,
        MoveChipToTo,
        MoveChipToCc,
        MoveChipToBcc,
        RemoveChip,
    ]
);

/// W3 (DRF-5): a draft is saved here this long after typing stops.
pub const AUTOSAVE_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
/// And on Gmail at most this often, and when it closes.
pub const REMOTE_SAVE_EVERY_MS: i64 = 30_000;

/// W3 (DRF-5): what the footer says about saving.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaveState {
    #[default]
    Idle,
    /// Edited since the last save; it's saved when typing stops. Says
    /// nothing.
    Dirty,
    /// Between the autosave and the save landing.
    Saving,
    Saved,
}

/// How long "Saved 2:14 PM" stays before it fades.
const SAVED_SHOWN: std::time::Duration = std::time::Duration::from_secs(3);

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
        (Format::Inline(Inline::Code), Icon::Code, "Code"),
        (Format::Link, Icon::Link, "Link"),
        (Format::Lines(LineKind::Bullet), Icon::List, "Bulleted list"),
        (
            Format::Lines(LineKind::Numbered),
            Icon::ListOrdered,
            "Numbered list",
        ),
        (Format::Lines(LineKind::Quote), Icon::TextQuote, "Quote"),
        (Format::Clear, Icon::RemoveFormatting, "Clear formatting"),
    ];
    /// The toolbar's groups end after these: text styles, link, lines.
    const GROUP_ENDS: [usize; 3] = [4, 5, 8];

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
    /// W3 (MST-1): the address it goes from; empty for the default.
    pub from: String,
    /// W3 (MST-2): it carries the From address's signature.
    pub signature: bool,
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
    /// W3 (DRF-5): typing stopped; time to save.
    Autosave,
    /// A chip's "Show Contact Card".
    /// MB-069: at the right-click that asked for it.
    ShowContact(Contact, Option<gpui::Point<Pixels>>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    From,
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
    /// W2 (ATT-7): the paths of files taken out, so their held copies go
    /// with the send or the discard.
    pub dropped_files: Vec<String>,
    /// The problem, and the field it's shown under.
    error: Option<(Field, SharedString)>,
    /// A missing subject or attachment was pointed out: sending again goes
    /// ahead (ME-019). Any edit asks again.
    send_confirmed: bool,
    /// The chip ⌫ picked, as (field, chip): the next ⌫ removes it (MB-024).
    selected_chip: Option<(usize, usize)>,
    /// The chip a context menu is open on.
    menu_chip: Option<(usize, usize)>,
    /// Where that menu opened, for the contact card (MB-069).
    menu_at: Option<gpui::Point<Pixels>>,
    /// A reply's quote shows, past its "•••" (ME-009).
    quote_open: bool,
    /// The body's length and width when its rows were last checked, so the
    /// next-frame check runs only after they change (MC-053).
    rows_checked: (usize, Pixels),
    /// Scrolls the fields of a draft alone in the pane, above its send row.
    scroll: ScrollHandle,
    /// Scrolls a draft under its conversation: the conversation's.
    pub outer_scroll: Option<ScrollHandle>,
    /// The body's caret and length when last seen, to follow the caret.
    body_caret: (usize, usize),
    /// Frames left to bring the caret into view; layout can lag an edit.
    reveal_tries: u8,
    initial: DraftContent,
    /// A Gmail draft opened before its text arrived (DRF-3): the text goes
    /// in when it does, unless the user has started editing.
    pub awaiting_body: bool,
    /// Text that arrived, put in at the next frame.
    pending: Option<DraftContent>,
    /// The field that had focus when the draft closed, for Undo to return to.
    resume_at: Option<Field>,
    book: SharedContactBook,
    // W3.
    /// The addresses it can go from (MST-1); From shows with two or more.
    identities: Vec<SendAs>,
    from: String,
    /// The From menu is open, on this row.
    from_menu: Option<usize>,
    from_focus: FocusHandle,
    signature: bool,
    /// The signature shown, parsed once per address.
    signature_doc: Option<(String, Rc<Document>)>,
    /// Opened from Drafts: titled "Draft" even once saving gives new ones
    /// an id too.
    opened_as_draft: bool,
    /// DRF-5: the last revision handed out, the save state, the pending
    /// save, and when Gmail may next get one (Unix ms).
    revision: u64,
    /// What it said when last saved.
    last_saved: Option<DraftContent>,
    pub save_state: SaveState,
    /// What the save status says, kept while it fades out, and whether it
    /// shows; "Saved" hides `SAVED_SHOWN` after it lands.
    save_label: SharedString,
    save_shown: bool,
    save_hide: Option<Task<()>>,
    save_task: Option<Task<()>>,
    pub remote_due: Option<i64>,
    /// Sent, discarded or closed: no more saves while typing.
    closed: bool,
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
            cx.subscribe_in(&subject, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: false, ..
                } => this.focus_field(Field::Body, window, cx),
                InputEvent::Change => {
                    if this
                        .error
                        .as_ref()
                        .is_some_and(|(at, _)| *at == Field::Subject)
                    {
                        this.error = None;
                    }
                    this.edited(cx)
                }
                _ => {}
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
                // Render sizes the body to its rows.
                cx.notify();
                let body = body.read(cx);
                let at_end = body.cursor() >= body.text().len();
                // A growing draft alone in the pane keeps its end in view.
                if at_end && !this.inline && !this.shows_quote() {
                    this.scroll.scroll_to_bottom();
                }
                cx.emit(ComposerEvent::Edited { at_end });
                this.edited(cx);
            }
        }));
        let body_caret = {
            let body = body.read(cx);
            (body.cursor(), body.text().len())
        };

        let opened_as_draft = draft_id.is_some();
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
            dropped_files: Vec::new(),
            error: None,
            send_confirmed: false,
            selected_chip: None,
            menu_chip: None,
            menu_at: None,
            quote_open: false,
            rows_checked: (usize::MAX, px(0.)),
            scroll: ScrollHandle::new(),
            outer_scroll: None,
            body_caret,
            reveal_tries: 0,
            initial: content,
            awaiting_body: false,
            pending: None,
            resume_at: None,
            book,
            identities: Vec::new(),
            from: String::new(),
            from_menu: None,
            from_focus: cx.focus_handle(),
            signature: false,
            signature_doc: None,
            opened_as_draft,
            revision: 0,
            last_saved: None,
            save_state: SaveState::Idle,
            save_label: SharedString::default(),
            save_shown: false,
            save_hide: None,
            save_task: None,
            remote_due: None,
            closed: false,
            _subscriptions: subscriptions,
        }
    }

    // W3: From, signatures and saving while typing.

    /// The addresses it can go from; an empty or unknown From becomes the
    /// default one, as if it had opened with it.
    pub fn with_identities(mut self, identities: Vec<SendAs>) -> Self {
        self.identities = identities;
        self.from = self.resolve_from(&self.initial.from.clone());
        self.signature = self.initial.signature;
        self.initial.from = self.from.clone();
        self
    }

    /// The account's addresses were read again: From and the signature
    /// follow, in an open draft too.
    pub fn set_identities(&mut self, identities: Vec<SendAs>, cx: &mut Context<Self>) {
        if identities == self.identities {
            return;
        }
        self.identities = identities;
        self.from = self.resolve_from(&self.from.clone());
        cx.notify();
    }

    fn resolve_from(&self, email: &str) -> String {
        self.identities
            .iter()
            .find(|s| s.email.eq_ignore_ascii_case(email))
            .or_else(|| self.identities.iter().find(|s| s.is_default))
            .or(self.identities.first())
            .map_or_else(|| email.to_string(), |s| s.email.clone())
    }

    /// The identity it goes from.
    pub fn identity(&self) -> Option<&SendAs> {
        self.identities
            .iter()
            .find(|s| s.email.eq_ignore_ascii_case(&self.from))
    }

    /// From shows when there's a choice.
    fn shows_from(&self) -> bool {
        self.identities.len() > 1
    }

    /// The signature it carries now, as HTML: the From address's, unless
    /// it has none or the draft doesn't carry one.
    pub fn signature_html(&self) -> Option<&str> {
        signature_for(&self.identities, &self.from, self.signature)
    }

    /// Send from `email`; its signature takes the old one's place.
    pub fn set_from(&mut self, email: &str, cx: &mut Context<Self>) {
        let email = self.resolve_from(email);
        if email != self.from {
            self.from = email;
            self.edited(cx);
        }
        cx.notify();
    }

    /// Add the signature, or take it off this draft.
    pub fn toggle_signature(&mut self, cx: &mut Context<Self>) {
        self.signature = !self.signature;
        self.edited(cx);
        cx.notify();
    }

    /// ⌘K "Change From…": the menu opens on the current address.
    pub fn open_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shows_from() {
            return;
        }
        let current = self
            .identities
            .iter()
            .position(|s| s.email == self.from)
            .unwrap_or(0);
        self.from_menu = Some(current);
        self.from_focus.focus(window, cx);
        cx.notify();
    }

    fn picker_activate(&mut self, _: &FromActivate, window: &mut Window, cx: &mut Context<Self>) {
        match self.from_menu.take() {
            Some(ix) => {
                if let Some(email) = self.identities.get(ix).map(|s| s.email.clone()) {
                    self.set_from(&email, cx);
                }
            }
            None => self.open_from_menu(window, cx),
        }
        cx.notify();
    }

    fn picker_step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.identities.len() as isize;
        match self.from_menu {
            Some(ix) if count > 0 => {
                self.from_menu = Some((ix as isize + delta).rem_euclid(count) as usize);
                cx.notify();
            }
            _ => self.open_from_menu(window, cx),
        }
    }

    fn picker_next(&mut self, _: &FromNext, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_step(1, window, cx);
    }

    fn picker_prev(&mut self, _: &FromPrev, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_step(-1, window, cx);
    }

    fn picker_dismiss(&mut self, _: &FromDismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.from_menu.take().is_none() {
            return cx.propagate();
        }
        cx.notify();
    }

    /// Something changed: save once typing stops (DRF-5).
    fn edited(&mut self, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        self.send_confirmed = false;
        self.set_save_state(SaveState::Dirty, cx);
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            this.update(cx, |this, cx| {
                this.save_task = None;
                if this.closed {
                    return;
                }
                let saved = this.last_saved.as_ref().unwrap_or(&this.initial);
                if this.unchanged_since(saved, cx) {
                    let state = if this.last_saved.is_none() {
                        SaveState::Idle
                    } else {
                        SaveState::Saved
                    };
                    this.set_save_state(state, cx);
                } else {
                    this.set_save_state(SaveState::Saving, cx);
                    cx.emit(ComposerEvent::Autosave);
                }
            })
            .ok();
        }));
        cx.notify();
    }

    /// The save status follows `state`: "Saving…" shows, "Saved 2:14 PM"
    /// shows for a moment, the others say nothing.
    fn set_save_state(&mut self, state: SaveState, cx: &mut Context<Self>) {
        let was = self.save_state;
        self.save_state = state;
        match state {
            SaveState::Idle | SaveState::Dirty => {
                self.save_shown = false;
                self.save_hide = None;
            }
            SaveState::Saving => {
                self.save_label = "Saving…".into();
                self.save_shown = true;
                self.save_hide = None;
            }
            SaveState::Saved if was == SaveState::Saved => {}
            SaveState::Saved => {
                let at = crate::calendar::time::time_label(chrono::Local::now().time());
                self.save_label = format!("Saved {at}").into();
                self.save_shown = true;
                self.save_hide = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SAVED_SHOWN).await;
                    this.update(cx, |this, cx| {
                        this.save_shown = false;
                        cx.notify();
                    })
                    .ok();
                }));
            }
        }
        cx.notify();
    }

    /// A save is about to go: its revision, newer than any before it,
    /// across launches too (milliseconds).
    pub fn next_revision(&mut self) -> u64 {
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.revision = now.max(self.revision + 1);
        self.revision
    }

    /// Saved, saying `content`.
    pub fn mark_saved(&mut self, content: DraftContent, cx: &mut Context<Self>) {
        if self.save_task.is_none() {
            // A fresh time, even over an earlier "Saved".
            self.save_state = SaveState::Saving;
            self.set_save_state(SaveState::Saved, cx);
        }
        self.last_saved = Some(content);
        cx.notify();
    }

    /// Sent, discarded or closed: no more saves while typing, until Undo
    /// brings it back.
    pub fn set_closed(&mut self, closed: bool) {
        self.closed = closed;
        if closed {
            self.save_task = None;
        }
    }

    /// The body with the signature under it: what a draft saves.
    pub fn draft_html(&self, content: &DraftContent) -> String {
        let mut out = content.body_html();
        if let Some(signature) = self.signature_html() {
            out.push_str(&crate::backend::mime::signature_block(signature));
        }
        out
    }

    /// The draft's text arrived: show it, as if it had opened with it.
    pub fn load(&mut self, content: DraftContent, cx: &mut Context<Self>) {
        self.awaiting_body = false;
        self.pending = Some(content);
        cx.notify();
    }

    fn apply_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(content) = self.pending.take() else {
            return;
        };
        for (r, chips) in self
            .recipients
            .iter_mut()
            .zip([&content.to, &content.cc, &content.bcc])
        {
            r.chips = chips.clone();
        }
        self.show_cc_bcc |= !content.cc.is_empty() || !content.bcc.is_empty();
        let subject = content.subject.clone();
        self.subject
            .update(cx, |s, cx| s.set_value(subject, window, cx));
        let text = content.body.clone();
        self.body.update(cx, |body, cx| {
            body.set_selected_range(0..body.text().len(), cx);
            body.replace(text, window, cx);
            body.set_selected_range(0..0, cx);
        });
        self.attachments = content.attachments.clone();
        self.from = self.resolve_from(&content.from);
        self.signature = content.signature;
        self.initial = DraftContent {
            from: self.from.clone(),
            ..content
        };
        cx.notify();
    }

    /// What it's titled, here and in the reader's toolbar: what it is, and
    /// "Draft" when it was opened from Drafts (saving doesn't change it).
    pub fn title(&self) -> &'static str {
        match (self.kind, self.opened_as_draft) {
            (ComposeKind::New, false) => "New Message",
            (ComposeKind::New, true) => "Draft",
            (ComposeKind::Reply, false) => "Reply",
            (ComposeKind::Reply, true) => "Reply (Draft)",
            (ComposeKind::ReplyAll, false) => "Reply All",
            (ComposeKind::ReplyAll, true) => "Reply All (Draft)",
            (ComposeKind::Forward, false) => "Forward",
            (ComposeKind::Forward, true) => "Forward (Draft)",
        }
    }

    /// Whether the subject is a field of its own; replies keep the thread's.
    fn edits_subject(&self) -> bool {
        self.kind != ComposeKind::Reply && self.kind != ComposeKind::ReplyAll || !self.inline
    }

    /// The fields in tab order.
    fn fields(&self) -> Vec<Field> {
        let mut fields = Vec::new();
        if self.shows_from() {
            fields.push(Field::From);
        }
        fields.push(Field::To);
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
            Field::From => self.from_focus.clone(),
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
        if field != Field::From {
            self.from_menu = None;
        }
        match field {
            Field::From => self.from_focus.focus(window, cx),
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

    /// Closing (sent or discarded): note where the caret was, for Undo.
    pub fn remember_focus(&mut self, window: &Window, cx: &gpui::App) {
        self.resume_at = self.focused_field(window, cx);
    }

    /// Brought back by Undo: focus where it was, else where writing starts.
    pub fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.resume_at.take() {
            Some(field) if self.fields().contains(&field) => self.focus_field(field, window, cx),
            _ => self.focus_first(window, cx),
        }
    }

    /// Whether one of its fields has focus.
    pub fn has_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.focused_field(window, cx).is_some()
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
            from: self.from.clone(),
            signature: self.signature,
        }
    }

    /// The files on disk it has or had (W2): for a discard to release
    /// their held copies.
    pub fn file_paths(&self) -> Vec<String> {
        self.attachments
            .iter()
            .map(|a| a.path.clone())
            .chain(self.dropped_files.iter().cloned())
            .filter(|p| !p.is_empty())
            .collect()
    }

    /// Nothing changed since it opened. Text still in a recipient field
    /// counts as a change, even when it isn't an address yet.
    pub fn is_pristine(&self, cx: &gpui::App) -> bool {
        self.unchanged_since(&self.initial, cx)
    }

    /// Whether it says what `then` said.
    fn unchanged_since(&self, then: &DraftContent, cx: &gpui::App) -> bool {
        if self
            .recipients
            .iter()
            .any(|r| !r.input.read(cx).value().trim().is_empty())
        {
            return false;
        }
        let now = self.content(cx);
        let emails = |c: &[Contact]| c.iter().map(|c| c.email.to_lowercase()).collect::<Vec<_>>();
        now.attachments.len() == then.attachments.len()
            && emails(&now.to) == emails(&then.to)
            && emails(&now.cc) == emails(&then.cc)
            && emails(&now.bcc) == emails(&then.bcc)
            && now.subject == then.subject.trim()
            && now.body == then.body.trim_end()
            && now.from.eq_ignore_ascii_case(&then.from)
            && now.signature == then.signature
    }

    /// The body as HTML, its signature under it, then the forwarded or
    /// answered message quoted.
    pub fn html(&self, content: &DraftContent) -> String {
        let mut out = self.draft_html(content);
        if let Some(quoted) = &self.quoted {
            out.push_str(&quote_html(self.kind, quoted));
        }
        out
    }

    /// The quote shows in full: a forward's always, a reply's once its
    /// "•••" is open.
    pub fn shows_quote(&self) -> bool {
        self.quoted.is_some() && (self.kind == ComposeKind::Forward || self.quote_open)
    }

    /// A Gmail draft whose text hasn't arrived yet (ME-007). Text the
    /// draft opened with, or the user typed, counts as arrived.
    pub fn is_loading(&self, cx: &gpui::App) -> bool {
        self.awaiting_body && self.body.read(cx).text().len() == 0
    }

    /// Before sending a new message or a forward without a subject, or
    /// one that mentions an attachment it doesn't have: says so once, and
    /// false. Sending again goes ahead (ME-019).
    pub fn confirm_send(
        &mut self,
        content: &DraftContent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.send_confirmed {
            return true;
        }
        let warning = if self.edits_subject()
            && matches!(self.kind, ComposeKind::New | ComposeKind::Forward)
            && content.subject.is_empty()
        {
            Some((Field::Subject, "No subject. Send again to send it anyway."))
        } else if content.attachments.is_empty() && mentions_attachment(&content.body) {
            Some((
                Field::Body,
                "It mentions an attachment, but nothing is attached. Send again to send it anyway.",
            ))
        } else {
            None
        };
        let Some((field, warning)) = warning else {
            return true;
        };
        self.send_confirmed = true;
        self.error = Some((field, warning.into()));
        if field == Field::Subject {
            self.focus_field(field, window, cx);
        }
        cx.notify();
        false
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
                self.selected_chip = None;
                self.edited(cx);
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
        if chip >= self.recipients[ix].chips.len() {
            return;
        }
        self.recipients[ix].chips.remove(chip);
        self.selected_chip = None;
        self.edited(cx);
        cx.notify();
    }

    /// A click on a chip picks it, so ⌫ removes it.
    fn click_chip(
        &mut self,
        ix: usize,
        chip: usize,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_field(recipient_field(ix), window, cx);
        let input = self.recipients[ix].input.clone();
        // A double click takes it back into the field to edit.
        if clicks >= 2 && input.read(cx).value().is_empty() {
            let text = address_text(&self.recipients[ix].chips[chip]);
            self.remove_chip(ix, chip, cx);
            input.update(cx, |input, cx| input.set_value(text, window, cx));
            return;
        }
        self.selected_chip = Some((ix, chip));
        cx.notify();
    }

    /// Right-click on a chip: what can be done with that recipient.
    fn chip_menu(
        &mut self,
        ix: usize,
        chip: usize,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The menu's choice goes to the focused element: this field.
        self.focus_field(recipient_field(ix), window, cx);
        self.menu_chip = Some((ix, chip));
        self.menu_at = Some(position);
        let mut menu = NativeMenu::new()
            .menu("Copy Address", Box::new(CopyChipAddress))
            .menu("Show Contact Card", Box::new(ShowChipContact))
            .separator();
        for (to, label, action) in [
            (
                0,
                "Move to To",
                Box::new(MoveChipToTo) as Box<dyn gpui::Action>,
            ),
            (1, "Move to Cc", Box::new(MoveChipToCc)),
            (2, "Move to Bcc", Box::new(MoveChipToBcc)),
        ] {
            if to != ix {
                menu = menu.menu(label, action);
            }
        }
        menu.separator()
            .menu("Remove", Box::new(RemoveChip))
            .show(position, window, cx);
    }

    fn menu_contact(&self) -> Option<Contact> {
        let (ix, chip) = self.menu_chip?;
        self.recipients[ix].chips.get(chip).cloned()
    }

    fn copy_chip_address(&mut self, _: &CopyChipAddress, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(contact) = self.menu_contact() {
            cx.write_to_clipboard(ClipboardItem::new_string(contact.email));
        }
    }

    fn show_chip_contact(&mut self, _: &ShowChipContact, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(contact) = self.menu_contact() {
            cx.emit(ComposerEvent::ShowContact(contact, self.menu_at));
        }
    }

    fn move_chip(&mut self, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((ix, chip)) = self.menu_chip.take() else {
            return;
        };
        if ix == to || chip >= self.recipients[ix].chips.len() {
            return;
        }
        let contact = self.recipients[ix].chips.remove(chip);
        self.recipients[to].chips.push(contact);
        self.selected_chip = None;
        if to > 0 {
            self.show_cc_bcc = true;
        }
        self.focus_field(recipient_field(to), window, cx);
        self.edited(cx);
    }

    fn move_chip_to_to(&mut self, _: &MoveChipToTo, window: &mut Window, cx: &mut Context<Self>) {
        self.move_chip(0, window, cx);
    }

    fn move_chip_to_cc(&mut self, _: &MoveChipToCc, window: &mut Window, cx: &mut Context<Self>) {
        self.move_chip(1, window, cx);
    }

    fn move_chip_to_bcc(&mut self, _: &MoveChipToBcc, window: &mut Window, cx: &mut Context<Self>) {
        self.move_chip(2, window, cx);
    }

    fn remove_menu_chip(&mut self, _: &RemoveChip, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((ix, chip)) = self.menu_chip.take() {
            self.remove_chip(ix, chip, cx);
        }
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
        // The first ⌫ picks the last chip, the next removes it.
        match self.focused_recipients(window, cx) {
            Some(ix) if self.recipients[ix].input.read(cx).value().is_empty() => {
                match self.selected_chip {
                    Some((field, chip)) if field == ix => self.remove_chip(ix, chip, cx),
                    _ => {
                        self.selected_chip = self.recipients[ix]
                            .chips
                            .len()
                            .checked_sub(1)
                            .map(|last| (ix, last));
                        cx.notify();
                    }
                }
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
            Some(ix) if self.selected_chip.is_some_and(|(field, _)| field == ix) => {
                self.selected_chip = None;
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

        // Out of focus, a long list shows its first few (MB-024).
        let shown = if focused || field.chips.len() <= COLLAPSED_CHIPS + 1 {
            field.chips.len()
        } else {
            COLLAPSED_CHIPS
        };
        let hidden = field.chips.len() - shown;
        let chips = field
            .chips
            .iter()
            .take(shown)
            .enumerate()
            .map(|(chip_ix, contact)| {
                let invalid = parse_address(&contact.email).is_none();
                let selected = self.selected_chip == Some((ix, chip_ix));
                let name = contact.display_name().to_string();
                let tip: SharedString = if invalid {
                    format!("“{}” isn’t an email address", contact.email).into()
                } else {
                    contact.email.clone().into()
                };
                let label = if invalid {
                    format!("{name}, not an email address")
                } else if contact.name.is_empty() {
                    name.clone()
                } else {
                    format!("{name}, {}", contact.email)
                };
                let text = if selected {
                    theme.accent_text
                } else {
                    theme.text
                };
                div()
                    .id(("chip", ix * 1000 + chip_ix))
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .aria_selected(selected)
                    .h(CONTROL_SM)
                    .pl(SPACE_XS)
                    .pr(SPACE_XS)
                    .flex()
                    .items_center()
                    .gap(SPACE_XS)
                    .rounded_full()
                    .border_1()
                    .text_color(text)
                    .map(|el| match (selected, invalid) {
                        (true, _) => el.bg(theme.accent).border_color(theme.accent),
                        (false, true) => el.bg(theme.chip_bg).border_color(theme.danger),
                        (false, false) => el.bg(theme.chip_bg).border_color(theme.border),
                    })
                    // An invalid one says so with a glyph, not only a color
                    // (MA-024).
                    .child(if invalid {
                        div()
                            .size(AVATAR_XS)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(
                                Icon::OctagonAlert,
                                ICON_SM,
                                if selected { text } else { theme.text_danger },
                            ))
                    } else {
                        avatar(
                            contact.initials(),
                            theme.avatar_color(&contact.email),
                            AVATAR_XS,
                            theme,
                        )
                    })
                    .child(
                        div()
                            .text_size(TEXT_SM)
                            .font_weight(FontWeight::MEDIUM)
                            .child(name.clone()),
                    )
                    .child(
                        super::icon_button_small(
                            ("chip-remove", ix * 1000 + chip_ix),
                            Icon::X,
                            format!("Remove {name}"),
                            theme,
                        )
                        // Inside the chip it looks small; it still takes a
                        // full-size click.
                        .m(-SPACE_XS)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.remove_chip(ix, chip_ix, cx)
                        })),
                    )
                    .tooltip(move |window, cx| super::tooltip(tip.clone()).build(window, cx))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_any_mouse_down(cx.listener(
                        move |this, event: &gpui::MouseDownEvent, window, cx| {
                            if super::is_secondary(event) {
                                cx.stop_propagation();
                                this.chip_menu(ix, chip_ix, event.position, window, cx);
                            }
                        },
                    ))
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            if !event.modifiers().control {
                                this.click_chip(ix, chip_ix, event.click_count(), window, cx);
                            }
                        }),
                    )
            });
        let more = (hidden > 0).then(|| {
            focusable_control(div().id(("more-chips", ix)), theme)
                .role(gpui::Role::Button)
                .aria_label(format!("Show {hidden} more recipients"))
                .h(CONTROL_SM)
                .px(SPACE_SM)
                .flex()
                .items_center()
                .rounded_full()
                .bg(theme.chip_bg)
                .text_size(TEXT_SM)
                .text_color(theme.text_muted)
                .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                .active(|s| s.bg(theme.selected_bg))
                .child(format!("+{hidden} more"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.focus_field(recipient_field(ix), window, cx)
                }))
        });

        let suggestions = (focused && !field.suggestions.is_empty()).then(|| {
            let rows = field
                .suggestions
                .iter()
                .enumerate()
                .map(|(row, &contact_ix)| {
                    let contact = self.book.borrow().contact(contact_ix).clone();
                    let highlighted = row == field.highlighted;
                    controls::person_row(("suggestion", row), &contact, highlighted, theme)
                        .when(highlighted, |d| d.aria_active_descendant())
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.commit_suggestion(ix, row, window, cx)
                        }))
                });
            controls::popup(
                controls::popover_surface(theme)
                    .id(("suggestions", ix))
                    .role(gpui::Role::ListBox)
                    .aria_label("Suggested contacts")
                    .occlude()
                    .mt(SPACE_SM)
                    .w(SUGGESTIONS_WIDTH)
                    .p(SPACE_XS)
                    .flex()
                    .flex_col()
                    .children(rows),
            )
        });

        let error = self
            .error
            .as_ref()
            .filter(|(at, _)| *at == recipient_field(ix))
            .map(|(_, error)| {
                div()
                    .pb(SPACE_SM)
                    .ml(SPACE_XS)
                    .pl(LABEL_WIDTH)
                    .child(field_error(error.clone(), theme))
            });

        let row = div()
            .py(SPACE_SM)
            .flex()
            .items_start()
            .gap(SPACE_XS)
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
                    .children(more)
                    .child(
                        div()
                            .flex_1()
                            .min_w(RECIPIENT_INPUT_MIN)
                            .h(CONTROL_MD)
                            .flex()
                            .flex_col()
                            .child(
                                Input::new(&field.input)
                                    .appearance(false)
                                    .px(SPACE_2XS)
                                    .h(CONTROL_MD),
                            )
                            .children(suggestions),
                    ),
            )
            .when(ix == 0 && !self.show_cc_bcc, |this| {
                this.child(
                    focusable_control(div().id("show-cc-bcc"), theme)
                        .role(gpui::Role::Button)
                        .aria_label("Show Cc and Bcc")
                        .flex_none()
                        .h(CONTROL_MD)
                        .px(SPACE_SM)
                        .flex()
                        .items_center()
                        .rounded(RADIUS_SM)
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                        .active(|s| s.bg(theme.selected_bg))
                        .child("Cc Bcc")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowCcBcc), cx))
                        .tooltip(|window, cx| {
                            super::tooltip("Show Cc and Bcc")
                                .action(&ShowCcBcc, None)
                                .build(window, cx)
                        }),
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

    /// Files over Gmail's limit, all together, aren't attached.
    fn attach_paths(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        for path in paths {
            let Ok(meta) = std::fs::metadata(path) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            if attachments_size(&self.attachments) + meta.len() > ATTACHMENT_LIMIT {
                self.error = Some((Field::Body, ATTACHMENT_LIMIT_ERROR.into()));
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.attachments.push(Attachment {
                path: path.to_string_lossy().into_owned(),
                ..Attachment::from_file(name, meta.len())
            });
        }
        self.edited(cx);
        cx.notify();
    }

    /// Show `error` under the attachments.
    pub fn set_attachment_error(&mut self, error: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some((Field::Body, error.into()));
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
        let error = self
            .error
            .as_ref()
            .filter(|(at, _)| *at == Field::Body)
            .map(|(_, error)| div().w_full().child(field_error(error.clone(), theme)));
        if self.attachments.is_empty() && error.is_none() {
            return None;
        }
        let chips = self.attachments.iter().enumerate().map(|(ix, attachment)| {
            div()
                .id(("attachment", ix))
                .role(gpui::Role::Group)
                .aria_label(format!(
                    "{}, {}",
                    attachment.name,
                    file_size(attachment.size_bytes)
                ))
                .h(CONTROL_LG)
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
                        .max_w(ATTACHMENT_NAME_MAX)
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
                    super::icon_button_small(
                        ("attachment-remove", ix),
                        Icon::X,
                        format!("Remove {}", attachment.name),
                        theme,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let removed = this.attachments.remove(ix);
                        if !removed.path.is_empty() {
                            this.dropped_files.push(removed.path);
                        }
                        this.edited(cx);
                        if this
                            .error
                            .as_ref()
                            .is_some_and(|(at, _)| *at == Field::Body)
                        {
                            this.error = None;
                        }
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
                .children(chips)
                .children(error),
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
    ) -> gpui::AnyElement {
        // The editor scrolls on its own; sized to its rows, it grows like a
        // textarea and the draft scrolls instead.
        let min_rows = if self.inline { 5 } else { 14 };
        let rows = self.body_rows.max(min_rows);
        let line_height = self
            .body
            .read(cx)
            .line_height()
            .unwrap_or_else(|| TEXT_MD.to_pixels(window.rem_size()) * 1.55);
        let height = line_height * rows as f32 + EDITOR_PADDING_Y * 2.;
        // A Gmail draft whose text is on its way (ME-007).
        if self.is_loading(cx) {
            return div()
                .pt(SPACE_SM)
                .h(height)
                .child(
                    super::live_text(
                        "draft-loading",
                        "Loading draft…",
                        gpui::accesskit::Live::Polite,
                    )
                    .py(EDITOR_PADDING_Y)
                    .text_size(TEXT_MD)
                    .text_color(theme.text_faint),
                )
                .into_any_element();
        }
        let body = self.body.downgrade();
        // Cancels the editor's own padding, so text lines up with the
        // inputs above.
        div()
            .pt(SPACE_SM)
            .ml(-EDITOR_INSET_LEFT)
            .mr(-EDITOR_INSET_RIGHT)
            .child(
                Editor::new(&self.body)
                    .appearance(false)
                    .aria_label("Message body")
                    .h(height)
                    .font_family(cx.theme().font_family.clone())
                    .text_size(TEXT_MD)
                    .line_height(LINE_READING)
                    .text_color(theme.text)
                    // Prose items only, not the code editor's (MB-002).
                    .context_menu(move |menu, _, cx| {
                        let selected = body
                            .upgrade()
                            .is_some_and(|b| !b.read(cx).selected_range().is_empty());
                        menu.menu_with_disabled("Cut", !selected, Box::new(input::Cut))
                            .menu_with_disabled("Copy", !selected, Box::new(input::Copy))
                            .menu("Paste", Box::new(input::Paste))
                            .separator()
                            .menu("Select All", Box::new(input::SelectAll))
                            .separator()
                            .menu("Bold", Box::new(FormatBold))
                            .menu("Italic", Box::new(FormatItalic))
                            .menu("Insert Link…", Box::new(InsertLink))
                    }),
            )
            .into_any_element()
    }

    /// What's on at the body's caret: its inline styles and line kind
    /// (MB-037).
    fn caret_styles(&self, cx: &gpui::App) -> (markdown::Flags, Option<LineKind>) {
        let body = self.body.read(cx);
        let text = body.text();
        let caret = body.cursor().min(text.len());
        let row = text.offset_to_point(caret).row;
        let start = text.line_start_offset(row);
        let line = text.slice(start..text.line_end_offset(row)).to_string();
        markdown::styles_at(&line, caret - start)
    }

    fn render_format_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (flags, line) = self.caret_styles(cx);
        let active = |format: Format| match format {
            Format::Inline(kind) => flags & kind.flag() != 0,
            Format::Link => flags & markdown::LINK != 0,
            Format::Lines(kind) => line == Some(kind),
            Format::Clear => false,
        };
        let divider = || {
            div()
                .flex_none()
                .w(px(1.))
                .h(ICON_LG)
                .mx(SPACE_XS)
                .bg(theme.border)
        };
        div()
            .id("format-bar")
            .role(gpui::Role::Toolbar)
            .aria_label("Formatting")
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .children(Format::TOOLBAR.iter().enumerate().flat_map(
                |(ix, &(format, glyph, name))| {
                    let action = format.action();
                    let on = active(format);
                    let toggles = format != Format::Clear;
                    let button = focusable_control(div().id(("format", ix)), theme)
                        .role(gpui::Role::Button)
                        .aria_label(name)
                        .when(toggles, |d| d.aria_toggled(on.into()))
                        .size(HIT_TARGET)
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(RADIUS_SM)
                        .when(on, |d| d.bg(theme.selected_bg))
                        .hover(|s| s.bg(theme.hover_bg))
                        .active(|s| s.bg(theme.selected_bg))
                        .child(icon(
                            glyph,
                            ICON_MD,
                            if on { theme.text } else { theme.icon },
                        ))
                        // Keeps the body's focus and selection.
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.format(format, window, cx)),
                        )
                        .tooltip(move |window, cx| {
                            super::tooltip(name)
                                .action(action.as_ref(), None)
                                .build(window, cx)
                        })
                        .into_any_element();
                    let divider = Format::GROUP_ENDS
                        .contains(&ix)
                        .then(|| divider().into_any_element());
                    std::iter::once(button).chain(divider)
                },
            ))
    }
}

impl Composer {
    /// W3 (MST-1): the From row, with its menu of addresses.
    fn render_from(
        &self,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = self.from_focus.is_focused(window);
        let label = |s: &SendAs| {
            if s.name.is_empty() {
                s.email.clone()
            } else {
                format!("{} <{}>", s.name, s.email)
            }
        };
        let current = self.identity().map(label).unwrap_or_default();
        let menu = self.from_menu.map(|highlighted| {
            let rows = self.identities.iter().enumerate().map(|(row, identity)| {
                let chosen = identity.email == self.from;
                div()
                    .id(("from-option", row))
                    .role(gpui::Role::ListBoxOption)
                    .aria_label(label(identity))
                    .aria_selected(chosen)
                    .when(row == highlighted, |d| d.aria_active_descendant())
                    .py(SPACE_XS)
                    .line_height(LINE_UI)
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_SM)
                    .when(row == highlighted, |d| d.bg(theme.selected_bg))
                    .hover(|d| d.bg(theme.hover_bg))
                    .active(|d| d.bg(theme.selected_bg))
                    .child(controls::menu_check(chosen, theme))
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
                                    .child(if identity.name.is_empty() {
                                        identity.email.clone()
                                    } else {
                                        identity.name.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(TEXT_SM)
                                    .text_color(theme.text_muted)
                                    .child(identity.email.clone()),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.from_menu = Some(row);
                        this.picker_activate(&FromActivate, window, cx);
                    }))
            });
            controls::popup(
                controls::popover_surface(theme)
                    .id("from-menu")
                    .role(gpui::Role::ListBox)
                    .aria_label("From addresses")
                    .occlude()
                    .mt(SPACE_SM)
                    .w(FROM_MENU_WIDTH)
                    .p(SPACE_XS)
                    .flex()
                    .flex_col()
                    .children(rows),
            )
        });
        let ring = focus_ring(theme);
        div()
            .py(SPACE_SM)
            .flex()
            .items_start()
            .gap(SPACE_XS)
            .border_b_1()
            .border_color(theme.border)
            .child(field_label("From", theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("from")
                            .track_focus(&self.from_focus)
                            // Also an Input, so the mail's one-letter
                            // shortcuts stay out while it has focus.
                            .key_context(format!("{FROM_PICKER} Input").as_str())
                            .on_action(cx.listener(Self::picker_activate))
                            .on_action(cx.listener(Self::picker_next))
                            .on_action(cx.listener(Self::picker_prev))
                            .on_action(cx.listener(Self::picker_dismiss))
                            .role(gpui::Role::ComboBox)
                            .aria_label(format!("From: {current}"))
                            .h(CONTROL_MD)
                            .max_w_full()
                            .px(SPACE_SM)
                            .ml(-SPACE_SM)
                            .flex()
                            .items_center()
                            .gap(SPACE_XS)
                            .rounded(RADIUS_SM)
                            .text_size(TEXT_BASE)
                            .hover(|s| s.bg(theme.hover_bg))
                            .when(focused, |d| d.bg(theme.hover_bg).shadow(ring))
                            .child(div().min_w_0().truncate().child(current))
                            .child(icon(Icon::ChevronDown, ICON_SM, theme.text_faint))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.from_menu.take().is_none() {
                                    this.open_from_menu(window, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .children(menu),
            )
    }

    /// W3 (MST-2): Gmail's signature for the From address, under the body.
    /// Read-only: it's edited in Gmail's settings.
    fn render_signature(&mut self, theme: &Theme) -> Option<gpui::AnyElement> {
        let html = self.signature_html()?.to_string();
        let doc = match &self.signature_doc {
            Some((seen, doc)) if *seen == html => doc.clone(),
            _ => {
                let doc = Rc::new(html::parse(&html));
                self.signature_doc = Some((html, doc.clone()));
                doc
            }
        };
        Some(
            div()
                .id("signature")
                .role(gpui::Role::Group)
                .aria_label("Signature, from Gmail’s settings")
                .mt(SPACE_SM)
                .flex()
                .items_start()
                .gap(SPACE_SM)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(SPACE_2XS)
                        .text_size(TEXT_BASE)
                        .line_height(LINE_READING)
                        .text_color(theme.text_muted)
                        .child(div().text_color(theme.text_faint).child("--"))
                        .child(html_view::render(&doc, theme)),
                )
                .child(
                    super::icon_button(
                        "remove-signature",
                        Icon::X,
                        "Remove signature",
                        Box::new(crate::actions::ToggleSignature),
                        theme,
                    )
                    .size(HIT_TARGET),
                )
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .into_any_element(),
        )
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        // Focus went elsewhere: the From menu closes (W3).
        if self.from_menu.is_some() && !self.from_focus.is_focused(window) {
            self.from_menu = None;
        }
        // A picked chip stays picked only while its field has focus.
        if let Some((ix, _)) = self.selected_chip
            && !self.recipients[ix]
                .input
                .focus_handle(cx)
                .is_focused(window)
        {
            self.selected_chip = None;
        }
        if self.pending.is_some() {
            cx.defer_in(window, Self::apply_pending);
        }
        self.palette.set(Palette::new(&theme));
        let body = self.body.read(cx);
        self.body_rows = body.display_map().wrap_row_count();
        let key = (
            body.text().len(),
            body.text_bounds().map_or(px(0.), |b| b.size.width),
        );
        // How lines wrap is known once the body is laid out: grow then, if
        // they wrapped differently. Only text or width changes that.
        if key != self.rows_checked {
            self.rows_checked = key;
            cx.on_next_frame(window, |this, _, cx| {
                if this.body.read(cx).display_map().wrap_row_count() != this.body_rows {
                    cx.notify();
                }
            });
        }
        let standalone = !self.inline;
        let signature = self.render_signature(&theme);
        let save_opacity = crate::ui::motion::fade(
            "save-status",
            0.,
            if self.save_shown { 1. } else { 0. },
            window,
            cx,
        );

        let title = self.title();

        let quoted = self.quoted.as_ref().map(|quoted| {
            let forward = self.kind == ComposeKind::Forward;
            let body = div()
                .mt(SPACE_MD)
                .text_size(TEXT_BASE)
                .line_height(LINE_READING)
                .text_color(theme.text_muted)
                .child(html_view::render(&quoted.doc, &theme));
            if forward {
                return div()
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
                    .child(body)
                    .into_any_element();
            }
            // A reply's quote, folded behind "•••" like Gmail (ME-009).
            let open = self.quote_open;
            let attribution = quoted
                .header
                .first()
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            div()
                .mt(SPACE_SM)
                .flex()
                .flex_col()
                .items_start()
                .child(
                    focusable_control(div().id("toggle-quote"), &theme)
                        .role(gpui::Role::Button)
                        .aria_label(if open {
                            "Hide quoted text"
                        } else {
                            "Show quoted text"
                        })
                        .aria_expanded(open)
                        .h(CONTROL_XS)
                        .px(SPACE_SM)
                        .flex()
                        .items_center()
                        .rounded(RADIUS_SM)
                        .bg(theme.chip_bg)
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                        .active(|s| s.bg(theme.selected_bg))
                        .child("•••")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.quote_open = !this.quote_open;
                            cx.notify();
                        }))
                        .tooltip(move |window, cx| {
                            super::tooltip(if open {
                                "Hide quoted text"
                            } else {
                                "Show quoted text"
                            })
                            .build(window, cx)
                        }),
                )
                .when(open, |el| {
                    el.child(
                        div()
                            .mt(SPACE_SM)
                            .pl(SPACE_LG)
                            .border_l_2()
                            .border_color(theme.border)
                            .flex()
                            .flex_col()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_faint)
                            .child(attribution)
                            .child(body),
                    )
                })
                .into_any_element()
        });

        let hint = |keys: &str, label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .child(label)
                .child(keys_caps(keys, &theme))
        };

        let send_row = div()
            .flex()
            .items_center()
            .gap(SPACE_MD)
            .child(
                focusable_control(div().id("send"), &theme)
                    .role(gpui::Role::Button)
                    .aria_label("Send")
                    .h(CONTROL_LG)
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
                    .child(icon(Icon::Send, ICON_MD, theme.accent_text))
                    .child("Send")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(SendMessage), cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_MD)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(hint("cmd-enter", "Send"))
                    .child(hint("escape", "Save draft")),
            )
            .child(div().flex_1())
            // W3 (DRF-5): fades in and out (ME-006).
            .child(
                super::live_text(
                    "save-status",
                    self.save_label.clone(),
                    gpui::accesskit::Live::Polite,
                )
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
                .opacity(save_opacity),
            )
            .child(
                focusable_control(div().id("attach"), &theme)
                    .role(gpui::Role::Button)
                    .aria_label("Attach files")
                    .size(HIT_TARGET)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_MD)
                    .hover(|s| s.bg(theme.hover_bg))
                    .active(|s| s.bg(theme.selected_bg))
                    .child(icon(Icon::Paperclip, ICON_TOOLBAR, theme.icon))
                    .tooltip(|window, cx| {
                        super::tooltip("Attach files (or drop them here)").build(window, cx)
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
                            ICON_MD,
                            theme.icon,
                        ))
                        .child(title),
                )
            })
            .when(self.shows_from(), |this| {
                this.child(self.render_from(window, &theme, cx))
            })
            .child(self.render_recipients(0, window, &theme, cx))
            .when(self.show_cc_bcc, |this| {
                this.child(self.render_recipients(1, window, &theme, cx))
                    .child(self.render_recipients(2, window, &theme, cx))
            })
            .when(self.edits_subject(), |this| {
                this.child(
                    div()
                        .py(SPACE_SM)
                        .flex()
                        .items_center()
                        .gap(SPACE_XS)
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
                .children(
                    self.error
                        .as_ref()
                        .filter(|(at, _)| *at == Field::Subject)
                        .map(|(_, error)| {
                            div()
                                .pb(SPACE_SM)
                                .pt(SPACE_SM)
                                .ml(SPACE_XS)
                                .pl(LABEL_WIDTH)
                                .child(field_error(error.clone(), &theme))
                        }),
                )
            })
            // Lined up with the inputs, past the labels (ME-005).
            .child(
                div()
                    .ml(SPACE_XS)
                    .pl(LABEL_WIDTH)
                    .flex()
                    .flex_col()
                    .child(self.render_body(window, &theme, cx))
                    .children(signature)
                    .children(self.render_attachments(&theme, cx))
                    .children(quoted),
            );

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
            .on_action(cx.listener(Self::copy_chip_address))
            .on_action(cx.listener(Self::show_chip_contact))
            .on_action(cx.listener(Self::move_chip_to_to))
            .on_action(cx.listener(Self::move_chip_to_cc))
            .on_action(cx.listener(Self::move_chip_to_bcc))
            .on_action(cx.listener(Self::remove_menu_chip))
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
                    // Only in the body: elsewhere ⌘\ is the sidebar.
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

/// Fits "Subject", so every field's input starts at the same x (ME-005).
const LABEL_WIDTH: Rems = rems(3.5);
/// Out of focus, a field with more chips shows this many and "+N more".
const COLLAPSED_CHIPS: usize = 5;
const SUGGESTIONS_WIDTH: Pixels = px(320.);
const FROM_MENU_WIDTH: Pixels = px(360.);
const RECIPIENT_INPUT_MIN: Pixels = px(140.);
const ATTACHMENT_NAME_MAX: Pixels = px(220.);
/// The editor's own inset, which the body cancels so its text lines up
/// with the inputs above it.
const EDITOR_INSET_LEFT: Pixels = px(12.);
const EDITOR_INSET_RIGHT: Pixels = px(10.);
const EDITOR_PADDING_Y: Pixels = px(8.);

/// A field's name, right-aligned in a fixed column.
fn field_label(label: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .w(LABEL_WIDTH)
        .h(CONTROL_MD)
        .flex()
        .items_center()
        .justify_end()
        .text_size(TEXT_SM)
        .text_color(theme.text_faint)
        .child(label)
}

/// A problem under a field: selectable, and read out when it shows.
fn field_error(error: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .id("field-error")
        .role(gpui::Role::Label)
        .aria_value(error.clone())
        .a11y_synthetic_children(|builder| {
            builder
                .parent_node()
                .set_live(gpui::accesskit::Live::Assertive)
        })
        .child(controls::error_line(error, theme))
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

/// W3 (MST-2): the signature a draft from `from` shows and sends: that
/// address's, when the draft carries one and the address has one.
pub fn signature_for<'a>(identities: &'a [SendAs], from: &str, carries: bool) -> Option<&'a str> {
    identities
        .iter()
        .find(|s| carries && s.email.eq_ignore_ascii_case(from))
        .map(|s| s.signature.as_str())
        .filter(|s| !s.trim().is_empty())
}

/// Gmail's limit on what a message carries.
pub const ATTACHMENT_LIMIT: u64 = 25 * 1024 * 1024;
pub const ATTACHMENT_LIMIT_ERROR: &str = "Attachments are over Gmail’s 25 MB limit";

pub fn attachments_size(attachments: &[Attachment]) -> u64 {
    attachments.iter().map(|a| a.size_bytes).sum()
}

/// The quote under a sent message: a forward's header block, or a reply's
/// "On … wrote:" over the answered message, as Gmail sends them.
fn quote_html(kind: ComposeKind, quoted: &Quoted) -> String {
    let mut out = String::new();
    if kind == ComposeKind::Forward {
        out.push_str("<blockquote><p>---------- Forwarded message ----------");
        for (label, value) in &quoted.header {
            out.push_str(&format!("<br>{label}: {}", html::escape(value)));
        }
        out.push_str("</p>");
        out.push_str(&quoted.html);
        out.push_str("</blockquote>");
    } else {
        let attribution = quoted.header.first().map_or("", |(_, v)| v.as_str());
        out.push_str(&format!(
            "<div class=\"gmail_quote\"><div class=\"gmail_attr\">{}<br></div>\
             <blockquote class=\"gmail_quote\" style=\"margin:0 0 0 .8ex;\
             border-left:1px #ccc solid;padding-left:1ex\">",
            html::escape(attribution)
        ));
        out.push_str(&quoted.html);
        out.push_str("</blockquote></div>");
    }
    out
}

/// "On Sun, Oct 25, 9:12 AM, Ana Lima <ana@x.com> wrote:", a reply quote's
/// one header line.
pub fn reply_attribution(date: &str, from: &Contact) -> String {
    if from.name.is_empty() {
        format!("On {date}, {} wrote:", from.email)
    } else {
        format!("On {date}, {} <{}> wrote:", from.name, from.email)
    }
}

/// Whether `body` says something is attached, outside quoted lines.
fn mentions_attachment(body: &str) -> bool {
    body.lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .flat_map(|line| line.split(|c: char| !c.is_alphanumeric()))
        .any(|word| {
            let word = word.to_lowercase();
            word.starts_with("attach") || word == "enclosed"
        })
}

/// A contact as typed in a field: "Name <a@b.c>" or the bare address.
fn address_text(contact: &Contact) -> String {
    if contact.name.is_empty() {
        contact.email.clone()
    } else {
        format!("{} <{}>", contact.name, contact.email)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signature_follows_from() {
        let ids = [
            SendAs {
                email: "me@x.com".into(),
                signature: "<b>Me</b>".into(),
                is_default: true,
                ..Default::default()
            },
            SendAs {
                email: "work@x.com".into(),
                signature: "Work".into(),
                ..Default::default()
            },
            SendAs {
                email: "plain@x.com".into(),
                ..Default::default()
            },
        ];
        assert_eq!(signature_for(&ids, "me@x.com", true), Some("<b>Me</b>"));
        // Swapped with From, and gone for an address without one.
        assert_eq!(signature_for(&ids, "WORK@x.com", true), Some("Work"));
        assert_eq!(signature_for(&ids, "plain@x.com", true), None);
        // Taken off this draft.
        assert_eq!(signature_for(&ids, "work@x.com", false), None);
    }

    #[test]
    fn attachment_words_outside_quotes() {
        assert!(mentions_attachment("See the attached file"));
        assert!(mentions_attachment("Attachment: notes"));
        assert!(mentions_attachment("I've enclosed it."));
        assert!(!mentions_attachment("> see attached\nThanks"));
        assert!(!mentions_attachment("Lunch at noon?"));
    }

    #[test]
    fn a_reply_quotes_gmail_style() {
        let ana = Contact {
            name: "Ana".into(),
            email: "ana@x.com".into(),
        };
        let line = reply_attribution("Oct 25", &ana);
        assert_eq!(line, "On Oct 25, Ana <ana@x.com> wrote:");
        let quoted = Quoted::new(vec![(String::new(), line)], "<p>Hi</p>".into());
        let out = quote_html(ComposeKind::Reply, &quoted);
        assert!(out.starts_with("<div class=\"gmail_quote\"><div class=\"gmail_attr\">On Oct 25, Ana &lt;ana@x.com&gt; wrote:"));
        assert!(out.ends_with("<p>Hi</p></blockquote></div>"));
    }
}

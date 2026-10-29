//! The reading pane: toolbar, subject, and the thread as a conversation.

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use chrono::{DateTime, FixedOffset, Local};
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Bounds, Context, ElementId,
    FocusHandle, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, Render,
    SharedString, StatefulInteractiveElement, Styled, Task, UniformListScrollHandle, Window,
    base::TextSelection, canvas, component::native_menu::NativeMenu,
    component::scroll::ScrollableElement as _, div, prelude::FluentBuilder as _, px, relative,
    uniform_list,
};

use super::{
    HEADER_HEIGHT, avatar,
    controls::{ButtonKind, push_button},
    html_view, icon, icon_button, icon_button_tinted, icon_button_with,
    selectable::{ORDER_BASE, OnClear, Selectable},
    window_drag_area,
};
use crate::ui::style::*;
use crate::{
    actions::*,
    app::{Nav, PigeonApp, Row},
    assets::Icon,
    backend::attachments::Ready,
    data::{Body, Contact, Folder, Message, Thread, file_size, full_date, list_date},
    html::Document,
    settings::RemoteImages,
    theme::Theme,
};

/// ACT-11, CMP-17: when a snoozed conversation comes back or a scheduled
/// draft goes, with what changes it.
fn render_later_bar(thread: &Thread, theme: &Theme) -> Option<AnyElement> {
    let when = |at: i64| {
        crate::later::in_sentence(crate::later::local_time(at), crate::later::now_local())
    };
    type Buttons = Vec<(&'static str, &'static str, fn() -> Box<dyn gpui::Action>)>;
    let (glyph, text, buttons): (Icon, String, Buttons) =
        if let Some(at) = thread.later.snoozed_until {
            (
                Icon::AlarmClock,
                format!("Snoozed until {}", when(at)),
                vec![("later-unsnooze", "Unsnooze", || Box::new(Unsnooze))],
            )
        } else {
            let at = thread.later.send_at?;
            (
                Icon::Clock,
                format!("Scheduled to send {}", when(at)),
                vec![
                    ("later-edit", "Edit", || Box::new(EditScheduledSend)),
                    ("later-cancel", "Cancel Send", || {
                        Box::new(CancelScheduledSend)
                    }),
                ],
            )
        };
    Some(
        div()
            .w_full()
            .max_w(READER_MAX)
            .mt(SPACE_MD)
            .px(SPACE_MD)
            .py(SPACE_XS)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .bg(theme.chip_bg)
            .child(icon(glyph, ICON_MD, theme.accent))
            .child(
                super::label("later-status", text)
                    .flex_1()
                    .min_w_0()
                    .text_size(TEXT_SM)
                    .text_color(theme.text_muted),
            )
            .children(buttons.into_iter().map(|(id, label, action)| {
                push_button(id, label, ButtonKind::Plain, theme)
                    .on_click(move |_, window, cx| window.dispatch_action(action(), cx))
            }))
            .into_any_element(),
    )
}

/// Children of the conversation's scroll view before its first message: the
/// subject and the line under it. Every message stays a child, folded or
/// not, so `MESSAGES_START + ix` is always message `ix`.
pub const MESSAGES_START: usize = 2;

/// The widest the conversation gets, for a readable line (MA-061).
const READER_MAX: Pixels = px(760.);
/// A reader narrower than this drops the body's indent.
const NARROW: Pixels = px(520.);
/// The longest file name an attachment chip shows.
const CHIP_NAME_MAX: gpui::Rems = type_size(260.);
/// The bulk actions' column.
const BULK_MAX: Pixels = px(440.);

/// The reader's state that outlives a frame but isn't the app's data: what
/// the user opened, ⌥Tab's link, and attachment files on their way.
#[derive(Default)]
pub struct ReaderUi {
    /// ⌥Tab's link (MA-003): thread id, message, and the link's place in
    /// `html::links`.
    pub(crate) link: Option<(String, usize, usize)>,
    /// Threads whose "N more messages" the user opened (ME-008).
    pub(crate) open_folds: HashSet<String>,
    /// Messages whose quoted history is shown (ME-003).
    pub(crate) open_quotes: HashSet<String>,
    /// Messages whose whole recipient list is shown (ME-012).
    pub(crate) open_recipients: HashSet<String>,
    /// Attachments being fetched, by (message id, attachment), with the
    /// fetch; dropping it cancels (MA-067).
    pub(crate) fetching: HashMap<(String, usize), Task<()>>,
    /// Attachment files fetched this session, for dragging out (MA-086).
    pub(crate) files: HashMap<(String, usize), Ready>,
    /// Where the last save went (MB-060).
    pub(crate) save_dir: Option<PathBuf>,
    /// Where the toolbar's "…" and the first attachment chip were drawn,
    /// for the menus they open (MB-016).
    pub(crate) more_anchor: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub(crate) attachment_anchor: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Bodies on their way, by message id: since when (MA-066). A body
    /// whose placeholder showed fades in over it.
    pub(crate) body_waits: HashMap<String, BodyWait>,
    /// AI-10: messages shown while their Clean View was still being made,
    /// since the conversation opened; it arrives as a bar.
    pub(crate) clean_waiting: HashSet<String>,
    /// ⇧V and the bar's Show: true for the Clean View, by message.
    pub(crate) clean_choice: HashMap<String, bool>,
    /// Clean Views as HTML, by message, with the `created` of the answer
    /// they're from.
    pub(crate) clean_html: HashMap<String, (i64, SharedString)>,
}

impl gpui::Global for ReaderUi {}

#[derive(Clone, Copy)]
pub(crate) enum BodyWait {
    Loading(std::time::Instant),
    /// Loaded after its placeholder showed, so it faded in over it.
    Revealed,
}
/// When the reader's placeholder says what it's waiting for (MA-066).
const BODY_SLOW: std::time::Duration = std::time::Duration::from_secs(1);

/// Whether a loaded body should fade in: its placeholder showed. Notes a
/// body still loading.
fn body_reveal(id: &str, loaded: bool, cx: &mut App) -> bool {
    use std::time::Instant;
    let waits = &mut cx.default_global::<ReaderUi>().body_waits;
    match (waits.get(id).copied(), loaded) {
        (None, false) => {
            waits.insert(id.to_string(), BodyWait::Loading(Instant::now()));
            false
        }
        (_, false) => false,
        (None, true) => false,
        (Some(BodyWait::Loading(at)), true) => {
            if at.elapsed() > super::motion::LOADING_DELAY {
                waits.insert(id.to_string(), BodyWait::Revealed);
                true
            } else {
                waits.remove(id);
                false
            }
        }
        // Kept wrapped, so the body isn't remounted once the fade ends.
        (Some(BodyWait::Revealed), true) => true,
    }
}

/// Quick Look one attachment of the selected conversation, by message and
/// attachment index (MB-044): Space on its chip, and its menu.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct QuickLookAttachmentAt(pub usize, pub usize);

/// Show one attachment's file in Finder; from its menu.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct RevealAttachmentAt(pub usize, pub usize);

/// Fetch one attachment's file without opening it: a drag to Finder needs
/// it on disk (MA-086).
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct PrefetchAttachmentAt(pub usize, pub usize);

/// Show a contact's card; from a name's menu.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = pigeon, no_json)]
pub struct ShowContactFor(pub Contact, pub Option<gpui::Point<Pixels>>);

/// What a message's place in the conversation shows (ME-008).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Show,
    /// The first of `n` folded messages: an "n more messages" row.
    Fold(usize),
    /// Folded behind the row above.
    Hidden,
}

/// Over four messages, the ones between the first and the last two fold
/// into "N more" rows, but not those `keep` shows; a run of one stays.
fn fold_slots(len: usize, keep: impl Fn(usize) -> bool) -> Vec<Slot> {
    let mut slots = vec![Slot::Show; len];
    if len <= 4 {
        return slots;
    }
    let mut ix = 1;
    while ix < len - 2 {
        if keep(ix) {
            ix += 1;
            continue;
        }
        let run = (ix..len - 2).take_while(|&i| !keep(i)).count();
        if run > 1 {
            slots[ix] = Slot::Fold(run);
            slots[ix + 1..ix + run].fill(Slot::Hidden);
        }
        ix += run;
    }
    slots
}

/// "just now", "5 min ago" or "3 hr ago", for a date under a day old.
fn relative_age(date: &DateTime<FixedOffset>, now: DateTime<Local>) -> Option<String> {
    let minutes = (now - date.with_timezone(&Local)).num_minutes();
    match minutes {
        ..0 => None,
        0 => Some("just now".into()),
        1..60 => Some(format!("{minutes} min ago")),
        60..1440 => Some(format!("{} hr ago", minutes / 60)),
        _ => None,
    }
}

/// A label's own name, without the labels it's nested under.
fn leaf(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Records where an element was drawn, for a menu to open under it.
fn anchor(cell: &Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    let cell = cell.clone();
    canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .size_full()
}

/// What an attachment chip drags (MA-086); also its picture while dragged.
#[derive(Clone)]
pub struct DraggedAttachment {
    message_id: String,
    message: usize,
    attachment: usize,
    name: SharedString,
}

impl Render for DraggedAttachment {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .h(CONTROL_LG)
            .px(SPACE_MD)
            .flex()
            .items_center()
            .rounded(RADIUS_MD)
            .bg(theme.chip_bg)
            .text_size(TEXT_SM)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text)
            .child(self.name.clone())
    }
}

impl PigeonApp {
    pub(crate) fn render_reader(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let thread = self.selected.map(|ix| &self.data.threads[ix]);
        let standalone = self
            .composer
            .as_ref()
            .filter(|_| !self.composer_inline(cx))
            .map(|(composer, _)| composer.clone());
        // W2 (RDR-8): the source, while its conversation is selected.
        let original = self
            .original
            .as_ref()
            .filter(|o| thread.is_some_and(|t| t.id == o.thread_id));
        if self.original.is_some() && original.is_none() {
            cx.defer_in(window, |this, window, cx| {
                if let Some(original) = this.original.take() {
                    if original.focus.contains_focused(window, cx) {
                        window.focus(&this.focus_handle, cx);
                    }
                    cx.notify();
                }
            });
        }

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.reader_bg)
            .map(|this| match standalone {
                // The draft scrolls itself, above its send row.
                Some(composer) => this
                    .child(self.render_compose_toolbar(&composer, &theme, cx))
                    // MD-019: `c` fades the draft in.
                    .child(super::motion::enter(
                        "compose-in",
                        super::motion::Rise::None,
                        div()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .max_w(READER_MAX)
                            .child(composer),
                    )),
                None if !self.checked.is_empty() => this
                    .child(
                        window_drag_area(div().id("reader-toolbar"))
                            .h(super::HEADER_MIN)
                            .min_h(HEADER_HEIGHT),
                    )
                    .child(self.render_bulk_summary(&theme, window)),
                None if original.is_some() => {
                    this.child(render_original(original.expect("checked"), &theme))
                }
                None => this
                    .child(self.render_toolbar(thread, &theme, cx))
                    .child(match thread {
                        Some(thread) => self.render_conversation(thread, &theme, window, cx),
                        None => self.render_no_conversation(&theme),
                    }),
            })
    }

    /// The reader with nothing selected: how to pick something, or calm
    /// when the list has nothing to pick (ME-001, ME-030).
    fn render_no_conversation(&self, theme: &Theme) -> AnyElement {
        let any = self.rows.iter().any(|row| matches!(row, Row::Thread(_)));
        div()
            .id("no-conversation")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(SPACE_SM)
            .line_height(LINE_UI)
            .child(icon(Icon::Mail, ICON_HERO, theme.text_faint))
            .child(
                super::label(
                    "empty-title",
                    if any {
                        "No conversation selected"
                    } else {
                        "Nothing to read here"
                    },
                )
                .text_size(TEXT_BASE)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted),
            )
            .when(any, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_XS)
                        .text_size(TEXT_SM)
                        .text_color(theme.text_faint)
                        .child(super::keys_caps("j", theme))
                        .child(super::keys_caps("k", theme))
                        .child(super::label("empty-move", "to move"))
                        .child(div().mx(SPACE_XS).child("·"))
                        .child(super::keys_caps("enter", theme))
                        .child(super::label("empty-open", "to open")),
                )
            })
            .into_any_element()
    }

    fn render_compose_toolbar(
        &self,
        composer: &gpui::Entity<crate::ui::composer::Composer>,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let title = composer.read(cx).title();
        window_drag_area(div().id("compose-toolbar"))
            .h(super::HEADER_MIN)
            .min_h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .pl(SPACE_XL)
            .pr(SPACE_MD)
            .child(
                super::label("compose-title", title)
                    .role(gpui::Role::Heading)
                    .aria_level(1)
                    .text_size(TEXT_LG)
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(div().flex_1())
            // The bottom row scrolls away with a long draft (MA-064).
            .child(
                push_button("compose-send", "Send", ButtonKind::Primary, theme)
                    .mr(SPACE_SM)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(SendMessage), cx)),
            )
            .child(icon_button(
                "compose-send-later",
                Icon::Clock,
                "Send later",
                Box::new(SendLater),
                theme,
            ))
            .child(icon_button(
                "compose-new-window",
                Icon::PanelTop,
                "Open in new window",
                Box::new(OpenDraftInNewWindow),
                theme,
            ))
            .child(icon_button(
                "compose-discard",
                Icon::Trash,
                "Discard draft",
                Box::new(DiscardDraft),
                theme,
            ))
            .child(icon_button(
                "compose-close",
                Icon::X,
                "Close and save draft",
                Box::new(CloseDraft),
                theme,
            ))
    }

    fn render_toolbar(
        &self,
        thread: Option<&Thread>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let header = window_drag_area(div().id("reader-toolbar"))
            .h(super::HEADER_MIN)
            .min_h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            // The first glyph lines up with the content's edge below.
            .pl(SPACE_XL - (HIT_TARGET - ICON_TOOLBAR) / 2.)
            .pr(SPACE_MD);
        let Some(thread) = thread else {
            return header;
        };
        let more_anchor = cx.default_global::<ReaderUi>().more_anchor.clone();

        header
            .when_some(thread.folder.archive_target(), |this, target| {
                this.child(icon_button(
                    "archive",
                    if target == Folder::Inbox {
                        Icon::Inbox
                    } else {
                        Icon::Archive
                    },
                    thread.folder.archive_title().unwrap_or("Archive"),
                    Box::new(Archive),
                    theme,
                ))
            })
            .when(thread.folder != Folder::Trash, |this| {
                this.child(icon_button(
                    "delete",
                    Icon::Trash,
                    if thread.folder == Folder::Drafts {
                        "Discard draft"
                    } else {
                        "Delete"
                    },
                    Box::new(Delete),
                    theme,
                ))
            })
            .child(icon_button(
                "mark-unread",
                Icon::MailUnread,
                "Mark as unread",
                Box::new(MarkUnread),
                theme,
            ))
            // ACT-11.
            .when(
                !matches!(thread.folder, Folder::Drafts | Folder::Spam | Folder::Trash),
                |this| {
                    this.child(icon_button(
                        "snooze",
                        Icon::AlarmClock,
                        "Snooze",
                        Box::new(Snooze),
                        theme,
                    ))
                },
            )
            .child(if thread.starred {
                let star = icon(Icon::StarFilled, ICON_TOOLBAR, theme.star);
                // MD-027: a star set just now pops.
                let star = if self.star_pops(&thread.id) {
                    super::motion::pop("star-pop", star).into_any_element()
                } else {
                    star.into_any_element()
                };
                icon_button_with("star", star, "Unstar", Box::new(ToggleStar), theme)
            } else {
                icon_button("star", Icon::Star, "Star", Box::new(ToggleStar), theme)
            })
            .child(if thread.important {
                // W4b (FOL-12).
                icon_button_tinted(
                    "important",
                    Icon::Important,
                    theme.star,
                    "Mark as not important",
                    Box::new(ToggleImportant),
                    theme,
                )
            } else {
                icon_button(
                    "important",
                    Icon::ImportantOutline,
                    "Mark as important",
                    Box::new(ToggleImportant),
                    theme,
                )
            })
            .child(
                more_button(theme)
                    .relative()
                    .child(anchor(&more_anchor))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                        // Under the button, however it was pressed (MB-016).
                        let Some(bounds) = more_anchor.get() else {
                            return;
                        };
                        this.show_thread_menu(bounds.bottom_left(), false, window, cx)
                    })),
            )
            .child(div().flex_1())
            // Reply, Reply All and Forward are one group (MB-020).
            .child(icon_button(
                "reply",
                Icon::Reply,
                "Reply",
                Box::new(Reply),
                theme,
            ))
            .child(icon_button(
                "reply-all",
                Icon::ReplyAll,
                "Reply all",
                Box::new(ReplyAll),
                theme,
            ))
            .child(icon_button(
                "forward",
                Icon::Forward,
                "Forward",
                Box::new(Forward),
                theme,
            ))
    }

    fn render_conversation(
        &self,
        thread: &Thread,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips = thread
            .labels
            .iter()
            .filter_map(|id| self.data.labels.iter().position(|l| l.id == *id))
            .map(|ix| self.render_label_chip(ix, theme, cx))
            .collect::<Vec<_>>();
        // AI-6, AI-8, AI-9.
        let suggested = self
            .suggested_label(thread)
            .map(|ix| self.render_suggested_label(ix, theme));
        let summary = self
            .summary_view(thread, cx)
            .map(|view| render_summary(&view, theme));
        let last = thread.messages.len() - 1;
        let layout = self.pane_layout(window.viewport_size().width);
        let narrow =
            window.viewport_size().width - crate::app::RAIL_WIDTH - layout.sidebar - layout.list
                < NARROW;
        let active = window.is_window_active();
        let fold_open = cx
            .default_global::<ReaderUi>()
            .open_folds
            .contains(&thread.id);
        let slots = fold_slots(thread.messages.len(), |ix| {
            fold_open || self.expanded_messages.contains(&ix) || self.focused_message == Some(ix)
        });

        // Each message is a child of the scroll view, so `n` and `p` can
        // scroll to it; together they read as one card.
        let messages = thread
            .messages
            .iter()
            .enumerate()
            .map(|(ix, message)| {
                // Keyed by thread, so the text inside keeps no selection
                // state from another conversation.
                let id = ElementId::NamedInteger(thread.id.clone().into(), ix as u64);
                let card = || {
                    div()
                        .id(id.clone())
                        .w_full()
                        .max_w(READER_MAX)
                        .relative()
                        .bg(theme.card_bg)
                        .border_x_1()
                        .border_t_1()
                        .border_color(theme.border)
                        .when(ix == 0, |d| d.rounded_t(RADIUS_LG))
                        .when(ix == last, |d| d.border_b_1().rounded_b(RADIUS_LG))
                };
                match slots[ix] {
                    Slot::Hidden => div().id(id.clone()).into_any_element(),
                    Slot::Fold(n) => card()
                        .child(self.render_fold_row(&thread.id, n, theme, cx))
                        .into_any_element(),
                    Slot::Show => {
                        let focused = self.reader_focused && self.focused_message == Some(ix);
                        let body = if ix == last || self.expanded_messages.contains(&ix) {
                            let body = self.render_message(
                                thread,
                                ix,
                                message,
                                ix != last,
                                narrow,
                                theme,
                                cx,
                            );
                            // MD-020: opened with a click, it fades in.
                            if self.clicked_open == Some(ix) {
                                super::motion::enter(
                                    "opened",
                                    super::motion::Rise::None,
                                    div().w_full().child(body),
                                )
                                .into_any_element()
                            } else {
                                body
                            }
                        } else {
                            self.render_collapsed_message(ix, message, last, theme, cx)
                        };
                        card()
                            .role(gpui::Role::Article)
                            .aria_label(format!(
                                "{}, {}",
                                self.contact_name(&message.from),
                                full_date(&message.date),
                            ))
                            .when(focused, |d| d.bg(theme.card_focused_bg))
                            .child(body)
                            .when(focused, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left(-px(1.))
                                        .top(SPACE_SM + SPACE_2XS)
                                        .bottom(SPACE_SM + SPACE_2XS)
                                        .w(SPACE_2XS + px(1.))
                                        .rounded_r(RADIUS_SM)
                                        // Gray while another window has the keys (MB-042).
                                        .bg(if active {
                                            theme.accent
                                        } else {
                                            theme.text_faint
                                        }),
                                )
                            })
                            .on_any_mouse_down(cx.listener(
                                move |this, event: &gpui::MouseDownEvent, window, cx| {
                                    if super::is_secondary(event) {
                                        cx.stop_propagation();
                                        this.show_message_menu(ix, event.position, window, cx);
                                    }
                                },
                            ))
                            .into_any_element()
                    }
                }
            })
            .collect::<Vec<_>>();
        // MD-019: rises in, and fades out after a send, Esc or discard.
        let rise = super::motion::moving(super::motion::Rise::Up, cx);
        let leaving = !self.composer_inline(cx);
        let composer = self.reply_card_composer().map(|composer| {
            super::motion::card_motion(
                div()
                    .relative()
                    .w_full()
                    .max_w(READER_MAX)
                    .mt(SPACE_LG)
                    .px(SPACE_LG)
                    .pt(SPACE_MD)
                    .pb(SPACE_LG)
                    .rounded(RADIUS_LG)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.card_bg)
                    .child(composer)
                    .when(leaving, |this| this.child(super::motion::inert_cover())),
                self.reply_card,
                rise,
            )
        });

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("conversation")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.reader_scroll)
                    // The trackpad wins over a keyboard glide (MD-018).
                    .on_scroll_wheel(cx.listener(|this, _, _, _| this.reader_glide = None))
                    .px(SPACE_XL)
                    .pt(SPACE_SM)
                    .pb(SPACE_2XL)
                    .child(
                        div()
                            .id("subject")
                            .w_full()
                            .max_w(READER_MAX)
                            .text_size(TEXT_TITLE2)
                            .line_height(LINE_UI)
                            .font_weight(FontWeight::SEMIBOLD)
                            // "(no subject)" reads as a placeholder (ME-021).
                            .when(!thread.has_subject(), |d| {
                                d.text_color(theme.text_faint).italic()
                            })
                            .child(
                                Selectable::plain(
                                    ElementId::Name(format!("subject-{}", thread.id).into()),
                                    thread.subject_or_placeholder().to_string(),
                                    theme.text_selection,
                                )
                                .order(ORDER_BASE)
                                .heading(1),
                            ),
                    )
                    // One child, so `MESSAGES_START` holds with the bars.
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .children(render_later_bar(thread, theme))
                            .children(summary)
                            .child(
                                div()
                                    .w_full()
                                    .max_w(READER_MAX)
                                    .mt(SPACE_MD)
                                    .mb(SPACE_LG)
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(SPACE_SM)
                                    .children(chips)
                                    .children(suggested)
                                    // The count only where there's more than one (ME-026).
                                    .children((thread.messages.len() > 1).then(|| {
                                        div()
                                            .text_size(TEXT_SM)
                                            .text_color(theme.text_faint)
                                            .child(format!("{} messages", thread.messages.len()))
                                    })),
                            ),
                    )
                    .children(messages)
                    .children(composer),
            )
            .vertical_scrollbar(&self.reader_scroll)
            .into_any_element()
    }

    /// A label on the conversation: its own name, the whole path in the
    /// tooltip, and a click opens it (ME-022).
    fn render_label_chip(&self, ix: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let label = &self.data.labels[ix];
        let path: SharedString = label.name.replace('/', " › ").into();
        let tip = path.clone();
        let shown = crate::data::Label {
            name: leaf(&label.name).to_string(),
            ..label.clone()
        };
        focusable_control(div().id(("label-chip", ix)), theme)
            .role(gpui::Role::Link)
            .aria_label(format!("Label {path}"))
            .flex_none()
            .rounded(RADIUS_SM)
            .hover(|s| s.opacity(0.85))
            .child(super::label_chip(&shown, theme))
            .tooltip(move |window, cx| super::tooltip(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.set_nav(Nav::Label(ix), cx)))
            .into_any_element()
    }

    /// AI-6: the label Jev suggests, as a ghost chip; a click or ⌥L
    /// applies it, with Undo.
    fn render_suggested_label(&self, ix: usize, theme: &Theme) -> AnyElement {
        let label = &self.data.labels[ix];
        let path = label.name.replace('/', " › ");
        focusable_control(div().id("suggested-label"), theme)
            .role(gpui::Role::Button)
            .aria_label(format!("Apply suggested label {path}"))
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .py(SPACE_2XS)
            .px(SPACE_SM)
            .rounded(RADIUS_SM)
            .border_1()
            .border_color(theme.border)
            .line_height(LINE_UI)
            .text_size(TEXT_XS)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.hover_bg))
            .child(icon(Icon::Plus, ICON_SM, theme.text_faint))
            .child(leaf(&label.name).to_string())
            .tooltip(move |window, cx| {
                super::tooltip(format!("Suggested label {path} · ⌥L")).build(window, cx)
            })
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ApplySuggestedLabel), cx))
            .into_any_element()
    }

    /// In place of folded messages: "3 more messages" (ME-008).
    fn render_fold_row(
        &self,
        thread_id: &str,
        count: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let thread_id = thread_id.to_string();
        let text = format!("{count} more messages");
        focusable_control(div().id("more-messages"), theme)
            .role(gpui::Role::Button)
            .aria_label(text.clone())
            .aria_expanded(false)
            .py(SPACE_SM)
            .px(SPACE_LG)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .hover(|s| s.bg(theme.hover_bg))
            .text_size(TEXT_SM)
            .text_color(theme.text_muted)
            .child(icon(Icon::ChevronDown, ICON_SM, theme.text_faint))
            .child(text)
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.default_global::<ReaderUi>()
                    .open_folds
                    .insert(thread_id.clone());
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_collapsed_message(
        &self,
        ix: usize,
        message: &Message,
        last: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = self.contact_name(&message.from).to_string();
        let full = full_date(&message.date);
        let tip = full.clone();
        focusable_control(div().id(("message", ix)), theme)
            .role(gpui::Role::DisclosureTriangle)
            .aria_expanded(false)
            .aria_label(format!("{name}, {full}: {}", message.snippet))
            .py(SPACE_MD)
            .px(SPACE_LG)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            // Square inside the stacked card, but its first row (ME-032).
            .when(ix == 0, |d| d.rounded_t(RADIUS_LG))
            .when(ix == last, |d| d.rounded_b(RADIUS_LG))
            .hover(|s| s.bg(theme.hover_bg))
            .child(self.sender_avatar(&message.from, AVATAR_SM, theme))
            .child(
                // At most half the row, so the snippet and date keep room
                // (ME-013).
                div()
                    .min_w_0()
                    .max_w(relative(0.5))
                    .truncate()
                    .text_size(TEXT_BASE)
                    .font_weight(FontWeight::MEDIUM)
                    .child(name),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(message.snippet.clone()),
            )
            .child(
                div()
                    .id(("message-date", ix))
                    .flex_none()
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .font_features(tabular_figures())
                    .child(list_date(&message.date))
                    .tooltip(move |window, cx| super::tooltip(tip.clone()).build(window, cx)),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_message(ix, cx)))
            .into_any_element()
    }

    /// To, Cc and, on the user's own mail, Bcc: three names a group, then
    /// "+N others" until opened (ME-012). Each name opens its contact card.
    fn render_recipients(
        &self,
        ix: usize,
        message: &Message,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        const SHOWN: usize = 3;
        let open = cx
            .default_global::<ReaderUi>()
            .open_recipients
            .contains(&message.id);
        let mine = self.data.is_me(&message.from);
        let mut groups = vec![("to", &message.to), ("cc", &message.cc)];
        if mine {
            groups.push(("bcc", &message.bcc));
        }
        let mut line: Vec<AnyElement> = Vec::new();
        let mut hidden = 0;
        for (group, (prefix, contacts)) in groups
            .into_iter()
            .filter(|(_, contacts)| !contacts.is_empty())
            .enumerate()
        {
            if group > 0 {
                line.push(div().mx(SPACE_XS).child("·").into_any_element());
            }
            line.push(div().mr(SPACE_XS).child(prefix).into_any_element());
            let shown = if open {
                contacts.len()
            } else {
                contacts.len().min(SHOWN)
            };
            hidden += contacts.len() - shown;
            for (n, contact) in contacts.iter().take(shown).enumerate() {
                let name = self.contact_name(contact).to_string();
                let last = n + 1 == shown;
                line.push(
                    self.contact_link(
                        ("recipient", ix * 10_000 + group * 3_000 + n),
                        contact,
                        &name,
                        theme,
                        cx,
                    )
                    .child(if last { name } else { format!("{name},") })
                    .when(!last, |d| d.mr(SPACE_XS))
                    .into_any_element(),
                );
            }
        }
        if hidden > 0 {
            let id = message.id.clone();
            let text = match hidden {
                1 => "+1 other".to_string(),
                n => format!("+{n} others"),
            };
            line.push(
                focusable_control(div().id(("recipients-more", ix)), theme)
                    .role(gpui::Role::Button)
                    .aria_label(format!("Show {text}"))
                    .aria_expanded(false)
                    .ml(SPACE_XS)
                    .px(SPACE_XS)
                    .rounded(RADIUS_SM)
                    .text_color(theme.text)
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(text)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.default_global::<ReaderUi>()
                            .open_recipients
                            .insert(id.clone());
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .text_size(TEXT_SM)
            .text_color(theme.text_muted)
            .map(|d| {
                if open {
                    d.flex_wrap()
                } else {
                    d.overflow_hidden().whitespace_nowrap()
                }
            })
            .children(line)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_message(
        &self,
        thread: &Thread,
        ix: usize,
        message: &Message,
        collapsible: bool,
        narrow: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let to_line = self.render_recipients(ix, message, theme, cx);
        let reveal = match &message.body {
            Body::Failed(_) => {
                cx.default_global::<ReaderUi>()
                    .body_waits
                    .remove(&message.id);
                false
            }
            body => body_reveal(&message.id, matches!(body, Body::Loaded { .. }), cx),
        };
        // AI-10: a newsletter's Clean View, in place of its body.
        let clean = match &message.body {
            Body::Loaded { .. } => self.clean_view(thread, ix, cx),
            _ => crate::app::ai_reader::CleanView::Off,
        };
        let body = match (&message.body, &clean) {
            (Body::Loaded { .. }, crate::app::ai_reader::CleanView::Clean(html)) => {
                self.parsed_html(&format!("clean-{}", message.id), html, cx)
            }
            (Body::Loaded { .. }, _) => self.parsed(message, cx),
            _ => None,
        };
        // AI-16, AI-11.
        let event = self
            .event_offer(thread, cx)
            .filter(|(m, _)| *m == ix)
            .map(|(_, candidate)| render_event_bar(ix, &candidate, theme));
        let replies = if ix + 1 == thread.messages.len() {
            self.quick_replies(thread)
        } else {
            Vec::new()
        };
        let clean_label = render_clean_label(ix, &message.id, &clean, theme, cx);
        let clean_bar = (clean == crate::app::ai_reader::CleanView::Ready)
            .then(|| render_clean_bar(ix, &message.id, theme, cx));
        let images = body
            .as_ref()
            .map(|body| self.message_images(message, body, cx))
            .unwrap_or_default();
        let attachments = self.render_attachments(ix, message, theme, cx);

        let mine = self.data.is_me(&message.from);
        let focused = self.reader_focused && self.focused_message == Some(ix);
        let order = ORDER_BASE + ((ix as u64 + 1) << 20);
        let name = self.contact_name(&message.from).to_string();
        let date = full_date(&message.date);
        let age = relative_age(&message.date, Local::now());
        let draft = thread.folder == Folder::Drafts;
        let header = div()
            .id(("message-header", ix))
            .group("message-header")
            .flex()
            .items_start()
            .gap(SPACE_MD)
            .child(
                self.contact_link(("sender-avatar", ix), &message.from, &name, theme, cx)
                    .flex_none()
                    .child(self.sender_avatar(&message.from, AVATAR_MD, theme)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2XS)
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(SPACE_SM)
                            .child(
                                // Truncates before the date does (ME-013).
                                self.contact_link(
                                    ("sender-name", ix),
                                    &message.from,
                                    &name,
                                    theme,
                                    cx,
                                )
                                .min_w_0()
                                .truncate()
                                .text_size(TEXT_BASE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(
                                    Selectable::plain(
                                        "sender-name",
                                        name.clone(),
                                        theme.text_selection,
                                    )
                                    .order(order)
                                    .silent(),
                                ),
                            )
                            // Others' addresses on hover and while the
                            // message has the keys (MA-047); never my own.
                            .when(!mine, |this| {
                                this.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(TEXT_SM)
                                        .text_color(theme.text_faint)
                                        .when(!focused, |d| {
                                            d.opacity(0.)
                                                .group_hover("message-header", |s| s.opacity(1.))
                                        })
                                        .child(
                                            Selectable::plain(
                                                "sender-address",
                                                message.from.email.clone(),
                                                theme.text_selection,
                                            )
                                            .order(order + 1),
                                        ),
                                )
                            }),
                    )
                    .child(to_line),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(SPACE_XS)
                    .children(clean_label)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_faint)
                            .font_features(tabular_figures())
                            .child(date)
                            .children(age.map(|age| div().text_size(TEXT_XS).child(age))),
                    )
                    // Reply to this message, rather than the newest (ME-010).
                    .when(!draft, |this| {
                        this.child(
                            div()
                                .flex()
                                .when(!focused, |d| {
                                    d.opacity(0.)
                                        .group_hover("message-header", |s| s.opacity(1.))
                                })
                                .child(
                                    super::icon_button_small(
                                        ("message-reply", ix),
                                        Icon::Reply,
                                        "Reply",
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.focus_message(ix, cx);
                                            window.dispatch_action(Box::new(Reply), cx);
                                        },
                                    )),
                                )
                                .child(
                                    super::icon_button_small(
                                        ("message-more", ix),
                                        Icon::Ellipsis,
                                        "Reply all, forward and more",
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        move |this, event: &gpui::ClickEvent, window, cx| {
                                            cx.stop_propagation();
                                            this.show_message_menu(
                                                ix,
                                                event.position(),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                ),
                        )
                    }),
            )
            .when(collapsible, |this| {
                this.on_click(cx.listener(move |this, _, window, cx| {
                    // Not when the click ended selecting the sender.
                    if TextSelection::selected_text(window, cx).is_empty() {
                        this.toggle_message(ix, cx)
                    }
                }))
            });

        let link = cx
            .default_global::<ReaderUi>()
            .link
            .as_ref()
            .filter(|(t, m, _)| self.reader_focused && *t == thread.id && *m == ix)
            .map(|(_, _, n)| *n);
        let quote_open = cx
            .default_global::<ReaderUi>()
            .open_quotes
            .contains(&message.id);
        let selection = html_view::Selection {
            order: order + 2,
            all: (self.select_all_text == Some(ix)).then(|| {
                let app = cx.entity().downgrade();
                Rc::new(move |cx: &mut gpui::App| {
                    _ = app.update(cx, |this, cx| {
                        if this.select_all_text.take().is_some() {
                            cx.notify();
                        }
                    });
                }) as OnClear
            }),
            focused_link: link,
        };
        let fold = body
            .as_ref()
            .and_then(|doc| crate::html::trailing_quote_start(doc))
            .map(|start| {
                let app = cx.entity().downgrade();
                let id = message.id.clone();
                html_view::Fold {
                    start,
                    open: quote_open,
                    id: ("quote-toggle", ix).into(),
                    on_toggle: Rc::new(move |_, cx| {
                        let quotes = &mut cx.default_global::<ReaderUi>().open_quotes;
                        if !quotes.remove(&id) {
                            quotes.insert(id.clone());
                        }
                        app.update(cx, |_, cx| cx.notify()).ok();
                    }),
                }
            });

        let indent = |d: gpui::Div| {
            if narrow {
                d
            } else {
                // Under the sender's name: past the avatar and the gap.
                d.ml(SPACE_MD).pl(AVATAR_MD)
            }
        };
        div()
            .px(SPACE_LG)
            .pt(SPACE_LG)
            .pb(SPACE_XL)
            .flex()
            .flex_col()
            .child(header)
            .child(
                indent(div().mt(SPACE_LG))
                    .children(event)
                    .children(clean_bar)
                    .when_some(images.ask, |this, hidden| {
                        this.child(render_images_bar(ix, &message.id, hidden, theme, cx))
                    })
                    .child(match (&message.body, body) {
                        (Body::Loaded { .. }, Some(body)) => {
                            let el = render_body(
                                &body,
                                selection,
                                html_view::ShowImages {
                                    loaded: &self.images,
                                    remote: images.show,
                                },
                                fold,
                                theme,
                            );
                            if reveal {
                                super::motion::enter(
                                    ElementId::Name(format!("body-in-{}", message.id).into()),
                                    super::motion::Rise::None,
                                    div().child(el),
                                )
                                .into_any_element()
                            } else {
                                el.into_any_element()
                            }
                        }
                        (Body::Failed(reason), _) => render_body_failed(ix, reason, theme),
                        _ => render_body_loading(theme),
                    }),
            )
            .when_some(attachments, |this, attachments| {
                this.child(
                    indent(div().mt(SPACE_LG))
                        .flex()
                        .flex_wrap()
                        .gap(SPACE_SM)
                        .children(attachments),
                )
            })
            .when(!replies.is_empty(), |this| {
                this.child(indent(div().mt(SPACE_LG)).child(render_quick_replies(&replies, theme)))
            })
            .into_any_element()
    }

    /// The message's body, parsed; a big one is parsed off the main thread
    /// first (MC-054), and shows as loading until it is.
    fn parsed(&self, message: &Message, cx: &mut Context<Self>) -> Option<Rc<Document>> {
        self.parsed_html(&message.id, message.html(), cx)
    }

    /// `parsed` for any HTML kept under `id`: a body, or its Clean View.
    fn parsed_html(&self, id: &str, html: &str, cx: &mut Context<Self>) -> Option<Rc<Document>> {
        if let Some(doc) = self.html_cache.get(id, html) {
            return Some(doc);
        }
        let key = crate::html::HtmlCache::key(id, html);
        if self.html_cache.start_parse(&key) {
            let html = html.to_string();
            cx.spawn(async move |this, cx| {
                let doc = cx
                    .background_executor()
                    .spawn(async move { crate::html::parse(&html) })
                    .await;
                this.update(cx, |this, cx| {
                    this.html_cache.insert(key, Rc::new(doc));
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        None
    }

    /// The message's attachment chips: open on click, Quick Look on Space,
    /// a menu on right-click, and a drag to Finder.
    fn render_attachments(
        &self,
        ix: usize,
        message: &Message,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<Vec<AnyElement>> {
        if message.attachments.is_empty() {
            return None;
        }
        let demo = self.backend.read(cx).is_demo();
        let ui = cx.default_global::<ReaderUi>();
        let anchor_cell = ui.attachment_anchor.clone();
        let fetching: Vec<bool> = (0..message.attachments.len())
            .map(|n| ui.fetching.contains_key(&(message.id.clone(), n)))
            .collect();
        // The picker opens under the conversation's first chip.
        let first = self.data.threads[self.selected?]
            .messages
            .iter()
            .position(|m| !m.attachments.is_empty())
            == Some(ix);
        Some(
            message
                .attachments
                .iter()
                .enumerate()
                .map(|(n, attachment)| {
                    let size = file_size(attachment.size_bytes);
                    let fetching = fetching[n];
                    let glyph = super::attachment_icon(attachment, theme);
                    let glyph = if fetching {
                        div()
                            .flex_none()
                            .child(glyph)
                            .with_animation(
                                ("attachment-fetching", ix * 100 + n),
                                Animation::new(std::time::Duration::from_millis(1000))
                                    .repeat()
                                    .with_easing(gpui::pulsating_between(0.3, 1.)),
                                |d, t| d.opacity(t),
                            )
                            .into_any_element()
                    } else {
                        glyph.into_any_element()
                    };
                    let drag = DraggedAttachment {
                        message_id: message.id.clone(),
                        message: ix,
                        attachment: n,
                        name: attachment.name.clone().into(),
                    };
                    focusable_control(div().id(("attachment", ix * 100 + n)), theme)
                        .key_context(format!("{CONTROL} {ATTACHMENT}").as_str())
                        .role(gpui::Role::Button)
                        .aria_label(if fetching {
                            format!("Getting {}…", attachment.name)
                        } else {
                            format!("Open {}, {size}", attachment.name)
                        })
                        .relative()
                        .h(CONTROL_LG)
                        .pl(SPACE_MD)
                        .pr(SPACE_XS)
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .rounded(RADIUS_MD)
                        .bg(theme.chip_bg)
                        .hover(|s| s.bg(theme.selected_bg))
                        .when(first && n == 0, |d| d.child(anchor(&anchor_cell)))
                        .child(glyph)
                        .child(
                            div()
                                .max_w(CHIP_NAME_MAX)
                                .truncate()
                                .text_size(TEXT_SM)
                                .font_weight(FontWeight::MEDIUM)
                                .child(attachment.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_faint)
                                .child(size),
                        )
                        .child(
                            super::icon_button_small(
                                ("attachment-save", ix * 100 + n),
                                Icon::Download,
                                format!("Save {}…", attachment.name),
                                theme,
                            )
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                window.dispatch_action(Box::new(SaveAttachmentAt(ix, n)), cx)
                            }),
                        )
                        .on_action(move |_: &QuickLookAttachment, window, cx| {
                            window.dispatch_action(Box::new(QuickLookAttachmentAt(ix, n)), cx)
                        })
                        .on_any_mouse_down(move |event, window, cx| {
                            if super::is_secondary(event) {
                                cx.stop_propagation();
                                attachment_menu(ix, n, event.position, window, cx);
                            }
                        })
                        .when(!demo, |d| {
                            d.on_drag(drag, |drag, _, window, cx| {
                                // The file has to be on disk before the drag
                                // leaves the window.
                                window.dispatch_action(
                                    Box::new(PrefetchAttachmentAt(drag.message, drag.attachment)),
                                    cx,
                                );
                                cx.new(|_| drag.clone())
                            })
                            .external_drag_payload(
                                |drag: &DraggedAttachment, _, cx| {
                                    let ready = cx
                                        .try_global::<ReaderUi>()?
                                        .files
                                        .get(&(drag.message_id.clone(), drag.attachment))?;
                                    (!ready.executable).then(|| {
                                        gpui::ExternalDragPayload::Files(gpui::FileDragPaths::new(
                                            [(ready.path.clone(), false)],
                                        ))
                                    })
                                },
                            )
                        })
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(OpenAttachmentAt(ix, n)), cx)
                        })
                        .into_any_element()
                })
                .collect(),
        )
    }

    /// Makes `ix` the message the keys and replies act on.
    pub(crate) fn focus_message(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.focused_message = Some(ix);
        self.reader_focused = true;
        cx.notify();
    }

    /// A message's context menu (MB-010): Copy for selected text, else
    /// replies and its source.
    fn show_message_menu(
        &mut self,
        ix: usize,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !TextSelection::selected_text(window, cx).is_empty() {
            NativeMenu::new()
                .menu("Copy", Box::new(EditCopy))
                .show(position, window, cx);
            return;
        }
        let Some(thread) = self.selected.map(|t| &self.data.threads[t]) else {
            return;
        };
        let draft = thread.folder == Folder::Drafts;
        self.focus_message(ix, cx);
        let mut menu = NativeMenu::new();
        if !draft {
            menu = menu
                .menu("Reply", Box::new(Reply))
                .menu("Reply All", Box::new(ReplyAll))
                .menu("Forward", Box::new(Forward))
                .separator();
        }
        menu.menu("Show Original", Box::new(ShowOriginal))
            .menu("Save as .eml…", Box::new(SaveOriginal))
            .show(position, window, cx);
    }

    /// In place of a conversation while several are selected: what they
    /// are, and what can be done to all of them.
    fn render_bulk_summary(&self, theme: &Theme, window: &Window) -> AnyElement {
        let targets = self.targets();
        let count = targets.len();
        let all_unread = targets.iter().all(|&ix| self.data.threads[ix].unread);
        let all_starred = targets.iter().all(|&ix| self.data.threads[ix].starred);
        let first = targets.first().map(|&ix| &self.data.threads[ix]);
        // The same label and icon as the toolbar's (ME-027).
        let folder = first.map(|t| t.folder);
        let archive_label = folder.and_then(Folder::archive_title).unwrap_or("Archive");
        let archive_icon = match folder.and_then(Folder::archive_target) {
            Some(Folder::Inbox) => Icon::Inbox,
            _ => Icon::Archive,
        };
        let subjects = targets.iter().take(5).map(|&ix| {
            let thread = &self.data.threads[ix];
            div()
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .min_w_0()
                .child(icon(Icon::Mail, ICON_SM, theme.text_faint))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .when(!thread.has_subject(), |d| {
                            d.italic().text_color(theme.text_faint)
                        })
                        .child(thread.subject_or_placeholder().to_string()),
                )
        });
        let action =
            |id: &'static str, glyph: Icon, label: &'static str, action: Box<dyn gpui::Action>| {
                let caps = self
                    .binding_for(action.as_ref(), window)
                    .map(|b| super::binding_caps(&b, theme));
                focusable_control(div().id(id), theme)
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .h(CONTROL_LG)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .bg(theme.chip_bg)
                    .hover(|s| s.bg(theme.selected_bg))
                    .active(|s| s.opacity(0.8))
                    .text_size(TEXT_BASE)
                    .child(icon(glyph, ICON_MD, theme.icon))
                    .child(div().flex_1().child(label))
                    .children(caps)
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            };
        div()
            .id("bulk-summary")
            .role(gpui::Role::Region)
            .aria_label(format!("{count} conversations selected"))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .pt(SPACE_2XL * 2. + SPACE_SM)
            .px(SPACE_XL)
            .child(
                div()
                    .w_full()
                    .max_w(BULK_MAX)
                    .flex()
                    .flex_col()
                    .gap(SPACE_LG)
                    .child(
                        super::label(
                            "bulk-title",
                            match count {
                                1 => "1 conversation selected".to_string(),
                                n => format!("{n} conversations selected"),
                            },
                        )
                        .text_size(TEXT_TITLE2)
                        .line_height(LINE_UI)
                        .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SPACE_XS)
                            .text_size(TEXT_SM)
                            .text_color(theme.text_muted)
                            .children(subjects)
                            .when(count > 5, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .gap(SPACE_SM)
                                        .text_color(theme.text_faint)
                                        .child(div().flex_none().w(ICON_SM))
                                        .child(format!("and {} more", count - 5)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SPACE_XS)
                            .child(action(
                                "bulk-archive",
                                archive_icon,
                                archive_label,
                                Box::new(Archive),
                            ))
                            .child(action(
                                "bulk-delete",
                                Icon::Trash,
                                "Delete",
                                Box::new(Delete),
                            ))
                            .child(if all_unread {
                                action(
                                    "bulk-read",
                                    Icon::MailOpen,
                                    "Mark as Read",
                                    Box::new(MarkRead),
                                )
                            } else {
                                action(
                                    "bulk-unread",
                                    Icon::MailUnread,
                                    "Mark as Unread",
                                    Box::new(MarkUnread),
                                )
                            })
                            .child(action(
                                "bulk-star",
                                Icon::Star,
                                if all_starred { "Unstar" } else { "Star" },
                                Box::new(ToggleStar),
                            ))
                            .child(action(
                                "bulk-move",
                                Icon::FolderInput,
                                "Move to…",
                                Box::new(MoveTo),
                            ))
                            .child(action(
                                "bulk-label",
                                Icon::Tag,
                                "Label…",
                                Box::new(ApplyLabel),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .text_size(TEXT_SM)
                            .text_color(theme.text_faint)
                            .child(super::keys_caps("escape", theme))
                            .child("to clear the selection"),
                    ),
            )
            .into_any_element()
    }

    /// W6: a name or avatar that opens `contact`'s card (CON-5); not when
    /// the click ended selecting text. Right-click has its menu (MB-010).
    fn contact_link(
        &self,
        id: impl Into<ElementId>,
        contact: &Contact,
        name: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let menu_contact = contact.clone();
        let contact = contact.clone();
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(format!("Show contact: {name}, {}", contact.email))
            .rounded(RADIUS_SM)
            .hover(|s| s.text_color(theme.text))
            .on_any_mouse_down(move |event, window, cx| {
                if super::is_secondary(event) {
                    cx.stop_propagation();
                    name_menu(&menu_contact, event.position, window, cx);
                }
            })
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if TextSelection::selected_text(window, cx).is_empty() {
                        let at = (!event.is_keyboard()).then(|| event.position());
                        this.show_contact(contact.clone(), at, window, cx);
                    }
                }),
            )
    }

    fn contact_name<'a>(&self, contact: &'a Contact) -> &'a str {
        if self.data.is_me(contact) {
            "me"
        } else {
            contact.display_name()
        }
    }

    fn sender_avatar(&self, contact: &Contact, size: gpui::Rems, theme: &Theme) -> gpui::Div {
        avatar(
            contact.initials(),
            theme.avatar_color(&contact.email),
            size,
            theme,
        )
    }
}

/// An attachment's context menu (MB-010).
fn attachment_menu(
    message: usize,
    attachment: usize,
    position: gpui::Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    NativeMenu::new()
        .menu("Open", Box::new(OpenAttachmentAt(message, attachment)))
        .menu(
            "Quick Look",
            Box::new(QuickLookAttachmentAt(message, attachment)),
        )
        .separator()
        .menu("Save…", Box::new(SaveAttachmentAt(message, attachment)))
        .menu(
            "Show in Finder",
            Box::new(RevealAttachmentAt(message, attachment)),
        )
        .show(position, window, cx);
}

/// A sender's or recipient's context menu (MB-010).
fn name_menu(contact: &Contact, position: gpui::Point<Pixels>, window: &mut Window, cx: &mut App) {
    NativeMenu::new()
        .menu(
            "Copy Address",
            Box::new(html_view::CopyText(contact.email.clone().into())),
        )
        .menu(
            "New Message",
            Box::new(html_view::OpenLink(
                format!("mailto:{}", contact.email).into(),
            )),
        )
        .menu(
            "Show Contact",
            Box::new(ShowContactFor(contact.clone(), Some(position))),
        )
        .show(position, window, cx);
}

/// The message's HTML, rendered natively. Parsed once per message and cached.
fn render_body(
    doc: &Document,
    selection: html_view::Selection,
    images: html_view::ShowImages,
    fold: Option<html_view::Fold>,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .text_size(TEXT_MD)
        .line_height(LINE_READING)
        .text_color(theme.text)
        .child(html_view::render_with(
            doc,
            theme,
            Some(selection),
            Some(images),
            fold,
        ))
}

// Remote images (W10, ATT-1).

impl PigeonApp {
    /// A message's remote images: fetched (after this frame) once the user
    /// let them load, or always per the setting.
    fn message_images(
        &self,
        message: &Message,
        body: &Document,
        cx: &mut Context<Self>,
    ) -> ImageConsent {
        if !matches!(message.body, Body::Loaded { .. }) {
            return ImageConsent::default();
        }
        // W2 (ATT-2): inline images, part of the mail, need no consent.
        let inline: Vec<SharedString> = body
            .cid_images
            .iter()
            .filter(|url| !self.images.requested.contains(*url))
            .cloned()
            .collect();
        if !inline.is_empty()
            && let Some(thread) = self.selected.map(|ix| self.data.threads[ix].id.clone())
        {
            let this = cx.entity().downgrade();
            let message_id = message.id.clone();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| {
                    this.load_inline_images(&thread, &message_id, inline, cx)
                })
                .ok();
            });
        }
        if self.backend.read(cx).is_demo() {
            return ImageConsent::default();
        }
        let remote = &body.remote_images;
        let consent = image_consent(
            self.remote_images_setting(),
            self.images.allowed.contains(&message.id),
            remote.len(),
        );
        if !consent.show {
            return consent;
        }
        let missing: Vec<SharedString> = remote
            .iter()
            .filter(|url| !self.images.requested.contains(*url))
            .cloned()
            .collect();
        if !missing.is_empty() {
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.fetch_images(missing, cx))
                    .ok();
            });
        }
        consent
    }

    /// Let a message's remote images load.
    pub(crate) fn allow_images(&mut self, message_id: &str, cx: &mut Context<Self>) {
        if self.images.allowed.insert(message_id.to_string()) {
            cx.notify();
        }
    }

    fn fetch_images(&mut self, mut urls: Vec<SharedString>, cx: &mut Context<Self>) {
        urls.retain(|url| self.images.requested.insert(url.clone()));
        if urls.is_empty() {
            return;
        }
        let task = self.backend.update(cx, |b, cx| {
            b.fetch_images(urls.iter().map(|u| u.to_string()).collect(), cx)
        });
        cx.spawn(async move |this, cx| {
            let fetched = task.await;
            this.update(cx, |this, cx| this.images_ready(fetched, cx))
                .ok();
        })
        .detach();
    }

    /// W2 (ATT-2): a message's inline images, from the cache.
    fn load_inline_images(
        &mut self,
        thread_id: &str,
        message_id: &str,
        mut urls: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        urls.retain(|url| self.images.requested.insert(url.clone()));
        if urls.is_empty() {
            return;
        }
        let task = self.backend.update(cx, |b, cx| {
            b.inline_images(
                thread_id,
                message_id,
                urls.iter().map(|u| u.to_string()).collect(),
                cx,
            )
        });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            this.update(cx, |this, cx| this.images_ready(found, cx))
                .ok();
        })
        .detach();
    }

    fn images_ready(
        &mut self,
        images: Vec<(String, Option<crate::backend::images::FetchedImage>)>,
        cx: &mut Context<Self>,
    ) {
        for (url, image) in images {
            let format = image
                .as_ref()
                .and_then(|image| gpui::ImageFormat::from_mime_type(image.mime));
            let (Some(image), Some(format)) = (image, format) else {
                // Shown as failed, not loading forever (MA-075).
                self.images.failed.insert(url.into());
                continue;
            };
            self.images.ready.insert(
                url.into(),
                html_view::ReadyImage {
                    image: Arc::new(gpui::Image::from_bytes(format, image.bytes)),
                    width: image.width as f32,
                    height: image.height as f32,
                },
            );
        }
        cx.notify();
    }
}

/// Whether a message shows its remote images, and how many the "Load images"
/// bar says are hidden. Consent is per message: an image another message
/// already loaded stays hidden here until this one is allowed, so the count
/// is always true.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ImageConsent {
    show: bool,
    ask: Option<usize>,
}

fn image_consent(setting: RemoteImages, allowed: bool, remote: usize) -> ImageConsent {
    if remote == 0 {
        return ImageConsent::default();
    }
    let show = setting == RemoteImages::Always || allowed;
    ImageConsent {
        show,
        ask: (!show && setting == RemoteImages::Ask).then_some(remote),
    }
}

/// Above a message whose remote images wait for the user (ATT-1).
fn render_images_bar(
    ix: usize,
    message_id: &str,
    hidden: usize,
    theme: &Theme,
    cx: &mut Context<PigeonApp>,
) -> AnyElement {
    let id = message_id.to_string();
    div()
        .id(("images-bar", ix))
        .role(gpui::Role::Region)
        .aria_label("Remote images hidden")
        .mb(SPACE_MD)
        .px(SPACE_MD)
        .py(SPACE_SM)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .bg(theme.chip_bg)
        .text_size(TEXT_SM)
        .text_color(theme.text_muted)
        .child(icon(Icon::Image, ICON_MD, theme.text_faint))
        .child(div().flex_1().min_w_0().child(match hidden {
            1 => "1 remote image is hidden to protect your privacy".to_string(),
            n => format!("{n} remote images are hidden to protect your privacy"),
        }))
        .child(
            focusable_control(div().id(("images-load", ix)), theme)
                .role(gpui::Role::Button)
                .aria_label("Load images")
                .flex_none()
                .h(CONTROL_MD)
                .px(SPACE_MD)
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border)
                .hover(|s| s.bg(theme.selected_bg))
                .text_color(theme.text)
                .font_weight(FontWeight::MEDIUM)
                .child("Load images")
                .child(super::keys_caps("cmd-shift-i", theme))
                .on_click(cx.listener(move |this, _, _, cx| this.allow_images(&id, cx))),
        )
        .into_any_element()
}

// The reader's AI (W3, AI-6 to AI-16); what shows is decided in
// `app::ai_reader`.

/// AI-8, AI-9: the summary or catch-up under the subject, as it streams.
fn render_summary(view: &crate::app::ai_reader::SummaryView, theme: &Theme) -> AnyElement {
    let title = match view.catch_up {
        Some(1) => "Catch-up: 1 new message".to_string(),
        Some(n) => format!("Catch-up: {n} new messages"),
        None => "Summary".to_string(),
    };
    let (spoken, lines): (String, Vec<AnyElement>) = match &view.text {
        Some(text) => (
            text.replace("**", ""),
            text.lines()
                .map(|l| l.trim().replace("**", ""))
                .filter(|l| !l.is_empty())
                .map(|line| {
                    let bullet = ["- ", "* ", "• "]
                        .iter()
                        .find_map(|b| line.strip_prefix(b))
                        .map(str::to_string);
                    match bullet {
                        Some(point) => div()
                            .flex()
                            .gap(SPACE_SM)
                            .child(div().flex_none().text_color(theme.text_faint).child("•"))
                            .child(div().flex_1().min_w_0().child(point))
                            .into_any_element(),
                        None if line
                            .starts_with(crate::ai::reading::ASKED.trim_end_matches(':')) =>
                        {
                            div()
                                .mt(SPACE_XS)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(line)
                                .into_any_element()
                        }
                        None => div().child(line).into_any_element(),
                    }
                })
                .collect(),
        ),
        None => {
            let status = if view.failed {
                "Couldn't write a summary"
            } else {
                "Summarizing…"
            };
            (
                status.to_string(),
                vec![
                    super::live_text("ai-summary-status", status, gpui::accesskit::Live::Polite)
                        .text_color(theme.text_faint)
                        .into_any_element(),
                ],
            )
        }
    };
    super::motion::enter(
        "ai-summary-in",
        super::motion::Rise::None,
        div()
            .id("ai-summary")
            .role(gpui::Role::Region)
            .aria_label(format!("AI summary: {spoken}"))
            .w_full()
            .max_w(READER_MAX)
            .mt(SPACE_MD)
            .px(SPACE_MD)
            .py(SPACE_SM)
            .flex()
            .flex_col()
            .gap(SPACE_XS)
            .rounded(RADIUS_MD)
            .bg(theme.chip_bg)
            .text_size(TEXT_SM)
            .line_height(LINE_READING)
            .text_color(theme.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_XS)
                    .text_size(TEXT_XS)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(icon(Icon::TextQuote, ICON_SM, theme.accent))
                    .child(div().flex_1().child(title))
                    .child(super::keys_caps("shift-s", theme)),
            )
            .children(lines),
    )
    .into_any_element()
}

/// AI-10: what the message header says about its Clean View.
fn render_clean_label(
    ix: usize,
    message_id: &str,
    view: &crate::app::ai_reader::CleanView,
    theme: &Theme,
    cx: &mut Context<PigeonApp>,
) -> Option<AnyElement> {
    use crate::app::ai_reader::CleanView;
    let id = message_id.to_string();
    let chip = |label: &'static str, tip: &'static str, clean: bool, color| {
        focusable_control(div().id(("clean-label", ix)), theme)
            .role(gpui::Role::Button)
            .aria_label(tip)
            .flex_none()
            .px(SPACE_SM)
            .py(SPACE_2XS)
            .rounded(RADIUS_SM)
            .bg(theme.chip_bg)
            .hover(|s| s.bg(theme.hover_bg))
            .text_size(TEXT_XS)
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .child(label)
            .tooltip(move |window, cx| super::tooltip(format!("{tip} · ⇧V")).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.set_clean_choice(&id, clean, cx);
            }))
            .into_any_element()
    };
    match view {
        CleanView::Clean(_) => Some(chip(
            "Clean view",
            "AI rewrite of this newsletter. Show the original",
            false,
            theme.accent,
        )),
        CleanView::Original => Some(chip(
            "Original",
            "Show the clean view",
            true,
            theme.text_muted,
        )),
        CleanView::Failed => Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .child(
                    super::label(("clean-failed", ix), "Clean view failed")
                        .text_size(TEXT_XS)
                        .text_color(theme.text_faint),
                )
                .child(
                    push_button(("clean-retry", ix), "Retry", ButtonKind::Quiet, theme).on_click(
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.focus_message(ix, cx);
                            window.dispatch_action(Box::new(RetryCleanView), cx);
                        }),
                    ),
                )
                .into_any_element(),
        ),
        _ => None,
    }
}

/// AI-10: a Clean View made while the original showed.
fn render_clean_bar(
    ix: usize,
    message_id: &str,
    theme: &Theme,
    cx: &mut Context<PigeonApp>,
) -> AnyElement {
    let id = message_id.to_string();
    super::motion::enter(
        ("clean-bar-in", ix),
        super::motion::Rise::Down,
        div()
            .id(("clean-bar", ix))
            .role(gpui::Role::Region)
            .aria_label("Clean view ready")
            .mb(SPACE_MD)
            .px(SPACE_MD)
            .py(SPACE_SM)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .bg(theme.chip_bg)
            .text_size(TEXT_SM)
            .text_color(theme.text_muted)
            .child(icon(Icon::FileText, ICON_MD, theme.accent))
            .child(div().flex_1().min_w_0().child("Clean view ready"))
            .child(
                push_button(("clean-show", ix), "Show", ButtonKind::Plain, theme)
                    .child(super::keys_caps("shift-v", theme))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_clean_choice(&id, true, cx)),
                    ),
            ),
    )
    .into_any_element()
}

/// AI-16: a date in this message, offered as an event.
fn render_event_bar(ix: usize, candidate: &crate::ai::EventCandidate, theme: &Theme) -> AnyElement {
    let start = candidate.start.with_timezone(&Local);
    let when = if candidate.all_day {
        start.format("%A, %B %-d").to_string()
    } else {
        start.format("%A, %B %-d at %-I:%M %p").to_string()
    };
    let text = match &candidate.location {
        Some(place) if !place.trim().is_empty() => format!("{when} · {}", place.trim()),
        _ => when,
    };
    div()
        .id(("event-bar", ix))
        .role(gpui::Role::Region)
        .aria_label(format!("Event in this message: {text}"))
        .mb(SPACE_MD)
        .px(SPACE_MD)
        .py(SPACE_SM)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .bg(theme.chip_bg)
        .text_size(TEXT_SM)
        .text_color(theme.text_muted)
        .child(icon(Icon::Calendar, ICON_MD, theme.accent))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_color(theme.text)
                        .font_weight(FontWeight::MEDIUM)
                        .child(candidate.title.clone()),
                )
                .child(div().truncate().child(text)),
        )
        .child(
            push_button(
                ("event-add", ix),
                "Add to Calendar",
                ButtonKind::Plain,
                theme,
            )
            .child(super::keys_caps("shift-t", theme))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(AddToCalendar), cx)),
        )
        .into_any_element()
}

/// AI-11: three short replies; each opens a reply draft with its text.
fn render_quick_replies(replies: &[String], theme: &Theme) -> AnyElement {
    div()
        .id("quick-replies")
        .role(gpui::Role::Group)
        .aria_label("AI quick replies")
        .flex()
        .flex_wrap()
        .gap(SPACE_SM)
        .children(replies.iter().take(3).enumerate().map(|(n, reply)| {
            let tip: SharedString = reply.clone().into();
            push_button(("quick-reply", n), reply.clone(), ButtonKind::Plain, theme)
                .aria_label(format!("AI reply: {reply}"))
                // A long one wraps rather than running out of the card.
                .flex_shrink(1.)
                .h_auto()
                .min_h(CONTROL_MD)
                .py(SPACE_XS)
                .max_w(relative(1.))
                .child(super::keys_caps(&format!("alt-{}", n + 1), theme))
                .tooltip(move |window, cx| super::tooltip(tip.clone()).build(window, cx))
                .on_click(move |_, window, cx| window.dispatch_action(Box::new(QuickReply(n)), cx))
        }))
        .into_any_element()
}

// The original message (W2, RDR-8).

/// A message's source, read-only, in place of its conversation.
pub struct Original {
    pub thread_id: String,
    pub message_id: String,
    pub subject: String,
    pub state: OriginalState,
    pub focus: FocusHandle,
    pub scroll: UniformListScrollHandle,
}

pub enum OriginalState {
    Loading,
    Failed(SharedString),
    Loaded {
        /// The whole message, for Save as .eml.
        raw: Arc<Vec<u8>>,
        /// Its first `ORIGINAL_SHOWN` bytes as text, for Copy.
        shown: SharedString,
        lines: Arc<Vec<SharedString>>,
        truncated: bool,
    },
}

impl Original {
    pub fn new(thread_id: String, message_id: String, subject: String, cx: &mut gpui::App) -> Self {
        Self {
            thread_id,
            message_id,
            subject,
            state: OriginalState::Loading,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        }
    }
}

impl OriginalState {
    pub fn loaded(raw: Vec<u8>) -> Self {
        let limit = crate::backend::attachments::ORIGINAL_SHOWN;
        let truncated = raw.len() > limit;
        let shown = String::from_utf8_lossy(&raw[..raw.len().min(limit)]).into_owned();
        OriginalState::Loaded {
            lines: Arc::new(source_lines(&shown)),
            shown: shown.into(),
            raw: Arc::new(raw),
            truncated,
        }
    }
}

/// The source as lines to show: tabs as spaces, long lines broken every
/// `WRAP` characters.
fn source_lines(text: &str) -> Vec<SharedString> {
    const WRAP: usize = 100;
    let mut out = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r').replace('\t', "    ");
        if line.is_empty() {
            out.push(SharedString::default());
            continue;
        }
        let mut start = 0;
        for (n, (at, _)) in line.char_indices().enumerate() {
            if n > 0 && n % WRAP == 0 {
                out.push(line[start..at].to_string().into());
                start = at;
            }
        }
        out.push(line[start..].to_string().into());
    }
    if text.ends_with('\n') {
        out.pop();
    }
    out
}

fn render_original(original: &Original, theme: &Theme) -> AnyElement {
    let toolbar = window_drag_area(div().id("original-toolbar"))
        .h(super::HEADER_MIN)
        .min_h(HEADER_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .gap(SPACE_2XS)
        .pl(SPACE_XL)
        .pr(SPACE_MD)
        .child(
            super::label("original-title", "Original Message")
                .flex_none()
                .text_size(TEXT_LG)
                .font_weight(FontWeight::SEMIBOLD),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .pl(SPACE_SM)
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
                .child(original.subject.clone()),
        )
        .child(icon_button(
            "original-copy",
            Icon::Copy,
            "Copy",
            Box::new(CopyOriginal),
            theme,
        ))
        .child(icon_button(
            "original-save",
            Icon::Download,
            "Save as .eml",
            Box::new(SaveOriginal),
            theme,
        ))
        .child(icon_button(
            "original-close",
            Icon::X,
            "Close original",
            Box::new(CloseOriginal),
            theme,
        ));
    let note = |text: SharedString| {
        div()
            .px(SPACE_XL)
            .py(SPACE_SM)
            .text_size(TEXT_SM)
            .text_color(theme.text_faint)
            .child(text)
    };
    let body = match &original.state {
        OriginalState::Loading => note("Loading the original…".into()).into_any_element(),
        OriginalState::Failed(reason) => {
            note(format!("Couldn't load the original. {reason}").into()).into_any_element()
        }
        OriginalState::Loaded {
            lines, truncated, ..
        } => {
            let lines = lines.clone();
            let count = lines.len();
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .when(*truncated, |this| {
                    this.child(note(
                        "Showing the first 1 MB. Save as .eml for the whole message.".into(),
                    ))
                })
                .child(
                    div().flex_1().min_h_0().px(SPACE_XL).pb(SPACE_LG).child(
                        uniform_list("original-lines", count, move |range, _, _| {
                            range
                                .map(|ix| {
                                    div()
                                        .whitespace_nowrap()
                                        .truncate()
                                        .child(lines[ix].clone())
                                })
                                .collect::<Vec<_>>()
                        })
                        .track_scroll(&original.scroll)
                        .size_full(),
                    ),
                )
                .into_any_element()
        }
    };
    div()
        .id("original")
        .key_context(ORIGINAL)
        .track_focus(&original.focus)
        .role(gpui::Role::Region)
        .aria_label("Original message")
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(toolbar)
        .child(
            div()
                .id("original-source")
                .role(gpui::Role::Document)
                .aria_label("Message source")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .font_family(MONO)
                .text_size(TEXT_SM)
                .line_height(LINE_READING)
                .text_color(theme.text_muted)
                .child(body),
        )
        .into_any_element()
}

/// In place of a body still on its way (RDR-6): pulsing lines, like the
/// list's (ME-030).
fn render_body_loading(theme: &Theme) -> AnyElement {
    let bar = |width: f32| {
        div()
            .w(relative(width))
            .h(SPACE_MD)
            .rounded(RADIUS_SM)
            .bg(theme.chip_bg)
    };
    // MA-066: only once the body takes a moment, so a fast one never
    // flashes it; it pulses at 30 frames a second.
    super::motion::after_loading_delay(
        "message-loading-in",
        div()
            .child(
                div()
                    .id("message-loading")
                    .role(gpui::Role::Status)
                    .aria_label("Loading message")
                    .flex()
                    .flex_col()
                    .gap(SPACE_SM)
                    .child(bar(0.9))
                    .child(bar(0.75))
                    .child(bar(0.82))
                    .child(bar(0.4))
                    .with_animation(
                        "message-loading",
                        Animation::new(std::time::Duration::from_millis(1200))
                            .repeat()
                            .with_max_fps(30.)
                            .with_easing(gpui::pulsating_between(0.5, 1.)),
                        |this, delta| this.opacity(delta),
                    ),
            )
            // A slow one says what it's waiting for.
            .child(super::motion::after_delay(
                "message-downloading",
                BODY_SLOW,
                div()
                    .mt(SPACE_MD)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child("Downloading message…"),
            )),
    )
    .into_any_element()
}

/// A body that didn't load, with why and a way to try again.
fn render_body_failed(ix: usize, reason: &str, theme: &Theme) -> AnyElement {
    div()
        .id(("message-failed", ix))
        .role(gpui::Role::Alert)
        .aria_label(format!("Couldn't load this message. {reason}"))
        .flex()
        .flex_col()
        .items_start()
        .gap(SPACE_SM)
        .child(
            div()
                .text_size(TEXT_BASE)
                .text_color(theme.text_muted)
                .child("Couldn't load this message"),
        )
        .child(
            // Selectable, to copy into a search or a report (MB-035).
            div().text_size(TEXT_SM).text_color(theme.text_faint).child(
                Selectable::plain(
                    ("message-failed-reason", ix),
                    reason.to_string(),
                    theme.text_selection,
                )
                .silent(),
            ),
        )
        .child(
            focusable_control(div().id(("message-retry", ix)), theme)
                .role(gpui::Role::Button)
                .aria_label("Retry loading message")
                .h(CONTROL_MD)
                .px(SPACE_MD)
                .flex()
                .items_center()
                .rounded(RADIUS_MD)
                .bg(theme.chip_bg)
                .hover(|s| s.bg(theme.selected_bg))
                .text_size(TEXT_SM)
                .child("Retry")
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(RetryLoadingMessage), cx)
                }),
        )
        .into_any_element()
}

/// The toolbar's "…": the rest of the thread actions, in a menu.
fn more_button(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    focusable_control(div().id("more-actions"), theme)
        .aria_label("More actions")
        .role(gpui::Role::Button)
        .size(HIT_TARGET)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(icon(Icon::Ellipsis, ICON_TOOLBAR, theme.icon))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(|window, cx| super::tooltip("Move, label and more").build(window, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_threads_fold_their_middle() {
        use Slot::*;
        assert_eq!(fold_slots(4, |_| false), [Show; 4]);
        assert_eq!(
            fold_slots(7, |_| false),
            [Show, Fold(4), Hidden, Hidden, Hidden, Show, Show]
        );
        // An expanded message splits the fold; a run of one stays shown.
        assert_eq!(
            fold_slots(8, |ix| ix == 3),
            [Show, Fold(2), Hidden, Show, Fold(2), Hidden, Show, Show]
        );
        assert_eq!(
            fold_slots(6, |ix| ix == 2),
            [Show, Show, Show, Show, Show, Show]
        );
    }

    #[test]
    fn ages_under_a_day() {
        let now = Local::now();
        let ago = |minutes| (now - chrono::Duration::minutes(minutes)).fixed_offset();
        assert_eq!(relative_age(&ago(0), now).as_deref(), Some("just now"));
        assert_eq!(relative_age(&ago(5), now).as_deref(), Some("5 min ago"));
        assert_eq!(relative_age(&ago(150), now).as_deref(), Some("2 hr ago"));
        assert_eq!(relative_age(&ago(60 * 25), now), None);
    }

    #[test]
    fn source_lines_wrap_by_character() {
        let long = "é".repeat(150);
        let lines = source_lines(&format!("a\tb\r\n\n{long}\n"));
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].as_ref(), "a    b");
        assert_eq!(lines[1].as_ref(), "");
        assert_eq!(lines[2].chars().count(), 100);
        assert_eq!(lines[3].chars().count(), 50);
    }

    #[test]
    fn images_show_only_with_consent_for_this_message() {
        let ask = image_consent(RemoteImages::Ask, false, 5);
        assert_eq!(
            ask,
            ImageConsent {
                show: false,
                ask: Some(5)
            }
        );
        let allowed = image_consent(RemoteImages::Ask, true, 5);
        assert_eq!(
            allowed,
            ImageConsent {
                show: true,
                ask: None
            }
        );
        let always = image_consent(RemoteImages::Always, false, 5);
        assert_eq!(
            always,
            ImageConsent {
                show: true,
                ask: None
            }
        );
        let never = image_consent(RemoteImages::Never, false, 5);
        assert_eq!(never, ImageConsent::default());
        assert_eq!(
            image_consent(RemoteImages::Ask, false, 0),
            ImageConsent::default()
        );
    }
}

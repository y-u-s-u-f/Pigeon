//! The menu bar panel (sprint 2 W7b, NTF-4), design A "Agenda first"
//! (`docs/design/menubar.html`): the date and sync status, a card for the
//! next meeting, the rest of today, recent Primary mail, and a footer.
//!
//! Keys (context `MenuBar`, bound in `actions.rs`): ↑/↓ move through the
//! card, events and mail, Enter opens the selected one in the main window,
//! `j` joins the next meeting, `e` archives and ⇧I marks read the selected
//! mail (Undo with `z`), `c` composes, `n` makes an event, ⌘O opens
//! Pigeon, ⌘, opens Settings, Esc closes. The gear in the footer has
//! Settings… and Quit Pigeon (ME-072). The window is shown, hidden and
//! sized by `app::menu_bar`; this view only draws and asks.

use std::{cell::Cell, rc::Rc, time::Instant};

use chrono::{Duration, NaiveDateTime};
use gpui::{
    AnyElement, App, Context, Div, Entity, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Pixels, Render, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Task, Window, actions,
    component::scroll::ScrollableElement as _, div, prelude::FluentBuilder as _, px,
};

use super::{
    controls::{dropdown, menu_check, menu_row},
    icon, keycap_on_accent, keys_caps,
    motion::{self, Rise},
    style::*,
    tooltip,
};
use crate::{
    app::{
        menu_bar::{self, HideReason},
        shell::Show,
        today::{self, Agenda, MailRow, Today},
    },
    assets::Icon,
    backend::{Backend, BackendEvent, MailOp, OpId, SyncStatus, UNDO_WINDOW, gmail},
    calendar::{CalendarData, Occurrence, policy, time},
    data::{Folder, list_date},
    settings,
    theme::Theme,
    ui::status,
};

actions!(
    menu_bar,
    [
        SelectNext,
        SelectPrev,
        OpenSelected,
        Join,
        Archive,
        MarkRead,
        Undo,
        Compose,
        NewEvent,
        OpenPigeon,
        Close,
    ]
);

/// Key context of the panel.
pub const MENU_BAR: &str = "MenuBar";

pub const PANEL_WIDTH: Pixels = px(352.);
/// The panel grows with what it shows, up to this; the lists scroll past it.
pub const PANEL_MAX_HEIGHT: Pixels = px(640.);
/// Its height before the first measure.
pub const PANEL_START_HEIGHT: Pixels = px(420.);
/// Rows of "Later today" and of recent mail, at most.
const MAX_EVENTS: usize = 6;
const MAX_MAIL: usize = 4;
const TIME_COLUMN: gpui::Rems = type_size(56.);
const COLOR_BAR: Pixels = px(3.);
const UNREAD_DOT: Pixels = px(6.);
const GEAR_MENU_WIDTH: gpui::Rems = type_size(200.);

/// What ↑/↓ move through.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    Next(Occurrence),
    Event(Occurrence),
    Mail(String),
}

/// An archive from the panel, until its undo window closes.
struct PanelUndo {
    thread: String,
    subject: String,
    ops: Vec<OpId>,
}

pub struct MenuBarPanel {
    today: Entity<Today>,
    focus: FocusHandle,
    selected: usize,
    /// Said once at the top, until the panel closes ("No meeting …").
    notice: Option<SharedString>,
    undo: Option<PanelUndo>,
    /// The notice and undo bar as last drawn, kept while they fade out
    /// (MD-032).
    notice_shown: Option<SharedString>,
    undo_shown: Option<String>,
    /// Set on show: the first render after logs how long it took.
    opening: Option<(Instant, &'static str)>,
    scroll: ScrollHandle,
    reveal: bool,
    /// The content's height as last laid out, to size the window to it.
    height: Rc<Cell<f32>>,
    /// Today's agenda and what ↑/↓ move through, as last drawn (MC-042).
    agenda: Agenda,
    items: Vec<Item>,
    /// The gear's menu is open.
    gear_open: bool,
    _undo_timer: Option<Task<()>>,
}

impl MenuBarPanel {
    pub fn new(today: Entity<Today>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&today, |_, _, cx| cx.notify()).detach();
        cx.observe_window_activation(window, |_, window, _| {
            if !window.is_window_active() {
                menu_bar::hide_later(HideReason::LostKey);
            }
        })
        .detach();
        // The system's light or dark, or the user's choice, with the main
        // window closed too.
        cx.observe_window_appearance(window, |_, window, cx| {
            sync_theme(window, cx);
            cx.notify();
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            today,
            focus,
            selected: 0,
            notice: None,
            undo: None,
            notice_shown: None,
            undo_shown: None,
            opening: None,
            scroll: ScrollHandle::new(),
            reveal: false,
            height: Rc::new(Cell::new(f32::from(PANEL_START_HEIGHT))),
            agenda: Agenda::default(),
            items: Vec::new(),
            gear_open: false,
            _undo_timer: None,
        }
    }

    /// About to show: start on the first item, at the app's text size.
    pub fn will_show(
        &mut self,
        source: &'static str,
        notice: Option<SharedString>,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.opening = Some((started, source));
        self.notice = notice;
        self.selected = 0;
        self.reveal = true;
        self.gear_open = false;
        window.set_rem_size(px(BASE_REM * settings::get(cx).text_scale));
        sync_theme(window, cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The content's height in points, for the window.
    pub fn content_height(&self) -> f64 {
        self.height.get() as f64
    }

    fn items(agenda: &Agenda, mail: &[MailRow]) -> Vec<Item> {
        let mut items = Vec::new();
        items.extend(agenda.next.map(Item::Next));
        items.extend(
            agenda
                .later
                .iter()
                .take(MAX_EVENTS)
                .chain(agenda.tomorrow.iter())
                .map(|o| Item::Event(*o)),
        );
        items.extend(mail.iter().take(MAX_MAIL).map(|m| Item::Mail(m.id.clone())));
        items
    }

    fn selected_item(&self) -> Option<Item> {
        self.items
            .get(self.selected.min(self.items.len().saturating_sub(1)))
            .cloned()
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() - 1;
        self.selected = if forward {
            (self.selected + 1).min(last)
        } else {
            self.selected.min(last).saturating_sub(1)
        };
        self.reveal = true;
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn open_selected(&mut self, _: &OpenSelected, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.selected_item() {
            self.open_item(&item, cx);
        }
    }

    fn open_item(&self, item: &Item, cx: &App) {
        let show = match item {
            Item::Next(o) | Item::Event(o) => event_show(&self.today.read(cx).calendar, o),
            Item::Mail(id) => Some(Show::Thread(id.clone())),
        };
        if let Some(show) = show {
            menu_bar::open_in_main(show);
        }
    }

    fn join(&mut self, _: &Join, _: &mut Window, cx: &mut Context<Self>) {
        let today = self.today.read(cx);
        let url = self
            .agenda
            .next
            .and_then(|o| policy::video_link(&today.calendar.events[o.event]).map(str::to_string));
        if let Some(url) = url {
            menu_bar::hide_later(HideReason::Opened);
            menu_bar::open_link(&url, cx);
        }
    }

    fn selected_mail(&self, cx: &App) -> Option<MailRow> {
        let Some(Item::Mail(id)) = self.selected_item() else {
            return None;
        };
        self.today
            .read(cx)
            .mail
            .iter()
            .find(|m| m.id == id)
            .cloned()
    }

    /// `e`: out of the Inbox, as the main app's Archive, with its undo.
    fn archive(&mut self, _: &Archive, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_mail(cx) else {
            return;
        };
        let backend = self.today.read(cx).backend().clone();
        let ops = archive_thread(&backend, &row.id, cx);
        self.today.update(cx, |t, _| {
            t.update_mail(&row.id, |rows, ix| {
                rows.remove(ix);
            })
        });
        self.undo = Some(PanelUndo {
            thread: row.id,
            subject: row.subject,
            ops,
        });
        self._undo_timer = Some(cx.spawn(async |this, cx| {
            cx.background_executor().timer(UNDO_WINDOW).await;
            this.update(cx, |this, cx| {
                this.undo = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// `z`: put the archived conversation back. Inside the undo window
    /// nothing went; past it, it moves back at once.
    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.take() else {
            return;
        };
        let backend = self.today.read(cx).backend().clone();
        backend.update(cx, |b, cx| {
            let cancelled: Vec<bool> = undo.ops.iter().map(|&op| b.cancel(op)).collect();
            if !cancelled.into_iter().all(|c| c) {
                for op in MailOp::moves(vec![undo.thread.clone()], Folder::Archive, Folder::Inbox) {
                    b.mail_now(op, cx);
                }
            }
            cx.emit(BackendEvent::ThreadsChanged(vec![undo.thread.clone()]));
        });
        self._undo_timer = None;
        cx.notify();
    }

    /// ⇧I: read, as the main app's Mark as Read.
    fn mark_read(&mut self, _: &MarkRead, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_mail(cx).filter(|r| r.unread) else {
            return;
        };
        let backend = self.today.read(cx).backend().clone();
        mark_thread_read(&backend, &row.id, cx);
        self.today.update(cx, |t, _| {
            t.update_mail(&row.id, |rows, ix| rows[ix].unread = false)
        });
        cx.notify();
    }

    fn compose(&mut self, _: &Compose, _: &mut Window, _: &mut Context<Self>) {
        menu_bar::open_in_main(Show::Compose(None));
    }

    fn new_event(&mut self, _: &NewEvent, _: &mut Window, _: &mut Context<Self>) {
        menu_bar::open_in_main(Show::NewEvent);
    }

    fn open_pigeon(&mut self, _: &OpenPigeon, _: &mut Window, _: &mut Context<Self>) {
        menu_bar::open_in_main(Show::Main);
    }

    /// Esc closes the gear's menu first.
    fn close(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.gear_open) {
            return cx.notify();
        }
        menu_bar::hide_later(HideReason::Escape);
    }

    /// Say something at the top while it shows.
    pub fn say(&mut self, notice: SharedString, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
    }

    /// Hidden: what was said once goes.
    pub fn did_hide(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }
}

/// The theme for the user's choice, or this window's appearance.
fn sync_theme(window: &mut Window, cx: &mut App) {
    Theme::sync_for(settings::get(cx).appearance, window, cx);
}

/// Archive conversation `id` as the main app's Archive does, queued, so it
/// can be undone; the ops, to cancel. For the panel and mail notices
/// (MB-050), with or without the main window.
pub(crate) fn archive_thread(backend: &Entity<Backend>, id: &str, cx: &mut App) -> Vec<OpId> {
    let ids = vec![id.to_string()];
    backend.update(cx, |b, cx| {
        let ops = MailOp::moves(ids.clone(), Folder::Inbox, Folder::Archive)
            .into_iter()
            .map(|op| b.mail(op, cx))
            .collect();
        // The main window and the badge read it back, queued.
        cx.emit(BackendEvent::ThreadsChanged(ids));
        ops
    })
}

/// Mark conversation `id` read, as the main app's Mark as Read does.
pub(crate) fn mark_thread_read(backend: &Entity<Backend>, id: &str, cx: &mut App) {
    backend.update(cx, |b, cx| {
        b.mail(
            MailOp::Modify {
                threads: vec![id.to_string()],
                add: Vec::new(),
                remove: vec![gmail::UNREAD.to_string()],
            },
            cx,
        );
        cx.emit(BackendEvent::ThreadsChanged(vec![id.to_string()]));
    });
}

fn event_show(data: &CalendarData, o: &Occurrence) -> Option<Show> {
    let event = data.events.get(o.event)?;
    Some(Show::Event {
        calendar: event.calendar.clone(),
        id: event.id.clone(),
        start: o.start,
    })
}

/// "in 12 min", "Now · 25 min left", "Tomorrow".
fn countdown_chip(o: &Occurrence, now: NaiveDateTime) -> String {
    let minutes = |to: NaiveDateTime| {
        let m = (to - now).num_minutes().max(1);
        time::duration_label(Duration::minutes(m))
    };
    if o.start <= now {
        format!("Now · {} left", minutes(o.end))
    } else if o.start.date() == now.date() {
        format!("in {}", minutes(o.start))
    } else {
        "Tomorrow".into()
    }
}

/// "Priya, Sam and 2 others": the guests but the user, by first name.
fn guests_line(data: &CalendarData, o: &Occurrence) -> Option<String> {
    let event = &data.events[o.event];
    let names: Vec<String> = event
        .attendees
        .iter()
        .filter(|a| !data.account.is_me(&a.email))
        .map(|a| {
            let name = if a.name.trim().is_empty() {
                a.email.split('@').next().unwrap_or_default()
            } else {
                a.name.split_whitespace().next().unwrap_or_default()
            };
            name.to_string()
        })
        .collect();
    match names.as_slice() {
        [] => None,
        [a] => Some(a.clone()),
        [a, b] => Some(format!("{a} and {b}")),
        [a, b, c] => Some(format!("{a}, {b} and {c}")),
        [a, b, rest @ ..] => Some(format!("{a}, {b} and {} others", rest.len())),
    }
}

impl Render for MenuBarPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((started, source)) = self.opening.take() {
            crate::logging::info(&format!(
                "menu-bar: shown in {:.1} ms ({source})",
                started.elapsed().as_secs_f64() * 1000.
            ));
        }
        // The bars stay drawn while they fade out, the window at its old
        // height until they're gone; it then shrinks at once.
        let notice_p = motion::presence("menu-bar-notice", self.notice.is_some(), window, cx);
        if let Some(notice) = &self.notice {
            self.notice_shown = Some(notice.clone());
        } else if notice_p <= 0. {
            self.notice_shown = None;
        }
        let undo_p = motion::presence("menu-bar-undo", self.undo.is_some(), window, cx);
        if let Some(undo) = &self.undo {
            self.undo_shown = Some(format!("Archived “{}”", today::truncate(&undo.subject, 32)));
        } else if undo_p <= 0. {
            self.undo_shown = None;
        }
        let rise = if cx.reduce_motion() {
            Rise::None
        } else {
            Rise::Up
        };

        let theme = Theme::of(cx).clone();
        let now = today::now();
        let today_entity = self.today.clone();
        let today = today_entity.read(cx);
        let agenda = today::agenda(&today.calendar, now);
        let items = Self::items(&agenda, &today.mail);
        let sync = today.backend().read(cx).status().clone();
        let status = status::label(&sync, status::now_ms());
        let header = now.format("%A, %b %-d").to_string();
        let mail = &today.mail;
        let data = &today.calendar;
        self.selected = self.selected.min(items.len().saturating_sub(1));
        let reveal = std::mem::take(&mut self.reveal);
        let selected = items.get(self.selected).cloned();
        self.agenda = agenda.clone();
        self.items = items;
        let is_selected = |item: &Item| selected.as_ref() == Some(item);

        // The scrolling middle: one flat list, so the selection can be
        // scrolled to by child index.
        let mut middle: Vec<AnyElement> = Vec::new();
        let mut reveal_ix = None;
        let mut push_row = |middle: &mut Vec<AnyElement>, item: &Item, row: AnyElement| {
            if is_selected(item) {
                reveal_ix = Some(middle.len());
            }
            middle.push(row);
        };

        let later: Vec<&Occurrence> = agenda.later.iter().take(MAX_EVENTS).collect();
        let more = agenda.later.len().saturating_sub(MAX_EVENTS);
        let count = agenda.later.len();
        if !later.is_empty() || agenda.next.is_some() || agenda.tomorrow.is_none() {
            middle.push(section_header(
                "later-today",
                "Later today",
                (count > 0).then(|| status::plural(count, "event", "events").into_any_element()),
                &theme,
            ));
        }
        for o in &later {
            let item = Item::Event(**o);
            let row = self.event_row(data, o, now, is_selected(&item), &theme, cx);
            push_row(&mut middle, &item, row);
        }
        if more > 0 {
            middle.push(quiet_line(
                "more-events",
                format!("{more} more today"),
                &theme,
            ));
        }
        if later.is_empty() && agenda.tomorrow.is_none() {
            let text = if agenda.next.is_some() {
                "Nothing else today"
            } else {
                "Nothing else today or tomorrow"
            };
            middle.push(quiet_line("nothing-today", text.into(), &theme));
        }
        if let Some(o) = &agenda.tomorrow {
            middle.push(section_header("tomorrow", "Tomorrow", None, &theme));
            let item = Item::Event(*o);
            let row = self.event_row(data, o, now, is_selected(&item), &theme, cx);
            push_row(&mut middle, &item, row);
        }

        let unread = mail.iter().filter(|m| m.unread).count();
        let mail_selected = matches!(selected, Some(Item::Mail(_)));
        middle.push(section_header(
            "recent-mail",
            "Recent mail",
            if mail_selected {
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_XS)
                        .child("Archive")
                        .child(keys_caps("e", &theme))
                        .child(div().w(SPACE_2XS))
                        .child("Read")
                        .child(keys_caps("shift-i", &theme))
                        .into_any_element(),
                )
            } else {
                (unread > 0).then(|| format!("{unread} unread").into_any_element())
            },
            &theme,
        ));
        if mail.is_empty() {
            middle.push(quiet_line("no-mail", "Nothing in Primary".into(), &theme));
        }
        for row in mail.iter().take(MAX_MAIL) {
            let item = Item::Mail(row.id.clone());
            let el = self.mail_row(row, is_selected(&item), &theme, cx);
            push_row(&mut middle, &item, el);
        }
        middle.push(div().h(SPACE_XS).into_any_element());
        if reveal {
            match reveal_ix {
                Some(ix) => self.scroll.scroll_to_item(ix),
                None => self.scroll.scroll_to_top_of_item(0),
            }
        }

        let height = self.height.clone();
        div()
            .on_children_prepainted(move |bounds, window, _| {
                let Some(content) = bounds.first() else {
                    return;
                };
                let h = f32::from(content.size.height).ceil();
                height.set(h);
                if (h - f32::from(window.viewport_size().height)).abs() > 0.5 {
                    menu_bar::resize_later(h as f64);
                }
            })
            .id("menu-bar-panel")
            .role(gpui::Role::Dialog)
            .aria_label("Pigeon: today and recent mail")
            .track_focus(&self.focus)
            .key_context(MENU_BAR)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::join))
            .on_action(cx.listener(Self::archive))
            .on_action(cx.listener(Self::mark_read))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::compose))
            .on_action(cx.listener(Self::new_event))
            .on_action(cx.listener(Self::open_pigeon))
            .on_action(cx.listener(Self::close))
            .size_full()
            .bg(theme.menu_bar_bg)
            .text_color(theme.text)
            .text_size(TEXT_BASE)
            .line_height(LINE_UI)
            .child(
                div()
                    .w_full()
                    .max_h(PANEL_MAX_HEIGHT)
                    .flex()
                    .flex_col()
                    .child(self.header(header, status, &sync, &theme))
                    .when_some(self.notice_shown.clone(), |el, notice| {
                        el.child(motion::card_motion(
                            div()
                                .id("menu-bar-notice")
                                .role(gpui::Role::Status)
                                .aria_label(notice.clone())
                                .flex_none()
                                .mx(SPACE_SM)
                                .mt(SPACE_SM)
                                .px(SPACE_MD)
                                .py(SPACE_SM)
                                .rounded(RADIUS_MD)
                                .bg(theme.chip_bg)
                                .text_size(TEXT_SM)
                                .text_color(theme.text_muted)
                                .child(notice),
                            notice_p,
                            rise,
                        ))
                    })
                    .when_some(agenda.next, |el, o| {
                        let selected = is_selected(&Item::Next(o));
                        el.child(self.card(data, &o, now, selected, &theme, cx))
                    })
                    .child(
                        // The scroll bar beside the lists, not in them
                        // (MB-068).
                        div()
                            .relative()
                            .min_h(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .id("menu-bar-lists")
                                    // As tall as its rows; it gives way, and
                                    // scrolls, once the panel is at its maximum.
                                    .min_h(px(0.))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .children(middle),
                            )
                            .vertical_scrollbar(&self.scroll),
                    )
                    .when_some(self.undo_shown.clone(), |el, message| {
                        el.child(motion::card_motion(
                            self.undo_bar(message, &theme, cx),
                            undo_p,
                            rise,
                        ))
                    })
                    .when(self.gear_open, |el| el.child(self.gear_menu(&theme)))
                    .child(self.footer(&theme, cx)),
            )
    }
}

impl MenuBarPanel {
    fn header(
        &self,
        date: String,
        status: String,
        sync: &SyncStatus,
        theme: &Theme,
    ) -> impl IntoElement {
        // Offline needs attention, and syncing is under way: a dot and
        // brighter text set them apart from "Synced" (ME-057).
        let dot = match sync {
            SyncStatus::Offline { .. } => Some(theme.warning),
            SyncStatus::Syncing { .. } => Some(theme.accent),
            _ => None,
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(SPACE_SM)
            .px(SPACE_LG)
            .pt(SPACE_MD)
            .pb(SPACE_XS)
            .child(
                div()
                    .id("menu-bar-date")
                    .role(gpui::Role::Heading)
                    .aria_label(SharedString::from(date.clone()))
                    .text_size(TEXT_LG)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(date),
            )
            .child(
                div()
                    .id("menu-bar-sync")
                    .role(gpui::Role::Status)
                    .aria_label(SharedString::from(status.clone()))
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(SPACE_XS + SPACE_2XS)
                    .text_size(TEXT_XS)
                    .text_color(if dot.is_some() {
                        theme.text_muted
                    } else {
                        theme.text_faint
                    })
                    .children(
                        dot.map(|color| {
                            div().flex_none().size(UNREAD_DOT).rounded_full().bg(color)
                        }),
                    )
                    .child(div().min_w_0().truncate().child(status)),
            )
    }

    fn card(
        &self,
        data: &CalendarData,
        o: &Occurrence,
        now: NaiveDateTime,
        selected: bool,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let event = &data.events[o.event];
        let title = event.display_title().to_string();
        let chip = countdown_chip(o, now);
        let range = time::time_range_label(o.start.time(), o.end.time());
        let service = policy::video_link(event).map(today::video_service);
        let subline = [service.map(str::to_string), guests_line(data, o)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        let item = Item::Next(*o);
        let open = item.clone();
        div()
            .id("menu-bar-next")
            .role(gpui::Role::Group)
            .aria_label(SharedString::from(format!(
                "Next meeting: {title}, {chip}, {range}"
            )))
            .aria_selected(selected)
            .flex_none()
            .mx(SPACE_SM)
            .mt(SPACE_SM)
            .mb(SPACE_XS)
            .p(SPACE_MD)
            .rounded(RADIUS_LG)
            // The keyboard's item as in every overlay (ME-080).
            .bg(if selected {
                theme.selected_bg
            } else {
                theme.menu_bar_card
            })
            .border_1()
            .border_color(theme.border)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = 0;
                this.open_item(&open, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(SPACE_SM)
                    .child(
                        div()
                            .px(SPACE_SM)
                            .py(SPACE_2XS)
                            .rounded(RADIUS_SM)
                            .bg(theme.chip_bg)
                            .text_size(TEXT_XS)
                            .font_weight(FontWeight::MEDIUM)
                            .font_features(tabular_figures())
                            .text_color(theme.accent)
                            .child(chip),
                    )
                    .child(
                        div()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_faint)
                            .font_features(tabular_figures())
                            .child(range),
                    ),
            )
            .child(
                div()
                    .mt(SPACE_SM)
                    .text_size(TEXT_LG)
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title.clone()),
            )
            .when(!subline.is_empty(), |el| {
                el.child(
                    div()
                        .mt(SPACE_2XS)
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .truncate()
                        .child(subline),
                )
            })
            .child(
                div()
                    .mt(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .when(service.is_some(), |el| {
                        el.child(
                            div()
                                .id("menu-bar-join")
                                .role(gpui::Role::Button)
                                .aria_label(SharedString::from(format!("Join {title}")))
                                .h(CONTROL_MD)
                                .px(SPACE_MD)
                                .flex()
                                .items_center()
                                .gap(SPACE_SM)
                                .rounded(RADIUS_MD)
                                .bg(theme.accent)
                                .text_color(theme.accent_text)
                                .text_size(TEXT_SM)
                                .font_weight(FontWeight::SEMIBOLD)
                                .hover(|s| s.opacity(0.9))
                                .active(|s| s.opacity(0.8))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.join(&Join, window, cx);
                                }))
                                .child("Join Meeting")
                                .child(keycap_on_accent("J", theme)),
                        )
                    })
                    .child(
                        div()
                            .id("menu-bar-details")
                            .role(gpui::Role::Button)
                            .aria_label("Open in Calendar")
                            .h(CONTROL_MD)
                            .px(SPACE_MD)
                            .flex()
                            .items_center()
                            .gap(SPACE_SM)
                            .rounded(RADIUS_MD)
                            .bg(theme.chip_bg)
                            .hover(|s| s.bg(theme.selected_bg))
                            .active(|s| s.bg(theme.selected_bg).opacity(0.8))
                            .text_size(TEXT_SM)
                            .font_weight(FontWeight::SEMIBOLD)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_item(&item, cx);
                            }))
                            .child("Details")
                            .when(selected, |el| el.child(keys_caps("enter", theme))),
                    ),
            )
    }

    fn event_row(
        &self,
        data: &CalendarData,
        o: &Occurrence,
        now: NaiveDateTime,
        selected: bool,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let event = &data.events[o.event];
        let title = event.display_title().to_string();
        let when = if event.all_day {
            "All day".to_string()
        } else if o.start <= now {
            "Now".to_string()
        } else {
            time::compact_time_label(o.start.time())
        };
        let place = event.location.trim().to_string();
        let item = Item::Event(*o);
        let open = item.clone();
        let label = [when.as_str(), title.as_str(), place.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        row_shell(
            (
                "menu-bar-event",
                o.event as u64 ^ o.start.and_utc().timestamp() as u64,
            ),
            label,
            selected,
            theme,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.select(&open, cx);
            this.open_item(&open, cx);
        }))
        .child(
            div()
                .w(TIME_COLUMN)
                .flex_none()
                .text_size(TEXT_SM)
                .text_color(theme.text_muted)
                .text_right()
                .font_features(tabular_figures())
                .child(when),
        )
        .child(
            div()
                .w(COLOR_BAR)
                .flex_none()
                .self_stretch()
                .rounded(RADIUS_SM)
                .bg(theme.event_accent(data.color_of(event), false)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(title),
                )
                .when(!place.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(TEXT_SM)
                            .text_color(theme.text_faint)
                            .truncate()
                            .child(place),
                    )
                }),
        )
        .when(selected, |el| el.child(keys_caps("enter", theme)))
        .into_any_element()
    }

    fn mail_row(
        &self,
        row: &MailRow,
        selected: bool,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let item = Item::Mail(row.id.clone());
        let time = list_date(&row.date);
        let label = format!(
            "{}{}, {}, {time}",
            if row.unread { "Unread, " } else { "" },
            row.sender,
            row.subject
        );
        row_shell(
            SharedString::from(format!("menu-bar-mail-{}", row.id)),
            label,
            selected,
            theme,
        )
        .items_start()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.select(&item, cx);
            this.open_item(&item, cx);
        }))
        .child(
            div()
                .mt(SPACE_XS)
                .size(UNREAD_DOT)
                .flex_none()
                .rounded_full()
                .when(row.unread, |el| el.bg(theme.accent)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .map(|el| {
                                    if row.unread {
                                        el.font_weight(FontWeight::SEMIBOLD)
                                    } else {
                                        el.font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.text_muted)
                                    }
                                })
                                .child(row.sender.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(TEXT_XS)
                                .text_color(theme.text_faint)
                                .font_features(tabular_figures())
                                .child(time),
                        ),
                )
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .truncate()
                        .child(row.subject.clone()),
                ),
        )
        .into_any_element()
    }

    fn select(&mut self, item: &Item, cx: &mut Context<Self>) {
        if let Some(ix) = self.items.iter().position(|i| i == item) {
            self.selected = ix;
            cx.notify();
        }
    }

    /// While it fades out, its Undo does nothing: `self.undo` is gone.
    fn undo_bar(&self, message: String, theme: &Theme, cx: &Context<Self>) -> Stateful<Div> {
        div()
            .id("menu-bar-undo")
            .role(gpui::Role::Status)
            .aria_label(SharedString::from(message.clone()))
            .flex_none()
            .mx(SPACE_SM)
            .mb(SPACE_SM)
            .pl(SPACE_MD)
            .pr(SPACE_XS)
            .py(SPACE_2XS)
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .rounded(RADIUS_MD)
            .bg(theme.chip_bg)
            .text_size(TEXT_SM)
            .child(div().flex_1().min_w(px(0.)).truncate().child(message))
            .child(footer_button(
                "menu-bar-undo-button",
                "Undo",
                "z",
                "Undo archive",
                theme,
                cx.listener(|this, _, window, cx| this.undo(&Undo, window, cx)),
            ))
    }

    fn footer(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("menu-bar-footer")
            .role(gpui::Role::Toolbar)
            .aria_label("Actions")
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_XS)
            .px(SPACE_SM)
            .py(SPACE_XS)
            .border_t_1()
            .border_color(theme.border)
            .text_size(TEXT_SM)
            .text_color(theme.text_muted)
            .child(footer_button(
                "menu-bar-compose",
                "Compose",
                "c",
                "Compose a new message",
                theme,
                cx.listener(|this, _, window, cx| this.compose(&Compose, window, cx)),
            ))
            .child(footer_button(
                "menu-bar-new-event",
                "Event",
                "n",
                "New event",
                theme,
                cx.listener(|this, _, window, cx| this.new_event(&NewEvent, window, cx)),
            ))
            .child(div().flex_1())
            .child(footer_button(
                "menu-bar-open",
                "Open Pigeon",
                "cmd-o",
                "Open Pigeon",
                theme,
                cx.listener(|this, _, window, cx| this.open_pigeon(&OpenPigeon, window, cx)),
            ))
            .child(
                div()
                    .id("menu-bar-gear")
                    .role(gpui::Role::Button)
                    .aria_label("Settings and Quit")
                    .aria_expanded(self.gear_open)
                    .size(HIT_TARGET)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_MD)
                    .when(self.gear_open, |el| el.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg))
                    .active(|s| s.bg(theme.selected_bg))
                    .child(icon(Icon::Settings, ICON_MD, theme.icon))
                    .tooltip(|window, cx| tooltip("Settings and quit").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.gear_open = !this.gear_open;
                        cx.notify();
                    })),
            )
    }

    /// The gear's menu, in the flow above the footer, so the panel grows to
    /// hold it rather than clipping a pop-up.
    fn gear_menu(&self, theme: &Theme) -> impl IntoElement {
        let row = |id: &'static str, label: &'static str, keys: &'static str| {
            menu_row(id, label, false, false, theme)
                .child(menu_check(false, theme))
                .child(div().flex_1().child(label))
                .child(keys_caps(keys, theme))
        };
        div().flex_none().flex().justify_end().px(SPACE_SM).child(
            dropdown("menu-bar-gear-menu", "Pigeon", GEAR_MENU_WIDTH, theme)
                .mb(SPACE_XS)
                .child(
                    row("menu-bar-settings", "Settings…", "cmd-,").on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::OpenSettings), cx)
                    }),
                )
                .child(
                    row("menu-bar-quit", "Quit Pigeon", "cmd-q").on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::Quit), cx)
                    }),
                ),
        )
    }
}

/// A selectable row: events and mail.
fn row_shell(
    id: impl Into<gpui::ElementId>,
    label: String,
    selected: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(gpui::Role::ListBoxOption)
        .aria_label(SharedString::from(label))
        .aria_selected(selected)
        .mx(SPACE_XS)
        .px(SPACE_MD)
        .py(SPACE_XS + SPACE_2XS)
        .min_h(CONTROL_MD)
        .flex()
        .items_center()
        .gap(SPACE_SM + SPACE_2XS)
        .rounded(RADIUS_MD)
        .when(selected, |el| el.aria_active_descendant())
        .map(|el| {
            // The keyboard's row as in every overlay (ME-080).
            if selected {
                el.bg(theme.selected_bg)
            } else {
                el.hover(|s| s.bg(theme.hover_bg))
            }
        })
}

fn section_header(
    id: &'static str,
    title: &'static str,
    right: Option<AnyElement>,
    theme: &Theme,
) -> AnyElement {
    super::section_caption(title, theme)
        .id(id)
        .role(gpui::Role::Heading)
        .aria_label(title)
        .flex()
        .items_center()
        .justify_between()
        .px(SPACE_LG)
        .pt(SPACE_MD)
        .pb(SPACE_XS)
        .when_some(right, |el, right| {
            el.child(div().font_weight(FontWeight::NORMAL).child(right))
        })
        .into_any_element()
}

fn quiet_line(id: &'static str, text: String, theme: &Theme) -> AnyElement {
    div()
        .id(id)
        .px(SPACE_LG)
        .py(SPACE_XS)
        .text_size(TEXT_SM)
        .text_color(theme.text_faint)
        .child(text)
        .into_any_element()
}

fn footer_button(
    id: &'static str,
    label: &'static str,
    key: &'static str,
    aria: &'static str,
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(aria)
        .h(CONTROL_MD)
        .px(SPACE_SM)
        .flex()
        .items_center()
        .gap(SPACE_XS + SPACE_2XS)
        .rounded(RADIUS_MD)
        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
        .active(|s| s.bg(theme.selected_bg).text_color(theme.text))
        .on_click(on_click)
        .child(label)
        .child(keys_caps(key, theme))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calendar::{Attendee, Event, RsvpStatus, time::at},
        data::Account,
    };
    use chrono::NaiveDate;

    #[test]
    fn card_words() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let o = Occurrence {
            event: 0,
            start: at(day, 11, 0),
            end: at(day, 11, 45),
        };
        assert_eq!(countdown_chip(&o, at(day, 10, 48)), "in 12 min");
        assert_eq!(countdown_chip(&o, at(day, 11, 20)), "Now · 25 min left");
        assert_eq!(
            countdown_chip(&o, at(day - Duration::days(1), 18, 0)),
            "Tomorrow"
        );
        let guest = |name: &str| Attendee {
            name: name.into(),
            email: format!("{}@x.com", name.to_lowercase()),
            status: RsvpStatus::Accepted,
        };
        let mut data = CalendarData {
            account: Account {
                email: "me@x.com".into(),
                ..Account::default()
            },
            calendars: Vec::new(),
            events: vec![Event {
                attendees: vec![guest("Me"), guest("Priya Raman"), guest("Sam")],
                ..Event::default()
            }],
        };
        assert_eq!(guests_line(&data, &o).as_deref(), Some("Priya and Sam"));
        data.events[0].attendees.extend([guest("Ana"), guest("Bo")]);
        assert_eq!(
            guests_line(&data, &o).as_deref(),
            Some("Priya, Sam and 2 others")
        );
    }
}

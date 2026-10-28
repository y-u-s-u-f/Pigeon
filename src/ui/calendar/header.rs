//! The calendar's header: the period shown, the calendars menu, the view
//! switcher, today and the arrows, and the new-event button.

use chrono::{Datelike, Duration, NaiveDate};
use gpui::{
    Action, Anchor, AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, SharedString, StatefulInteractiveElement as _, Styled,
    Window, anchored, component::input::Input, deferred, div, prelude::FluentBuilder as _, px,
};

use super::{CalendarView, GoToDate, Menu, ViewKind};
use crate::ui::style::*;
use crate::{
    actions::*,
    assets::Icon,
    calendar::{AccessRole, Calendar},
    theme::Theme,
    ui::{
        HEADER_HEIGHT,
        controls::{ButtonKind, dropdown, popover_surface, push_button, segmented},
        icon, icon_button, live_text,
        motion::{self, Rise},
        tooltip, window_drag_area,
    },
};

impl CalendarView {
    /// "October 2026", "Oct – Nov 2026", "Dec 2026 – Jan 2027", or for a day
    /// "Friday, October 30" (with the year when it isn't this one): the bold
    /// part and the muted rest.
    fn title(&self) -> (String, String) {
        let (from, to) = self.visible_range();
        match self.view {
            ViewKind::Day => (
                if crate::data::day_first() {
                    self.anchor.format("%A, %-d %B").to_string()
                } else {
                    self.anchor.format("%A, %B %-d").to_string()
                },
                if self.anchor.year() == super::today().year() {
                    String::new()
                } else {
                    self.anchor.year().to_string()
                },
            ),
            ViewKind::Month => (
                self.anchor.format("%B").to_string(),
                self.anchor.year().to_string(),
            ),
            ViewKind::Week => {
                let last = to - Duration::days(1);
                if from.month() == last.month() {
                    (from.format("%B").to_string(), from.year().to_string())
                } else if from.year() == last.year() {
                    (
                        format!("{} – {}", from.format("%b"), last.format("%b")),
                        from.year().to_string(),
                    )
                } else {
                    (
                        format!(
                            "{} {} – {}",
                            from.format("%b"),
                            from.year(),
                            last.format("%b")
                        ),
                        last.year().to_string(),
                    )
                }
            }
        }
    }

    /// A click on a header button leaves focus on the grid, so Return
    /// still opens the selected event (W2-R14); Tab focus stays put.
    fn refocus_grid(
        &self,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(|this, event: &ClickEvent, window, cx| {
            if !event.is_keyboard() {
                this.focus_grid(window, cx);
            }
        })
    }

    pub(crate) fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let (title, mut rest) = self.title();
        // The week's number, when they're on (ME-094); the Month view has
        // them down its side.
        if self.show_week_numbers && self.view != ViewKind::Month {
            let week = format!("W{}", self.visible_range().0.iso_week().week());
            rest = if rest.is_empty() {
                week
            } else {
                format!("{rest} · {week}")
            };
        }
        // Muted while today is in view, still there to bring it back into
        // place (ME-093).
        let (from, to) = self.visible_range();
        let today_shown = (from..to).contains(&super::today());

        // The shared segmented control; ←/→ change it while it has focus
        // (`step_view`).
        let selected = match self.view {
            ViewKind::Day => 0,
            ViewKind::Week => 1,
            ViewKind::Month => 2,
        };
        let view_switch = segmented(
            "view-switch",
            "View",
            &["Day", "Week", "Month"],
            selected,
            &theme,
            window,
            cx,
            |ix, window, cx| {
                let action: Box<dyn Action> = match ix {
                    0 => Box::new(ShowDayView),
                    1 => Box::new(ShowWeekView),
                    _ => Box::new(ShowMonthView),
                };
                window.dispatch_action(action, cx);
            },
        )
        .track_focus(&self.view_switch_focus)
        .tooltip(|window, cx| tooltip("Day, week or month (D, W, M)").build(window, cx));

        let today_kind = if today_shown {
            ButtonKind::Quiet
        } else {
            ButtonKind::Plain
        };
        let today_button = push_button("today", "Today", today_kind, &theme)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(GoToToday), cx))
            .on_click(self.refocus_grid(cx))
            .tooltip(|window, cx| {
                tooltip("Go to today")
                    .action(&GoToToday, None)
                    .build(window, cx)
            });

        let new_button = push_button("new-event", "New", ButtonKind::Primary, &theme)
            .aria_label("New event")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(NewEvent), cx))
            .tooltip(|window, cx| {
                tooltip("New event")
                    .action(&NewEvent, None)
                    .build(window, cx)
            });

        window_drag_area(div().id("calendar-header"))
            .h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .pl(SPACE_SM)
            .pr(SPACE_LG)
            .child(self.render_title(title, rest, &theme, window, cx))
            .child(div().flex_1())
            // Google's zone isn't this Mac's (CAL-41).
            .when(self.zone_mismatch().is_some(), |d| {
                let note: SharedString = self.time_zone_note().into();
                d.child(
                    div()
                        .id("zone-note")
                        .role(gpui::Role::Note)
                        .aria_label(note.clone())
                        .size(HIT_TARGET)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(RADIUS_MD)
                        .hover(|s| s.bg(theme.hover_bg))
                        .child(icon(Icon::Clock, ICON_MD, theme.text_muted))
                        .tooltip(move |window, cx| tooltip(note.clone()).build(window, cx)),
                )
            })
            .child(self.render_calendars_button(&theme, window, cx))
            .child(view_switch)
            .child(div().w(px(1.)).h(ICON_XL).mx(SPACE_XS).bg(theme.border))
            .child(
                icon_button(
                    "prev-period",
                    Icon::ChevronLeft,
                    "Previous",
                    Box::new(PrevPeriod),
                    &theme,
                )
                .on_click(self.refocus_grid(cx)),
            )
            .child(today_button)
            .child(
                icon_button(
                    "next-period",
                    Icon::ChevronRight,
                    "Next",
                    Box::new(NextPeriod),
                    &theme,
                )
                .on_click(self.refocus_grid(cx)),
            )
            .child(div().w(SPACE_XS))
            .child(new_button)
            .into_any_element()
    }

    /// The period shown; a click opens the go-to-date field and month.
    fn render_title(
        &self,
        title: String,
        rest: String,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.menu == Some(Menu::GoToDate);
        let popover = open.then(|| {
            // Parsed as typed, not per frame (MC-056).
            let hint = if self.date_input.read(cx).value().trim().is_empty() {
                None
            } else {
                Some(match self.typed_date() {
                    Some(date) => (full_date(date), theme.text_muted),
                    None => ("Not a date yet".to_string(), theme.text_faint),
                })
            };
            deferred(
                anchored()
                    .snap_to_window_with_margin(SPACE_SM)
                    .child(motion::enter(
                        "go-to-date",
                        Rise::Down,
                        popover_surface(theme)
                            .id("go-to-date")
                            .occlude()
                            .role(gpui::Role::Dialog)
                            .aria_label("Go to date")
                            .mt(SPACE_2XL + SPACE_SM)
                            // Seven days, the padding and the 1px border.
                            .w(HIT_TARGET * 7. + SPACE_MD * 2. + SPACE_2XS)
                            .p(SPACE_MD)
                            .flex()
                            .flex_col()
                            .gap(SPACE_SM)
                            .child(
                                div()
                                    .h(CONTROL_LG)
                                    .px(SPACE_SM)
                                    .flex()
                                    .items_center()
                                    .gap(SPACE_SM)
                                    .rounded(RADIUS_MD)
                                    .bg(theme.search_bg)
                                    .border_1()
                                    .border_color(theme.accent.alpha(0.55))
                                    .child(icon(Icon::Calendar, ICON_MD, theme.text_faint))
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            Input::new(&self.date_input)
                                                .appearance(false)
                                                .px(px(0.))
                                                .text_size(TEXT_BASE),
                                        ),
                                    ),
                            )
                            .children(hint.map(|(text, color)| {
                                live_text("go-to-date-hint", text, gpui::accesskit::Live::Polite)
                                    .px(SPACE_SM)
                                    .text_size(TEXT_SM)
                                    .text_color(color)
                            }))
                            .child(self.render_mini_month("go-to-date-day", theme, cx)),
                    )),
            )
            .with_priority(2)
        });
        div()
            .min_w_0()
            .child(
                focusable_control(div().id("calendar-title"), theme)
                    .role(gpui::Role::Button)
                    .aria_label(format!("{title} {rest}, go to date"))
                    .aria_expanded(open)
                    .min_w_0()
                    .h(CONTROL_LG)
                    .px(SPACE_SM)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .when(open, |d| d.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(
                        div()
                            .text_size(TEXT_TITLE1)
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .when(!rest.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(TEXT_TITLE1)
                                .font_weight(FontWeight::NORMAL)
                                .text_color(theme.text_faint)
                                .child(rest),
                        )
                    })
                    .child(icon(Icon::ChevronDown, ICON_MD, theme.text_faint))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_date_picker(window, cx)),
                    )
                    .tooltip(|window, cx| {
                        tooltip("Go to date")
                            .action(&GoToDate, None)
                            .build(window, cx)
                    }),
            )
            .when(open, |d| d.child(self.menu_backdrop(window, cx)))
            .children(popover)
            .into_any_element()
    }

    /// The calendars button: a stack of their colors, opening a menu to show
    /// and hide each. With focus, arrows move through the open menu and
    /// Space or Enter shows or hides the highlighted calendar (ME-045).
    fn render_calendars_button(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.menu == Some(Menu::Calendars);
        let hidden = self.data.calendars.iter().filter(|c| !c.visible).count();
        let dots = self
            .data
            .calendars
            .iter()
            .filter(|c| c.visible)
            .take(4)
            .enumerate()
            .map(|(ix, c)| {
                div()
                    .size(CALENDAR_DOT)
                    .when(ix > 0, |d| d.ml(-SPACE_XS))
                    .rounded_full()
                    .bg(c.color)
                    .border_1()
                    .border_color(theme.overlay_bg)
            })
            .collect::<Vec<_>>();
        let order = self.calendars_menu_order();
        let highlight = self.calendars_highlight.and_then(|h| order.get(h)).copied();

        let menu = open.then(|| {
            let row = |ix: usize, calendar: &Calendar| {
                let visible = calendar.visible;
                let read_only = matches!(
                    calendar.access_role,
                    AccessRole::Reader | AccessRole::FreeBusyReader
                );
                let highlighted = highlight == Some(ix);
                div()
                    .id(("calendar-toggle", ix))
                    .role(gpui::Role::CheckBox)
                    .aria_label(SharedString::from(calendar.name.clone()))
                    .aria_toggled(if visible {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .aria_selected(highlighted)
                    .h(ROW_SM)
                    .flex_none()
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .when(highlighted, |d| d.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(
                        div()
                            .size(ICON_LG)
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS_SM)
                            .border_1()
                            .border_color(calendar.color)
                            .when(visible, |d| d.bg(calendar.color))
                            .when(visible, |d| {
                                d.child(icon(Icon::Check, ICON_SM, theme.on_fill(calendar.color)))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(TEXT_BASE)
                            .text_color(if visible {
                                theme.text
                            } else {
                                theme.text_muted
                            })
                            .child(calendar.name.clone()),
                    )
                    .when(read_only, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(TEXT_XS)
                                .text_color(theme.text_faint)
                                .child("View only"),
                        )
                    })
                    // ⌥-click: only this calendar; again, all of them.
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.modifiers().alt {
                            this.show_only_calendar(ix, cx);
                        } else {
                            this.toggle_calendar(ix, cx);
                        }
                        window.focus(&this.calendars_focus, cx);
                    }))
                    .tooltip(|window, cx| {
                        tooltip("Option-click to show only this calendar").build(window, cx)
                    })
            };
            // Like Google Calendar: the user's own calendars, then the
            // others (shared, subscribed, holidays), each in list order.
            let (mine, others): (Vec<_>, Vec<_>) = self
                .data
                .calendars
                .iter()
                .enumerate()
                .partition(|(_, c)| c.access_role == AccessRole::Owner);
            let section = |title: &'static str, calendars: Vec<(usize, &Calendar)>| {
                (!calendars.is_empty()).then(|| {
                    div()
                        .id(title)
                        .flex()
                        .flex_col()
                        .role(gpui::Role::Group)
                        .aria_label(title)
                        .child(
                            div()
                                .px(SPACE_MD)
                                .pt(SPACE_SM)
                                .pb(SPACE_XS)
                                .text_size(TEXT_XS)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text_muted)
                                .child(title),
                        )
                        .children(calendars.into_iter().map(|(ix, c)| row(ix, c)))
                })
            };
            // Anchored to the button's right edge (against the view switch),
            // which stays put when the label changes width.
            div().absolute().top_0().right_0().child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .snap_to_window_with_margin(SPACE_SM)
                        .child(motion::enter(
                            "calendars-menu",
                            Rise::Down,
                            dropdown("calendars-menu", "Calendars", CALENDARS_MENU_WIDTH, theme)
                                .mt(SPACE_2XL)
                                .max_h(CALENDARS_MENU_HEIGHT)
                                .overflow_y_scroll()
                                .children(section("My Calendars", mine))
                                .children(section("Other Calendars", others)),
                        )),
                )
                .with_priority(2),
            )
        });

        let label: SharedString = if hidden > 0 {
            format!("{hidden} hidden").into()
        } else {
            "Calendars".into()
        };
        let ring = focus_ring(theme);

        div()
            .flex_none()
            .relative()
            .child(
                div()
                    .id("calendars-button")
                    .key_context(PANEL_CONTROL)
                    .track_focus(&self.calendars_focus)
                    .focus_visible(move |s| s.shadow(ring))
                    .on_action(cx.listener(Self::calendars_menu_next))
                    .on_action(cx.listener(Self::calendars_menu_prev))
                    .on_action(cx.listener(Self::calendars_menu_activate))
                    .role(gpui::Role::Button)
                    .aria_label(format!("Calendars, {label}"))
                    .aria_expanded(open)
                    .h(CONTROL_MD)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_muted)
                    .when(open, |d| d.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                    .active(|s| s.bg(theme.selected_bg))
                    .child(div().flex().items_center().pl(SPACE_XS).children(dots))
                    .child(div().whitespace_nowrap().child(label))
                    .child(icon(Icon::ChevronDown, ICON_SM, theme.text_faint))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_menu(Menu::Calendars, cx);
                        this.calendars_highlight = None;
                        window.focus(&this.calendars_focus, cx);
                    }))
                    .tooltip(|window, cx| {
                        tooltip("Show and hide calendars")
                            .action(&ToggleCalendarsMenu, None)
                            .build(window, cx)
                    }),
            )
            .when(open, |this| this.child(self.menu_backdrop(window, cx)))
            .children(menu)
            .into_any_element()
    }
}

/// "Friday, October 30, 2026", in the region's day and month order.
pub(crate) fn full_date(date: NaiveDate) -> String {
    if crate::data::day_first() {
        date.format("%A, %-d %B %Y").to_string()
    } else {
        date.format("%A, %B %-d, %Y").to_string()
    }
}

const CALENDARS_MENU_WIDTH: Pixels = px(260.);
/// A calendar's color dot on the Calendars button.
const CALENDAR_DOT: Pixels = px(10.);
const CALENDARS_MENU_HEIGHT: Pixels = px(440.);

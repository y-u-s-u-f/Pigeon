//! A small month for jumping to a date: under the header's title, and beside
//! the Day view. A click on a day goes to it. Its grid is also the event
//! panel's date picker (`month_grid`).

use chrono::{Datelike, Duration, NaiveDate};
use gpui::{
    AbsoluteLength, AnyElement, App, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, StatefulInteractiveElement as _, Styled, Window,
    div, prelude::FluentBuilder as _,
};

use super::{CalendarView, today};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    calendar::time::{add_months, midnight, month_start, week_start},
    theme::Theme,
    ui::icon,
};

const WEEKS: i64 = 6;

impl CalendarView {
    /// For each of the six weeks from `first`, whether a day has events.
    fn busy_days(&self, first: NaiveDate) -> [bool; (WEEKS * 7) as usize] {
        let mut busy = [false; (WEEKS * 7) as usize];
        let to = first + Duration::days(WEEKS * 7);
        for o in self.occurrences_between(midnight(first), midnight(to)) {
            let last = (o.end - Duration::seconds(1)).date().max(o.start.date());
            let mut day = o.start.date().max(first);
            while day <= last && day < to {
                busy[(day - first).num_days() as usize] = true;
                day += Duration::days(1);
            }
        }
        busy
    }

    /// The month the mini months show.
    pub(crate) fn mini_month_shown(&self) -> NaiveDate {
        self.mini_month.unwrap_or_else(|| month_start(self.anchor))
    }

    pub(crate) fn render_mini_month(
        &self,
        id: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let month = self.mini_month_shown();
        let typed = (self.menu == Some(super::Menu::GoToDate))
            .then(|| self.typed_date())
            .flatten();
        let first = week_start(month);
        let busy = self.busy_days(first);
        let anchor = self.anchor;
        month_grid(
            MonthGrid {
                id,
                nav: ("mini-prev", "mini-next"),
                month,
                cell: HIT_TARGET.into(),
            },
            |date| {
                let chosen = typed.map_or(date == anchor, |t| date == t);
                Day {
                    chosen,
                    busy: busy[(date - first).num_days() as usize] && !chosen,
                    ..Day::default()
                }
            },
            |this, months, cx| {
                this.mini_month = Some(add_months(this.mini_month_shown(), months));
                cx.notify();
            },
            |this, date, window, cx| this.go_to_date(date, window, cx),
            theme,
            cx,
        )
    }
}

/// How one day of a `month_grid` shows.
#[derive(Clone, Copy, Default)]
pub(crate) struct Day {
    pub chosen: bool,
    /// Can't be picked (an end before the start).
    pub disabled: bool,
    /// Between a chosen start and end.
    pub in_range: bool,
    /// Has events: a dot under the number.
    pub busy: bool,
}

pub(crate) struct MonthGrid {
    /// The day cells' id, with each day's number.
    pub id: &'static str,
    /// The previous and next month buttons' ids.
    pub nav: (&'static str, &'static str),
    /// Any day of the month shown.
    pub month: NaiveDate,
    /// A day's width and height.
    pub cell: AbsoluteLength,
}

/// The month grid that the mini month and the event panel's date picker
/// share (MC-069): the month's name with previous and next, the weekday
/// letters, and six weeks of days.
pub(crate) fn month_grid(
    grid: MonthGrid,
    day: impl Fn(NaiveDate) -> Day,
    on_nav: impl Fn(&mut CalendarView, i32, &mut Context<CalendarView>) + Clone + 'static,
    on_pick: impl Fn(&mut CalendarView, NaiveDate, &mut Window, &mut Context<CalendarView>)
    + Clone
    + 'static,
    theme: &Theme,
    cx: &mut Context<CalendarView>,
) -> AnyElement {
    let MonthGrid {
        id,
        nav: (prev_id, next_id),
        month,
        cell,
    } = grid;
    let first = week_start(month);
    let today = today();
    let swallow = |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
        window.prevent_default();
        cx.stop_propagation();
    };

    let nav = |nav_id: &'static str, glyph: Icon, label: &'static str, months: i32| {
        let on_nav = on_nav.clone();
        div()
            .id(nav_id)
            .role(gpui::Role::Button)
            .aria_label(label)
            .size(HIT_TARGET)
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .hover(|s| s.bg(theme.hover_bg))
            .active(|s| s.bg(theme.selected_bg))
            .child(icon(glyph, ICON_MD, theme.icon))
            .on_mouse_down(MouseButton::Left, swallow)
            .on_click(cx.listener(move |this, _, _, cx| on_nav(this, months, cx)))
    };
    let header = div()
        .pb(SPACE_XS)
        .pl(SPACE_SM)
        .flex()
        .items_center()
        .child(
            div()
                .flex_1()
                .text_size(TEXT_BASE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(month.format("%B %Y").to_string()),
        )
        .child(nav(prev_id, Icon::ChevronLeft, "Previous month", -1))
        .child(nav(next_id, Icon::ChevronRight, "Next month", 1));

    let weekdays = crate::calendar::time::weekday_letters().map(|d| {
        div()
            .w(cell)
            .flex()
            .justify_center()
            .text_size(TEXT_XS)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text_faint)
            .child(d)
    });

    let weeks = (0..WEEKS).map(|w| {
        div().flex().children((0..7).map(|d| {
            let date = first + Duration::days(w * 7 + d);
            let Day {
                chosen,
                disabled,
                in_range,
                busy,
            } = day(date);
            let in_month = date.month() == month.month();
            let is_today = date == today;
            let on_pick = on_pick.clone();
            div()
                .id((id, date.num_days_from_ce() as usize))
                .role(gpui::Role::Button)
                .aria_label(if disabled {
                    format!("{}, unavailable", super::header::full_date(date))
                } else {
                    super::header::full_date(date)
                })
                .aria_selected(chosen)
                .size(cell)
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .text_size(TEXT_SM)
                .map(|d| {
                    if chosen {
                        d.bg(theme.accent)
                            .text_color(theme.accent_text)
                            .font_weight(FontWeight::SEMIBOLD)
                    } else if disabled {
                        d.text_color(theme.text_disabled)
                    } else {
                        d.text_color(if is_today {
                            theme.accent
                        } else if in_month {
                            theme.text
                        } else {
                            theme.text_faint
                        })
                        .when(in_range, |d| d.bg(theme.selected_bg))
                        .when(is_today, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .hover(|s| s.bg(theme.hover_bg))
                        .active(|s| s.bg(theme.selected_bg))
                    }
                })
                .child(date.day().to_string())
                .when(busy, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom(SPACE_2XS)
                            .flex()
                            .justify_center()
                            .child(div().size(SPACE_XS).rounded_full().bg(if in_month {
                                theme.text_muted
                            } else {
                                theme.text_faint
                            })),
                    )
                })
                .on_mouse_down(MouseButton::Left, swallow)
                .when(!disabled, |d| {
                    d.on_click(
                        cx.listener(move |this, _, window, cx| on_pick(this, date, window, cx)),
                    )
                })
        }))
    });

    div()
        .flex()
        .flex_col()
        .child(header)
        .child(div().flex().pb(SPACE_2XS).children(weekdays))
        .children(weeks)
        .into_any_element()
}

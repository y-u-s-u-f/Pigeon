//! The Month view: weeks of days, with all-day and multi-day events as bars
//! across them and timed events as pills; a day that overflows says how many
//! more it has.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, StatefulInteractiveElement as _, Styled, Window, anchored, deferred, div,
    prelude::FluentBuilder as _, px, relative,
};

use super::{CalendarView, Menu, occurrence_key, week_view::Look};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    calendar::{
        Occurrence,
        layout::pack_lanes,
        time::{self, compact_time_label, is_weekend, month_grid},
    },
    theme::Theme,
    ui::{
        HEADER_HEIGHT,
        controls::{bar_colors, popover_surface},
        icon, is_secondary,
        motion::{Rise, enter},
    },
};

const WEEKDAY_ROW: f32 = 30.;
/// Space above a cell's events, for its day number.
const DAY_NUMBER: f32 = 30.;
const SLOT: f32 = 21.;
/// The week number column, when shown.
const WEEK_NUMBER: f32 = 36.;
/// A timed event's calendar dot.
const EVENT_DOT: gpui::Pixels = px(7.);
/// How much an event on a day around the month shows.
const OUTSIDE_OPACITY: f32 = 0.55;
const DAY_LIST_WIDTH: gpui::Pixels = px(260.);

/// An event's place in one week row.
struct Item {
    occurrence: Occurrence,
    /// Days of the week it covers, end exclusive.
    start: usize,
    end: usize,
    lane: usize,
    bar: bool,
}

impl CalendarView {
    pub(crate) fn render_month(
        &self,
        occurrences: &[Occurrence],
        now: NaiveDateTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let (grid_start, weeks) = month_grid(self.anchor);
        let month = self.anchor.month();
        let height = window.viewport_size().height - HEADER_HEIGHT - px(WEEKDAY_ROW);
        let row_height = f32::from(height) / weeks as f32;
        let slots = (((row_height - DAY_NUMBER - 4.) / SLOT).floor() as usize).max(1);

        let weekday_row = div()
            .h(px(WEEKDAY_ROW))
            .flex_none()
            .flex()
            .border_b_1()
            .border_color(theme.cal_line)
            .when(self.show_week_numbers, |d| {
                d.child(div().w(px(WEEK_NUMBER)).flex_none())
            })
            .children((0..7).map(|i| {
                let date = grid_start + Duration::days(i);
                div()
                    .flex_1()
                    .pl(SPACE_MD)
                    .flex()
                    .items_center()
                    .text_size(TEXT_XS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if is_weekend(date) {
                        theme.text_faint
                    } else {
                        theme.text_muted
                    })
                    .child(date.format("%a").to_string().to_uppercase())
            }));

        let rows = (0..weeks).map(|w| {
            let start = grid_start + Duration::days(7 * w as i64);
            self.render_week_row(start, month, occurrences, slots, now, &theme, cx)
        });

        let day_list_open = matches!(self.menu, Some(Menu::DayList(_)));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(weekday_row)
            .child(div().flex_1().min_h_0().flex().flex_col().children(rows))
            .when(day_list_open, |d| d.child(self.menu_backdrop(window, cx)))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_week_row(
        &self,
        start: NaiveDate,
        month: u32,
        occurrences: &[Occurrence],
        slots: usize,
        now: NaiveDateTime,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = now.date();
        let week_start = time::midnight(start);
        let week_end = week_start + Duration::days(7);
        let day_of = |t: chrono::NaiveDateTime| (t.date() - start).num_days();

        // Bars first, in lanes; then each day's timed events in the lanes
        // left free on that day.
        let mut items: Vec<Item> = Vec::new();
        let in_week = occurrences
            .iter()
            .filter(|o| o.start < week_end && o.end > week_start);
        let (bars, timed): (Vec<&Occurrence>, Vec<&Occurrence>) =
            in_week.partition(|o| self.data.events[o.event].is_spanning());
        let spans: Vec<(i64, i64)> = bars
            .iter()
            .map(|o| {
                let last = (o.end - Duration::minutes(1)).date();
                (
                    day_of(o.start).max(0),
                    ((last - start).num_days() + 1).min(7),
                )
            })
            .collect();
        let lanes = pack_lanes(&spans);
        let mut taken = [[false; 64]; 7];
        for ((o, &(s, e)), &lane) in bars.iter().zip(&spans).zip(&lanes) {
            for day in s..e {
                if lane < 64 {
                    taken[day as usize][lane] = true;
                }
            }
            items.push(Item {
                occurrence: **o,
                start: s as usize,
                end: e as usize,
                lane,
                bar: true,
            });
        }
        for o in timed {
            // A timed event past midnight shows on the day it starts.
            let day = day_of(o.start).max(0) as usize;
            let lane = (0..64).find(|&l| !taken[day][l]).unwrap_or(63);
            taken[day][lane] = true;
            items.push(Item {
                occurrence: *o,
                start: day,
                end: day + 1,
                lane,
                bar: false,
            });
        }

        // A day with more than fits gives its last slot to "+N more".
        let overflows = |day: usize| {
            items
                .iter()
                .any(|i| i.start <= day && day < i.end && i.lane >= slots)
        };
        let shown: [usize; 7] =
            std::array::from_fn(|day| if overflows(day) { slots - 1 } else { slots });
        let more: [usize; 7] = std::array::from_fn(|day| {
            items
                .iter()
                .filter(|i| i.start <= day && day < i.end && i.lane >= shown[day])
                .count()
        });

        let mut pills: Vec<AnyElement> = Vec::new();
        for item in &items {
            // Bars are cut where a day hides their lane.
            let mut day = item.start;
            while day < item.end {
                if item.lane >= shown[day] {
                    day += 1;
                    continue;
                }
                let run_start = day;
                while day < item.end && item.lane < shown[day] {
                    day += 1;
                }
                pills.push(self.render_pill(item, run_start, day, week_start, now, theme, cx));
            }
        }

        let cells = (0..7).map(|i| {
            let date = start + Duration::days(i);
            let is_today = date == today;
            let is_anchor = date == self.anchor;
            let in_month = date.month() == month;
            let label = if date.day() == 1 {
                date.format("%b %-d").to_string()
            } else {
                date.day().to_string()
            };
            div()
                .id(("month-cell", date.num_days_from_ce() as usize))
                .flex_1()
                .min_w_0()
                .h_full()
                .border_r_1()
                .border_color(theme.cal_line)
                // The days around the month are tinted deeper than
                // weekends (ME-087).
                .when(is_weekend(date) && in_month, |d| d.bg(theme.cal_weekend_bg))
                .when(!in_month, |d| d.bg(theme.cal_outside_bg))
                .when(is_today, |d| d.bg(theme.cal_today_bg))
                .pt(SPACE_XS)
                // With the number's padding, it lines up with the weekday
                // and the events at 12 (ME-086).
                .pl(SPACE_XS)
                .flex()
                .flex_col()
                .items_start()
                // A double click makes an all-day event on the day (ME-044).
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.press_day(date, event, window, cx)
                    }),
                )
                .child(
                    // A HIT_TARGET box around the number (MA-041).
                    div()
                        .id(("month-day", date.num_days_from_ce() as usize))
                        .role(gpui::Role::Button)
                        .aria_label(format!("Open {}", super::header::full_date(date)))
                        .group("month-day")
                        .min_w(HIT_TARGET)
                        .h(HIT_TARGET)
                        .mt(-SPACE_XS)
                        .flex()
                        .flex_none()
                        .items_center()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| this.open_day(date, cx)))
                        .child(
                            div()
                                .h(CONTROL_XS)
                                .min_w(CONTROL_XS)
                                .px(SPACE_SM)
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .text_size(TEXT_SM)
                                .map(|d| {
                                    if is_today {
                                        d.bg(theme.accent)
                                            .text_color(theme.accent_text)
                                            .font_weight(FontWeight::SEMIBOLD)
                                    } else {
                                        d.text_color(if in_month {
                                            theme.text
                                        } else {
                                            theme.text_faint
                                        })
                                        .font_weight(if date.day() == 1 || is_anchor {
                                            FontWeight::SEMIBOLD
                                        } else {
                                            FontWeight::MEDIUM
                                        })
                                        // The selected day, where left and right move.
                                        .when(is_anchor, |d| d.bg(theme.selected_bg))
                                        .group_hover("month-day", |s| s.bg(theme.hover_bg))
                                    }
                                })
                                .child(label),
                        ),
                )
        });

        let more_links = (0..7).filter(|&d| more[d] > 0).map(|d| {
            let date = start + Duration::days(d as i64);
            let open = self.menu == Some(Menu::DayList(date));
            let list = open.then(|| self.render_day_list(date, occurrences, theme, cx));
            div()
                .absolute()
                .top(px(DAY_NUMBER + shown[d] as f32 * SLOT))
                .left(relative(d as f32 / 7.))
                .w(relative(1. / 7.))
                .h(px(SLOT - 2.))
                .px(SPACE_XS)
                .child(
                    div()
                        .id(("more", date.num_days_from_ce() as usize))
                        .size_full()
                        .px(SPACE_SM)
                        .flex()
                        .items_center()
                        .rounded(RADIUS_SM)
                        .text_size(TEXT_SM)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_muted)
                        .role(gpui::Role::Button)
                        .aria_label(format!("{} more events", more[d]))
                        .aria_expanded(open)
                        .when(open, |d| d.bg(theme.selected_bg))
                        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                        .child(format!("+{} more", more[d]))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        // The day's events over the month, not the Day view
                        // (ME-087).
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_menu(Menu::DayList(date), cx)
                        })),
                )
                .children(list)
        });

        let days = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .flex()
            .overflow_hidden()
            .children(cells)
            .children(pills)
            .children(more_links);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .border_b_1()
            .border_color(theme.cal_line)
            .when(self.show_week_numbers, |d| {
                d.child(
                    div()
                        .w(px(WEEK_NUMBER))
                        .flex_none()
                        .pt(SPACE_SM)
                        .flex()
                        .justify_center()
                        .border_r_1()
                        .border_color(theme.cal_line)
                        .text_size(TEXT_XS)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_faint)
                        .child(format!("W{}", start.iso_week().week())),
                )
            })
            .child(days)
            .into_any_element()
    }

    /// An event over days `start..end` of the week: a filled bar for all-day
    /// and multi-day events, a dot, time and title for timed ones.
    #[allow(clippy::too_many_arguments)]
    fn render_pill(
        &self,
        item: &Item,
        start: usize,
        end: usize,
        week_start: NaiveDateTime,
        now: NaiveDateTime,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let occurrence = item.occurrence;
        let event = &self.data.events[occurrence.event];
        let color = self.data.color_of(event);
        let selected = self.is_selected(&occurrence);
        let past = occurrence.end <= now;
        let look = Look::of(event.my_status(&self.data.account));
        let read_only = !self.can_edit_ix(occurrence.event);
        let clipped_left = item.bar
            && (start > item.start || occurrence.start < week_start + Duration::days(start as i64));
        let clipped_right = item.bar
            && (end < item.end || occurrence.end > week_start + Duration::days(end as i64));

        // Out of the month shown: faded with its day (ME-087).
        let outside = (week_start.date() + Duration::days(start as i64)).month()
            != self.anchor.month()
            && (week_start.date() + Duration::days(end as i64 - 1)).month() != self.anchor.month();
        let pill = div()
            .id((
                "month-event",
                occurrence_key(
                    &occurrence,
                    (week_start.date().num_days_from_ce() + start as i32) as i64,
                ),
            ))
            .role(gpui::Role::Button)
            .aria_label(self.occurrence_label(&occurrence))
            .aria_selected(selected)
            .when(selected, |d| d.aria_active_descendant())
            .size_full()
            .px(SPACE_SM)
            .flex()
            .items_center()
            .gap(SPACE_XS)
            .overflow_hidden()
            .rounded(RADIUS_SM)
            .text_size(TEXT_SM);

        let pill = if item.bar {
            let (fill, text, hover) = bar_colors(color, past, selected, theme);
            let accent = theme.event_accent(color, past);
            // A timed event across days says when it ends, on its last day.
            let until = (!event.all_day && !clipped_right)
                .then(|| format!("until {}", compact_time_label(occurrence.end.time())));
            pill.when(clipped_left, |d| d.rounded_l(px(0.)).pl(SPACE_XS))
                .when(clipped_right, |d| d.rounded_r(px(0.)).pr(SPACE_XS))
                .map(|d| {
                    if selected {
                        d.bg(fill)
                    } else {
                        look.bar(d, fill, accent)
                            .hover(move |s| s.bg(hover))
                            .relative()
                            .children(look.stripes(accent))
                    }
                })
                // It goes on past the week's edge (ME-090).
                .when(clipped_left, |d| {
                    d.child(icon(Icon::ChevronLeft, ICON_SM, text.alpha(0.75)))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text)
                        .when(look == Look::Declined, |d| d.line_through())
                        .child(event.display_title().to_string()),
                )
                .children(
                    until.map(|until| div().flex_none().text_color(text.alpha(0.75)).child(until)),
                )
                .when(clipped_right, |d| {
                    d.child(icon(Icon::ChevronRight, ICON_SM, text.alpha(0.75)))
                })
        } else {
            let (title, muted) = if selected {
                let text = theme.on_fill(theme.event_selected_fill(color));
                (text, text.alpha(0.8))
            } else if past {
                (theme.text_muted, theme.text_muted)
            } else {
                (theme.text, theme.text_muted)
            };
            let accent = theme.event_accent(color, past);
            pill.when(selected, |d| d.bg(theme.event_selected_fill(color)))
                .when(!selected, |d| d.hover(|s| s.bg(theme.hover_bg)))
                .child(
                    div()
                        .size(EVENT_DOT)
                        .flex_none()
                        .rounded_full()
                        .map(|d| match look {
                            _ if selected => d.bg(theme.on_fill(theme.event_selected_fill(color))),
                            Look::Normal if event.busy => d.bg(accent),
                            // Free: a lighter dot (ME-051).
                            Look::Normal => d.bg(accent.alpha(super::week_view::FREE_ALPHA)),
                            _ => d.border_1().border_color(accent),
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(muted)
                        .font_features(tabular_figures())
                        .child(compact_time_label(occurrence.start.time())),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(title)
                        .when(look == Look::Declined, |d| d.line_through())
                        .child(event.display_title().to_string()),
                )
        };

        div()
            .absolute()
            .top(px(DAY_NUMBER + item.lane as f32 * SLOT))
            .left(relative(start as f32 / 7.))
            .w(relative((end - start) as f32 / 7.))
            .h(px(SLOT - 2.))
            .pl(if clipped_left { px(0.) } else { SPACE_XS })
            .pr(if clipped_right { px(0.) } else { SPACE_XS })
            .when(outside && !selected, |d| d.opacity(OUTSIDE_OPACITY))
            .child(
                pill.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.event_menu(occurrence, event.position, window, cx)
                    }),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        if is_secondary(event) {
                            this.event_menu(occurrence, event.position, window, cx);
                        } else if read_only {
                            this.press_read_only(occurrence, event, window, cx)
                        }
                    }),
                )
                .when(!read_only, |d| {
                    d.on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            if !event.is_right_click() && !event.modifiers().control {
                                this.click_occurrence(occurrence, window, cx)
                            }
                        }),
                    )
                }),
            )
            .into_any_element()
    }

    /// A day's events in a list under its "+N more" (ME-087): a click
    /// opens one, as in the grid.
    fn render_day_list(
        &self,
        date: NaiveDate,
        occurrences: &[Occurrence],
        theme: &Theme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let from = time::midnight(date);
        let to = from + Duration::days(1);
        let rows = occurrences
            .iter()
            .filter(|o| o.start < to && o.end > from)
            .map(|&occurrence| {
                let event = &self.data.events[occurrence.event];
                let color = self.data.color_of(event);
                let when = if event.is_spanning() {
                    "All day".to_string()
                } else {
                    compact_time_label(occurrence.start.time())
                };
                let selected = self.is_selected(&occurrence);
                div()
                    .id(("day-list-event", occurrence_key(&occurrence, 0)))
                    .role(gpui::Role::MenuItem)
                    .aria_label(self.occurrence_label(&occurrence))
                    .aria_selected(selected)
                    .h(ROW_SM)
                    .flex_none()
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_SM)
                    .text_size(TEXT_SM)
                    .when(selected, |d| d.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(
                        div()
                            .size(EVENT_DOT)
                            .flex_none()
                            .rounded_full()
                            .bg(theme.event_accent(color, false)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.text_muted)
                            .font_features(tabular_figures())
                            .child(when),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(event.display_title().to_string()),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.menu = None;
                        this.click_occurrence(occurrence, window, cx)
                    }))
            })
            .collect::<Vec<_>>();
        deferred(
            anchored().snap_to_window_with_margin(SPACE_SM).child(enter(
                ("day-list", date.num_days_from_ce() as usize),
                Rise::Down,
                popover_surface(theme)
                    .id("day-list")
                    .occlude()
                    .role(gpui::Role::Menu)
                    .aria_label(super::header::full_date(date))
                    .mt(SPACE_XS)
                    .w(DAY_LIST_WIDTH)
                    .p(SPACE_XS)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(SPACE_SM)
                            .py(SPACE_XS)
                            .text_size(TEXT_XS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text_muted)
                            .child(super::header::full_date(date)),
                    )
                    .children(rows),
            )),
        )
        .with_priority(2)
        .into_any_element()
    }
}

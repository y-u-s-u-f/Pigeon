//! The Month view: weeks of days, with all-day and multi-day events as bars
//! across them and timed events as pills; a day that overflows says how many
//! more it has.

use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Timelike};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
    relative,
};

use super::{CalendarView, now, occurrence_key, today, week_view::Look};
use crate::ui::style::*;
use crate::{
    calendar::{
        Occurrence,
        layout::pack_lanes,
        time::{self, is_weekend, month_grid},
    },
    theme::Theme,
    ui::HEADER_HEIGHT,
};

const WEEKDAY_ROW: f32 = 30.;
/// Space above a cell's events, for its day number.
const DAY_NUMBER: f32 = 30.;
const SLOT: f32 = 21.;
/// The week number column, when shown.
const WEEK_NUMBER: f32 = 36.;

/// An event's place in one week row.
struct Item {
    occurrence: Occurrence,
    /// Days of the week it covers, end exclusive.
    start: usize,
    end: usize,
    lane: usize,
    bar: bool,
}

/// "9am", "9:30pm".
fn pill_time(time: NaiveTime) -> String {
    if time::is_24_hour() {
        return time.format("%H:%M").to_string();
    }
    let meridiem = if time.hour() < 12 { "am" } else { "pm" };
    if time.minute() == 0 {
        format!("{}{meridiem}", time.format("%-I"))
    } else {
        format!("{}{meridiem}", time.format("%-I:%M"))
    }
}

impl CalendarView {
    pub(crate) fn render_month(
        &self,
        occurrences: &[Occurrence],
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
            self.render_week_row(start, month, occurrences, slots, &theme, cx)
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(weekday_row)
            .child(div().flex_1().min_h_0().flex().flex_col().children(rows))
            .into_any_element()
    }

    fn render_week_row(
        &self,
        start: NaiveDate,
        month: u32,
        occurrences: &[Occurrence],
        slots: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = today();
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
                pills.push(self.render_pill(item, run_start, day, week_start, theme, cx));
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
                .flex_1()
                .min_w_0()
                .h_full()
                .border_r_1()
                .border_color(theme.cal_line)
                .when(is_weekend(date), |d| d.bg(theme.cal_weekend_bg))
                .when(is_today, |d| d.bg(theme.cal_today_bg))
                .pt(SPACE_XS)
                .pl(SPACE_SM)
                .flex()
                .flex_col()
                .items_start()
                .child(
                    div()
                        .id(("month-day", date.num_days_from_ce() as usize))
                        .h(px(22.))
                        .min_w(px(22.))
                        .px(SPACE_SM)
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .text_size(TEXT_SM)
                        .cursor_pointer()
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
                                .hover(|s| s.bg(theme.hover_bg))
                            }
                        })
                        .child(label)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| this.open_day(date, cx))),
                )
        });

        let more_links = (0..7).filter(|&d| more[d] > 0).map(|d| {
            let date = start + Duration::days(d as i64);
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
                        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                        .cursor_pointer()
                        .child(format!("+{} more", more[d]))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| this.open_day(date, cx))),
                )
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
    fn render_pill(
        &self,
        item: &Item,
        start: usize,
        end: usize,
        week_start: chrono::NaiveDateTime,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let occurrence = item.occurrence;
        let event = &self.data.events[occurrence.event];
        let color = self.data.color_of(event);
        let selected = self.is_selected(&occurrence);
        let past = occurrence.end <= now();
        let look = Look::of(event.my_status(&self.data.account));
        let hover = theme.event_fill_hover(color);
        let clipped_left = item.bar
            && (start > item.start || occurrence.start < week_start + Duration::days(start as i64));
        let clipped_right = item.bar
            && (end < item.end || occurrence.end > week_start + Duration::days(end as i64));

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
            .size_full()
            .px(SPACE_SM)
            .flex()
            .items_center()
            .gap(SPACE_XS)
            .overflow_hidden()
            .rounded(RADIUS_SM)
            .text_size(TEXT_SM)
            .cursor_pointer();

        let pill = if item.bar {
            let (fill, text) = if selected {
                (theme.event_selected_fill(color), theme.cal_selected_text)
            } else {
                (theme.event_fill(color, past), theme.event_text(color, past))
            };
            pill.bg(fill)
                .when(clipped_left, |d| d.rounded_l(px(0.)))
                .when(clipped_right, |d| d.rounded_r(px(0.)))
                .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text)
                        .child(event.display_title().to_string()),
                )
        } else {
            let (title, muted) = if selected {
                let text = theme.cal_selected_text;
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
                        .size(px(7.))
                        .flex_none()
                        .rounded_full()
                        .map(|d| match look {
                            _ if selected => d.bg(theme.cal_selected_text),
                            Look::Normal => d.bg(accent),
                            Look::Pending | Look::Declined => d.border_1().border_color(accent),
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(muted)
                        .child(pill_time(occurrence.start.time())),
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
            .pl(px(if clipped_left { 0. } else { 4. }))
            .pr(px(if clipped_right { 0. } else { 5. }))
            .child(
                pill.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.click_occurrence(occurrence, window, cx)
                    })),
            )
            .into_any_element()
    }
}

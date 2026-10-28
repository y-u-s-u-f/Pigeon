//! The Day view: the time grid, no wider than reads well, and beside it a
//! month to jump from and the day's events as a list.

use chrono::{Duration, NaiveDate, NaiveDateTime};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, StatefulInteractiveElement as _, Styled, Window,
    component::scroll::ScrollableElement as _, div, prelude::FluentBuilder as _, px,
};

use super::{CalendarView, ViewKind, occurrence_key, week_view::Look};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    calendar::{
        Occurrence,
        time::{compact_time_label, time_range_label},
    },
    theme::Theme,
    ui::{icon, is_secondary, keys_caps},
};

/// What the Day list says of an occurrence's time on `date`: its times, or
/// for one across days "From 9 AM" on the first, "Until 5 PM" on the last
/// and "All day" between (ME-089).
fn when_on(date: NaiveDate, all_day: bool, start: NaiveDateTime, end: NaiveDateTime) -> String {
    let starts = start.date() == date;
    let ends = (end - Duration::minutes(1)).date() == date;
    match (all_day, starts, ends) {
        (true, _, _) | (false, false, false) => "All day".into(),
        (false, true, true) => time_range_label(start.time(), end.time()),
        (false, true, false) => format!("From {}", compact_time_label(start.time())),
        (false, false, true) => format!("Until {}", compact_time_label(end.time())),
    }
}

/// The widest the Day view's event column gets.
pub const DAY_COLUMN_MAX: f32 = 640.;
/// The narrowest the area beside it is worth showing at.
pub(crate) const SIDE_MIN: f32 = 280.;
const SIDE_MAX: f32 = 400.;

impl CalendarView {
    /// The width the grid has: beside the rail, and beside a docked panel
    /// (`lay_out_grid`).
    pub(crate) fn grid_area_width(&self) -> f32 {
        self.grid_width
    }

    /// Whether the Day view has room for its month and list.
    pub(crate) fn day_side_shown(&self) -> bool {
        self.view == ViewKind::Day && self.day_side
    }

    pub(crate) fn render_day(
        &self,
        occurrences: &[Occurrence],
        now: NaiveDateTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let grid = self.render_time_grid(vec![self.anchor], occurrences, now, window, cx);
        if !self.day_side_shown() {
            return grid;
        }
        let theme = Theme::of(cx).clone();
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .w(px(super::week_view::gutter_width(window) + DAY_COLUMN_MAX))
                    .flex_none()
                    .h_full()
                    .child(grid),
            )
            .child(
                div()
                    .id("day-side")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .border_l_1()
                    .border_color(theme.cal_line)
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .max_w(px(SIDE_MAX))
                            .px(SPACE_XL)
                            .py(SPACE_LG)
                            .flex()
                            .flex_col()
                            .gap(SPACE_XL)
                            .child(self.render_mini_month("day-side-day", &theme, cx))
                            .child(self.render_agenda(self.anchor, occurrences, now, &theme, cx)),
                    ),
            )
            .into_any_element()
    }

    /// The day's events, all-day ones first.
    fn render_agenda(
        &self,
        date: NaiveDate,
        occurrences: &[Occurrence],
        now: NaiveDateTime,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut events: Vec<&Occurrence> = occurrences.iter().collect();
        events.sort_by_key(|o| (!self.data.events[o.event].is_spanning(), o.start));
        let count = events.len();
        let rows = events.into_iter().map(|&o| {
            let event = &self.data.events[o.event];
            let selected = self.is_selected(&o);
            let past = o.end <= now;
            let color = self.data.color_of(event);
            let look = Look::of(event.my_status(&self.data.account));
            let accent = theme.event_accent(color, past);
            let when = when_on(date, event.all_day, o.start, o.end);
            div()
                .id(("agenda", occurrence_key(&o, 0)))
                .role(gpui::Role::Button)
                .aria_label(self.occurrence_label(&o))
                .aria_selected(selected)
                .when(selected, |d| d.aria_active_descendant())
                .px(SPACE_SM)
                .py(SPACE_SM)
                .flex()
                .gap(SPACE_MD)
                .rounded(RADIUS_MD)
                .when(look == Look::Declined, |d| d.opacity(0.6))
                .map(|d| {
                    if selected {
                        d.bg(theme.selected_bg)
                    } else {
                        d.hover(|s| s.bg(theme.hover_bg))
                    }
                })
                // Pending answers outline the stripe, as blocks are (ME-041).
                .child(
                    div()
                        .w(super::week_view::STRIPE_WIDTH)
                        .flex_none()
                        .rounded_full()
                        .map(|d| {
                            if look.outlined() {
                                d.border_1().border_color(accent)
                            } else {
                                d.bg(accent)
                            }
                        }),
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
                                .truncate()
                                .text_size(TEXT_BASE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if past { theme.text_muted } else { theme.text })
                                .when(look == Look::Declined, |d| d.line_through())
                                .child(event.display_title().to_string()),
                        )
                        .child(
                            div()
                                .text_size(TEXT_SM)
                                .text_color(theme.text_muted)
                                .child(when),
                        )
                        .when(!event.location.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(SPACE_XS)
                                    .text_size(TEXT_SM)
                                    .text_color(theme.text_faint)
                                    .child(icon(Icon::MapPin, ICON_SM, theme.text_faint))
                                    .child(div().truncate().child(event.location.clone())),
                            )
                        }),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        if is_secondary(event) {
                            this.event_menu(o, event.position, window, cx);
                        }
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.event_menu(o, event.position, window, cx);
                    }),
                )
                .on_click(
                    cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                        if !event.is_right_click() && !event.modifiers().control {
                            this.click_occurrence(o, window, cx)
                        }
                    }),
                )
        });

        div()
            .flex()
            .flex_col()
            .gap(SPACE_2XS)
            .child(
                div()
                    .px(SPACE_SM)
                    .pb(SPACE_XS)
                    .flex()
                    .items_baseline()
                    .gap(SPACE_SM)
                    .child(
                        div()
                            .text_size(TEXT_BASE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(date.format("%A").to_string()),
                    )
                    .child(div().text_size(TEXT_SM).text_color(theme.text_muted).child(
                        match count {
                            0 => "No events".to_string(),
                            1 => "1 event".to_string(),
                            n => format!("{n} events"),
                        },
                    )),
            )
            .children(rows)
            .when(count == 0, |d| {
                d.child(
                    div()
                        .px(SPACE_SM)
                        .flex()
                        .items_center()
                        .gap(SPACE_XS)
                        .text_size(TEXT_SM)
                        .text_color(theme.text_muted)
                        .child("Press")
                        .child(keys_caps("c", theme))
                        .child("to add one"),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timed_event_across_days_reads_from_until_and_all_day() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let at = |d, h| day(d).and_hms_opt(h, 0, 0).unwrap();
        let (start, end) = (at(2, 9), at(4, 17));
        // The times follow the Mac's 12- or 24-hour clock.
        assert!(when_on(day(2), false, start, end).starts_with("From "));
        assert_eq!(when_on(day(3), false, start, end), "All day");
        assert!(when_on(day(4), false, start, end).starts_with("Until "));
        assert_eq!(when_on(day(2), true, at(2, 0), at(3, 0)), "All day");
    }
}

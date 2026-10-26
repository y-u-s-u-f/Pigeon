//! The Day view: the time grid, no wider than reads well, and beside it a
//! month to jump from and the day's events as a list.

use chrono::NaiveDate;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
};

use super::{CalendarView, ViewKind, event_panel::PANEL_WIDTH, now, occurrence_key};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    calendar::{Occurrence, time::time_range_label},
    theme::Theme,
    ui::{icon, keycap},
};

/// The widest the Day view's event column gets.
pub const DAY_COLUMN_MAX: f32 = 640.;
/// The narrowest the area beside it is worth showing at.
const SIDE_MIN: f32 = 280.;
const SIDE_MAX: f32 = 400.;

impl CalendarView {
    /// The width the grid has: beside the rail, and beside a docked panel.
    pub(crate) fn grid_area_width(&self, window: &Window) -> f32 {
        let panel = if self.panel.is_some() && self.panel_docked(window) {
            PANEL_WIDTH
        } else {
            0.
        };
        Self::content_width(window) - panel
    }

    /// Whether the Day view has room for its month and list.
    pub(crate) fn day_side_shown(&self, window: &Window) -> bool {
        self.view == ViewKind::Day
            && self.grid_area_width(window) - super::week_view::GUTTER - DAY_COLUMN_MAX >= SIDE_MIN
    }

    pub(crate) fn render_day(
        &self,
        occurrences: &[Occurrence],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let grid = self.render_time_grid(vec![self.anchor], occurrences, window, cx);
        if !self.day_side_shown(window) {
            return grid;
        }
        let theme = Theme::of(cx).clone();
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .w(px(super::week_view::GUTTER + DAY_COLUMN_MAX))
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
                    .overflow_y_scroll()
                    .child(
                        div()
                            .max_w(px(SIDE_MAX))
                            .px(SPACE_XL)
                            .py(SPACE_LG)
                            .flex()
                            .flex_col()
                            .gap(SPACE_XL)
                            .child(self.render_mini_month("day-side-day", &theme, cx))
                            .child(self.render_agenda(self.anchor, occurrences, &theme, cx)),
                    ),
            )
            .into_any_element()
    }

    /// The day's events, all-day ones first.
    fn render_agenda(
        &self,
        date: NaiveDate,
        occurrences: &[Occurrence],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut events: Vec<&Occurrence> = occurrences.iter().collect();
        events.sort_by_key(|o| (!self.data.events[o.event].is_spanning(), o.start));
        let count = events.len();
        let rows = events.into_iter().map(|&o| {
            let event = &self.data.events[o.event];
            let selected = self.is_selected(&o);
            let past = o.end <= now();
            let color = self.data.color_of(event);
            let when = if event.is_spanning() {
                "All day".to_string()
            } else {
                time_range_label(o.start.time(), o.end.time())
            };
            div()
                .id(("agenda", occurrence_key(&o, 0)))
                .role(gpui::Role::Button)
                .aria_label(self.occurrence_label(&o))
                .aria_selected(selected)
                .px(SPACE_SM)
                .py(SPACE_SM)
                .flex()
                .gap(SPACE_MD)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .map(|d| {
                    if selected {
                        d.bg(theme.selected_bg)
                    } else {
                        d.hover(|s| s.bg(theme.hover_bg))
                    }
                })
                .child(
                    div()
                        .w(px(3.))
                        .flex_none()
                        .rounded_full()
                        .bg(theme.event_accent(color, past)),
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
                                    .child(icon(Icon::MapPin, px(11.), theme.text_faint))
                                    .child(div().truncate().child(event.location.clone())),
                            )
                        }),
                )
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.click_occurrence(o, window, cx)),
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
                        .child(keycap("C", theme))
                        .child("to add one"),
                )
            })
            .into_any_element()
    }
}

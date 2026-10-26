//! The Day and Week views: a header per day, a lane of all-day and
//! multi-day events, and the scrolling time grid with events side by side
//! where they overlap.

use chrono::{Datelike, Duration, NaiveDate, Timelike};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, canvas,
    component::{scroll::ScrollableElement as _, tooltip::Tooltip},
    div,
    prelude::FluentBuilder as _,
    px, relative,
};

use super::{
    CalendarView, HOUR_HEIGHT, ViewKind, event_panel::PANEL_WIDTH, now, occurrence_key, today,
};
use crate::{
    app::RAIL_WIDTH,
    assets::Icon,
    calendar::{
        Occurrence, RsvpStatus,
        layout::{Placement, pack_columns, pack_lanes},
        time::{self, compact_time_label, is_weekend, time_range_label},
    },
    theme::Theme,
    ui::icon,
};

/// Width of the hour labels on the left.
pub const GUTTER: f32 = 58.;
const LANE_HEIGHT: f32 = 24.;
/// Events shorter than this are laid out as if they were this long, since
/// they're drawn that tall.
const MIN_LAYOUT_MINUTES: i64 = 25;
/// Events starting this much after another can be drawn over it: its
/// title line stays visible above them.
const CASCADE_GAP_MINUTES: i64 = 30;
/// How far each cascaded event steps in from the ones it's drawn over.
const CASCADE_INDENT: f32 = 14.;
const MAX_CASCADE_DEPTH: usize = 4;

fn indent(depth: usize) -> f32 {
    depth.min(MAX_CASCADE_DEPTH) as f32 * CASCADE_INDENT
}

/// How an event is drawn, from the user's answer to it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Normal,
    /// Not answered yet, or maybe: outlined.
    Pending,
    Declined,
}

impl Look {
    pub fn of(status: Option<RsvpStatus>) -> Self {
        match status {
            Some(RsvpStatus::NeedsAction | RsvpStatus::Tentative) => Look::Pending,
            Some(RsvpStatus::Declined) => Look::Declined,
            _ => Look::Normal,
        }
    }
}

impl CalendarView {
    pub(crate) fn render_time_grid(
        &self,
        days: Vec<NaiveDate>,
        occurrences: &[Occurrence],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let today = today();
        let n = days.len();
        // Roughly how wide a day is, to fit each event's text to its width.
        let panel = if self.panel.is_some() {
            PANEL_WIDTH
        } else {
            0.
        };
        let day_width =
            (f32::from(window.viewport_size().width) - f32::from(RAIL_WIDTH) - GUTTER - panel)
                / n as f32;
        let first = days[0];
        let day_ix = |date: NaiveDate| (date - first).num_days();

        // All-day and multi-day events, in lanes over the days they cover.
        let spanning: Vec<&Occurrence> = occurrences
            .iter()
            .filter(|o| self.data.events[o.event].is_spanning())
            .collect();
        let spans: Vec<(i64, i64)> = spanning
            .iter()
            .map(|o| {
                let last = (o.end - Duration::minutes(1)).date();
                (
                    day_ix(o.start.date()).max(0),
                    (day_ix(last) + 1).min(n as i64),
                )
            })
            .collect();
        let lanes = pack_lanes(&spans);
        let lane_count = lanes.iter().map(|l| l + 1).max().unwrap_or(0);
        let bars = spanning
            .iter()
            .zip(&spans)
            .zip(&lanes)
            .map(|((o, &(start, end)), &lane)| {
                self.render_bar(**o, start, end, n, lane, &theme, cx)
            })
            .collect::<Vec<_>>();

        let gutter = || div().w(px(GUTTER)).flex_none();
        let single = n == 1;

        let day_headers = days.iter().map(|&date| {
            let is_today = date == today;
            let weekend = is_weekend(date);
            div()
                .id(("day-header", date.num_days_from_ce() as usize))
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(1.))
                .map(|d| {
                    if single {
                        d.items_start().pl(px(14.))
                    } else {
                        d.items_center()
                    }
                })
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if is_today {
                            theme.accent
                        } else if weekend {
                            theme.text_faint
                        } else {
                            theme.text_muted
                        })
                        .child(date.format("%a").to_string().to_uppercase()),
                )
                .child(
                    div()
                        .id(("day-number", date.num_days_from_ce() as usize))
                        .size(px(32.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .text_size(px(19.))
                        .map(|d| {
                            if is_today {
                                d.bg(theme.accent)
                                    .text_color(theme.accent_text)
                                    .font_weight(FontWeight::SEMIBOLD)
                            } else {
                                d.text_color(if weekend {
                                    theme.text_muted
                                } else {
                                    theme.text
                                })
                                .font_weight(FontWeight::MEDIUM)
                                .when(!single, |d| d.hover(|s| s.bg(theme.hover_bg)))
                            }
                        })
                        .child(date.day().to_string())
                        .when(!single, |d| {
                            d.cursor_pointer().on_click(
                                cx.listener(move |this, _, _, cx| this.open_day(date, cx)),
                            )
                        }),
                )
        });

        let header = div()
            .h(px(62.))
            .flex_none()
            .flex()
            .child(gutter())
            .children(day_headers);

        let all_day = div()
            .flex_none()
            .flex()
            .border_b_1()
            .border_color(theme.cal_line)
            .child(
                gutter().pt(px(7.)).pr(px(8.)).flex().justify_end().child(
                    div()
                        .text_size(px(10.5))
                        .text_color(theme.text_faint)
                        .when(lane_count > 0, |d| d.child("all-day")),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .relative()
                    .h(px(
                        lane_count as f32 * LANE_HEIGHT + if lane_count > 0 { 8. } else { 4. }
                    ))
                    .children(bars),
            );

        // The time grid.
        let grid_height = px(24. * HOUR_HEIGHT);
        let now = now();
        let now_minutes = time::minutes_into(today, now);
        let today_visible = days.contains(&today);

        let hour_labels = (1..24).filter_map(|hour| {
            // Make room for the current time's label.
            let minutes = hour * 60;
            if today_visible && (minutes - now_minutes).abs() < 14 {
                return None;
            }
            Some(
                div()
                    .absolute()
                    .top(px(hour as f32 * HOUR_HEIGHT - 7.))
                    .right(px(9.))
                    .text_size(px(10.5))
                    .text_color(theme.text_faint)
                    .child(compact_time_label(
                        chrono::NaiveTime::from_hms_opt(hour as u32, 0, 0).unwrap(),
                    )),
            )
        });

        let lines = (1..48).map(|half| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(half as f32 * HOUR_HEIGHT / 2.))
                .h(px(1.))
                .bg(if half % 2 == 0 {
                    theme.cal_line
                } else {
                    theme.cal_line_faint
                })
        });

        let columns = days.iter().map(|&date| {
            let day_start = time::midnight(date);
            let day_end = day_start + Duration::days(1);
            let timed: Vec<&Occurrence> = occurrences
                .iter()
                .filter(|o| {
                    !self.data.events[o.event].is_spanning()
                        && o.start < day_end
                        && o.end > day_start
                })
                .collect();
            let minutes: Vec<(i64, i64)> = timed
                .iter()
                .map(|o| {
                    let start = time::minutes_into(date, o.start).max(0);
                    let end = time::minutes_into(date, o.end).min(24 * 60);
                    (start, end.max(start))
                })
                .collect();
            let layout_spans: Vec<(i64, i64)> = minutes
                .iter()
                .map(|&(s, e)| (s, e.max(s + MIN_LAYOUT_MINUTES)))
                .collect();
            let placements = pack_columns(&layout_spans, CASCADE_GAP_MINUTES);
            // Later starts are drawn last, on top of the ones they cascade over.
            let mut order: Vec<usize> = (0..timed.len()).collect();
            order.sort_by_key(|&i| (minutes[i].0, placements[i].depth));
            // Where the first event drawn over each one starts: its text
            // stops above that.
            let covered_at = |i: usize| {
                let (start, end) = layout_spans[i];
                let p = placements[i];
                (0..timed.len())
                    .filter(|&j| {
                        let q = placements[j];
                        let (s, e) = layout_spans[j];
                        s > start
                            && s < end
                            && e > start
                            && q.col < p.col + p.span
                            && p.col < q.col + q.span
                    })
                    .map(|j| layout_spans[j].0)
                    .min()
            };
            let blocks = order
                .into_iter()
                .map(|i| {
                    let placement = placements[i];
                    let (start, end) = minutes[i];
                    let width = day_width * placement.span as f32 / placement.cols as f32
                        - indent(placement.depth);
                    let visible = covered_at(i).map_or(end, |at| at.min(end)) - start;
                    let ends_here = time::minutes_into(date, timed[i].end) <= 24 * 60;
                    self.render_block(
                        *timed[i],
                        (start, end, visible),
                        ends_here,
                        placement,
                        width,
                        &theme,
                        cx,
                    )
                })
                .collect::<Vec<_>>();

            let is_today = date == today;
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .relative()
                .border_l_1()
                .border_color(theme.cal_line)
                .when(is_weekend(date) && !single, |d| d.bg(theme.cal_weekend_bg))
                .when(is_today && !single, |d| d.bg(theme.cal_today_bg))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event, window, cx| this.press_grid(event, window, cx)),
                )
                .children(blocks)
                .children(self.render_drag_ghost(date, &theme))
                .when(is_today, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(-1.))
                            .right_0()
                            .top(px(now_minutes as f32 / 60. * HOUR_HEIGHT - 1.))
                            .h(px(2.))
                            .bg(theme.cal_now)
                            .child(
                                div()
                                    .absolute()
                                    .left(px(-5.))
                                    .top(px(-4.))
                                    .size(px(10.))
                                    .rounded_full()
                                    .bg(theme.cal_now),
                            ),
                    )
                })
        });

        let now_label = today_visible.then(|| {
            div()
                .absolute()
                .right(px(5.))
                .top(px(now_minutes as f32 / 60. * HOUR_HEIGHT - 8.))
                .h(px(16.))
                .px(px(4.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .bg(theme.cal_now)
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.cal_selected_text)
                .child(now.format("%-I:%M").to_string())
        });

        let grid = div()
            .h(grid_height)
            .flex()
            .child(
                gutter()
                    .relative()
                    .h_full()
                    .children(hour_labels)
                    .children(now_label),
            )
            .child({
                let bounds = self.grid_bounds.clone();
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .relative()
                    .flex()
                    .children(lines)
                    .children(columns)
                    .child(
                        canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    // A release before the drag layer is up.
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.finish_drag(window, cx)),
                    )
            });

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(all_day)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("time-grid")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.grid_scroll)
                            .child(grid),
                    )
                    .vertical_scrollbar(&self.grid_scroll),
            )
            .into_any_element()
    }

    /// A timed event in its day's column, `start`..`end` minutes into it,
    /// with its first `visible` minutes not covered by events drawn over it.
    #[allow(clippy::too_many_arguments)]
    fn render_block(
        &self,
        occurrence: Occurrence,
        (start, end, visible): (i64, i64, i64),
        ends_here: bool,
        placement: Placement,
        width: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let event = &self.data.events[occurrence.event];
        let color = self.data.color_of(event);
        let selected = self.is_selected(&occurrence);
        let past = occurrence.end <= now();
        let look = Look::of(event.my_status(self.me()));
        let height = ((end - start) as f32 / 60. * HOUR_HEIGHT - 2.).max(18.);
        let compact = height < 34.;
        // Narrow blocks give the title the room; the block's place already
        // says when it is.
        let narrow = width < 110.;
        let roomy = width > 260.;
        // Drawn over other events: opaque, so theirs don't show through.
        let cascaded = placement.depth > 0;

        let (fill, text, muted) = if selected {
            let text = theme.cal_selected_text;
            (theme.event_selected_fill(color), text, text.alpha(0.82))
        } else {
            let text = theme.event_text(color, past);
            let fill = match look {
                Look::Normal => theme.event_fill(color, past),
                Look::Pending | Look::Declined => theme.event_fill(color, true).alpha(0.25),
            };
            (fill, text, text.alpha(if past { 0.5 } else { 0.75 }))
        };
        let fill = if cascaded {
            theme.overlay_bg.alpha(1.).blend(fill)
        } else {
            fill
        };
        let accent = theme.event_accent(color, past);
        let time = time_range_label(occurrence.start.time(), occurrence.end.time());
        let title = event.display_title().to_string();
        let tooltip: SharedString = format!("{title}\n{time}").into();

        let content = if compact {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .overflow_hidden()
                .child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text)
                        .when(look == Look::Declined, |d| d.line_through())
                        .child(title),
                )
                .when(!narrow, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .text_color(muted)
                            .child(compact_time_label(occurrence.start.time())),
                    )
                })
        } else {
            let visible_height = (visible as f32 / 60. * HOUR_HEIGHT - 2.).min(height);
            let lines = ((visible_height - 8.) / 15.).floor().max(1.) as usize;
            let show_time = lines >= 3 || (lines == 2 && !narrow);
            // Too narrow to wrap without breaking words: one line.
            let max_lines = if width < 80. {
                1
            } else if narrow {
                5
            } else {
                3
            };
            let title_lines = if show_time { lines - 1 } else { lines }.min(max_lines);
            div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text)
                        .line_height(px(15.))
                        .line_clamp(title_lines)
                        .text_ellipsis()
                        .when(look == Look::Declined, |d| d.line_through())
                        .child(title),
                )
                .when(show_time, |d| {
                    d.child(div().truncate().text_color(muted).child(time))
                })
                .when(
                    show_time && !event.location.is_empty() && lines >= title_lines + 2,
                    |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .text_color(muted)
                                .child(icon(Icon::MapPin, px(11.), muted))
                                .child(div().truncate().child(event.location.clone())),
                        )
                    },
                )
                .when(
                    roomy && height >= 100. && !event.description.is_empty(),
                    |d| {
                        d.child(
                            div()
                                .mt(px(3.))
                                .text_color(muted)
                                .line_clamp(2)
                                .child(event.description.clone()),
                        )
                    },
                )
        };

        let col = placement.col as f32;
        let cols = placement.cols as f32;
        let span = placement.span as f32;
        let hover = theme.event_fill_hover(color);
        let hover = if cascaded {
            theme.overlay_bg.alpha(1.).blend(hover)
        } else {
            hover
        };
        let key = occurrence_key(&occurrence, (start == 0) as i64);
        div()
            .absolute()
            .top(px(start as f32 / 60. * HOUR_HEIGHT + 1.))
            .left(relative(col / cols))
            .w(relative(span / cols))
            .h(px(height))
            .pl(px(2. + indent(placement.depth)))
            .pr(px(if col + span >= cols { 6. } else { 1. }))
            .when(self.is_dragged(&occurrence), |d| d.opacity(0.35))
            .child(
                div()
                    .id((
                        "event",
                        // The part after midnight of one that runs past it
                        // is drawn on the next day.
                        key,
                    ))
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .rounded(px(6.))
                    .bg(fill)
                    .pl(px(8.))
                    .pr(px(5.))
                    .py(px(if compact { 0. } else { 4. }))
                    .when(compact, |d| d.flex().items_center())
                    .text_size(px(11.5))
                    .line_height(px(15.))
                    .cursor_pointer()
                    .map(|d| match look {
                        _ if selected => d.shadow(vec![gpui::BoxShadow {
                            color: theme.overlay_shadow.alpha(0.35),
                            offset: gpui::point(px(0.), px(3.)),
                            blur_radius: px(10.),
                            spread_radius: px(-2.),
                            inset: false,
                        }]),
                        Look::Normal => d
                            .hover(move |s| s.bg(hover))
                            .when(cascaded, |d| d.border_1().border_color(theme.cal_ring)),
                        Look::Pending | Look::Declined => {
                            d.border_1().border_color(accent.alpha(0.7))
                        }
                    })
                    .when(!selected, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(px(3.))
                                .bg(accent),
                        )
                    })
                    .child(content)
                    .when(ends_here, |d| {
                        d.child(
                            div()
                                .id(("event-resize", key))
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .h(px(if compact { 4. } else { 6. }))
                                .cursor_ns_resize()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event, window, cx| {
                                        cx.stop_propagation();
                                        this.press_occurrence(occurrence, true, event, window, cx)
                                    }),
                                ),
                        )
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, window, cx| {
                            cx.stop_propagation();
                            this.press_occurrence(occurrence, false, event, window, cx)
                        }),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)),
            )
            .into_any_element()
    }

    /// An all-day or multi-day event in the lane above the grid, over days
    /// `start..end` of `days`.
    #[allow(clippy::too_many_arguments)]
    fn render_bar(
        &self,
        occurrence: Occurrence,
        start: i64,
        end: i64,
        days: usize,
        lane: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let event = &self.data.events[occurrence.event];
        let color = self.data.color_of(event);
        let selected = self.is_selected(&occurrence);
        let past = occurrence.end <= now();
        let (fill, text) = if selected {
            (theme.event_selected_fill(color), theme.cal_selected_text)
        } else {
            (theme.event_fill(color, past), theme.event_text(color, past))
        };
        let hover = theme.event_fill_hover(color);
        let first_day = time::midnight(self.visible_range().0);
        let clipped_left = occurrence.start < first_day;
        let clipped_right = occurrence.end > first_day + Duration::days(days as i64);
        let time = (!event.all_day
            && occurrence.start.time().hour() + occurrence.start.minute() > 0)
            .then(|| compact_time_label(occurrence.start.time()));
        let single_view = self.view == ViewKind::Day;

        div()
            .absolute()
            .top(px(4. + lane as f32 * LANE_HEIGHT))
            .left(relative(start as f32 / days as f32))
            .w(relative((end - start) as f32 / days as f32))
            .h(px(LANE_HEIGHT - 3.))
            .pl(px(if clipped_left { 0. } else { 2. }))
            .pr(px(if clipped_right { 0. } else { 3. }))
            .child(
                div()
                    .id(("bar", occurrence_key(&occurrence, 0)))
                    .size_full()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .overflow_hidden()
                    .rounded(px(5.))
                    .when(clipped_left, |d| d.rounded_l(px(0.)))
                    .when(clipped_right, |d| d.rounded_r(px(0.)))
                    .bg(fill)
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .text_size(px(11.5))
                    .cursor_pointer()
                    .when(single_view || !clipped_left, |d| {
                        d.children(
                            time.map(|time| {
                                div().flex_none().text_color(text.alpha(0.75)).child(time)
                            }),
                        )
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(text)
                            .child(event.display_title().to_string()),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.click_occurrence(occurrence, window, cx)
                    })),
            )
            .into_any_element()
    }
}

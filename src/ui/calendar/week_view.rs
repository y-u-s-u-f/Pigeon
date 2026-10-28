//! The Day and Week views: a header per day, a lane of all-day and
//! multi-day events, and the scrolling time grid with events side by side
//! where they overlap. Declined events take no column: they're drawn faded
//! behind the rest (ME-042).

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, SharedString, StatefulInteractiveElement as _, Styled, Window,
    canvas, component::scroll::ScrollableElement as _, div, prelude::FluentBuilder as _, px,
    relative,
};

use super::{CalendarView, HOUR_HEIGHT, ViewKind, agenda::DAY_COLUMN_MAX, drag, occurrence_key};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    calendar::{
        Occurrence, RsvpStatus,
        layout::{column_box, pack_columns, pack_lanes},
        policy,
        time::{self, compact_time_label, is_weekend, time_range_label},
    },
    theme::Theme,
    ui::{
        controls::bar_colors,
        icon, icon_button_small, is_secondary,
        motion::{Rise, enter},
        tooltip,
    },
};

/// Width of the hour labels on the left at a text scale of 1.
const GUTTER: f32 = 58.;

/// The hour labels' width at the window's text scale: they hold text, so
/// they grow with it (MA-021).
pub fn gutter_width(window: &Window) -> f32 {
    GUTTER * f32::from(window.rem_size()) / BASE_REM
}
const LANE_HEIGHT: f32 = 24.;
/// The gap under and after an all-day bar in its lane.
const LANE_GAP: f32 = 3.;
/// The calendar color's stripe on an event's leading edge.
pub const STRIPE_WIDTH: gpui::Pixels = px(3.);
/// The grab area at a timed event's bottom edge.
const RESIZE_HANDLE: gpui::Pixels = px(6.);
/// The day number's circle in the column headers.
const DAY_NUMBER: gpui::Rems = type_size(32.);
const DAY_HEADER_HEIGHT: gpui::Rems = type_size(62.);
/// The dot at the start of the now line, and the time label on it.
const NOW_DOT: gpui::Pixels = px(10.);
const NOW_LABEL: gpui::Rems = type_size(16.);
/// The all-day lanes shown before the rest fold into "+N" (ME-040).
const MAX_LANES: usize = 3;
/// Events shorter than this are laid out as if they were this long, since
/// they're drawn that tall.
const MIN_LAYOUT_MINUTES: i64 = 25;
/// Side-by-side events narrower than this overlap a little instead.
const MIN_COLUMN: f32 = 60.;
/// The strip kept free on the right of each day, to drag-create beside a
/// full-width event (ME-053): a tenth of the day, within these bounds.
const FREE_STRIP: (f32, f32) = (8., 16.);
/// How long before its start a meeting offers Join on its block.
const JOIN_LEAD_MINUTES: i64 = 10;
/// The size of text in blocks, in rems, and its line height: block layout
/// reads them with the window's rem size (ME-039).
const BLOCK_TEXT_REMS: f32 = 12. / BASE_REM;
const BLOCK_LINE: f32 = 1.25;
/// About how wide a character of block text is, per point of its size.
const CHAR_WIDTH: f32 = 0.54;

/// How an event is drawn, from the user's answer to it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Normal,
    /// Not answered yet: outlined.
    Pending,
    /// Maybe: outlined, with faint stripes (ME-051).
    Tentative,
    Declined,
}

impl Look {
    pub fn of(status: Option<RsvpStatus>) -> Self {
        match status {
            Some(RsvpStatus::NeedsAction) => Look::Pending,
            Some(RsvpStatus::Tentative) => Look::Tentative,
            Some(RsvpStatus::Declined) => Look::Declined,
            _ => Look::Normal,
        }
    }

    /// Outlined over a light fill: not answered, or maybe.
    pub fn outlined(self) -> bool {
        matches!(self, Look::Pending | Look::Tentative)
    }

    /// A bar or a pill as answered: pending outlined over a light fill,
    /// declined faded (ME-041). The title strikes through separately, and
    /// a maybe's stripes are `stripes`.
    pub fn bar<E: Styled>(self, el: E, fill: Hsla, accent: Hsla, theme: &Theme) -> E {
        match self {
            Look::Normal => el.bg(fill),
            Look::Pending | Look::Tentative => el
                .bg(theme.event_answer_fill(fill))
                .border_1()
                .border_color(accent.alpha(0.7)),
            Look::Declined => el.bg(theme.event_answer_fill(fill)).opacity(0.6),
        }
    }

    /// A maybe's faint diagonal stripes over its block (ME-051).
    pub fn stripes(self, accent: Hsla) -> Option<gpui::Div> {
        (self == Look::Tentative).then(|| {
            div()
                .absolute()
                .inset_0()
                .bg(gpui::pattern_slash(accent.alpha(0.16), 1., 6.))
        })
    }
}

/// Line height and character width of block text at the window's rem size.
#[derive(Clone, Copy)]
struct TextMetrics {
    line: f32,
    char: f32,
}

impl TextMetrics {
    fn of(window: &Window) -> Self {
        let size = f32::from(window.rem_size()) * BLOCK_TEXT_REMS;
        TextMetrics {
            line: size * BLOCK_LINE,
            char: size * CHAR_WIDTH,
        }
    }
}

/// A block's place in its day: `left` and `width` across a day `day`
/// wide.
#[derive(Clone, Copy)]
struct BlockBox {
    left: f32,
    width: f32,
    day: f32,
}

impl CalendarView {
    pub(crate) fn render_time_grid(
        &self,
        days: Vec<NaiveDate>,
        occurrences: &[Occurrence],
        now: NaiveDateTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let today = now.date();
        let metrics = TextMetrics::of(window);
        // All-day lanes hold text, so they grow with it (MA-021).
        let scale = f32::from(window.rem_size()) / BASE_REM;
        let lane_height = LANE_HEIGHT * scale;
        let gutter_px = GUTTER * scale;
        let now_label_height = NOW_LABEL.to_pixels(window.rem_size());
        let n = days.len();
        // How wide a day is, to lay events out and fit their text.
        let day_width = if self.day_side_shown() {
            DAY_COLUMN_MAX
        } else {
            (self.grid_area_width() - gutter_px) / n as f32
        } - 1.;
        let first = days[0];
        let first_day = time::midnight(first);
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
        // A busy week folds its lanes past the first few into "+N" per day;
        // a selected bar in them unfolds them, so the keyboard reaches it.
        let selected_hidden = spanning
            .iter()
            .zip(&lanes)
            .any(|(o, &lane)| lane + 1 >= MAX_LANES && self.is_selected(o));
        let folds = lane_count > MAX_LANES;
        let expanded = !folds || self.all_day_expanded || selected_hidden;
        let shown_lanes = if expanded { lane_count } else { MAX_LANES - 1 };
        let hidden: Vec<usize> = (0..n as i64)
            .map(|day| {
                spans
                    .iter()
                    .zip(&lanes)
                    .filter(|&(&(s, e), &lane)| lane >= shown_lanes && s <= day && day < e)
                    .count()
            })
            .collect();
        let rows = if expanded { lane_count } else { MAX_LANES };
        let bars = spanning
            .iter()
            .zip(&spans)
            .zip(&lanes)
            .filter(|&(_, &lane)| lane < shown_lanes)
            .map(|((o, &(start, end)), &lane)| {
                self.render_bar(
                    **o,
                    (start, end),
                    n,
                    first_day,
                    (lane, lane_height),
                    now,
                    &theme,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let more = hidden
            .iter()
            .enumerate()
            .filter(|&(_, &count)| count > 0)
            .map(|(day, &count)| {
                div()
                    .absolute()
                    .top(px(f32::from(SPACE_XS) + shown_lanes as f32 * lane_height))
                    .left(relative(day as f32 / n as f32))
                    .w(relative(1. / n as f32))
                    .h(px(lane_height - LANE_GAP))
                    .px(SPACE_2XS)
                    .child(
                        div()
                            .id(("all-day-more", day))
                            .role(gpui::Role::Button)
                            .aria_label(format!("{count} more all-day events"))
                            .size_full()
                            .px(SPACE_SM)
                            .flex()
                            .items_center()
                            .rounded(RADIUS_SM)
                            .text_size(TEXT_SM)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text_muted)
                            .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                            .child(format!("+{count} more"))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.all_day_expanded = true;
                                cx.notify();
                            })),
                    )
            })
            .collect::<Vec<_>>();

        let gutter = || div().w(px(gutter_px)).flex_none();
        let single = n == 1;

        let day_headers = days.iter().map(|&date| {
            let is_today = date == today;
            let is_anchor = date == self.anchor;
            let weekend = is_weekend(date);
            div()
                .id(("day-header", date.num_days_from_ce() as usize))
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .justify_center()
                .gap(SPACE_2XS)
                .map(|d| {
                    if single {
                        d.items_start().pl(SPACE_LG)
                    } else {
                        d.items_center()
                    }
                })
                .child(
                    div()
                        .text_size(TEXT_XS)
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
                        .size(DAY_NUMBER)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .text_size(TEXT_XL)
                        .role(gpui::Role::Button)
                        .aria_label(format!("Open {}", super::header::full_date(date)))
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
                                // The selected day, where left and right move.
                                .when(is_anchor, |d| {
                                    d.bg(theme.selected_bg).font_weight(FontWeight::SEMIBOLD)
                                })
                                .hover(|s| s.bg(theme.hover_bg))
                            }
                        })
                        .child(date.day().to_string())
                        .on_click(cx.listener(move |this, _, _, cx| this.open_day(date, cx))),
                )
        });

        let week_number = self.show_week_numbers.then(|| {
            div()
                .text_size(TEXT_XS)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_faint)
                .child(crate::calendar::time::week_label(first))
        });
        // The Day view's title already names the day.
        let header = (!single).then(|| {
            div()
                .h(DAY_HEADER_HEIGHT)
                .flex_none()
                .flex()
                .child(
                    gutter()
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(week_number),
                )
                .children(day_headers)
        });

        // Under the bars, one slot per day: a double click there makes an
        // all-day event on it (ME-044), a click clears the selection.
        let lane_days = days.iter().map(|&date| {
            div()
                .id(("all-day-slot", date.num_days_from_ce() as usize))
                .flex_1()
                .h_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.press_day(date, event, window, cx)
                    }),
                )
        });
        let fold_toggle = folds.then(|| {
            let (glyph, label) = if expanded {
                (Icon::ChevronUp, "Show fewer all-day events")
            } else {
                (Icon::ChevronDown, "Show all all-day events")
            };
            icon_button_small("all-day-fold", glyph, label, &theme)
                .aria_expanded(expanded)
                .mr(-SPACE_XS)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.all_day_expanded = !this.all_day_expanded;
                    cx.notify();
                }))
        });
        let all_day = div()
            .flex_none()
            .flex()
            .border_b_1()
            .border_color(theme.cal_line)
            .child(
                gutter()
                    .pt(SPACE_XS)
                    .pr(SPACE_SM)
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(
                        div()
                            .pt(SPACE_XS)
                            .text_size(TEXT_XS)
                            .text_color(theme.text_faint)
                            .when(lane_count > 0, |d| d.child("All day")),
                    )
                    .children(fold_toggle),
            )
            .child(
                div()
                    .flex_1()
                    .relative()
                    .h(px(rows as f32 * lane_height
                        + if rows > 0 {
                            f32::from(SPACE_SM)
                        } else {
                            f32::from(SPACE_XS)
                        }))
                    .child(div().absolute().inset_0().flex().children(lane_days))
                    .children(bars)
                    .children(more),
            );

        // The time grid.
        let grid_height = px(24. * HOUR_HEIGHT);
        let now_minutes = time::minutes_into(today, now);
        let now_top = now_minutes as f32 / 60. * HOUR_HEIGHT;
        let today_visible = days.contains(&today);

        let hour_labels = (1..24).filter_map(|hour| {
            // Make room for the current time's label.
            let minutes = hour * 60;
            if today_visible && ((minutes - now_minutes).abs() as f32) < 14. * scale {
                return None;
            }
            Some(
                div()
                    .absolute()
                    .top(px(hour as f32 * HOUR_HEIGHT - 7. * scale))
                    .right(SPACE_SM)
                    .text_size(TEXT_XS)
                    .text_color(theme.text_faint)
                    .font_features(tabular_figures())
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

        // Room on the right of each day to drag-create beside a block.
        let free = (day_width * 0.1).clamp(FREE_STRIP.0, FREE_STRIP.1);
        let layout_width = (day_width - free).max(MIN_COLUMN);

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
                    let end = time::minutes_into(date, o.end).min(drag::DAY_MINUTES);
                    (start, end.max(start))
                })
                .collect();
            // Only the events the user goes to share the width.
            let declined: Vec<bool> = timed
                .iter()
                .map(|o| {
                    Look::of(self.data.events[o.event].my_status(&self.data.account))
                        == Look::Declined
                })
                .collect();
            let active: Vec<usize> = (0..timed.len()).filter(|&i| !declined[i]).collect();
            let layout_spans: Vec<(i64, i64)> = active
                .iter()
                .map(|&i| {
                    let (s, e) = minutes[i];
                    (s, e.max(s + MIN_LAYOUT_MINUTES))
                })
                .collect();
            let placements = pack_columns(&layout_spans);
            // Declined ones first, behind; then left to right, so where
            // narrow columns overlap, each is drawn over the one to its left.
            let mut order: Vec<(Option<usize>, usize)> = (0..timed.len())
                .filter(|&i| declined[i])
                .map(|i| (None, i))
                .collect();
            let mut placed: Vec<(usize, usize)> =
                (0..active.len()).map(|p| (p, active[p])).collect();
            placed.sort_by_key(|&(p, i)| (placements[p].col, minutes[i].0));
            order.extend(placed.into_iter().map(|(p, i)| (Some(p), i)));
            let blocks = order
                .into_iter()
                .map(|(placement, i)| {
                    let (left, width, overlapped) = match placement {
                        Some(p) => {
                            let placement = placements[p];
                            let (left, width) = column_box(placement, layout_width, MIN_COLUMN);
                            let overlapped = placement.cols > 1
                                && layout_width / (placement.cols as f32) < MIN_COLUMN;
                            (left, width, overlapped)
                        }
                        None => (0., layout_width, false),
                    };
                    // Read-only events have no resize edge (CAL-22).
                    let ends_here = time::minutes_into(date, timed[i].end) <= drag::DAY_MINUTES
                        && self.can_edit_ix(timed[i].event);
                    self.render_block(
                        *timed[i],
                        minutes[i],
                        ends_here,
                        BlockBox {
                            left,
                            width,
                            day: day_width,
                        },
                        overlapped,
                        now,
                        metrics,
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
                // A faint line at now across the other days of the week.
                .when(today_visible && !is_today, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(now_top))
                            .h(px(1.))
                            .bg(theme.cal_now.alpha(0.3)),
                    )
                })
                .children(blocks)
                .children(self.render_removed_block(date, &theme))
                .children(self.render_drag_ghost(date, &theme, window, cx))
                .when(is_today, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(-1.))
                            .right_0()
                            .top(px(now_top - 1.))
                            .h(px(2.))
                            .bg(theme.cal_now)
                            .child(
                                div()
                                    .absolute()
                                    .left(-NOW_DOT / 2.)
                                    .top(px(1.) - NOW_DOT / 2.)
                                    .size(NOW_DOT)
                                    .rounded_full()
                                    .bg(theme.cal_now),
                            ),
                    )
                })
        });

        let now_label = today_visible.then(|| {
            div()
                .absolute()
                .right(SPACE_XS)
                .top(px(now_top) - now_label_height / 2.)
                .h(NOW_LABEL)
                .px(SPACE_XS)
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .bg(theme.cal_now)
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.on_fill(theme.cal_now))
                .font_features(tabular_figures())
                .child(time::clock_label(now.time()))
        });

        // The zone the times are in, where midnight's label would be
        // (ME-094).
        let zone = div()
            .absolute()
            .top(SPACE_2XS)
            .right(SPACE_SM)
            .text_size(TEXT_XS)
            .text_color(theme.text_faint)
            .child(time::zone_offset_label(
                chrono::Local::now().offset().local_minus_utc(),
            ));
        let grid = div()
            .h(grid_height)
            .flex()
            .child(
                gutter()
                    .relative()
                    .h_full()
                    .child(zone)
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
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::drop_drag))
            });

        div()
            .size_full()
            .flex()
            .flex_col()
            .children(header)
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
                            // A sideways swipe pages rather than scrolls
                            // (MA-080).
                            .restrict_scroll_to_axis()
                            .track_scroll(&self.grid_scroll)
                            .child(grid),
                    )
                    .vertical_scrollbar(&self.grid_scroll),
            )
            .into_any_element()
    }

    /// A timed event in its day's column, `start`..`end` minutes into it,
    /// at `place`. `overlapped` blocks are drawn opaque, since their
    /// columns overlap.
    #[allow(clippy::too_many_arguments)]
    fn render_block(
        &self,
        occurrence: Occurrence,
        (start, end): (i64, i64),
        ends_here: bool,
        place: BlockBox,
        overlapped: bool,
        now: NaiveDateTime,
        metrics: TextMetrics,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let BlockBox { left, width, day } = place;
        let event = &self.data.events[occurrence.event];
        let color = self.data.color_of(event);
        let selected = self.is_selected(&occurrence);
        let past = occurrence.end <= now;
        let look = Look::of(event.my_status(&self.data.account));
        let height = ((end - start) as f32 / 60. * HOUR_HEIGHT - 2.).max(18.);
        // Two lines and the padding, at the text size in use.
        let compact = height < metrics.line * 2. + 4.;
        // Narrow blocks give the title the room; the block's place already
        // says when it is.
        let narrow = width < 110.;
        let roomy = width > 260.;
        // Drawn over other events: opaque, so theirs don't show through.

        let (fill, text, muted) = if selected {
            let fill = theme.event_selected_fill(color);
            let text = theme.on_fill(fill);
            (fill, text, theme.event_secondary(text))
        } else {
            let text = theme.event_text(color, past);
            let fill = match look {
                // Free time: a lighter fill (ME-051).
                Look::Normal if !event.busy => theme.event_free(theme.event_fill(color, past)),
                Look::Normal => theme.event_fill(color, past),
                _ => theme.event_answer_fill(theme.event_fill(color, true)),
            };
            (fill, text, theme.event_secondary(text))
        };
        let fill = if overlapped {
            theme.overlay_bg.alpha(1.).blend(fill)
        } else {
            fill
        };
        let accent = theme.event_accent(color, past);
        let time = time_range_label(occurrence.start.time(), occurrence.end.time());
        let title: SharedString = event.display_title().to_string().into();
        let tooltip_text: SharedString = format!("{title}\n{time}").into();
        // A meeting about to start or under way offers Join (ME-096).
        // The link search last: it reads the description.
        let join = (now >= occurrence.start - Duration::minutes(JOIN_LEAD_MINUTES)
            && now < occurrence.end
            && look != Look::Declined
            && !compact
            && width >= 90.)
            .then(|| policy::video_link(event))
            .flatten()
            .map(str::to_string);

        let start_label = compact_time_label(occurrence.start.time());
        let fits_time_beside = !narrow
            && (title.chars().count() + start_label.len() + 1) as f32 * metrics.char + 16. <= width;
        let content = if compact {
            div()
                .flex()
                .items_center()
                .gap(SPACE_XS)
                .overflow_hidden()
                .child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text)
                        .when(look == Look::Declined, |d| d.line_through())
                        .child(title.clone()),
                )
                // The time only when the title still fits beside it: the
                // title goes first.
                .when(fits_time_beside, |d| {
                    d.child(div().flex_none().text_color(muted).child(start_label))
                })
        } else {
            let lines = ((height - 8.) / metrics.line).floor().max(1.) as usize;
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
                .gap(SPACE_2XS)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text)
                        .line_height(LINE_UI)
                        .line_clamp(title_lines)
                        .text_ellipsis()
                        .when(look == Look::Declined, |d| d.line_through())
                        .when(join.is_some(), |d| d.pr(SPACE_2XL + SPACE_XL))
                        .child(title.clone()),
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
                                .gap(SPACE_XS)
                                .text_color(muted)
                                .child(icon(Icon::MapPin, ICON_SM, muted))
                                .child(div().truncate().child(event.location.clone())),
                        )
                    },
                )
                .when(
                    roomy && height >= 100. && !event.description.is_empty(),
                    |d| {
                        d.child(
                            div()
                                .mt(SPACE_XS)
                                .text_color(muted)
                                .line_clamp(2)
                                .child(event.description.clone()),
                        )
                    },
                )
        };

        let hover = theme.event_fill_hover(color);
        let hover = if overlapped {
            theme.overlay_bg.alpha(1.).blend(hover)
        } else {
            hover
        };
        let key = occurrence_key(&occurrence, (start == 0) as i64);
        let added = self.just_added.as_deref() == Some(event.id.as_str());
        let block = div()
            .absolute()
            .top(px(start as f32 / 60. * HOUR_HEIGHT + 1.))
            .left(relative(left / day))
            .w(relative(width / day))
            .h(px(height))
            .pl(SPACE_2XS)
            .pr(px(1.))
            .when_some(self.drag_opacity(&occurrence), |d, o| d.opacity(o))
            // Declined: behind the rest, faded.
            .when(look == Look::Declined && !selected, |d| d.opacity(0.6))
            .child(
                div()
                    .id((
                        "event",
                        // The part after midnight of one that runs past it
                        // is drawn on the next day.
                        key,
                    ))
                    .role(gpui::Role::Button)
                    .aria_label(self.occurrence_label(&occurrence))
                    .aria_selected(selected)
                    .when(selected, |d| d.aria_active_descendant())
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .rounded(RADIUS_SM)
                    .bg(fill)
                    .pl(SPACE_SM)
                    .pr(SPACE_XS)
                    .py(if compact { px(0.) } else { SPACE_XS })
                    .when(compact, |d| d.flex().items_center())
                    .text_size(TEXT_SM)
                    .line_height(LINE_UI)
                    // Overlapping blocks share pixels: only the top one
                    // gets hover, tooltip and clicks.
                    .block_mouse_except_scroll()
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
                            .when(overlapped, |d| d.border_1().border_color(theme.cal_ring)),
                        _ => d
                            .hover(move |s| s.bg(hover.alpha(0.5)))
                            .border_1()
                            .border_color(accent.alpha(0.7)),
                    })
                    .when(!selected, |d| d.children(look.stripes(accent)))
                    .when(!selected, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(STRIPE_WIDTH)
                                .bg(accent),
                        )
                    })
                    .child(content)
                    .children(join.map(|link| {
                        let label: SharedString = format!("Join {title}").into();
                        div()
                            .id(("event-join", key))
                            .absolute()
                            .top(SPACE_XS)
                            .right(SPACE_XS)
                            .h(CONTROL_XS)
                            .px(SPACE_SM)
                            .flex()
                            .items_center()
                            .gap(SPACE_XS)
                            .rounded(RADIUS_SM)
                            .bg(theme.accent)
                            .text_color(theme.accent_text)
                            .text_size(TEXT_XS)
                            .font_weight(FontWeight::SEMIBOLD)
                            .role(gpui::Role::Link)
                            .aria_label(label)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.9))
                            .child(icon(Icon::Video, ICON_SM, theme.accent_text))
                            .child("Join")
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| cx.open_url(&link))
                    }))
                    .when(ends_here, |d| {
                        d.child(
                            div()
                                .id(("event-resize", key))
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .h(if compact { SPACE_XS } else { RESIZE_HANDLE })
                                .cursor_ns_resize()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        if is_secondary(event) {
                                            return this.event_menu(
                                                occurrence,
                                                event.position,
                                                window,
                                                cx,
                                            );
                                        }
                                        this.press_occurrence(occurrence, true, event, window, cx)
                                    }),
                                ),
                        )
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if is_secondary(event) {
                                return this.event_menu(occurrence, event.position, window, cx);
                            }
                            this.press_occurrence(occurrence, false, event, window, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.event_menu(occurrence, event.position, window, cx)
                        }),
                    )
                    .tooltip(move |window, cx| tooltip(tooltip_text.clone()).build(window, cx)),
            );
        // Only the event just made or brought back fades in, keyed on its id
        // and this part's start, not its index, which a reload can shift
        // (MD-038).
        if added {
            let minutes = occurrence.start.and_utc().timestamp() / 60;
            let id = SharedString::from(format!("event-added-{}", event.id));
            let part = ((minutes << 1) | (start == 0) as i64) as usize;
            enter((id, part), Rise::None, block).into_any_element()
        } else {
            block.into_any_element()
        }
    }

    /// An all-day or multi-day event in the lane above the grid, over days
    /// `start..end` of `days` from `first_day`.
    #[allow(clippy::too_many_arguments)]
    fn render_bar(
        &self,
        occurrence: Occurrence,
        (start, end): (i64, i64),
        days: usize,
        first_day: NaiveDateTime,
        (lane, lane_height): (usize, f32),
        now: NaiveDateTime,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let event = &self.data.events[occurrence.event];
        let color = self.data.color_of(event);
        let selected = self.is_selected(&occurrence);
        let past = occurrence.end <= now;
        let look = Look::of(event.my_status(&self.data.account));
        let (fill, text, hover) = bar_colors(color, past, selected, theme);
        let accent = theme.event_accent(color, past);
        let clipped_left = occurrence.start < first_day;
        let clipped_right = occurrence.end > first_day + Duration::days(days as i64);
        let time = (!event.all_day
            && occurrence.start.time().hour() + occurrence.start.minute() > 0)
            .then(|| compact_time_label(occurrence.start.time()));
        // A timed event across days says when it ends on its last day.
        let until = (!event.all_day && !clipped_right)
            .then(|| format!("until {}", compact_time_label(occurrence.end.time())));
        let single_view = self.view == ViewKind::Day;
        let read_only = !self.can_edit_ix(occurrence.event);

        div()
            .absolute()
            .top(px(f32::from(SPACE_XS) + lane as f32 * lane_height))
            .left(relative(start as f32 / days as f32))
            .w(relative((end - start) as f32 / days as f32))
            .h(px(lane_height - LANE_GAP))
            .pl(if clipped_left { px(0.) } else { SPACE_2XS })
            .pr(if clipped_right { px(0.) } else { px(LANE_GAP) })
            .child(
                div()
                    .id(("bar", occurrence_key(&occurrence, 0)))
                    .role(gpui::Role::Button)
                    .aria_label(self.occurrence_label(&occurrence))
                    .aria_selected(selected)
                    .when(selected, |d| d.aria_active_descendant())
                    .size_full()
                    .px(SPACE_SM)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .overflow_hidden()
                    .rounded(RADIUS_SM)
                    .when(clipped_left, |d| d.rounded_l(px(0.)).pl(SPACE_XS))
                    .when(clipped_right, |d| d.rounded_r(px(0.)).pr(SPACE_XS))
                    .map(|d| {
                        if selected {
                            d.bg(fill)
                        } else {
                            look.bar(d, fill, accent, theme)
                                .hover(move |s| s.bg(hover))
                                .children(look.stripes(accent))
                        }
                    })
                    .relative()
                    .text_size(TEXT_SM)
                    // It goes on before the days shown (ME-090).
                    .when(clipped_left, |d| {
                        d.child(icon(
                            Icon::ChevronLeft,
                            ICON_SM,
                            theme.event_secondary(text),
                        ))
                    })
                    .when(single_view || !clipped_left, |d| {
                        d.children(time.map(|time| {
                            div()
                                .flex_none()
                                .text_color(theme.event_secondary(text))
                                .child(time)
                        }))
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
                    .children(until.map(|until| {
                        div()
                            .flex_none()
                            .text_color(theme.event_secondary(text))
                            .child(until)
                    }))
                    // And after them.
                    .when(clipped_right, |d| {
                        d.child(icon(
                            Icon::ChevronRight,
                            ICON_SM,
                            theme.event_secondary(text),
                        ))
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.event_menu(occurrence, event.position, window, cx)
                        }),
                    )
                    .map(|d| {
                        if read_only {
                            d.on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    if is_secondary(event) {
                                        return this.event_menu(
                                            occurrence,
                                            event.position,
                                            window,
                                            cx,
                                        );
                                    }
                                    this.press_read_only(occurrence, event, window, cx)
                                }),
                            )
                        } else {
                            d.on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    if is_secondary(event) {
                                        this.event_menu(occurrence, event.position, window, cx);
                                    }
                                }),
                            )
                            .on_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, window, cx| {
                                    if !event.is_right_click() && !event.modifiers().control {
                                        this.click_occurrence(occurrence, window, cx)
                                    }
                                },
                            ))
                        }
                    }),
            )
            .into_any_element()
    }

    /// A deleted event's block on `date`, fading out where it was: drawn
    /// only, it takes no mouse (MD-038).
    fn render_removed_block(&self, date: NaiveDate, theme: &Theme) -> Option<AnyElement> {
        let block = self.removed_block.as_ref()?;
        let top = time::minutes_into(date, block.start).max(0);
        let bottom = time::minutes_into(date, block.end).min(drag::DAY_MINUTES);
        if bottom <= top {
            return None;
        }
        let fill = theme.event_fill(block.color, false);
        Some(
            div()
                .absolute()
                .top(px(top as f32 / 60. * HOUR_HEIGHT + 1.))
                .left(SPACE_2XS)
                .right(px(1.))
                .h(px(((bottom - top) as f32 / 60. * HOUR_HEIGHT - 2.).max(18.)))
                .opacity(block.fade.clamp(0., 1.))
                .rounded(RADIUS_SM)
                .bg(fill)
                .pl(SPACE_SM)
                .py(SPACE_XS)
                .overflow_hidden()
                .text_size(TEXT_SM)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.event_text(block.color, false))
                .child(div().truncate().child(block.title.clone()))
                .into_any_element(),
        )
    }
}

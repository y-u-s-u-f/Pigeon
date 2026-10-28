//! A small month for jumping to a date: under the header's title, and beside
//! the Day view. A click on a day goes to it.

use chrono::{Datelike, Duration, NaiveDate};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement as _, Styled, div, prelude::FluentBuilder as _,
};

use super::{CalendarView, today};
use crate::ui::style::*;
use crate::{
    assets::Icon,
    calendar::time::{add_months, month_start, week_start},
    theme::Theme,
    ui::icon,
};

const WEEKS: i64 = 6;

impl CalendarView {
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
        let first = week_start(month);
        let today = today();
        let typed = (self.menu == Some(super::Menu::GoToDate))
            .then(|| self.typed_date())
            .flatten();

        let nav = |nav_id: &'static str, glyph: Icon, label: &'static str, months: i32| {
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
                .child(icon(glyph, ICON_MD, theme.icon))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mini_month = Some(add_months(this.mini_month_shown(), months));
                    cx.notify();
                }))
        };

        let weekdays = crate::calendar::time::weekday_letters().map(|d| {
            div()
                .w(HIT_TARGET)
                .flex()
                .justify_center()
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_faint)
                .child(d)
        });

        let weeks = (0..WEEKS).map(|w| {
            div().flex().children((0..7).map(|d| {
                let ix = w * 7 + d;
                let date = first + Duration::days(ix);
                let chosen = typed.map_or(date == self.anchor, |t| date == t);
                let in_month = date.month() == month.month();
                let is_today = date == today;
                div()
                    .id((id, date.num_days_from_ce() as usize))
                    .role(gpui::Role::Button)
                    .aria_label(super::header::full_date(date))
                    .aria_selected(chosen)
                    .size(HIT_TARGET)
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
                        } else {
                            d.text_color(if is_today {
                                theme.accent
                            } else if in_month {
                                theme.text
                            } else {
                                theme.text_faint
                            })
                            .when(is_today, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .hover(|s| s.bg(theme.hover_bg))
                        }
                    })
                    .child(date.day().to_string())
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.go_to_date(date, window, cx)),
                    )
            }))
        });

        div()
            .flex()
            .flex_col()
            .child(
                div()
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
                    .child(nav("mini-prev", Icon::ChevronLeft, "Previous month", -1))
                    .child(nav("mini-next", Icon::ChevronRight, "Next month", 1)),
            )
            .child(div().flex().pb(SPACE_2XS).children(weekdays))
            .children(weeks)
            .into_any_element()
    }
}

//! The calendar's header: the period shown, the calendars menu, the view
//! switcher, today and the arrows, and the new-event button.

use chrono::{Datelike, Duration};
use gpui::{
    Action, AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, StatefulInteractiveElement as _, Styled, Window, anchored,
    component::tooltip::Tooltip, deferred, div, prelude::FluentBuilder as _, px,
};

use super::{CalendarView, Menu, ViewKind};
use crate::{
    actions::*,
    app::RAIL_WIDTH,
    assets::Icon,
    theme::Theme,
    ui::{HEADER_HEIGHT, icon, icon_button, overlay_shadow, window_drag_area},
};

/// Where the traffic lights end, measured from the window's left edge.
const TRAFFIC_LIGHTS_END: gpui::Pixels = px(84.);

impl CalendarView {
    /// "October 2026", "Oct – Nov 2026", "Dec 2026 – Jan 2027", or for a day
    /// "Monday, October 26": the bold part and the muted rest.
    fn title(&self) -> (String, String) {
        let (from, to) = self.visible_range();
        match self.view {
            ViewKind::Day => (
                self.anchor.format("%B %-d").to_string(),
                self.anchor.year().to_string(),
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

    pub(crate) fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let (title, rest) = self.title();

        let segment =
            |id: &'static str, label: &'static str, view: ViewKind, action: Box<dyn Action>| {
                let active = self.view == view;
                let tooltip_action = action.boxed_clone();
                div()
                    .id(id)
                    .h(px(24.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .map(|this| {
                        if active {
                            this.bg(theme.overlay_bg)
                                .text_color(theme.text)
                                .shadow(vec![gpui::BoxShadow {
                                    color: theme.overlay_shadow.alpha(0.18),
                                    offset: gpui::point(px(0.), px(1.)),
                                    blur_radius: px(2.),
                                    spread_radius: px(0.),
                                    inset: false,
                                }])
                        } else {
                            this.text_color(theme.text_muted)
                                .hover(|s| s.text_color(theme.text))
                        }
                    })
                    .child(label)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                    .tooltip(move |window, cx| {
                        Tooltip::new(label)
                            .action(tooltip_action.as_ref(), None)
                            .build(window, cx)
                    })
            };

        let segmented = div()
            .flex_none()
            .p(px(2.))
            .flex()
            .items_center()
            .gap(px(2.))
            .rounded(px(8.))
            .bg(theme.chip_bg)
            .child(segment(
                "view-day",
                "Day",
                ViewKind::Day,
                Box::new(ShowDayView),
            ))
            .child(segment(
                "view-week",
                "Week",
                ViewKind::Week,
                Box::new(ShowWeekView),
            ))
            .child(segment(
                "view-month",
                "Month",
                ViewKind::Month,
                Box::new(ShowMonthView),
            ));

        let today_button = div()
            .id("today")
            .flex_none()
            .h(px(28.))
            .px(px(12.))
            .flex()
            .items_center()
            .rounded(px(7.))
            .border_1()
            .border_color(theme.border)
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .hover(|s| s.bg(theme.hover_bg))
            .active(|s| s.bg(theme.selected_bg))
            .child("Today")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, window, cx| window.dispatch_action(Box::new(GoToToday), cx))
            .tooltip(|window, cx| {
                Tooltip::new("Go to today")
                    .action(&GoToToday, None)
                    .build(window, cx)
            });

        let new_button = div()
            .id("new-event")
            .flex_none()
            .h(px(28.))
            .pl(px(8.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(7.))
            .bg(theme.accent)
            .text_color(theme.accent_text)
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .hover(|s| s.opacity(0.9))
            .active(|s| s.opacity(0.8))
            .child(icon(Icon::Plus, px(15.), theme.accent_text))
            .child("New")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, window, cx| window.dispatch_action(Box::new(NewEvent), cx))
            .tooltip(|window, cx| {
                Tooltip::new("New event")
                    .action(&NewEvent, None)
                    .build(window, cx)
            });

        window_drag_area(div().id("calendar-header"))
            .h(HEADER_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(TRAFFIC_LIGHTS_END - RAIL_WIDTH + px(14.))
            .pr(px(14.))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_baseline()
                    .gap(px(7.))
                    .child(
                        div()
                            .text_size(px(19.))
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(19.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme.text_faint)
                            .child(rest),
                    ),
            )
            .child(div().flex_1())
            .child(self.render_calendars_button(&theme, window, cx))
            .child(segmented)
            .child(div().w(px(1.)).h(px(18.)).mx(px(4.)).bg(theme.border))
            .child(icon_button(
                "prev-period",
                Icon::ChevronLeft,
                "Previous",
                Box::new(PrevPeriod),
                &theme,
            ))
            .child(today_button)
            .child(icon_button(
                "next-period",
                Icon::ChevronRight,
                "Next",
                Box::new(NextPeriod),
                &theme,
            ))
            .child(div().w(px(4.)))
            .child(new_button)
            .into_any_element()
    }

    /// The calendars button: a stack of their colors, opening a menu to show
    /// and hide each.
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
                    .size(px(10.))
                    .when(ix > 0, |d| d.ml(px(-4.)))
                    .rounded_full()
                    .bg(c.color)
                    .border_1()
                    .border_color(theme.overlay_bg)
            })
            .collect::<Vec<_>>();

        let menu = open.then(|| {
            let rows = self
                .data
                .calendars
                .iter()
                .enumerate()
                .map(|(ix, calendar)| {
                    let visible = calendar.visible;
                    div()
                        .id(("calendar-toggle", ix))
                        .h(px(32.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(7.))
                        .hover(|s| s.bg(theme.hover_bg))
                        .child(
                            div()
                                .size(px(16.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .border_1()
                                .border_color(calendar.color)
                                .when(visible, |d| d.bg(calendar.color))
                                .when(visible, |d| {
                                    d.child(icon(Icon::Check, px(12.), theme.cal_selected_text))
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .text_color(if visible {
                                    theme.text
                                } else {
                                    theme.text_muted
                                })
                                .child(calendar.name.clone()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_calendar(ix, cx)))
                });
            deferred(
                anchored().snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id("calendars-menu")
                        .occlude()
                        .mt(px(34.))
                        .w(px(220.))
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .rounded(px(10.))
                        .bg(theme.overlay_bg)
                        .border_1()
                        .border_color(theme.overlay_border)
                        .shadow(overlay_shadow(theme))
                        .child(
                            div()
                                .px(px(10.))
                                .pt(px(6.))
                                .pb(px(4.))
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text_faint)
                                .child("CALENDARS"),
                        )
                        .children(rows),
                ),
            )
            .with_priority(2)
        });

        let label: SharedString = if hidden > 0 {
            format!("{hidden} hidden").into()
        } else {
            "Calendars".into()
        };

        div()
            .flex_none()
            .child(
                div()
                    .id("calendars-button")
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(7.))
                    .text_size(px(12.5))
                    .text_color(theme.text_muted)
                    .when(open, |d| d.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                    .child(div().flex().items_center().pl(px(4.)).children(dots))
                    .child(label)
                    .child(icon(Icon::ChevronDown, px(13.), theme.text_faint))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Calendars, cx)))
                    .tooltip(|window, cx| {
                        Tooltip::new("Show and hide calendars").build(window, cx)
                    }),
            )
            .when(open, |this| this.child(self.menu_backdrop(window, cx)))
            .children(menu)
            .into_any_element()
    }
}

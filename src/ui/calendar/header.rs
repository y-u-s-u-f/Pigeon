//! The calendar's header: the period shown, the calendars menu, the view
//! switcher, today and the arrows, and the new-event button.

use chrono::{Datelike, Duration};
use gpui::{
    Action, Anchor, AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, StatefulInteractiveElement as _, Styled, Window, anchored,
    component::{input::Input, tooltip::Tooltip},
    deferred, div,
    prelude::FluentBuilder as _,
    px,
};

use super::{CalendarView, GoToDate, Menu, ViewKind};
use crate::ui::style::*;
use crate::{
    actions::*,
    assets::Icon,
    calendar::{AccessRole, Calendar},
    theme::Theme,
    ui::{HEADER_HEIGHT, icon, icon_button, popover_shadow, window_drag_area},
};

impl CalendarView {
    /// "October 2026", "Oct – Nov 2026", "Dec 2026 – Jan 2027", or for a day
    /// "Friday, October 30" (with the year when it isn't this one): the bold
    /// part and the muted rest.
    fn title(&self) -> (String, String) {
        let (from, to) = self.visible_range();
        match self.view {
            ViewKind::Day => (
                self.anchor.format("%A, %B %-d").to_string(),
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

    pub(crate) fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let (title, rest) = self.title();

        let segment =
            |id: &'static str, label: &'static str, view: ViewKind, action: Box<dyn Action>| {
                let active = self.view == view;
                let tooltip_action = action.boxed_clone();
                div()
                    .id(id)
                    .role(gpui::Role::Tab)
                    .aria_label(label)
                    .aria_selected(active)
                    .h(HIT_TARGET)
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .rounded(RADIUS_SM)
                    .text_size(TEXT_SM)
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
            .p(SPACE_2XS)
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .rounded(RADIUS_MD)
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
            .px(SPACE_MD)
            .flex()
            .items_center()
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(theme.border)
            .text_size(TEXT_SM)
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
            .pl(SPACE_SM)
            .pr(SPACE_MD)
            .flex()
            .items_center()
            .gap(SPACE_XS)
            .rounded(RADIUS_MD)
            .bg(theme.accent)
            .text_color(theme.accent_text)
            .text_size(TEXT_SM)
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
                        .child(icon(Icon::Clock, px(15.), theme.text_muted))
                        .tooltip(move |window, cx| Tooltip::new(note.clone()).build(window, cx)),
                )
            })
            .child(self.render_calendars_button(&theme, window, cx))
            .child(segmented)
            .child(div().w(px(1.)).h(px(18.)).mx(SPACE_XS).bg(theme.border))
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
        let typed = self.typed_date(cx);
        let popover = open.then(|| {
            let hint = if self.date_input.read(cx).value().trim().is_empty() {
                None
            } else {
                Some(match typed {
                    Some(date) => (date.format("%A, %B %-d, %Y").to_string(), theme.text_muted),
                    None => ("Not a date yet".to_string(), theme.text_faint),
                })
            };
            deferred(
                anchored().snap_to_window_with_margin(px(8.)).child(enter(
                    "go-to-date",
                    div()
                        .id("go-to-date")
                        .occlude()
                        .role(gpui::Role::Dialog)
                        .aria_label("Go to date")
                        .mt(px(40.))
                        .w(HIT_TARGET * 7. + SPACE_MD * 2. + px(2.))
                        .p(SPACE_MD)
                        .flex()
                        .flex_col()
                        .gap(SPACE_SM)
                        .rounded(RADIUS_LG)
                        .bg(theme.overlay_bg)
                        .border_1()
                        .border_color(theme.overlay_border)
                        .shadow(popover_shadow(theme))
                        .child(
                            div()
                                .h(px(32.))
                                .px(SPACE_SM)
                                .flex()
                                .items_center()
                                .gap(SPACE_SM)
                                .rounded(RADIUS_MD)
                                .bg(theme.search_bg)
                                .border_1()
                                .border_color(theme.accent.alpha(0.55))
                                .child(icon(Icon::Calendar, px(14.), theme.text_faint))
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
                            div()
                                .px(SPACE_SM)
                                .text_size(TEXT_SM)
                                .text_color(color)
                                .child(text)
                        }))
                        .child(self.render_mini_month("go-to-date-day", theme, cx)),
                )),
            )
            .with_priority(2)
        });
        div()
            .min_w_0()
            .child(
                div()
                    .id("calendar-title")
                    .role(gpui::Role::Button)
                    .aria_label("Go to date")
                    .min_w_0()
                    .h(px(36.))
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
                            .text_size(TEXT_XL)
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .when(!rest.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(TEXT_XL)
                                .font_weight(FontWeight::NORMAL)
                                .text_color(theme.text_faint)
                                .child(rest),
                        )
                    })
                    .child(icon(Icon::ChevronDown, px(14.), theme.text_faint))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_date_picker(window, cx)),
                    )
                    .tooltip(|window, cx| {
                        Tooltip::new("Go to date")
                            .action(&GoToDate, None)
                            .build(window, cx)
                    }),
            )
            .when(open, |d| d.child(self.menu_backdrop(window, cx)))
            .children(popover)
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
            let row = |ix: usize, calendar: &Calendar| {
                let visible = calendar.visible;
                let read_only = matches!(
                    calendar.access_role,
                    AccessRole::Reader | AccessRole::FreeBusyReader
                );
                div()
                    .id(("calendar-toggle", ix))
                    .role(gpui::Role::CheckBox)
                    .aria_label(SharedString::from(calendar.name.clone()))
                    .aria_toggled(if visible {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .h(px(32.))
                    .flex_none()
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .hover(|s| s.bg(theme.hover_bg))
                    .child(
                        div()
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS_SM)
                            .border_1()
                            .border_color(calendar.color)
                            .when(visible, |d| d.bg(calendar.color))
                            .when(visible, |d| {
                                d.child(icon(Icon::Check, px(12.), theme.on_fill(calendar.color)))
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
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_calendar(ix, cx)))
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
                                .text_color(theme.text_faint)
                                .child(title.to_uppercase()),
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
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .id("calendars-menu")
                                .occlude()
                                .role(gpui::Role::Group)
                                .aria_label("Calendars")
                                .mt(SPACE_2XL)
                                .w(px(260.))
                                .max_h(px(440.))
                                .overflow_y_scroll()
                                .p(SPACE_XS)
                                .flex()
                                .flex_col()
                                .rounded(RADIUS_MD)
                                .bg(theme.overlay_bg)
                                .border_1()
                                .border_color(theme.overlay_border)
                                .shadow(popover_shadow(theme))
                                .children(section("My calendars", mine))
                                .children(section("Other calendars", others)),
                        ),
                )
                .with_priority(2),
            )
        });

        let label: SharedString = if hidden > 0 {
            format!("{hidden} hidden").into()
        } else {
            "Calendars".into()
        };

        div()
            .flex_none()
            .relative()
            .child(
                div()
                    .id("calendars-button")
                    .h(px(28.))
                    .px(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .rounded(RADIUS_MD)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_muted)
                    .when(open, |d| d.bg(theme.selected_bg))
                    .hover(|s| s.bg(theme.hover_bg).text_color(theme.text))
                    .child(
                        div()
                            .min_w(px(30.))
                            .flex()
                            .items_center()
                            .pl(SPACE_XS)
                            .children(dots),
                    )
                    .child(div().min_w(px(64.)).whitespace_nowrap().child(label))
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

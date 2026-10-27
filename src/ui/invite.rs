//! The invite card above an invitation email's body: what, when, where and
//! who, the user's other events that day on a small timeline, and Yes /
//! Maybe / No. A cancellation gets a muted card that removes the event.
//! Answers go to the calendar (`CalendarView::set_rsvp`), so the grid shows
//! them, and the card reads the calendar, so answers given there show here.
//! An invitation whose event isn't stored is looked up in Google Calendar
//! by its UID; until then, or if it isn't there, a read-only card shows what
//! the invitation's iCalendar part says (CAL-20).

use chrono::{Duration, NaiveDateTime};
use gpui::{
    Action, AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, component::tooltip::Tooltip,
    div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, relative,
};

use super::{icon, keycap};
use crate::ui::style::*;
use crate::{
    actions::*,
    app::{Mode, PigeonApp},
    assets::Icon,
    calendar::{
        Event, Occurrence, RsvpStatus,
        layout::pack_lanes,
        occurrences,
        time::{self, date_label, time_range_label},
    },
    data::{Invite, InviteDetails, InviteMethod},
    theme::Theme,
    ui::calendar::{next_start, today},
};

/// Rows of the timeline's other events, at most.
const TIMELINE_LANES: usize = 3;
const LANE_HEIGHT: f32 = 16.;

impl PigeonApp {
    /// The invite of the selected conversation: its newest one.
    pub(crate) fn selected_invite(&self) -> Option<Invite> {
        let thread = &self.data.threads[self.selected?];
        thread.messages.iter().rev().find_map(|m| m.invite.clone())
    }

    fn answer_invite(&mut self, status: RsvpStatus, cx: &mut Context<Self>) {
        let Some(invite) = self
            .selected_invite()
            .filter(|i| i.method == InviteMethod::Request)
        else {
            return;
        };
        self.calendar
            .update(cx, |c, cx| c.set_rsvp(&invite.event_id, status, cx));
        cx.notify();
    }

    pub(crate) fn accept_invite(
        &mut self,
        _: &AcceptInvite,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.answer_invite(RsvpStatus::Accepted, cx);
    }

    pub(crate) fn maybe_invite(&mut self, _: &MaybeInvite, _: &mut Window, cx: &mut Context<Self>) {
        self.answer_invite(RsvpStatus::Tentative, cx);
    }

    pub(crate) fn decline_invite(
        &mut self,
        _: &DeclineInvite,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.answer_invite(RsvpStatus::Declined, cx);
    }

    /// Switch to the calendar, on the event's day, with it open.
    pub(crate) fn open_invite_in_calendar(
        &mut self,
        _: &OpenInviteInCalendar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(invite) = self.selected_invite() else {
            return;
        };
        let Some(ix) = self.calendar.read(cx).data.event_ix(&invite.event_id) else {
            return;
        };
        self.set_mode(Mode::Calendar, window, cx);
        self.calendar
            .update(cx, |c, cx| c.reveal_event(ix, window, cx));
    }

    pub(crate) fn remove_cancelled_event(
        &mut self,
        _: &RemoveCancelledEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(invite) = self
            .selected_invite()
            .filter(|i| i.method == InviteMethod::Cancel)
        else {
            return;
        };
        // Only the user's copy goes, and nobody is told: the organizer
        // called it off already (CAL-20). The card then says it's removed.
        if self
            .calendar
            .read(cx)
            .data
            .event_ix(&invite.event_id)
            .is_none()
        {
            return;
        }
        self.backend
            .update(cx, |b, cx| b.remove_cancelled_event(&invite.event_id, cx));
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Ask Google Calendar for an invitation's event that isn't stored,
    /// once per UID (CAL-20). Not for a cancellation: Google lists no
    /// cancelled events, so there's nothing to find.
    fn look_up_invite(&self, invite: &Invite, cx: &mut Context<Self>) {
        if invite.method == InviteMethod::Cancel
            || invite.ical_uid.is_empty()
            || self.invite_lookups.contains(&invite.ical_uid)
            || self.backend.read(cx).is_demo()
        {
            return;
        }
        let uid = invite.ical_uid.clone();
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            this.update(cx, |this, cx| {
                if this.invite_lookups.insert(uid.clone()) {
                    this.backend
                        .update(cx, |b, cx| b.find_invite_event(&uid, cx));
                }
            })
            .ok();
        });
    }

    pub(crate) fn render_invite(
        &self,
        invite: &Invite,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self
            .calendar
            .read(cx)
            .data
            .event_ix(&invite.event_id)
            .is_none()
        {
            self.look_up_invite(invite, cx);
        }
        let cx: &Context<Self> = cx;
        let calendar = self.calendar.read(cx);
        let data = &calendar.data;
        let event = data.event_ix(&invite.event_id).map(|ix| &data.events[ix]);
        let Some(event) = event else {
            if let Some(details) = &invite.details {
                return render_ics_card(invite.method, details, theme);
            }
            let text = match invite.method {
                InviteMethod::Request => "This event is no longer in your calendar.",
                InviteMethod::Cancel => "Removed from your calendar.",
            };
            return card(theme.text_faint, theme)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .text_size(TEXT_BASE)
                        .text_color(theme.text_muted)
                        .child(icon(Icon::Check, px(15.), theme.text_faint))
                        .child(text),
                )
                .into_any_element();
        };
        let start = next_start(event);
        let end = start + event.duration();
        let cancelled = invite.method == InviteMethod::Cancel;
        let color = if cancelled {
            theme.text_faint
        } else {
            theme.event_accent(data.color_of(event), false)
        };
        let calendar_name = data
            .calendar(&event.calendar)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        let kicker = div()
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .child(
                div()
                    .flex_none()
                    .text_size(TEXT_XS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if cancelled { theme.danger } else { color })
                    .child(if cancelled { "CANCELLED" } else { "INVITATION" }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(TEXT_SM)
                    .text_color(theme.text_faint)
                    .child(calendar_name),
            )
            .when(!cancelled, |d| {
                d.child(
                    button(
                        "invite-open",
                        Icon::Calendar,
                        "Open in Calendar",
                        None,
                        theme,
                    )
                    .flex_none()
                    .tooltip(|window, cx| {
                        Tooltip::new("Open in Calendar")
                            .action(&OpenInviteInCalendar, None)
                            .build(window, cx)
                    })
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(OpenInviteInCalendar), cx)
                    }),
                )
            });

        let title = div()
            .mt(SPACE_XS)
            .text_size(TEXT_LG)
            .font_weight(FontWeight::SEMIBOLD)
            .line_height(relative(1.3))
            .when(cancelled, |d| d.line_through().text_color(theme.text_muted))
            .child(event.display_title().to_string());

        let when = when_label(start, end, event.all_day);
        let detail = |glyph: Icon, text: String| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_SM)
                .min_w_0()
                .text_size(TEXT_BASE)
                .text_color(theme.text_muted)
                .child(icon(glyph, px(14.), theme.text_faint))
                .child(div().min_w_0().truncate().child(text))
        };
        let organizer = if data.account.is_me(&event.organizer.email) {
            "You".to_string()
        } else {
            event.organizer.display_name().to_string()
        };
        let people = event.attendees.len()
            + !event
                .attendees
                .iter()
                .any(|a| a.email.eq_ignore_ascii_case(&event.organizer.email))
                as usize;
        let link = event.video_link.clone();

        let details = div()
            .mt(SPACE_MD)
            .flex()
            .flex_col()
            .gap(SPACE_SM)
            .child(detail(
                Icon::Clock,
                match &event.recurrence {
                    Some(r) => format!("{when} · {}", r.label()),
                    None => when,
                },
            ))
            .when(!event.location.is_empty(), |d| {
                d.child(detail(Icon::MapPin, event.location.clone()))
            })
            .when(!link.is_empty(), |d| {
                let shown = link
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .to_string();
                d.child(
                    detail(Icon::Video, shown).child(
                        div()
                            .id("invite-join")
                            .flex_none()
                            .ml(SPACE_XS)
                            .text_size(TEXT_SM)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.accent)
                            .cursor_pointer()
                            .hover(|s| s.underline())
                            .child("Join")
                            .on_click(move |_, _, cx| cx.open_url(&link)),
                    ),
                )
            })
            .child(detail(
                Icon::Users,
                format!(
                    "Organized by {organizer} · {people} {}",
                    if people == 1 { "person" } else { "people" }
                ),
            ));

        let mut card = card(color, theme).when(!cancelled, |d| {
            d.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(90.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(color.alpha(if theme.is_dark { 0.12 } else { 0.08 }), 0.),
                        linear_color_stop(color.alpha(0.), 1.),
                    )),
            )
        });
        card = card.child(kicker).child(title).child(details);

        if cancelled {
            let by = event.organizer.display_name().to_string();
            return card
                .child(
                    div()
                        .mt(SPACE_MD)
                        .pt(SPACE_MD)
                        .border_t_1()
                        .border_color(theme.border)
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child(
                            div()
                                .flex_1()
                                .text_size(TEXT_BASE)
                                .text_color(theme.text_muted)
                                .child(format!("{by} cancelled this event.")),
                        )
                        .child(
                            button(
                                "invite-remove",
                                Icon::Trash,
                                "Remove from calendar",
                                Some("⇧X"),
                                theme,
                            )
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(RemoveCancelledEvent), cx)
                            }),
                        ),
                )
                .into_any_element();
        }

        card = card.children(self.render_day_timeline(event, start, end, theme, cx));

        let status = event.my_status(&data.account);
        let answers = div()
            .mt(SPACE_MD)
            .pt(SPACE_MD)
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(SPACE_SM)
            .map(|d| match status {
                Some(status) => d
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(SPACE_SM)
                            .child(
                                div()
                                    .text_size(TEXT_BASE)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Going?"),
                            )
                            .child(
                                div()
                                    .text_size(TEXT_SM)
                                    .text_color(match status {
                                        RsvpStatus::NeedsAction => theme.text_muted,
                                        RsvpStatus::Accepted => theme.success,
                                        RsvpStatus::Tentative => theme.text_muted,
                                        RsvpStatus::Declined => theme.danger,
                                    })
                                    .child(match status {
                                        RsvpStatus::NeedsAction => "Not answered",
                                        RsvpStatus::Accepted => "You're going",
                                        RsvpStatus::Tentative => "You might go",
                                        RsvpStatus::Declined => "You declined",
                                    }),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div().flex().gap(SPACE_SM).children(
                            [
                                (RsvpStatus::Accepted, "Yes", Icon::Check, theme.success),
                                (RsvpStatus::Tentative, "Maybe", Icon::Help, theme.star),
                                (RsvpStatus::Declined, "No", Icon::X, theme.danger),
                            ]
                            .into_iter()
                            .enumerate()
                            .map(
                                |(ix, (answer, label, glyph, tint))| {
                                    rsvp_button(ix, label, glyph, tint, status == answer, theme)
                                },
                            ),
                        ),
                    ),
                None => d.child(
                    div()
                        .text_size(TEXT_BASE)
                        .text_color(theme.text_muted)
                        .child("You're the organizer"),
                ),
            });
        card.child(answers).into_any_element()
    }

    /// Whether the user is free then. When the invite clashes with other
    /// events, a strip of the hours around it shows how, with the clashes
    /// named under it (and each bar's name in its tooltip).
    fn render_day_timeline(
        &self,
        event: &Event,
        start: NaiveDateTime,
        end: NaiveDateTime,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if event.is_spanning() {
            return None;
        }
        let calendar = self.calendar.read(cx);
        let data = &calendar.data;
        let me = &data.account;
        let day = start.date();
        let others: Vec<Occurrence> = occurrences(
            &data.events,
            |e| {
                e.id != event.id
                    && !e.is_spanning()
                    && e.my_status(me) != Some(RsvpStatus::Declined)
            },
            time::midnight(day),
            time::midnight(day + Duration::days(1)),
        );
        let minutes = |t: NaiveDateTime| time::minutes_into(day, t).clamp(0, 24 * 60);
        let (s, e) = (minutes(start), minutes(end));
        let conflicts: Vec<&Occurrence> = others
            .iter()
            .filter(|o| o.start < end && o.end > start && data.events[o.event].busy)
            .collect();

        if conflicts.is_empty() {
            return Some(
                div()
                    .mt(SPACE_MD)
                    .flex()
                    .items_center()
                    .gap(SPACE_SM)
                    .text_size(TEXT_SM)
                    .text_color(theme.text_muted)
                    .child(icon(Icon::Check, px(14.), theme.success))
                    .child(if others.is_empty() {
                        "You're free · nothing else that day".to_string()
                    } else {
                        format!(
                            "You're free then · {} other event{} that day",
                            others.len(),
                            if others.len() == 1 { "" } else { "s" }
                        )
                    })
                    .into_any_element(),
            );
        }

        // An hour either side, on the hour, widened to the clashes.
        let from = conflicts
            .iter()
            .map(|o| minutes(o.start))
            .fold(s - 60, i64::min)
            .max(0)
            / 60
            * 60;
        let to = conflicts
            .iter()
            .map(|o| minutes(o.end))
            .fold(e + 60, i64::max)
            .min(24 * 60);
        let to = ((to + 59) / 60 * 60).min(24 * 60);
        let span = (to - from).max(60) as f32;
        let x = |m: i64| ((m - from) as f32 / span).clamp(0., 1.);

        let shown: Vec<&Occurrence> = others
            .iter()
            .filter(|o| minutes(o.end) > from && minutes(o.start) < to)
            .collect();
        let lanes = pack_lanes(
            &shown
                .iter()
                .map(|o| (minutes(o.start), minutes(o.end)))
                .collect::<Vec<_>>(),
        );
        let lane_count = lanes
            .iter()
            .map(|l| l + 1)
            .max()
            .unwrap_or(1)
            .min(TIMELINE_LANES);
        let track_height = lane_count as f32 * (LANE_HEIGHT + 3.) + 7.;

        let bars = shown
            .iter()
            .zip(&lanes)
            .enumerate()
            .filter(|(_, (_, l))| **l < TIMELINE_LANES)
            .map(|(bar_ix, (o, &lane))| {
                let other = &data.events[o.event];
                let tint = data.color_of(other);
                let left = x(minutes(o.start));
                let width = x(minutes(o.end)) - left;
                let tip: SharedString = format!(
                    "{} · {}",
                    other.display_title(),
                    time_range_label(o.start.time(), o.end.time())
                )
                .into();
                div()
                    .id(("invite-bar", bar_ix))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .absolute()
                    .top(px(4. + lane as f32 * (LANE_HEIGHT + 3.)))
                    .left(relative(left))
                    .w(relative(width))
                    .h(px(LANE_HEIGHT))
                    .px(SPACE_2XS)
                    // No names inside: at this size they'd be cut to a
                    // letter or two. The tooltip and the line below name them.
                    .child(
                        div()
                            .size_full()
                            .rounded(RADIUS_SM)
                            .bg(theme.event_fill(tint, false))
                            .border_l_2()
                            .border_color(theme.event_accent(tint, false)),
                    )
            });

        let color = theme.event_accent(data.color_of(event), false);
        let this = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(relative(x(s)))
            .w(relative(x(e) - x(s)))
            .rounded(RADIUS_SM)
            .bg(color.alpha(0.12))
            .border_2()
            .border_color(color);

        // At most five labels, so they never run into each other.
        let step = ((to - from) / 60 / 5 + 1).max(1);
        let hours = (from / 60..=to / 60).map(|h| {
            let label = time::compact_time_label(
                chrono::NaiveTime::from_hms_opt((h % 24) as u32, 0, 0).expect("valid hour"),
            );
            let left = x(h * 60);
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(left))
                .w(px(1.))
                .bg(theme.border)
                .when((h - from / 60) % step == 0 && h * 60 < to, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(track_height + 3.))
                            .left(px(3.))
                            .whitespace_nowrap()
                            .text_size(TEXT_XS)
                            .text_color(theme.text_faint)
                            .child(label),
                    )
                })
        });

        let clash = {
            let names = conflicts
                .iter()
                .map(|o| {
                    let other = &data.events[o.event];
                    format!(
                        "{} ({})",
                        other.display_title(),
                        time_range_label(o.start.time(), o.end.time())
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            div()
                .mt(SPACE_SM)
                .flex()
                .items_start()
                .gap(SPACE_SM)
                .text_size(TEXT_SM)
                .text_color(theme.danger)
                .child(
                    div()
                        .mt(SPACE_2XS)
                        .child(icon(Icon::OctagonAlert, px(14.), theme.danger)),
                )
                .child(div().min_w_0().child(format!("Conflicts with {names}")))
        };

        Some(
            div()
                .mt(SPACE_LG)
                .flex()
                .flex_col()
                .child(
                    div()
                        .relative()
                        .h(px(track_height))
                        .mb(SPACE_LG)
                        .rounded(RADIUS_SM)
                        .bg(theme.chip_bg)
                        .children(hours)
                        .child(this)
                        .children(bars),
                )
                .child(clash)
                .into_any_element(),
        )
    }
}

/// "Tomorrow · 10:00 – 11:00 AM", or the days of an all-day event.
fn when_label(start: NaiveDateTime, end: NaiveDateTime, all_day: bool) -> String {
    if all_day {
        let last = (end - Duration::days(1)).date().max(start.date());
        if last == start.date() {
            format!("{} · All day", date_label(start.date(), today()))
        } else {
            format!(
                "{} – {}",
                date_label(start.date(), today()),
                date_label(last, today())
            )
        }
    } else {
        format!(
            "{} · {}",
            date_label(start.date(), today()),
            time_range_label(start.time(), end.time())
        )
    }
}

/// An invitation whose event isn't in the calendar: what its iCalendar
/// part says, with nothing to answer (CAL-20).
fn render_ics_card(method: InviteMethod, details: &InviteDetails, theme: &Theme) -> AnyElement {
    let cancelled = method == InviteMethod::Cancel;
    let line = |glyph: Icon, text: String| {
        div()
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .min_w_0()
            .text_size(TEXT_BASE)
            .text_color(theme.text_muted)
            .child(icon(glyph, px(14.), theme.text_faint))
            .child(div().min_w_0().truncate().child(text))
    };
    let when = details.start.map(|start| {
        let end = details.end.unwrap_or(start).max(start);
        when_label(start, end, details.all_day)
    });
    let organizer = if details.organizer.email.is_empty() {
        None
    } else {
        Some(details.organizer.display_name().to_string())
    };
    let title = if details.title.trim().is_empty() {
        "(No title)".to_string()
    } else {
        details.title.clone()
    };
    card(theme.text_faint, theme)
        .id("invite-ics")
        .role(gpui::Role::Region)
        .aria_label(format!(
            "{}: {title}",
            if cancelled { "Cancelled" } else { "Invitation" }
        ))
        .child(
            div()
                .text_size(TEXT_XS)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if cancelled {
                    theme.danger
                } else {
                    theme.text_faint
                })
                .child(if cancelled { "CANCELLED" } else { "INVITATION" }),
        )
        .child(
            div()
                .mt(SPACE_XS)
                .text_size(TEXT_LG)
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(relative(1.3))
                .when(cancelled, |d| d.line_through().text_color(theme.text_muted))
                .child(title),
        )
        .child(
            div()
                .mt(SPACE_MD)
                .flex()
                .flex_col()
                .gap(SPACE_SM)
                .children(when.map(|w| line(Icon::Clock, w)))
                .when(!details.location.is_empty(), |d| {
                    d.child(line(Icon::MapPin, details.location.clone()))
                })
                .children(organizer.map(|o| line(Icon::Users, format!("Organized by {o}")))),
        )
        .child(
            div()
                .mt(SPACE_MD)
                .pt(SPACE_MD)
                .border_t_1()
                .border_color(theme.border)
                .text_size(TEXT_SM)
                .text_color(theme.text_faint)
                .child(if cancelled {
                    "Not in your calendar."
                } else {
                    "Not in your calendar, so it can't be answered here."
                }),
        )
        .into_any_element()
}

/// A line of an invitation's body that its card already shows: "When: …",
/// "Where: …", "Organizer: …", "Guests: …", "X invited you to Y".
pub(crate) fn is_invite_summary(block: &crate::html::Block) -> bool {
    const PREFIXES: [&str; 6] = [
        "When:",
        "Where:",
        "Organizer:",
        "Guests:",
        "Who:",
        "Joining:",
    ];
    match block {
        crate::html::Block::Text { text, .. } => {
            let text = text.text.trim();
            PREFIXES.iter().any(|p| text.starts_with(p))
                || text.contains(" invited you to ")
                || (text.len() < 160 && text.contains(" cancelled "))
        }
        _ => false,
    }
}

/// The card: a box with a stripe of `color` down its left edge.
fn card(color: Hsla, theme: &Theme) -> gpui::Div {
    div()
        .relative()
        .mb(SPACE_LG)
        .pl(SPACE_LG)
        .pr(SPACE_LG)
        .py(SPACE_MD)
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(theme.border)
        .bg(theme.card_bg)
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(px(4.))
                .bg(color),
        )
}

fn button(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    key: Option<&'static str>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(28.))
        .px(SPACE_MD)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(theme.border)
        .bg(theme.chip_bg)
        .text_size(TEXT_SM)
        .font_weight(FontWeight::MEDIUM)
        .hover(|s| s.bg(theme.selected_bg))
        .active(|s| s.opacity(0.8))
        .child(icon(glyph, px(14.), theme.icon))
        .child(label)
        .children(key.map(|k| keycap(k, theme)))
}

/// Yes, Maybe or No: tinted when it's the answer given.
fn rsvp_button(
    ix: usize,
    label: &'static str,
    glyph: Icon,
    tint: Hsla,
    chosen: bool,
    theme: &Theme,
) -> impl IntoElement {
    let action: fn() -> Box<dyn Action> = match ix {
        0 => || Box::new(AcceptInvite),
        1 => || Box::new(MaybeInvite),
        _ => || Box::new(DeclineInvite),
    };
    let tip: SharedString = label.into();
    focusable_control(div().id(("invite-rsvp", ix)), theme)
        .role(gpui::Role::Button)
        .aria_label(label)
        .aria_selected(chosen)
        .h(HIT_TARGET + SPACE_XS)
        .px(SPACE_MD)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .border_1()
        .text_size(TEXT_SM)
        .font_weight(FontWeight::SEMIBOLD)
        .map(|d| {
            if chosen {
                d.bg(tint.alpha(0.16))
                    .border_color(tint.alpha(0.55))
                    .text_color(tint)
            } else {
                d.bg(theme.chip_bg)
                    .border_color(theme.border)
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.selected_bg).text_color(theme.text))
            }
        })
        .child(icon(glyph, px(15.), if chosen { tint } else { theme.icon }))
        .child(label)
        .tooltip(move |window, cx| {
            Tooltip::new(tip.clone())
                .action(action().as_ref(), None)
                .build(window, cx)
        })
        .on_click(move |_, window, cx| window.dispatch_action(action(), cx))
}

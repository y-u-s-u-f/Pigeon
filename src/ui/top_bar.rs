//! The window's top bar, across Mail, Calendar and Settings: the traffic
//! lights and the sidebar's toggle on the left, the search field in the
//! middle, and Refresh, the app's main action and the account on the right.
//! The rail under it (`rail`) has Mail, Calendar and Settings; the two are
//! one L-shaped frame. Empty space moves the window. It's a pane of its own
//! (`app/panes.rs`), so its hovers and its spinner redraw it alone.

use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Focusable as _, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, SharedString, StatefulInteractiveElement as _, Styled,
    Window, component::input::Input, div, point, prelude::FluentBuilder as _, px,
};

use super::context_menu::{ContextMenu, Item};
use super::controls::{ButtonKind, push_button_frame};
use super::motion::spinner_while;
use super::style::*;
use super::{
    account_avatar, icon, icon_button, icon_button_small, keycap, shortcut_text, status, tooltip,
    window_drag_area,
};
use crate::{
    actions::{
        CloseSearch, Compose, MAILBOX, NewEvent, OpenSettings, OpenSettingsSection, SEARCH, SignIn,
        SignOut, SyncNow, ToggleCommandPalette, ToggleSearch, ToggleSidebar,
    },
    app::{Mode, PigeonApp, RAIL_WIDTH, shell},
    assets::Icon,
    backend::{Source, SyncStatus},
    data::Contact,
    settings::Section,
    theme::Theme,
};

/// The search field, at most; it narrows with the window.
const SEARCH_WIDTH: gpui::Rems = type_size(460.);
/// The search field, at least: the trailing end gives way before it does.
const SEARCH_MIN_WIDTH: gpui::Rems = type_size(200.);
/// The demo chip's words, and what's left of them when the bar is tight.
const DEMO_LABEL: &str = "Demo Data · Sign In";
const DEMO_SHORT: &str = "Demo";
/// A rough average glyph width, in ems, for sizing the bar's words ahead
/// of layout; on the wide side, so it gives way early rather than late.
const GLYPH_EM: f32 = 0.6;

/// How much of the trailing end gives way to keep the search usable, each
/// step on top of the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Compact {
    Full,
    /// The sync progress text hides.
    NoProgress,
    /// The demo chip says only "Demo".
    ShortChip,
    /// Compose and New Event show only their glyph.
    IconOnly,
}

/// What the trailing end holds, for working out its width.
#[derive(Clone, Copy, Debug, Default)]
struct Trailing<'a> {
    status: bool,
    demo: bool,
    progress: Option<&'a str>,
    primary: Option<&'a str>,
}

/// The trailing end's width at `compact`, in px, with `rem` px to a rem.
fn trailing_width(t: &Trailing, compact: Compact, rem: f32) -> f32 {
    let r = |size: gpui::Rems| size.0 * rem;
    let text = |s: &str, size: gpui::Rems| s.chars().count() as f32 * GLYPH_EM * r(size);
    let hit = f32::from(HIT_TARGET);
    let gap = f32::from(SPACE_XS);
    // Refresh and the account.
    let mut parts = vec![hit, hit.max(r(AVATAR_XS) + 2. * f32::from(SPACE_2XS))];
    if t.status {
        parts.push(hit);
    }
    if t.demo {
        let label = if compact >= Compact::ShortChip {
            DEMO_SHORT
        } else {
            DEMO_LABEL
        };
        parts.push(text(label, TEXT_XS) + 2. * f32::from(SPACE_SM));
    }
    if let Some(progress) = t.progress.filter(|_| compact < Compact::NoProgress) {
        parts.push(gap + text(progress, TEXT_XS));
    }
    if let Some(label) = t.primary {
        let button = if compact >= Compact::IconOnly {
            r(CONTROL_MD).max(hit)
        } else {
            f32::from(SPACE_SM + SPACE_XS + SPACE_MD) + r(ICON_MD) + text(label, TEXT_SM)
        };
        parts.push(2. * gap + button);
    }
    let gaps = gap * parts.len().saturating_sub(1) as f32;
    f32::from(SPACE_MD) + gaps + parts.iter().sum::<f32>()
}

/// The least the trailing end gives way so the search keeps its minimum
/// in `room` px (the bar less its leading end).
fn compact_for(room: f32, t: &Trailing, rem: f32) -> Compact {
    let search = SEARCH_MIN_WIDTH.0 * rem;
    [Compact::Full, Compact::NoProgress, Compact::ShortChip]
        .into_iter()
        .find(|&c| room - trailing_width(t, c, rem) >= search)
        .unwrap_or(Compact::IconOnly)
}
/// The bar's bottom line, which meets the rail's.
const HAIRLINE: Pixels = px(1.);

impl PigeonApp {
    pub(crate) fn render_top_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let settings_open = self.settings_page.open;
        let (source, status, can_sign_out, photo) = {
            let b = self.backend.read(cx);
            (
                b.source(),
                b.status().clone(),
                b.source() == Source::Google || b.demo_by_choice(),
                b.profile_photo().map(|p| p.to_path_buf()),
            )
        };
        let signed_out = source == Source::SignedOut;
        // Neither app is chosen while Settings shows.
        let mode = (!settings_open && !signed_out).then_some(self.mode);
        let sidebar_shown = self.pane_layout(window.viewport_size().width).sidebar > px(0.);
        let lights = shell::traffic_lights_inset(window.is_fullscreen());
        let progress = sync_progress(&status).filter(|_| !signed_out);
        let compact = {
            let toggle = if mode == Some(Mode::Mail) {
                SPACE_SM + HIT_TARGET
            } else {
                px(0.)
            };
            let room = window.viewport_size().width - lights.max(SPACE_MD) - toggle;
            let trailing = Trailing {
                status: matches!(status, SyncStatus::Offline { .. } | SyncStatus::AuthExpired),
                demo: source == Source::Demo,
                progress: progress.as_deref(),
                primary: mode.map(|mode| match mode {
                    Mode::Mail => "Compose",
                    Mode::Calendar => "New Event",
                }),
            };
            compact_for(f32::from(room), &trailing, f32::from(window.rem_size()))
        };
        let icon_only = compact >= Compact::IconOnly;

        let leading = div()
            .flex_1()
            .flex()
            .items_center()
            .gap(SPACE_SM)
            .pl(lights.max(SPACE_MD))
            .when(mode == Some(Mode::Mail), |this| {
                this.child(
                    icon_button(
                        "toggle-sidebar",
                        Icon::PanelLeft,
                        if sidebar_shown {
                            "Hide sidebar"
                        } else {
                            "Show sidebar"
                        },
                        Box::new(ToggleSidebar),
                        &theme,
                    )
                    .aria_expanded(sidebar_shown),
                )
            });

        let center = div()
            .flex_none()
            .flex_shrink(1.)
            .min_w(SEARCH_MIN_WIDTH)
            .w(SEARCH_WIDTH)
            .flex()
            .justify_center()
            .map(|this| match mode {
                Some(Mode::Mail) => this.child(self.render_mail_search(&theme, window, cx)),
                Some(Mode::Calendar) => this.child(calendar_search(&theme)),
                None => this,
            });

        let primary = mode.map(|mode| match mode {
            Mode::Mail => primary_button(
                "top-bar-compose",
                Icon::SquarePen,
                "Compose",
                "New message",
                Box::new(Compose),
                icon_only,
                &theme,
            ),
            Mode::Calendar => primary_button(
                "top-bar-new-event",
                Icon::Plus,
                "New Event",
                "New event",
                Box::new(NewEvent),
                icon_only,
                &theme,
            ),
        });

        let account = Contact {
            name: self.data.account.name.clone(),
            email: self.data.account.email.clone(),
        };
        let trailing = div()
            .flex_1()
            .flex()
            .items_center()
            .justify_end()
            .gap(SPACE_XS)
            .pr(SPACE_MD)
            .children(status_glyph(&status, &theme))
            .when(source == Source::Demo, |this| {
                this.child(demo_chip(compact >= Compact::ShortChip, &theme))
            })
            .when(!signed_out, |this| {
                let spinning = self.refresh_spin.is_some()
                    || matches!(status, SyncStatus::Syncing { .. } | SyncStatus::Waiting);
                let progress = progress.filter(|_| compact < Compact::NoProgress);
                this.children(progress.map(|progress| {
                    div()
                        .flex_none()
                        .pl(SPACE_XS)
                        .text_size(TEXT_XS)
                        .text_color(theme.text_muted)
                        .font_features(tabular_figures())
                        .whitespace_nowrap()
                        .child(progress)
                }))
                .child(refresh_button(
                    spinning,
                    status.clone(),
                    self.demo_synced_at,
                    &theme,
                    window,
                    cx,
                ))
            })
            .children(primary.map(|button| div().px(SPACE_XS).child(button)))
            .child(account_button(
                &account,
                photo,
                source,
                can_sign_out,
                &theme,
            ));

        window_drag_area(div().id("top-bar"))
            .role(gpui::Role::Toolbar)
            .aria_label("Top bar")
            .relative()
            .size_full()
            .flex()
            .items_center()
            .bg(theme.bar_bg)
            .child(leading)
            .child(center)
            .child(trailing)
            // Over the panes only: the rail goes on under the bar, and its
            // own line meets this one at the corner.
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left(RAIL_WIDTH - HAIRLINE)
                    .right_0()
                    .h(HAIRLINE)
                    .bg(theme.border),
            )
            .into_any_element()
    }

    /// Mail's search (SRC-1): the field, with where it looks, Clear, and
    /// ⌘K for the command palette at its end. Suggestions drop down under
    /// it; the results fill the list.
    fn render_mail_search(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = self.search_input.focus_handle(cx).is_focused(window);
        let has_text = !self.search_input.read(cx).value().is_empty();
        let open = self.search_open;

        // Where the search looks; a click widens it to all mail. An `in:`
        // in the query decides instead, and the chip names it (ME-016).
        let fixed = self.search_query.scope.is_some();
        let scope_label = self.search_scope_name();
        let scope = div()
            .id("search-scope")
            .flex_none()
            .h(CONTROL_XS)
            .pl(SPACE_SM)
            .pr(if fixed { SPACE_SM } else { SPACE_XS })
            .flex()
            .items_center()
            .gap(SPACE_2XS)
            .rounded(RADIUS_SM)
            .bg(theme.chip_bg)
            .text_size(TEXT_SM)
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .whitespace_nowrap()
            .child(scope_label.clone())
            .map(|this| {
                if fixed {
                    this.role(gpui::Role::Label)
                        .aria_label(format!(
                            "Searching {scope_label}, set by in: in your search"
                        ))
                        .tooltip(|window, cx| {
                            tooltip("Set by in: in your search").build(window, cx)
                        })
                } else {
                    this.role(gpui::Role::Button)
                        .aria_label(format!("Search in {scope_label}"))
                        .child(icon(Icon::ChevronDown, ICON_SM, theme.text_faint))
                        .hover(|s| s.bg(theme.selected_bg).text_color(theme.text))
                        .active(|s| s.bg(theme.selected_bg).opacity(0.8))
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_search_scope(cx)))
                        .tooltip(|window, cx| {
                            tooltip("Switch between this folder and All Mail").build(window, cx)
                        })
                }
            });

        // MA-082: X clears the text first and closes only an empty search.
        let clear = icon_button_small(
            "close-search",
            Icon::X,
            if has_text {
                "Clear search"
            } else {
                "Close search"
            },
            theme,
        )
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            if has_text {
                this.clear_search_text(window, cx);
            } else {
                window.dispatch_action(Box::new(CloseSearch), cx);
            }
        }));

        let input = self.search_input.clone();
        let field = super::search_field(Some(focused), theme)
            .id("top-bar-search")
            .key_context(SEARCH)
            .relative()
            .w_full()
            .min_w_0()
            .h(CONTROL_MD)
            .pl(SPACE_SM)
            .pr(SPACE_XS)
            .text_size(TEXT_SM)
            .when(!focused, |this| this.hover(|s| s.bg(theme.hover_bg)))
            .child(icon(
                Icon::Search,
                ICON_MD,
                if focused {
                    theme.text_muted
                } else {
                    theme.text_faint
                },
            ))
            .when(open, |this| this.child(scope))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(Input::new(&self.search_input).appearance(false).px(px(0.))),
            )
            .when(open || has_text, |this| this.child(clear))
            .child(palette_key(theme))
            // A click anywhere on it types into it, and doesn't move the
            // window.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                if !input.focus_handle(cx).is_focused(window) {
                    input.update(cx, |input, cx| input.focus(window, cx));
                }
            });
        let suggestions = self.search_suggestions(window, cx);
        // Mail's shortcuts still work while typing, as they did in the list.
        div()
            .key_context(MAILBOX)
            .relative()
            .w_full()
            .min_w_0()
            .child(field)
            .when(!suggestions.is_empty(), |this| {
                this.child(self.render_search_suggestions(suggestions, theme, window, cx))
            })
    }
}

/// Calendar's field: Find there searches the events, in the palette
/// (MA-081).
fn calendar_search(theme: &Theme) -> impl IntoElement {
    focusable_control(div().id("top-bar-search-events"), theme)
        .role(gpui::Role::Button)
        .aria_label("Search events")
        .when_some(shortcut_text(&ToggleSearch), |this, keys| {
            this.aria_keyshortcuts(keys)
        })
        .w_full()
        .min_w_0()
        .h(CONTROL_MD)
        .pl(SPACE_SM)
        .pr(SPACE_XS)
        .flex()
        .items_center()
        .gap(SPACE_SM)
        .rounded(RADIUS_MD)
        .bg(theme.search_bg)
        .border_1()
        .border_color(theme.border)
        .text_size(TEXT_SM)
        .text_color(theme.text_faint)
        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text_muted))
        .active(|s| s.bg(theme.selected_bg))
        .child(icon(Icon::Search, ICON_MD, theme.text_faint))
        .child(div().flex_1().min_w_0().truncate().child("Search events…"))
        .child(palette_key(theme))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleSearch), cx))
        .tooltip(|window, cx| {
            tooltip("Search events")
                .action(&ToggleSearch, None)
                .build(window, cx)
        })
}

/// ⌘K at the field's end: the command palette, which finds commands,
/// views and conversations.
fn palette_key(theme: &Theme) -> impl IntoElement {
    let keys = shortcut_text(&ToggleCommandPalette);
    focusable_control(div().id("top-bar-command"), theme)
        .role(gpui::Role::Button)
        .aria_label("Command palette")
        .when_some(keys.clone(), |this, keys| this.aria_keyshortcuts(keys))
        .flex_none()
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .map(|this| match keys {
            Some(keys) => this.child(keycap(keys, theme)),
            None => this.size(CONTROL_XS).justify_center().child(icon(
                Icon::Command,
                ICON_SM,
                theme.text_faint,
            )),
        })
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleCommandPalette), cx))
        .tooltip(|window, cx| {
            tooltip("Search and run commands")
                .action(&ToggleCommandPalette, None)
                .build(window, cx)
        })
}

/// "340 of 1,000" while a sync counts what it fetches, shown by Refresh
/// (STA-1).
fn sync_progress(status: &SyncStatus) -> Option<String> {
    match *status {
        SyncStatus::Syncing { done, total } if total > 0 => Some(format!(
            "{} of {}",
            status::thousands(done),
            status::thousands(total)
        )),
        _ => None,
    }
}

/// "Synced 4 min ago": Refresh's tooltip after "Refresh · ", and its
/// accessible description.
fn refresh_state(status: &SyncStatus, demo_synced_at: Option<i64>, now_ms: i64) -> String {
    match (status, demo_synced_at) {
        (SyncStatus::Demo, Some(at)) => format!("Demo data · Synced {}", status::ago(at, now_ms)),
        _ => status::label(status, now_ms),
    }
}

/// "Refresh · Synced 4 min ago", worked out as the tooltip shows.
fn refresh_tip(status: &SyncStatus, demo_synced_at: Option<i64>, now_ms: i64) -> String {
    format!(
        "Refresh · {}",
        refresh_state(status, demo_synced_at, now_ms)
    )
}

/// Refresh (`SyncNow`): turns while a sync runs, and for a moment after a
/// click even when there was nothing to fetch; then finishes its turn.
fn refresh_button(
    spinning: bool,
    status: SyncStatus,
    demo_synced_at: Option<i64>,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let glyph = icon(Icon::RefreshCw, ICON_TOOLBAR, theme.icon);
    let glyph = spinner_while("refresh-spinner", spinning, glyph, window, cx);
    let state: SharedString = refresh_state(&status, demo_synced_at, status::now_ms()).into();
    focusable_control(div().id("top-bar-refresh"), theme)
        .role(gpui::Role::Button)
        .aria_label("Refresh")
        .aria_description(state)
        .when_some(shortcut_text(&SyncNow), |this, keys| {
            this.aria_keyshortcuts(keys)
        })
        .size(HIT_TARGET)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(glyph)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, window, cx| window.dispatch_action(Box::new(SyncNow), cx))
        .tooltip(move |window, cx| {
            tooltip(refresh_tip(&status, demo_synced_at, status::now_ms()))
                .action(&SyncNow, None)
                .build(window, cx)
        })
}

/// MA-068: offline (with what's waiting) or signed out, as one glyph with
/// the status line's words in its tooltip. Nothing while all is well;
/// syncing turns Refresh instead. "Sign in again" is a button.
fn status_glyph(status: &SyncStatus, theme: &Theme) -> Option<AnyElement> {
    let (glyph, color) = match status {
        SyncStatus::Offline { .. } => (Icon::OctagonAlert, theme.text_warning),
        SyncStatus::AuthExpired => (Icon::LogIn, theme.text_danger),
        _ => return None,
    };
    let text: SharedString = status::label(status, 0).into();
    let tip = text.clone();
    let el = div()
        .id("top-bar-status")
        .size(HIT_TARGET)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .child(icon(glyph, ICON_TOOLBAR, color))
        .tooltip(move |window, cx| tooltip(tip.clone()).build(window, cx));
    Some(if *status == SyncStatus::AuthExpired {
        focusable_control(el, theme)
            .role(gpui::Role::Button)
            .aria_label(text)
            .hover(|s| s.bg(theme.hover_bg))
            .active(|s| s.bg(theme.selected_bg))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, window, cx| window.dispatch_action(Box::new(SignIn), cx))
            .into_any_element()
    } else {
        el.role(gpui::Role::Status)
            .aria_label(text)
            .into_any_element()
    })
}

/// On demo data: says so, and signs in.
fn demo_chip(short: bool, theme: &Theme) -> impl IntoElement {
    focusable_control(div().id("demo-sign-in"), theme)
        .role(gpui::Role::Button)
        .aria_label("Demo data. Sign in with Google")
        .flex_none()
        .h(CONTROL_SM)
        .px(SPACE_SM)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .text_size(TEXT_XS)
        .text_color(theme.text_faint)
        .bg(theme.chip_bg)
        .hover(|s| s.bg(theme.hover_bg).text_color(theme.text_muted))
        .active(|s| s.bg(theme.selected_bg))
        .whitespace_nowrap()
        .child(if short { DEMO_SHORT } else { DEMO_LABEL })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, window, cx| window.dispatch_action(Box::new(SignIn), cx))
        .tooltip(|window, cx| {
            tooltip("Showing demo data; sign in with Google to use your mail")
                .action(&SignIn, None)
                .build(window, cx)
        })
}

/// Compose in Mail, New Event in Calendar.
fn primary_button(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    tip: &'static str,
    action: Box<dyn Action>,
    icon_only: bool,
    theme: &Theme,
) -> impl IntoElement {
    let tip_action = action.boxed_clone();
    push_button_frame(id, label, ButtonKind::Primary, theme)
        .when_some(shortcut_text(action.as_ref()), |this, keys| {
            this.aria_keyshortcuts(keys)
        })
        .gap(SPACE_XS)
        .map(|this| {
            if icon_only {
                this.px(px(0.)).w(CONTROL_MD).min_w(HIT_TARGET)
            } else {
                this.pl(SPACE_SM)
            }
        })
        .child(icon(glyph, ICON_MD, theme.accent_text))
        .when(!icon_only, |this| this.child(label))
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .tooltip(move |window, cx| {
            tooltip(tip)
                .action(tip_action.as_ref(), None)
                .build(window, cx)
        })
}

/// The account: who this mail is for. Opens a menu with Account Settings
/// (ME-029) and signing in or out; the address is in its tooltip.
fn account_button(
    account: &Contact,
    photo: Option<std::path::PathBuf>,
    source: Source,
    can_sign_out: bool,
    theme: &Theme,
) -> impl IntoElement {
    let label = format!("{}, {}", account.name, account.email);
    let tip: SharedString = match source {
        Source::Demo => format!("{} · Demo data", account.email).into(),
        Source::SignedOut => "Not signed in".into(),
        Source::Google => account.email.clone().into(),
    };
    let email: SharedString = account.email.clone().into();
    focusable_control(div().id("account"), theme)
        .role(gpui::Role::Button)
        .aria_label(label)
        .aria_description("Opens the account menu")
        .flex_none()
        .min_w(HIT_TARGET)
        .min_h(HIT_TARGET)
        .p(SPACE_2XS)
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .hover(|s| s.bg(theme.hover_bg))
        .active(|s| s.bg(theme.selected_bg))
        .child(account_avatar(
            photo.as_deref(),
            account.initials(),
            theme.avatar_color(&account.email),
            AVATAR_XS,
            theme,
        ))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_: &ClickEvent, window, cx| {
            account_menu(email.clone(), source, can_sign_out).show(menu_anchor(window), window, cx)
        })
        .tooltip(move |window, cx| tooltip(tip.clone()).build(window, cx))
}

/// Under the bar's right end: the menu lines up with the window's edge.
fn menu_anchor(window: &Window) -> gpui::Point<Pixels> {
    point(
        window.viewport_size().width - SPACE_SM,
        super::top_bar_height(window),
    )
}

fn account_menu(email: SharedString, source: Source, can_sign_out: bool) -> ContextMenu {
    let menu = ContextMenu::new()
        .label("Account")
        .header(email)
        .item(
            Item::new(
                "Account Settings…",
                Box::new(OpenSettingsSection(Section::Accounts)),
            )
            .icon(Icon::User),
        )
        .item(Item::new("Settings…", Box::new(OpenSettings)).icon(Icon::Settings))
        .separator();
    // On demo data opened with PIGEON_DEMO, it says how to sign in instead.
    let menu = if source != Source::Google {
        menu.item(Item::new("Sign In…", Box::new(SignIn)).icon(Icon::LogIn))
    } else {
        menu
    };
    if can_sign_out {
        menu.item(Item::new("Sign Out", Box::new(SignOut)).icon(Icon::LogOut))
    } else {
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trailing_end_gives_way_before_the_search() {
        let busy = Trailing {
            status: false,
            demo: true,
            progress: Some("1,340 of 10,000"),
            primary: Some("Compose"),
        };
        let rem = BASE_REM;
        // Wide: everything shows.
        assert_eq!(compact_for(1400., &busy, rem), Compact::Full);
        // Each step frees room, in order.
        let widths: Vec<f32> = [
            Compact::Full,
            Compact::NoProgress,
            Compact::ShortChip,
            Compact::IconOnly,
        ]
        .iter()
        .map(|&c| trailing_width(&busy, c, rem))
        .collect();
        assert!(widths.windows(2).all(|w| w[1] < w[0]), "{widths:?}");
        // The minimum window with 200% text, past the lights and the
        // sidebar toggle: the progress goes, and the search still fits.
        let rem = BASE_REM * 2.;
        let search = SEARCH_MIN_WIDTH.0 * rem;
        let room = 900. - 80. - 36.;
        let at_min = compact_for(room, &busy, rem);
        assert!(at_min > Compact::Full, "{at_min:?}");
        assert!(room - trailing_width(&busy, at_min, rem) >= search);
        // Narrower still, it steps on in order, down to icons.
        let steps: Vec<Compact> = (0..=40)
            .map(|i| compact_for(room - i as f32 * 10., &busy, rem))
            .collect();
        assert!(steps.windows(2).all(|w| w[1] >= w[0]), "{steps:?}");
        assert_eq!(steps.last(), Some(&Compact::IconOnly));
        // Whenever some step leaves the search its minimum, it gets it.
        for i in 0..=40 {
            let room = room - i as f32 * 10.;
            if room - trailing_width(&busy, Compact::IconOnly, rem) >= search {
                let c = compact_for(room, &busy, rem);
                assert!(room - trailing_width(&busy, c, rem) >= search, "{room}");
            }
        }
        // Without progress, the progress step changes nothing.
        let calm = Trailing {
            progress: None,
            ..busy
        };
        assert_eq!(
            trailing_width(&calm, Compact::Full, rem),
            trailing_width(&calm, Compact::NoProgress, rem)
        );
    }

    #[test]
    fn refresh_tip_says_when_it_last_synced() {
        let now = 10 * 60_000;
        assert_eq!(
            refresh_tip(&SyncStatus::Synced { at: now }, None, now),
            "Refresh · Synced just now"
        );
        // Demo data: after a refresh, when that was.
        assert_eq!(
            refresh_tip(&SyncStatus::Demo, None, now),
            "Refresh · Demo data"
        );
        assert_eq!(
            refresh_tip(&SyncStatus::Demo, Some(now - 4 * 60_000), now),
            "Refresh · Demo data · Synced 4 min ago"
        );
    }
}

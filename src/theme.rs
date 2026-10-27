//! All of Pigeon's colors. Views read semantic colors from here and never
//! hardcode their own.

use gpui::{App, Global, Hsla, Window, WindowAppearance, hsla, rgba, transparent_black};

#[derive(Clone, Debug)]
pub struct Theme {
    pub is_dark: bool,

    // Pane tints, painted over the blurred window background. Enough alpha
    // that text keeps 4.5:1 whatever the wallpaper, while it still tints
    // through; the sidebar is the most see-through, the reader the most solid. GPUI adds the alphas of stacked layers (and clamps), so
    // a light layer over a light pane turns gray rather than whiter: keep
    // light-mode cards and fields faint.
    pub rail_bg: Hsla,
    pub sidebar_bg: Hsla,
    pub list_bg: Hsla,
    pub reader_bg: Hsla,
    pub border: Hsla,

    // Text.
    pub text: Hsla,
    pub text_muted: Hsla,
    /// Read rows in the thread list.
    pub text_dim: Hsla,
    pub text_faint: Hsla,

    // Interactive surfaces.
    pub hover_bg: Hsla,
    pub selected_bg: Hsla,
    /// The list's selection while the list has keyboard focus.
    pub selected_focused_bg: Hsla,
    /// Rows picked for a bulk action (`x`, shift- and ⌘-click).
    pub checked_bg: Hsla,
    pub nav_active_bg: Hsla,
    pub card_bg: Hsla,
    /// The message `n` / `p` moved to.
    pub card_focused_bg: Hsla,
    pub chip_bg: Hsla,
    pub search_bg: Hsla,
    /// An off switch's track, and the well of a segmented control.
    pub control_track: Hsla,
    /// A switch's knob.
    pub control_knob: Hsla,
    /// The chosen segment of a segmented control.
    pub segment_bg: Hsla,

    pub icon: Hsla,
    pub icon_active: Hsla,
    pub accent: Hsla,
    /// Text on an `accent` background.
    pub accent_text: Hsla,
    pub danger: Hsla,
    /// An accepted invite.
    pub success: Hsla,
    pub star: Hsla,

    // Floating overlays: the command palette and the shortcuts sheet.
    pub backdrop: Hsla,
    pub overlay_bg: Hsla,
    pub overlay_border: Hsla,
    pub overlay_shadow: Hsla,
    /// Matched characters in palette results.
    pub match_text: Hsla,
    // Keycaps.
    pub kbd_bg: Hsla,
    pub kbd_border: Hsla,
    pub kbd_text: Hsla,

    /// Avatar backgrounds, picked by hashing the contact's email.
    pub avatar_palette: [Hsla; 8],
    pub avatar_text: Hsla,

    // Calendar. Event colors derive from their calendar's color; see
    // `event_fill` and friends.
    pub cal_bg: Hsla,
    /// The event panel docked on the right.
    pub cal_panel_bg: Hsla,
    /// Hour lines and day dividers.
    pub cal_line: Hsla,
    /// Half-hour lines.
    pub cal_line_faint: Hsla,
    pub cal_weekend_bg: Hsla,
    /// Today's column in the week view and its cell in the month view.
    pub cal_today_bg: Hsla,
    /// The current-time line.
    pub cal_now: Hsla,
    /// Text on a selected (solid) event.
    pub cal_selected_text: Hsla,
    /// The edge between an event and one drawn over it.
    pub cal_ring: Hsla,
}

impl Global for Theme {}

fn hex(value: u32) -> Hsla {
    rgba(value).into()
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            is_dark: true,
            rail_bg: hex(0x0c0c10b3),
            sidebar_bg: hex(0x111115bf),
            list_bg: hex(0x141418d1),
            reader_bg: hex(0x18181ce0),
            border: hex(0xffffff14),

            text: hex(0xf4f4f7ff),
            text_muted: hex(0xb8b8c1ff),
            text_dim: hex(0xa8a8b2ff),
            text_faint: hex(0x9a9aa4ff),

            hover_bg: hex(0xffffff0a),
            selected_bg: hex(0xffffff1c),
            selected_focused_bg: hex(0x7aa5ff38),
            checked_bg: hex(0x7aa5ff1f),
            nav_active_bg: hex(0xffffff17),
            card_bg: hex(0xffffff0a),
            card_focused_bg: hex(0xffffff0d),
            chip_bg: hex(0xffffff12),
            search_bg: hex(0xffffff14),
            control_track: hex(0xffffff1f),
            control_knob: hex(0xf4f4f7ff),
            segment_bg: hex(0xffffff2b),

            icon: hex(0xadadb7ff),
            icon_active: hex(0xf2f2f5ff),
            accent: hex(0x7aa5ffff),
            accent_text: hex(0x0b1530ff),
            danger: hex(0xff7b72ff),
            success: hex(0x4fd07fff),
            star: hex(0xf5bf4fff),

            backdrop: hex(0x00000052),
            overlay_bg: hex(0x222226fd),
            overlay_border: hex(0xffffff1c),
            overlay_shadow: hex(0x00000099),
            match_text: hex(0x8fb3ffff),
            kbd_bg: hex(0xffffff10),
            kbd_border: hex(0xffffff17),
            kbd_text: hex(0xb8b8c1ff),

            avatar_palette: Self::avatar_palette(0.42),
            avatar_text: hex(0xffffffff),

            cal_bg: hex(0x18181ce0),
            cal_panel_bg: hex(0x1c1c20ea),
            cal_line: hex(0xffffff12),
            cal_line_faint: hex(0xffffff08),
            cal_weekend_bg: hex(0x00000010),
            cal_today_bg: hex(0x6e9bff0a),
            cal_now: hex(0xff5f57ff),
            cal_selected_text: hex(0xffffffff),
            cal_ring: hex(0x1a1a1eff),
        }
    }

    pub fn light() -> Self {
        Self {
            is_dark: false,
            rail_bg: hex(0xf2f2f699),
            sidebar_bg: hex(0xf7f7faa6),
            list_bg: hex(0xfafafcbf),
            reader_bg: hex(0xffffffd1),
            border: hex(0x0000001a),

            text: hex(0x141417ff),
            text_muted: hex(0x48484fff),
            text_dim: hex(0x52525aff),
            text_faint: hex(0x5a5a62ff),

            hover_bg: hex(0x0000000a),
            selected_bg: hex(0x00000012),
            selected_focused_bg: hex(0x1d5bd11c),
            checked_bg: hex(0x1d5bd10f),
            nav_active_bg: hex(0x00000012),
            card_bg: hex(0xffffff00),
            card_focused_bg: hex(0x00000008),
            chip_bg: hex(0x0000000d),
            search_bg: hex(0x0000000a),
            control_track: hex(0x0000001c),
            control_knob: hex(0xffffffff),
            segment_bg: hex(0xffffffff),

            icon: hex(0x55555dff),
            icon_active: hex(0x1c1c1fff),
            accent: hex(0x1d5bd1ff),
            accent_text: hex(0xffffffff),
            danger: hex(0xd93a32ff),
            success: hex(0x1e9a52ff),
            star: hex(0xe2a21cff),

            backdrop: hex(0x0000001f),
            overlay_bg: hex(0xfdfdfefd),
            overlay_border: hex(0x00000017),
            overlay_shadow: hex(0x00000038),
            match_text: hex(0x1d5bd1ff),
            kbd_bg: hex(0x0000000a),
            kbd_border: hex(0x00000014),
            kbd_text: hex(0x48484fff),

            avatar_palette: Self::avatar_palette(0.5),
            avatar_text: hex(0xffffffff),

            cal_bg: hex(0xffffffd1),
            cal_panel_bg: hex(0xfbfbfdd9),
            cal_line: hex(0x00000014),
            cal_line_faint: hex(0x0000000a),
            cal_weekend_bg: hex(0x00000007),
            cal_today_bg: hex(0x1d5bd108),
            cal_now: hex(0xe5483bff),
            cal_selected_text: hex(0xffffffff),
            cal_ring: hex(0xffffffff),
        }
    }

    fn avatar_palette(lightness: f32) -> [Hsla; 8] {
        [0.58, 0.72, 0.85, 0.95, 0.04, 0.10, 0.36, 0.47].map(|h| hsla(h, 0.45, lightness, 1.0))
    }

    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        }
    }

    pub fn of(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Chip background for a label color.
    pub fn label_chip_bg(&self, color: Hsla) -> Hsla {
        color.alpha(if self.is_dark { 0.2 } else { 0.14 })
    }

    /// Chip text for a label color, nudged for contrast on the current background.
    pub fn label_chip_text(&self, color: Hsla) -> Hsla {
        let l = if self.is_dark {
            (color.l + 0.18).min(0.82)
        } else {
            (color.l - 0.12).max(0.25)
        };
        hsla(color.h, color.s, l, 1.0)
    }

    /// Background of an event block or bar. Mostly opaque, so events stay
    /// crisp over the glass; past events fade back.
    pub fn event_fill(&self, color: Hsla, past: bool) -> Hsla {
        let fill = if self.is_dark {
            hsla(color.h, color.s * 0.6, 0.3, 0.9)
        } else {
            hsla(color.h, (color.s * 0.85).min(0.9), 0.93, 0.96)
        };
        if past {
            fill.alpha(fill.a * if self.is_dark { 0.45 } else { 0.55 })
        } else {
            fill
        }
    }

    pub fn event_fill_hover(&self, color: Hsla) -> Hsla {
        if self.is_dark {
            hsla(color.h, color.s * 0.6, 0.34, 0.95)
        } else {
            hsla(color.h, (color.s * 0.85).min(0.9), 0.895, 1.)
        }
    }

    /// Text on an event's fill, and of timed events in the month view.
    pub fn event_text(&self, color: Hsla, past: bool) -> Hsla {
        let text = if self.is_dark {
            hsla(color.h, color.s * 0.75, 0.86, 1.)
        } else {
            hsla(color.h, color.s.min(0.75), 0.27, 1.)
        };
        if past { text.alpha(0.6) } else { text }
    }

    /// The stripe on an event's leading edge, and its dot.
    pub fn event_accent(&self, color: Hsla, past: bool) -> Hsla {
        let accent = if self.is_dark {
            hsla(color.h, color.s, (color.l + 0.06).min(0.7), 1.)
        } else {
            hsla(color.h, color.s, color.l.min(0.55), 1.)
        };
        if past { accent.alpha(0.55) } else { accent }
    }

    /// A selected event: solid in its color.
    pub fn event_selected_fill(&self, color: Hsla) -> Hsla {
        if self.is_dark {
            hsla(color.h, color.s * 0.8, 0.48, 1.)
        } else {
            hsla(color.h, color.s, color.l.min(0.52), 1.)
        }
    }

    pub fn avatar_color(&self, seed: &str) -> Hsla {
        let hash = seed
            .bytes()
            .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        self.avatar_palette[hash as usize % self.avatar_palette.len()]
    }

    /// Install the theme matching the window's appearance, and keep
    /// gpui-component's theme (scrollbars, resize handles, future inputs) in step
    /// with it.
    pub fn sync(window: &mut Window, cx: &mut App) {
        Self::apply(window.appearance(), window, cx);
    }

    /// Install the theme for `appearance`, which the user may have chosen
    /// over the window's.
    pub fn apply(appearance: WindowAppearance, window: &mut Window, cx: &mut App) {
        let theme = Self::for_appearance(appearance);
        let (border, muted) = (theme.border, theme.text_muted);
        cx.set_global(theme);

        use gpui::component::Theme as ComponentTheme;
        ComponentTheme::change(appearance, Some(window), cx);
        ComponentTheme::update(cx, |t| {
            // The window is translucent; panes paint their own tints.
            t.colors.background = transparent_black();
            t.colors.border = border;
            t.colors.muted_foreground = muted;
            t.colors.window_border = transparent_black();
        });
    }
}

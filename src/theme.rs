//! All of Pigeon's colors. Views read semantic colors from here and never
//! hardcode their own.
//!
//! The palette follows the system: light or dark, Increase Contrast, Reduce
//! Transparency, the accent and highlight colors (MA-009, MA-010, MA-014),
//! and the user's glass amount (`Saved::glass`).

use gpui::{
    App, Global, Hsla, Rgba, Window, WindowAppearance, WindowBackgroundAppearance, hsla, rgba,
    transparent_black,
};

use crate::{
    native,
    settings::{self, Appearance},
};

#[derive(Clone, Debug)]
pub struct Theme {
    pub is_dark: bool,
    /// Increase Contrast is on: stronger lines, grays and states, opaque
    /// panes, past events at full strength.
    pub high_contrast: bool,
    /// The panes are fully opaque and so is the window behind them.
    pub opaque: bool,
    /// What the window's glass looks like with nothing on it: the gray an
    /// opaque pane is composited over, and what contrast is judged against.
    pub material: Hsla,

    // Pane tints, painted over the blurred window background. Enough alpha
    // that text keeps 4.5:1 whatever the wallpaper, while it still tints
    // through; the reader is the most solid. GPUI adds the alphas of stacked
    // layers (and clamps), so a light layer over a light pane turns gray
    // rather than whiter: keep light-mode cards and fields faint.
    pub rail_bg: Hsla,
    pub sidebar_bg: Hsla,
    pub list_bg: Hsla,
    pub reader_bg: Hsla,
    pub border: Hsla,

    // Text: primary, secondary, tertiary, and disabled (≈3:1).
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub text_disabled: Hsla,
    /// Status colors as text and small icons, at 4.5:1 (MA-015). `success`,
    /// `danger` and `warning` stay for fills, dots and badges.
    pub text_success: Hsla,
    pub text_danger: Hsla,
    pub text_warning: Hsla,
    /// Links, in the accent's hue at 4.5:1.
    pub link: Hsla,

    // Interactive surfaces.
    pub hover_bg: Hsla,
    pub selected_bg: Hsla,
    /// The list's selection while the list has keyboard focus.
    pub selected_focused_bg: Hsla,
    /// The quiet bar on the focused list's selected row.
    pub focus_bar: Hsla,
    /// Rows picked for a bulk action (`x`, shift- and ⌘-click).
    pub checked_bg: Hsla,
    /// The outline on the row a context menu is open on.
    pub context_ring: Hsla,
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
    /// An on switch's track: deep enough for the knob to stand out.
    pub switch_on: Hsla,
    /// The chosen segment of a segmented control.
    pub segment_bg: Hsla,

    pub icon: Hsla,
    pub icon_active: Hsla,
    pub accent: Hsla,
    /// Text on an `accent` background.
    pub accent_text: Hsla,
    /// Behind selected text in the reader.
    pub text_selection: Hsla,
    pub danger: Hsla,
    /// Text on a `danger` background.
    pub danger_text: Hsla,
    /// An accepted invite.
    pub success: Hsla,
    /// Offline, and other states that need attention but aren't errors.
    pub warning: Hsla,
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

    /// Avatar backgrounds, picked by hashing the contact's email; their
    /// initials are `on_fill`.
    pub avatar_palette: [Hsla; 8],

    // Calendar. Event colors derive from their calendar's color; see
    // `event_fill` and friends, and `on_fill` for text on a solid one.
    pub cal_bg: Hsla,
    /// The event panel docked on the right.
    pub cal_panel_bg: Hsla,
    /// Hour lines and day dividers.
    pub cal_line: Hsla,
    /// Half-hour lines.
    pub cal_line_faint: Hsla,
    pub cal_weekend_bg: Hsla,
    /// Days outside the month in the month view: deeper than a weekend.
    pub cal_outside_bg: Hsla,
    /// Today's column in the week view and its cell in the month view.
    pub cal_today_bg: Hsla,
    /// The current-time line, and its label's fill.
    pub cal_now: Hsla,
    /// The edge between an event and one drawn over it.
    pub cal_ring: Hsla,

    // The menu bar panel (W7b): its glass, and the next-meeting card.
    pub menu_bar_bg: Hsla,
    pub menu_bar_card: Hsla,
}

/// What the system's settings ask of the palette: read at launch, on their
/// notifications and when Pigeon comes forward (`native::Signal::DisplayChanged`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Display {
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    /// The system accent; none for Multicolor, which keeps Pigeon's blue.
    pub accent: Option<Rgba>,
    pub selection: Option<Rgba>,
}

impl Global for Display {}

impl Display {
    pub fn read() -> Self {
        Self {
            increase_contrast: native::increase_contrast(),
            reduce_transparency: native::reduce_transparency(),
            accent: native::accent_color(),
            selection: native::selection_color(),
        }
    }
}

impl Global for Theme {}

fn hex(value: u32) -> Hsla {
    rgba(value).into()
}

impl Theme {
    pub fn dark() -> Self {
        let danger = hex(0xff7b72ff);
        Self {
            is_dark: true,
            high_contrast: false,
            opaque: false,
            material: hex(0x1e1e21ff),
            rail_bg: hex(0x0c0c10cc),
            sidebar_bg: hex(0x111115cc),
            list_bg: hex(0x141418d1),
            reader_bg: hex(0x18181ce0),
            border: hex(0xffffff14),

            text: hex(0xf4f4f7ff),
            text_muted: hex(0xb8b8c1ff),
            text_faint: hex(0x9a9aa4ff),
            text_disabled: hex(0x6b6b74ff),
            text_success: hex(0x4fd07fff),
            text_danger: danger,
            text_warning: hex(0xf5bf4fff),
            link: hex(0x8fb3ffff),

            hover_bg: hex(0xffffff0a),
            selected_bg: hex(0xffffff1c),
            selected_focused_bg: hex(0x7aa5ff38),
            focus_bar: hex(0x7aa5ff8c),
            checked_bg: hex(0x7aa5ff1f),
            context_ring: hex(0x7aa5ff99),
            nav_active_bg: hex(0xffffff17),
            card_bg: hex(0xffffff0a),
            card_focused_bg: hex(0xffffff0d),
            chip_bg: hex(0xffffff12),
            search_bg: hex(0xffffff14),
            control_track: hex(0xffffff1f),
            control_knob: hex(0xf4f4f7ff),
            switch_on: hex(0x3a6bd6ff),
            segment_bg: hex(0xffffff2b),

            icon: hex(0xadadb7ff),
            icon_active: hex(0xf2f2f5ff),
            accent: hex(0x7aa5ffff),
            accent_text: hex(0x0b1530ff),
            text_selection: hex(0x7aa5ff59),
            danger,
            danger_text: on_fill(danger),
            success: hex(0x4fd07fff),
            warning: hex(0xf5a524ff),
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

            cal_bg: hex(0x18181ce0),
            cal_panel_bg: hex(0x1c1c20ea),
            cal_line: hex(0xffffff12),
            cal_line_faint: hex(0xffffff08),
            cal_weekend_bg: hex(0x00000010),
            cal_outside_bg: hex(0x00000026),
            cal_today_bg: hex(0x6e9bff0a),
            cal_now: hex(0xd93a32ff),
            cal_ring: hex(0x1a1a1eff),

            menu_bar_bg: hex(0x1e1e24cc),
            menu_bar_card: hex(0xffffff0d),
        }
    }

    pub fn light() -> Self {
        let danger = hex(0xd93a32ff);
        Self {
            is_dark: false,
            high_contrast: false,
            opaque: false,
            material: hex(0xececeeff),
            rail_bg: hex(0xf2f2f6cc),
            sidebar_bg: hex(0xf7f7facc),
            list_bg: hex(0xfafafcbf),
            reader_bg: hex(0xffffffd1),
            border: hex(0x0000001a),

            text: hex(0x141417ff),
            text_muted: hex(0x48484fff),
            text_faint: hex(0x5a5a62ff),
            text_disabled: hex(0x88888fff),
            text_success: hex(0x157a3fff),
            text_danger: hex(0xc4302aff),
            text_warning: hex(0x8a5d00ff),
            link: hex(0x1d5bd1ff),

            hover_bg: hex(0x0000000a),
            selected_bg: hex(0x00000012),
            selected_focused_bg: hex(0x1d5bd11c),
            focus_bar: hex(0x1d5bd166),
            checked_bg: hex(0x1d5bd10f),
            context_ring: hex(0x1d5bd180),
            nav_active_bg: hex(0x00000012),
            card_bg: hex(0xffffff00),
            card_focused_bg: hex(0x00000008),
            chip_bg: hex(0x0000000d),
            search_bg: hex(0x0000000a),
            control_track: hex(0x0000001c),
            control_knob: hex(0xffffffff),
            switch_on: hex(0x1d5bd1ff),
            segment_bg: hex(0xffffffff),

            icon: hex(0x55555dff),
            icon_active: hex(0x1c1c1fff),
            accent: hex(0x1d5bd1ff),
            accent_text: hex(0xffffffff),
            text_selection: hex(0x1d5bd138),
            danger,
            danger_text: on_fill(danger),
            success: hex(0x1e9a52ff),
            warning: hex(0xe09400ff),
            star: hex(0xb07a00ff),

            backdrop: hex(0x0000001f),
            overlay_bg: hex(0xfdfdfefd),
            overlay_border: hex(0x00000017),
            overlay_shadow: hex(0x00000038),
            match_text: hex(0x1d5bd1ff),
            kbd_bg: hex(0x0000000a),
            kbd_border: hex(0x00000014),
            kbd_text: hex(0x48484fff),

            avatar_palette: Self::avatar_palette(0.5),

            cal_bg: hex(0xffffffd1),
            cal_panel_bg: hex(0xfbfbfdd9),
            cal_line: hex(0x00000014),
            cal_line_faint: hex(0x0000000a),
            cal_weekend_bg: hex(0x00000007),
            cal_outside_bg: hex(0x00000012),
            cal_today_bg: hex(0x1d5bd108),
            cal_now: hex(0xc8322aff),
            cal_ring: hex(0xffffffff),

            menu_bar_bg: hex(0xfafafccc),
            menu_bar_card: hex(0xffffff8c),
        }
    }

    fn avatar_palette(lightness: f32) -> [Hsla; 8] {
        [0.58, 0.72, 0.85, 0.95, 0.04, 0.10, 0.36, 0.47].map(|h| hsla(h, 0.45, lightness, 1.0))
    }

    /// The palette for `appearance` with the system's `display` options and
    /// `glass` (0–100, `Saved::glass`).
    pub fn build(appearance: WindowAppearance, display: &Display, glass: u8) -> Self {
        let mut theme = match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        };
        if let Some(accent) = display.accent {
            theme.follow_accent(accent.into(), display.selection.map(Hsla::from));
        }
        if display.increase_contrast {
            theme.increase_contrast();
        }
        let glass = if display.reduce_transparency || display.increase_contrast {
            100
        } else {
            glass.min(100)
        };
        theme.thicken(glass);
        theme
    }

    /// MA-014: the system accent's hue for everything Pigeon's blue was.
    fn follow_accent(&mut self, system: Hsla, selection: Option<Hsla>) {
        let dark = self.is_dark;
        let surface = self.solid(self.reader_bg);
        let base = hsla(system.h, system.s, if dark { 0.72 } else { 0.45 }, 1.);
        // A control's parts need 3:1; text needs 4.5:1.
        let accent = readable(base, surface, 3.);
        self.accent = accent;
        self.accent_text = on_fill(accent);
        self.link = readable(accent, surface, 4.5);
        self.match_text = self.link;
        let alpha = |dark_a: f32, light_a: f32| accent.alpha(if dark { dark_a } else { light_a });
        self.selected_focused_bg = alpha(0.22, 0.11);
        self.focus_bar = alpha(0.55, 0.4);
        self.checked_bg = alpha(0.12, 0.06);
        self.context_ring = alpha(0.6, 0.5);
        self.cal_today_bg = alpha(0.04, 0.03);
        self.switch_on = if dark {
            hsla(system.h, system.s.min(0.75), 0.5, 1.)
        } else {
            accent
        };
        if let Some(selection) = selection {
            self.text_selection = selection.alpha(self.text_selection.a);
        }
    }

    /// MA-009: Increase Contrast.
    fn increase_contrast(&mut self) {
        self.high_contrast = true;
        let ink = |a: u8| {
            let base = if self.is_dark { 1. } else { 0. };
            hsla(0., 0., base, a as f32 / 255.)
        };
        self.border = ink(0x40);
        self.overlay_border = ink(0x40);
        self.kbd_border = ink(0x40);
        self.hover_bg = ink(0x14);
        self.selected_bg = ink(0x2e);
        self.nav_active_bg = ink(0x2e);
        self.chip_bg = ink(0x1f);
        self.search_bg = ink(0x1a);
        self.control_track = ink(0x47);
        self.cal_line = ink(0x2e);
        self.cal_line_faint = ink(0x17);
        self.selected_focused_bg = self
            .accent
            .alpha((self.selected_focused_bg.a * 1.6).min(1.));
        self.focus_bar = self.accent;
        self.context_ring = self.accent;
        if self.is_dark {
            self.text_muted = hex(0xd2d2daff);
            self.text_faint = hex(0xc4c4ccff);
            self.text_disabled = hex(0x85858eff);
            self.icon = hex(0xd2d2daff);
            self.kbd_text = self.text_muted;
        } else {
            self.text_muted = hex(0x2e2e33ff);
            self.text_faint = hex(0x3d3d44ff);
            self.text_disabled = hex(0x707078ff);
            self.icon = hex(0x2e2e33ff);
            self.kbd_text = self.text_muted;
        }
    }

    /// MA-010: blend the panes toward opaque by `glass` (0–100), composited
    /// over the material; at 100 they're solid and the window is opaque.
    fn thicken(&mut self, glass: u8) {
        if glass == 0 {
            return;
        }
        let t = glass as f32 / 100.;
        self.opaque = glass >= 100;
        let material = self.material;
        for pane in [
            &mut self.rail_bg,
            &mut self.sidebar_bg,
            &mut self.list_bg,
            &mut self.reader_bg,
            &mut self.cal_bg,
            &mut self.cal_panel_bg,
            &mut self.menu_bar_bg,
            &mut self.overlay_bg,
        ] {
            let (from, to) = (Rgba::from(*pane), Rgba::from(material.blend(*pane)));
            let mix = |a: f32, b: f32| a + (b - a) * t;
            *pane = Rgba {
                r: mix(from.r, to.r),
                g: mix(from.g, to.g),
                b: mix(from.b, to.b),
                a: if glass >= 100 { 1. } else { mix(from.a, 1.) },
            }
            .into();
        }
    }

    pub fn of(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// `pane` as it looks over the bare material, opaque: the background
    /// contrast is judged against.
    pub fn solid(&self, pane: Hsla) -> Hsla {
        self.material.blend(pane)
    }

    /// Text and icons on a solid `fill`, such as a selected event or the
    /// current-time label: white, or near-black where white doesn't read.
    pub fn on_fill(&self, fill: Hsla) -> Hsla {
        on_fill(self.material.blend(fill))
    }

    /// Chip background for a label color.
    pub fn label_chip_bg(&self, color: Hsla) -> Hsla {
        color.alpha(if self.is_dark { 0.2 } else { 0.14 })
    }

    /// Chip text for a label color: its hue, at 4.5:1 on the chip (MA-012).
    pub fn label_chip_text(&self, color: Hsla) -> Hsla {
        let l = if self.is_dark {
            (color.l + 0.18).min(0.82)
        } else {
            (color.l - 0.12).max(0.25)
        };
        let chip = self.solid(self.list_bg).blend(self.label_chip_bg(color));
        readable(hsla(color.h, color.s, l, 1.0), chip, 4.5)
    }

    /// Background of an event block or bar. Mostly opaque, so events stay
    /// crisp over the glass; past events fade back, except with Increase
    /// Contrast.
    pub fn event_fill(&self, color: Hsla, past: bool) -> Hsla {
        let fill = if self.is_dark {
            hsla(color.h, color.s * 0.6, 0.3, 0.9)
        } else {
            hsla(color.h, (color.s * 0.85).min(0.9), 0.93, 0.96)
        };
        if past && !self.high_contrast {
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

    /// Text on an event's fill, and of timed events in the month view. Past
    /// events fade their fill more than their text (MA-020).
    pub fn event_text(&self, color: Hsla, past: bool) -> Hsla {
        let text = if self.is_dark {
            hsla(color.h, color.s * 0.75, 0.86, 1.)
        } else {
            hsla(color.h, color.s.min(0.75), 0.27, 1.)
        };
        if past && !self.high_contrast {
            text.alpha(0.8)
        } else {
            text
        }
    }

    /// An event's second line (time, "until", the clipped chevrons) in
    /// `text`, its title's color. Full strength with Increase Contrast.
    pub fn event_secondary(&self, text: Hsla) -> Hsla {
        text.alpha(if self.high_contrast { 1. } else { 0.78 })
    }

    /// The light fill of an event not yet answered or declined (ME-041).
    pub fn event_answer_fill(&self, fill: Hsla) -> Hsla {
        fill.alpha(if self.high_contrast { 0.5 } else { 0.25 })
    }

    /// A free event's fill or dot, lighter than a busy one's (ME-051).
    pub fn event_free(&self, color: Hsla) -> Hsla {
        color.alpha(color.a * if self.high_contrast { 0.8 } else { 0.55 })
    }

    /// The stripe on an event's leading edge, and its dot.
    pub fn event_accent(&self, color: Hsla, past: bool) -> Hsla {
        let accent = if self.is_dark {
            hsla(color.h, color.s, (color.l + 0.06).min(0.7), 1.)
        } else {
            hsla(color.h, color.s, color.l.min(0.55), 1.)
        };
        if past && !self.high_contrast {
            accent.alpha(0.55)
        } else {
            accent
        }
    }

    /// A selected event: solid in its color. Its text is `on_fill`.
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

    /// Install the theme for the user's `appearance`, which `System` takes
    /// from the window (MC-085).
    pub fn sync_for(appearance: Appearance, window: &mut Window, cx: &mut App) {
        let appearance = appearance
            .window_appearance()
            .unwrap_or_else(|| window.appearance());
        Self::apply(appearance, window, cx);
    }

    /// Install the theme for `appearance`, which the user may have chosen
    /// over the window's, with the system's display options and the user's
    /// glass; make `window`'s background opaque or glass to match.
    pub fn apply(appearance: WindowAppearance, window: &mut Window, cx: &mut App) {
        let display = cx
            .try_global::<Display>()
            .copied()
            .unwrap_or_else(Display::read);
        let theme = Self::build(appearance, &display, settings::get(cx).glass);
        window.set_background_appearance(if theme.opaque {
            WindowBackgroundAppearance::Opaque
        } else {
            WindowBackgroundAppearance::Blurred
        });
        let colors = theme.clone();
        cx.set_global(theme);

        use gpui::component::Theme as ComponentTheme;
        ComponentTheme::change(appearance, Some(window), cx);
        ComponentTheme::update(cx, |t| {
            // The window is translucent; panes paint their own tints.
            t.colors.background = transparent_black();
            t.colors.border = colors.border;
            t.colors.muted_foreground = colors.text_muted;
            t.colors.window_border = transparent_black();
            // MA-017: inputs, tooltips and links in Pigeon's colors.
            t.colors.foreground = colors.text;
            t.colors.selection = colors.text_selection;
            t.colors.caret = colors.accent;
            t.colors.ring = colors.accent;
            t.colors.popover = colors.overlay_bg;
            t.colors.popover_foreground = colors.text;
            t.colors.link = colors.link;
            t.colors.link_hover = colors.link;
            t.colors.link_active = colors.link;
        });
    }
}

/// The gray a label or calendar gets when its own color is missing or
/// can't be read (ME-061).
pub const FALLBACK_COLOR: Hsla = hsla(0.6, 0.1, 0.5, 1.);

/// White, or near-black where white is under 4.5:1 and black reads better,
/// on an opaque `fill`.
pub fn on_fill(fill: Hsla) -> Hsla {
    let white = hsla(0., 0., 1., 1.);
    let ink = hex(0x1d1d1dff);
    let on_white = contrast(fill, white);
    if on_white >= 4.5 || on_white >= contrast(fill, ink) {
        white
    } else {
        ink
    }
}

/// `color` with its lightness moved just far enough to reach `ratio`
/// contrast on the opaque `background`, keeping its hue.
pub fn readable(color: Hsla, background: Hsla, ratio: f32) -> Hsla {
    let passes = |c: Hsla| contrast(c, background) >= ratio;
    let mut c = Hsla { a: 1., ..color };
    if passes(c) {
        return c;
    }
    let toward_light =
        contrast(background, hsla(0., 0., 1., 1.)) > contrast(background, hsla(0., 0., 0., 1.));
    let (mut failing, mut passing) = (c.l, if toward_light { 1. } else { 0. });
    for _ in 0..12 {
        c.l = (failing + passing) / 2.;
        if passes(c) {
            passing = c.l;
        } else {
            failing = c.l;
        }
    }
    c.l = passing;
    c
}

/// WCAG contrast ratio between two opaque colors.
pub fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn luminance(color: Hsla) -> f32 {
    let rgb = color.to_rgb();
    let channel = |v: f32| {
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn themes() -> Vec<Theme> {
        let mut all = Vec::new();
        for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
            for increase_contrast in [false, true] {
                let display = Display {
                    increase_contrast,
                    ..Display::default()
                };
                all.push(Theme::build(appearance, &display, 0));
            }
        }
        all
    }

    #[test]
    fn label_chip_text_reads_on_its_chip() {
        for theme in themes() {
            for pair in crate::backend::labels::COLORS {
                let color = pair.color();
                let chip = theme.solid(theme.list_bg).blend(theme.label_chip_bg(color));
                let ratio = contrast(theme.label_chip_text(color), chip);
                assert!(
                    ratio >= 4.49,
                    "{} on dark={}: {ratio}",
                    pair.name,
                    theme.is_dark
                );
            }
        }
    }

    #[test]
    fn text_on_fills_reads() {
        for theme in themes() {
            let now = theme.on_fill(theme.cal_now);
            assert!(contrast(now, theme.cal_now) >= 4.5, "now label");
            for pair in crate::backend::labels::COLORS {
                let fill = theme.event_selected_fill(pair.color());
                assert!(contrast(theme.on_fill(fill), fill) >= 4., "{}", pair.name);
            }
            assert!(contrast(theme.danger_text, theme.danger) >= 4.5);
        }
    }

    #[test]
    fn status_text_reads_on_panes() {
        for theme in themes() {
            let pane = theme.solid(theme.list_bg);
            for text in [
                theme.text_success,
                theme.text_danger,
                theme.text_warning,
                theme.link,
            ] {
                assert!(
                    contrast(text, pane) >= 4.5,
                    "{text:?} dark={}",
                    theme.is_dark
                );
            }
            let disabled = contrast(theme.text_disabled, pane);
            assert!(disabled >= 3., "disabled {disabled}");
        }
    }

    #[test]
    fn increase_contrast_is_opaque_and_stronger() {
        for theme in themes().into_iter().filter(|t| t.high_contrast) {
            assert!(theme.opaque);
            assert_eq!(theme.reader_bg.a, 1.);
            let pane = theme.solid(theme.sidebar_bg);
            assert!(contrast(theme.text_faint, pane) >= 7.);
        }
    }

    #[test]
    fn glass_runs_from_the_tints_to_opaque() {
        let display = Display::default();
        let tinted = Theme::build(WindowAppearance::Dark, &display, 0);
        assert!(!tinted.opaque);
        assert_eq!(tinted.sidebar_bg, Theme::dark().sidebar_bg);
        let half = Theme::build(WindowAppearance::Dark, &display, 50);
        assert!(half.sidebar_bg.a > tinted.sidebar_bg.a && half.sidebar_bg.a < 1.);
        let solid = Theme::build(WindowAppearance::Light, &display, 100);
        assert!(solid.opaque && solid.rail_bg.a == 1. && solid.menu_bar_bg.a == 1.);
        let reduce = Display {
            reduce_transparency: true,
            ..display
        };
        assert!(Theme::build(WindowAppearance::Light, &reduce, 0).opaque);
    }

    #[test]
    fn follows_the_system_accent() {
        let purple: Rgba = rgba(0x953d96ff);
        let yellow: Rgba = rgba(0xffc600ff);
        for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
            for accent in [purple, yellow] {
                let display = Display {
                    accent: Some(accent),
                    ..Display::default()
                };
                let theme = Theme::build(appearance, &display, 0);
                let hue = Hsla::from(accent).h;
                assert!((theme.accent.h - hue).abs() < 0.01);
                assert!(contrast(theme.accent_text, theme.accent) >= 4.5);
                assert!(contrast(theme.link, theme.solid(theme.reader_bg)) >= 4.5);
            }
        }
        // Multicolor keeps Pigeon's blue.
        let blue = Theme::build(WindowAppearance::Light, &Display::default(), 0);
        assert_eq!(blue.accent, Theme::light().accent);
    }
}

//! All of Pigeon's colors. Views read semantic colors from here and never
//! hardcode their own.

use gpui::{App, Global, Hsla, Window, WindowAppearance, hsla, rgba, transparent_black};

#[derive(Clone, Debug)]
pub struct Theme {
    pub is_dark: bool,

    // Pane tints, painted over the blurred window background. Low alpha so the
    // wallpaper shows through; the sidebar is the most see-through, the reader
    // the most solid. GPUI adds the alphas of stacked layers (and clamps), so
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
    /// The list's selection while the reader has keyboard focus.
    pub selected_inactive_bg: Hsla,
    pub nav_active_bg: Hsla,
    pub card_bg: Hsla,
    /// The message `n` / `p` moved to.
    pub card_focused_bg: Hsla,
    pub chip_bg: Hsla,
    pub search_bg: Hsla,

    pub icon: Hsla,
    pub icon_active: Hsla,
    pub accent: Hsla,
    /// Text on an `accent` background.
    pub accent_text: Hsla,
    pub danger: Hsla,
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
}

impl Global for Theme {}

fn hex(value: u32) -> Hsla {
    rgba(value).into()
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            is_dark: true,
            rail_bg: hex(0x0c0c1073),
            sidebar_bg: hex(0x11111566),
            list_bg: hex(0x1414188c),
            reader_bg: hex(0x18181ca6),
            border: hex(0xffffff14),

            text: hex(0xf4f4f7ff),
            text_muted: hex(0xb6b6bfff),
            text_dim: hex(0x9c9ca6ff),
            text_faint: hex(0x80808aff),

            hover_bg: hex(0xffffff0a),
            selected_bg: hex(0xffffff1c),
            selected_inactive_bg: hex(0xffffff0f),
            nav_active_bg: hex(0xffffff17),
            card_bg: hex(0xffffff0a),
            card_focused_bg: hex(0xffffff0d),
            chip_bg: hex(0xffffff12),
            search_bg: hex(0xffffff14),

            icon: hex(0xa8a8b2ff),
            icon_active: hex(0xf2f2f5ff),
            accent: hex(0x6e9bffff),
            accent_text: hex(0x0b1530ff),
            danger: hex(0xff7b72ff),
            star: hex(0xf5bf4fff),

            backdrop: hex(0x00000052),
            overlay_bg: hex(0x222226fd),
            overlay_border: hex(0xffffff1c),
            overlay_shadow: hex(0x00000099),
            match_text: hex(0x8fb3ffff),
            kbd_bg: hex(0xffffff10),
            kbd_border: hex(0xffffff17),
            kbd_text: hex(0xb4b4bdff),

            avatar_palette: Self::avatar_palette(0.42),
            avatar_text: hex(0xffffffff),
        }
    }

    pub fn light() -> Self {
        Self {
            is_dark: false,
            rail_bg: hex(0xf2f2f65c),
            sidebar_bg: hex(0xf7f7fa4d),
            list_bg: hex(0xfafafc80),
            reader_bg: hex(0xffffff9e),
            border: hex(0x0000001a),

            text: hex(0x141417ff),
            text_muted: hex(0x55555cff),
            text_dim: hex(0x6c6c75ff),
            text_faint: hex(0x8a8a93ff),

            hover_bg: hex(0x0000000a),
            selected_bg: hex(0x00000012),
            selected_inactive_bg: hex(0x0000000a),
            nav_active_bg: hex(0x00000012),
            card_bg: hex(0xffffff00),
            card_focused_bg: hex(0x00000008),
            chip_bg: hex(0x0000000d),
            search_bg: hex(0x0000000a),

            icon: hex(0x5f5f67ff),
            icon_active: hex(0x1c1c1fff),
            accent: hex(0x2f6fedff),
            accent_text: hex(0xffffffff),
            danger: hex(0xd93a32ff),
            star: hex(0xe2a21cff),

            backdrop: hex(0x0000001f),
            overlay_bg: hex(0xfdfdfefd),
            overlay_border: hex(0x00000017),
            overlay_shadow: hex(0x00000038),
            match_text: hex(0x2f6fedff),
            kbd_bg: hex(0x0000000a),
            kbd_border: hex(0x00000014),
            kbd_text: hex(0x5f5f67ff),

            avatar_palette: Self::avatar_palette(0.5),
            avatar_text: hex(0xffffffff),
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
        let appearance = window.appearance();
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

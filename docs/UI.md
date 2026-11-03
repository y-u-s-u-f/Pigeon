# UI

The design system as the code defines it. Read this instead of re-reading `style.rs`, `theme.rs`, `motion.rs` and `controls.rs`; when they disagree with this page, the code wins and this page gets fixed.

`src/ui/style.rs` holds type, control and row sizes, icons, avatars, spacing, radii, the hit target and the focus ring; `src/theme.rs` holds every color; `src/ui/motion.rs` holds springs and motion helpers; `src/ui/controls.rs` and `src/ui/mod.rs` hold the shared controls.

The split in `style.rs` is deliberate: type sizes and the sizes of anything holding text are in rems (`type_size(px)` = `px / 16`), so ⌘= and ⌘- scale them; spacing and radii stay in pixels, so the layout keeps its rhythm.

## Tokens

Sizes are points at text scale 1. Type sizes and control, icon and avatar sizes are in rems, so they scale with the text; layout is in pixels.

### Type

| Token | Value | Use for |
|---|---|---|
| `TEXT_XS` | 11 | Captions, keycaps, section headers, chips, the time gutter |
| `TEXT_SM` | 12 | Secondary text, snippets, dates, buttons, search fields |
| `TEXT_BASE` | 13 | The UI's body: rows, menus, fields |
| `TEXT_MD` | 14 | Message bodies |
| `TEXT_LG` | 15 | Pane and sheet titles |
| `TEXT_TITLE2` | 17 | Reader subject, event and contact titles (macOS Title 2) |
| `TEXT_XL` | 20 | Week day numbers, which have to fit their circle |
| `TEXT_TITLE1` | 22 | Calendar period, onboarding title (macOS Title 1) |
| `LINE_UI` | 1.25 | Line height for UI text; GPUI's 1.618 default is far looser than macOS |
| `LINE_READING` | 1.55 | Message bodies and other running text |
| `MONO` | `"Menlo"` | Search operators, message source, `<pre>` |

### Sizes

| Token | Value | Use for |
|---|---|---|
| `CONTROL_XS` | 20 | Keycaps, label chips, compact buttons, the face of `icon_button_small` |
| `CONTROL_SM` | 24 | Small buttons, segments, pop-ups, toggle chips, the stepper's face |
| `CONTROL_MD` | 28 | Buttons and fields; `push_button` is this tall and never under `HIT_TARGET` |
| `CONTROL_LG` | 32 | Search fields, primary buttons, composer and settings fields |
| `ROW_SM` | 30 | Sidebar, shortcut and menu rows. Thread list rows size to their content instead |
| `ROW_PALETTE` | 38 | Command palette rows, later picker rows |
| `ICON_SM` | 12 | Inside chips, compact buttons, metadata, a checkbox's check |
| `ICON_MD` | 14 | Next to body text, menu rows, fields |
| `ICON_LG` | 16 | List and pane rows, file icons, a checkbox's box |
| `ICON_XL` | 18 | Search fields and other large inline glyphs |
| `ICON_HERO` | 28 | Empty states and onboarding |
| `ICON_TOOLBAR` | 16 px | Toolbar buttons' glyph; chrome doesn't grow with text |
| `ICON_RAIL` | 18 px | The rail's glyph; chrome doesn't grow with text |
| `AVATAR_XS` | 20 | Chips and compact rows (first initial only), the account button |
| `AVATAR_SM` | 24 | Account switcher, collapsed messages, guests, `person_row` |
| `AVATAR_MD` | 32 | List rows and message headers |
| `AVATAR_LG` | 48 | The contact card |

### Layout (px)

| Token | Value | Use for |
|---|---|---|
| `SPACE_2XS` | 2 | Hairline gaps, the switch knob's inset, dot rows |
| `SPACE_XS` | 4 | Tight gaps inside a control, between keycaps, between icon buttons |
| `SPACE_SM` | 8 | The default gap between a glyph and its text, row padding |
| `SPACE_MD` | 12 | Control padding, pane and bar padding |
| `SPACE_LG` | 16 | Section padding, the thread list's row inset |
| `SPACE_XL` | 24 | Large gaps, empty-state blocks |
| `SPACE_2XL` | 32 | Empty states and onboarding |
| `RADIUS_XS` | 4 | Checkboxes and other small squares |
| `RADIUS_SM` | 6 | Chips, keycaps, small bars, menu rows, segments |
| `RADIUS_MD` | 8 | Buttons, rows, fields, icon buttons |
| `RADIUS_LG` | 12 | Cards and overlays |
| `RADIUS_PANEL` | `RADIUS_LG` | The panes' inner corners at the rail (`ui::panel_corners`) |
| `HIT_TARGET` | 28 | The smallest mouse target for an icon button (Apple's 28pt) |
| `DOT` | 8 | A label's or calendar's color dot (`ui::DOT`) |
| `HEADER_HEIGHT` / `HEADER_MIN` | 40 px / 36 rem | Pane headers: `.h(HEADER_MIN).min_h(HEADER_HEIGHT)`; the min grows with the text |
| `TOP_BAR_HEIGHT` / `TOP_BAR_MIN` | 44 px / 40 rem | The top bar; read it with `top_bar_height(window)`, pane headers with `header_height(window)` |

`BASE_REM` (16) is the rem size at text scale 1, which the app sets on the window. `type_size(px)` turns a point size into rems; use it for any size that holds text, widths included. `TEXT_SCALE_STEP` (0.1) and `TEXT_SCALE_RANGE` (0.9–2.0) are the ⌘= and ⌘- steps and bounds. `focus_ring(theme)` is a 2 px accent inset shadow, so focus never shifts layout or gets clipped; `focusable_control(el, theme)` makes a control reachable with Tab and shows the ring on keyboard focus only. `tabular_figures()` gives counts, dates and timers equal-width digits.

## Color

`Theme` in `src/theme.rs` is one struct of semantic fields. `dark()` and `light()` fill it; `build(appearance, &Display, glass)` applies the system accent, Increase Contrast, Reduce Transparency and the user's glass amount. `apply`, `sync` and `sync_for` install it and mirror the parts gpui-component draws (scrollbars, inputs, tooltips) into its own theme.

Views read `Theme::of(cx)` and never build a color. There is no `Theme::for_appearance`: for a chosen appearance use `sync_for(appearance, window, cx)`, and for a palette without installing it, `build`.

| Group | Fields | Meaning |
|---|---|---|
| Surface | `bar_bg`, `sidebar_bg`, `list_bg`, `reader_bg`, `cal_bg`, `cal_panel_bg`, `menu_bar_bg` | Pane tints over the glass; `material` is the gray they're judged against |
| Text | `text`, `text_muted`, `text_faint`, `text_disabled`, `link`, `match_text` | Three weights plus disabled (about 3:1); link at 4.5:1 |
| Border | `border`, `overlay_border`, `kbd_border`, `cal_line`, `cal_line_faint`, `cal_ring`, `context_ring` | Hairlines and outlines |
| Interactive | `hover_bg`, `row_hover_bg`, `selected_bg`, `selected_focused_bg`, `checked_bg`, `nav_active_bg`, `card_bg`, `card_focused_bg`, `chip_bg`, `search_bg` | States; the focused selection is the accent-tinted one |
| Control | `control_track`, `control_knob`, `switch_on`, `segment_bg` | Switch, slider, segmented well, checkbox |
| Accent | `accent`, `accent_text`, `text_selection`, `icon`, `icon_active` | `accent_text` is text on `accent` |
| Status | `success`, `warning`, `danger`, `star`, `text_success`, `text_warning`, `text_danger` | The plain ones for fills, dots and badges; the `text_` ones read at 4.5:1 |
| Overlay | `backdrop`, `overlay_bg`, `overlay_shadow`, `kbd_bg`, `kbd_border`, `kbd_text` | Palette, sheets, menus, keycaps |
| Avatar | `avatar_palette` | Eight backgrounds; initials use `on_fill` |
| Calendar | `cal_*` | Event colors derive from a calendar's own color |

Helpers: `theme.solid(pane)` is the pane over the bare material, opaque, and is what contrast is judged against. `theme.on_fill(fill)` is white or near-black text on a solid fill. `theme.avatar_color(seed)` is the stable avatar background for an email. `label_chip_bg` and `label_chip_text` make a label's chip. `event_fill`, `event_fill_hover`, `event_text`, `event_secondary`, `event_accent`, `event_selected_fill`, `event_answer_fill` and `event_free` derive events from a calendar's color. `readable(color, background, ratio)` moves a color until it reads, `contrast(a, b)` measures it, and `FALLBACK_COLOR` is the gray for a missing one. Branch on `is_dark`, `high_contrast`, `opaque` and `material` rather than guessing.

## Motion

Two drivers: `gpui::base::spring(id, target, TOKEN, window, cx)` for anything that opens and closes, sampled every render with the id held steady and the token switched; and `enter(id, rise, el)` for things that only play as they appear. Under Reduce Motion GPUI snaps springs, so `fade`, `presence` and `enter` keep opacity moving. `Glide` drives scroll offsets by hand.

| Spring | Response | Damping | Use for |
|---|---|---|---|
| `SPRING_PANE` | 240 ms | 1.0 | Panes and widths, height reveals, chevron rotation; no overshoot |
| `SPRING_POP` | 200 ms | 0.825 | Anything arriving: overlays, toasts, cards, the segmented thumb |
| `SPRING_EXIT` | 160 ms | 1.0 | Anything leaving; shorter than the arrival, never overshoots |
| `SPRING_CONTROL` | 200 ms | 0.7 | Switch knobs and glyph pops, in pixels |
| `SPRING_SCROLL` | 200 ms | 1.0 | Keyboard and reveal scrolling, in pixels |
| `SPRING_SNAP` | 90 ms | 1.0 | The drag ghost gliding between 15-minute slots |

| Helper | Reach for it when |
|---|---|
| `enter(id, rise, el)` | A card, toast or overlay plays in once as it mounts. Pair with `moving(rise, cx)` so Reduce Motion drops the travel |
| `card_motion(el, p, rise)` | You own the spring and need to draw a card at progress `p` |
| `presence(id, open, window, cx)` | A surface opens and closes. Sample it every render, closed included; keep drawing while it's above 0 |
| `fade(id, from, target, window, cx)` | Opacity under Reduce Motion, where a spring would snap |
| `after_loading_delay(id, el)` | A loading placeholder: nothing for 300 ms, then a fade, so a fast load never flashes it |
| `after_delay(id, delay, el)` | The same with your own wait |
| `spinner(id, glyph)`, `spinner_while(id, busy, glyph, ..)` | A glyph turning while work is under way, still under Reduce Motion; the second finishes its turn when the work ends |
| `pop(id, glyph)` | A glyph that pops from 1.2× to its size, only when the change just happened |
| `turn(id, glyph, open, turns)` | A disclosure chevron turning as it opens |
| `inert_cover()` | Content is leaving and must take no clicks on its way out |
| `Rise::Up`, `Rise::Down`, `Rise::None` | Which way an arriving card travels: up from the bottom, down from a control or the top edge, none inline |
| `Glide` | A value glides to a target and a key press retargets it without queueing: `Glide::new`, `page`, `exit`, `pane` |

Other constants: `MOTION_RISE` (6 px), `LOADING_DELAY` (300 ms), `MENU_TRAVEL`, `MENU_SUBMENU_DELAY`, `MENU_AIM_GRACE`, `MENU_HOLD`, and the `SWIPE_*` set with `PAGE_JUMP_TRAVEL`.

## Controls

Use these; restyling a div to look like one is a bug, because it loses the hover, press, focus, role and label the control already has.

| Control | Where | Use for |
|---|---|---|
| `push_button`, `push_button_frame`, `panel_button` | `controls.rs` | Text buttons with `ButtonKind::{Plain, Primary, Danger, Destructive, Quiet}`; the frame takes your own layout, the panel button is the event panel's footer |
| `switch` | `controls.rs` | On/off; the knob springs and the track's color follows |
| `segmented` | `controls.rs` | Equal-width options with a thumb under the chosen one |
| `stepper` | `controls.rs` | − value +, with a disabled button at either bound |
| `slider` | `controls.rs` | A horizontal 0…1 slider; `value_text` is what VoiceOver reads |
| `toggle_chip` | `controls.rs` | A small toggle in a row of options, such as repeat days |
| `check_box` | `controls.rs` | The box; the row around it is the control |
| `menu_row`, `menu_check` | `controls.rs` | Pop-up rows and the leading check column that keeps checks aligned |
| `popover_surface`, `sheet_surface`, `dropdown`, `popup` | `controls.rs` | Chrome for anything anchored to a control, for a sheet over the window, a dropdown list with a menu role and width, or content floating under the element before it |
| `person_row` | `controls.rs` | A contact in a suggestion list |
| `error_line` | `controls.rs` | An inline error under a field, announced and selectable |
| `bar_colors` | `controls.rs` | Fill, text and hover of an all-day bar |
| `icon_button`, `icon_button_tinted`, `icon_button_with` | `ui/mod.rs` | A square toolbar button that dispatches an action, with its tooltip and shortcut |
| `icon_button_small` | `ui/mod.rs` | The compact button inside rows, chips and fields |
| `tooltip` | `ui/mod.rs` | A tooltip at the size macOS uses |
| `keycap`, `keycap_on_accent` | `ui/mod.rs` | One key; the accent variant sits on a filled button |
| `binding_caps`, `bindings_caps`, `keys_caps` | `ui/mod.rs` | A live binding, several with a faint "or", fixed keymap text |
| `search_field` | `ui/mod.rs` | The rounded well around a search input; callers set height and padding |
| `section_caption` | `ui/mod.rs` | A list section's caption; callers set padding |
| `dot_marker` | `ui/mod.rs` | A label's or calendar's color dot, centered in an icon-sized box |
| `live_text`, `label` | `ui/mod.rs` | Status text VoiceOver announces, and text it reads at all: plain strings never reach the tree |
| `avatar`, `account_avatar` | `ui/mod.rs` | An initials circle; the user's photo when there is one |
| `label_chip`, `empty_state` | `ui/mod.rs` | A label as a chip; a centered placeholder for an empty pane |
| `window_drag_area` | `ui/mod.rs` | Empty header space that moves the window and zooms on a double click |

## Rules that bite

- No raw px for text size, spacing or radii. `TEXT_*`, `SPACE_*`, `RADIUS_*` only; if none fits, add a token to the right file rather than a literal.
- Anything holding text is in rems, through `type_size`: at 200% text a pixel height clips the words. That includes control heights, inline icons and avatars, and widths that must fit a phrase.
- Colors come only from `theme.rs`. No `hsla`, `rgb` or hex in a view, and no color sampled from a screenshot: the theme is what follows light and dark, Increase Contrast and the system accent.
- Every interactive element gets `.role(..)` and `.aria_label(..)`, and goes through `focusable_control` to be reachable with Tab and to show the ring. Text that isn't inside a control still needs `label` or `live_text` to be read.
- Motion comes only from `motion.rs`: no linear animation, no ad-hoc `Duration`. Arrivals are springy and quick, exits are shorter with no overshoot, everything is interruptible and retargetable, and Reduce Motion is respected.
- A closed surface stays drawn while `presence` is above 0. Put `inert_cover` over it so it takes no clicks while it leaves.
- Finish the states: hover, pressed, focused, disabled, empty, loading and error. Disabled text uses `text_disabled`, not `text_faint`.
- Prefer the shared control over a hand-rolled one, and the existing helper over a new abstraction.

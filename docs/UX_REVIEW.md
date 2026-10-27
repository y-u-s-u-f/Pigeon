# Pigeon UX review

> **Status (26 Oct 2026):** addressed in two waves. Not done by choice: UX-037 calendar sidebar (full-width calendar was chosen; replaced by a go-to-date popover), ⌘⇧P kept alongside ⌘K. Waiting on the backend (tracked in `docs/ROADMAP.md`): remote images, opening/saving attachments. Not built: compose text formatting, dragging threads onto folders.

Reviewed 26 Oct 2026 against the code at HEAD and a release build, driven with Cua Driver (pid- and window-scoped, background input wherever it worked; foreground only for calendar clicks and drags, which GPUI ignores in the background). Screenshots are in `/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/ux-*.png` (shortened to `ux-NN` below).

Dark mode was checked by forcing `Theme::dark()` in a scratch copy while the system stayed in Light. The window blur material then stayed light, so the dark screenshots (`ux-23`, `ux-24`) show the worst case of dark tints over a light material. Real dark mode will look better, but the tint math is the same.

## Verdict

The bones are good. The three-pane layout is calm, the type is readable, the keyboard model is coherent (one action table drives shortcuts, palette, tooltips and the `?` sheet), and the invite card and event panel are better thought out than most v1s. It looks closer to Apple Mail than to Superhuman or Linear.

What holds it back:
- **Contrast on the glass.** Most of the secondary text sits around 2.2–3.8:1.
- **Focus is barely visible.** You can't tell whether the list or the reader has the keyboard.
- **Selection gets lost.** A jump can leave the selected row off-screen.
- **The minimum window size overflows.** It clips the reader.
- **Missing everyday mail features.** No multi-select, no labels/move, no context menus, no hover actions.
- **Almost no motion.** One animation in the whole app.
- **No accessibility tree.** VoiceOver sees an empty window.

None of this is hard to fix. Fixing it is the difference between "nice demo" and "I'd switch to this."

## Findings

Severity: Critical / High / Medium / Low / Polish. Counts (unique findings UX-001–048 and UX-054; the bug table repeats some): Critical 0 · High 7 · Medium 19 · Low 21 · Polish 2.

### Global / visual system

**UX-001 · High · Secondary text fails contrast on the glass (light mode)**
I measured composited pane colors from screenshots (sidebar ≈ #D0D0D0, list ≈ #DCDCDD, reader ≈ #E7E7E7):

| token | sidebar | list | reader |
|---|---|---|---|
| `text_muted` #55555C | 4.8 | 5.4 | 6.0 |
| `text_dim` #6C6C75 (read rows) | **3.4** | **3.8** | 4.2 |
| `text_faint` #8A8A93 | **2.2** | **2.5** | **2.8** |
| `accent` #2F6FED (links, "Join") | **3.0** | **3.3** | **3.7** |

`text_faint` carries a lot of real content: read-row snippets and dates, the "Labels" header, sidebar counts, "N unread", empty-state hints, message dates, palette subtitles and the search result count. These numbers also depend on the wallpaper, because the panes are 30–62% opaque.
- Why it matters: read mail is most of the list and becomes hard to scan. On a bright wallpaper it gets worse.
- Fix: `text_faint` → #6E6E76 (~4.5:1 on the list), `text_dim` → #5A5A62, accent text → #1F5FD6. Raise `sidebar_bg` alpha from 0x4D to ~0x8C and `list_bg` from 0x80 to ~0xB3 so contrast stops depending on the wallpaper. Keep "read = dimmer" by dropping the weight (as you already do) rather than going lighter.
- Where: `src/theme.rs` `Theme::light()`.

**UX-002 · High · Dark theme tints are too thin to survive bright backgrounds** (`ux-23`, `ux-24`)
`sidebar_bg` is 40% opaque, `list_bg` 55% and `rail_bg` 45%. Over a light material, `text_faint` drops to 1.1:1 in the sidebar (the "Labels" header and counts disappear) and to 1.4:1 for read snippets in the list. `text_dim` read senders reach about 2.1:1. Real dark mode has a darker material, but a bright wallpaper still pushes it toward this.
- Fix: dark `sidebar_bg` alpha ≥ 0xB3, `list_bg` ≥ 0xC7, `reader_bg` ≥ 0xD9. Then retune `text_faint` to about #8E8E98. Test against a white wallpaper.
- Where: `Theme::dark()`.

**UX-003 · Medium · The type scale has too many near-identical sizes**
Sizes in use: 10.5, 11, 11.5, 12, 12.5, 13, 13.5, 14, 15, 19, 20 and 21. Steps of 0.5px read as inconsistency, not hierarchy. The list row alone uses 13.5, 13, 12.5, 12 and 11.5.
- Fix: settle on 11 / 12 / 13 / 15 / 20 (caption, secondary, body-UI, title, subject), plus 14 for the message body. Put them in `theme.rs` (or a `type.rs`) as constants.
- Where: every `text_size(px(…))` call; start with `thread_list.rs`, `reader.rs` and `event_panel.rs`.

**UX-004 · Medium · Radii and spacing are ad hoc**
Radii in use: 5, 7, 8, 9, 10, 12 and 14. Paddings include 6, 8, 10, 12, 14, 16, 18, 22 and 28. There is no 4- or 8-point rhythm (rows use 8 outside and 22/12 inside, the reader uses 28, the sidebar 10).
- Fix: a spacing scale of 4/8/12/16/24/32 and radii of 6 (chips, keycaps), 8 (buttons, rows), 12 (cards, overlays). Align the list header's left padding (18) with the row text (8 + 22 = 30) so the title sits over the senders.
- Where: `ui/mod.rs`, `thread_list.rs`, `sidebar.rs`, `reader.rs`.

**UX-005 · Polish · Almost no motion**
The search field expanding is the only animation. Overlays, the toast, the event panel, sidebar collapse, collapsed messages expanding and archive auto-advance are all instant cuts. Superhuman and Linear use 120–180 ms fades and slides that make "fast" feel deliberate rather than jumpy.
- Fix: 120 ms opacity + 4px translate for the palette, sheet and toast; 160 ms width for the sidebar and event panel; 100 ms for a row leaving on archive. Skip all of them when "Reduce motion" is on (`NSWorkspace.accessibilityDisplayShouldReduceMotion`).

### Window chrome

**UX-006 · High · The minimum window size overflows and clips the reader** (`ux-16`)
`window_min_size` is 900×560. Rail 52 + sidebar 236 + list 400 + reader minimum 360 = 1048. At 900pt the reader is cut off: the reply, reply-all and forward buttons and message dates are off-screen, and the subject is clipped with no ellipsis.
- Fix: shrink panes in priority order. List first, down to 300; then auto-collapse the sidebar below ~1000pt; reader minimum 360. Or raise the minimum width to 1050. Compute the list width as `min(list_width, window − rail − sidebar − 360)`.
- Where: `app.rs` `render` / `resize_pane`; `main.rs` `window_min_size`.

**UX-007 · Medium · The app menu is a single item**
The menu bar only has "Quit Pigeon". There is no Edit menu (so no menu Undo or Cut/Copy/Paste, and Services don't work), no Window menu (⌘M, ⌘W, zoom), no ⌘H Hide, no ⌘, Settings and no Help. This is the first thing a Mac user notices, and menus are also how macOS exposes shortcuts to VoiceOver and to menu search.
- Fix: add the standard Pigeon / File / Edit / View / Message / Go / Window / Help menus and route each item to an existing action, so they double as discoverability. Add ⌘N for compose and ⌘F for search as standard aliases.
- Where: `main.rs` `cx.set_menus`; `actions.rs` `bind_keys`.

**UX-008 · Low · The traffic lights crowd the rail**
The traffic lights at x=16 end around x=68, which is past the 52pt rail, so they spill over the rail border into the sidebar header. With the sidebar collapsed, the "show sidebar" button sits right against them (`ux-13`).
- Fix: either widen the rail to 72 (Notion Calendar does this) and start rail icons below the lights, or move the lights to y=20 and keep the rail border out of the top 52pt.
- Where: `main.rs` `traffic_light_position`, `ui/rail.rs`.

### Sidebar & rail

**UX-009 · Medium · The rail has no unread badge and no bottom section**
You can't tell from Calendar that mail arrived. Also, Settings, account, sync status and help have no home.
- Fix: a dot or count on the Mail icon. Put a settings/account button at the bottom of the rail.
- Where: `ui/rail.rs`.

**UX-010 · Low · The account card is decorative**
It looks like a button (border, card fill, avatar) but does nothing. It also takes 60pt at the top of the sidebar.
- Fix: make it the account switcher with a menu, or reduce it to a 28pt row. Remove the border; use `card_bg` only on hover.
- Where: `sidebar.rs`.

**UX-011 · Low · Sidebar counts mean different things**
Drafts shows a total, Inbox shows unread, and Starred/Sent show nothing. That is defensible, but it isn't explained, and the counts are `text_faint`, which fails contrast (UX-001).
- Fix: use `text_muted`, `tabular_nums`, and a semibold Inbox count when it's above 0.

### Thread list

**UX-012 · High · The selected row isn't scrolled into view after palette jumps or auto-advance** (`ux-04`, `ux-15`)
I opened "Harbor Bank: your October statement" (deep in the Inbox) from ⌘K. The reader showed it, but the list stayed scrolled near the top with no visible selection. Pressing `j` then moved an invisible selection (`ux-05`). `select_thread` calls `scroll_to_reveal_item`, but it doesn't take effect when focus comes back from the palette. That is likely a `ListState` measurement or ordering issue: `close_overlays` runs before `reveal_thread`.
- Why it matters: in a keyboard-first client, losing your place is the worst failure.
- Fix: defer the reveal to after the next layout (`cx.on_next_frame` or `window.defer`), or use `scroll_to(ListOffset{item_ix, offset_in_item: 0})` for rows outside the viewport. Add a test that ⌘K → thread → selection is within the viewport.
- Where: `app.rs` `select_thread`, `on_palette_event`, `reveal_thread`.

**UX-013 · High · You can't see whether the list or the reader has focus**
`selected_bg` (7% black) changes to `selected_inactive_bg` (4% black) when the reader has focus. On the glass the two look the same. Apple Mail uses an accent-filled row when the list has focus and grey when it doesn't. Superhuman uses a strong left bar.
- Fix: when the list has focus, use an accent tint (`accent` at 14% alpha) plus a 2px accent bar on the leading edge; otherwise grey. In the reader, show the same bar on the focused message (already done for `n`/`p`, keep it).
- Where: `thread_list.rs` `render_thread_row`; `theme.rs` (add `selected_focused_bg`).

**UX-014 · Medium · Labels crowd out the subject**
At the default 400pt list, rows with two labels truncate the subject to about 25 characters ("Invitation: Auth refresh rollout check-in @ Wed…", "Beta crash reports — 2.4.0 (mac…"). The subject is the most important text in the row.
- Fix: show at most one chip plus "+N", or move chips to the snippet line, aligned right. Consider colored 6px dots instead of chips (as Linear does) in the list, with full chips only in the reader.
- Where: `thread_list.rs`.

**UX-015 · Medium · No hover actions, context menu, multi-select or drag**
There are no row hover buttons (archive, delete, star, snooze), no right-click menu anywhere in the app, no `x` / shift-click / ⌘-click multi-select, and no drag to a folder or label. Gmail's `x` + `e` workflow is impossible.
- Fix: hover actions on the right of row line 1, replacing the date on hover (as Superhuman and Apple Mail do). Add multi-select with `x`, shift-j/k and shift-click, and bulk actions in the reader pane. Add a context menu that mirrors the toolbar.
- Where: `thread_list.rs`, `app.rs`.

**UX-016 · Low · Rows are tall for a "fast" client**
Rows are about 80pt (three lines, py 10). That fits roughly 9 rows on a 860pt window. Superhuman fits about 18.
- Fix: offer a compact density (single line: sender · subject — snippet, 36pt rows) in View.

**UX-017 · Low · Snippets repeat the sender**
HTML mail snippets start with the H1, which is usually the brand name: "Skyline Air Hi Alex…", "Rustacean Weekly Rustacean Weekly #412…", "Stripe payout Your payout…".
- Fix: drop leading headings, or the sender's display name, when building `snippet`.
- Where: `data.rs` / `html.rs` snippet generation.

**UX-018 · Polish · The archived section is easy to miss**
The "Archived N" header at the bottom of the Inbox is 12px `text_muted` with a 14px chevron and no divider. It reads as a footnote. Fine if that's the goal; add a hairline above it and a `⇧E`-style hint in its tooltip.

### Reading pane & HTML

**UX-019 · Medium · HTML mail loses its design**
The renderer turns every email into the same reader-mode document (`ux-06`). Images become grey alt-text boxes, CTA buttons become plain text, and brand colors and backgrounds are dropped. "skylineair.com/checkin" shows as grey text with no link affordance. For receipts and tickets this is clean. For marketing mail and newsletters it looks broken, and it won't match "native HTML rendering" expectations.
- Fix: render `<img>` (with a "load remote images" privacy toggle per sender), render `<a>` styled as buttons as real buttons, and keep background colors on table cells when contrast allows (you already have `readable()`). Make bare URLs into links.
- Where: `html.rs`, `ui/html_view.rs`.

**UX-020 · Medium · Attachments are inert**
The chips have no hover, click, Quick Look, save or drag-out. Each only has a filename and size, with a generic file icon.
- Fix: hover state, click to Quick Look, ⌘-click to save, drag to Finder. Use per-type icons (pdf, image, zip).
- Where: `reader.rs` `render_message`.

**UX-021 · Low · Your own messages show your address**
"me alex.rivera@example.com" repeats on every message you sent. It's noise.
- Fix: hide the email when the sender is me; show it on hover for others.

**UX-022 · Low · The toolbar has no overflow**
There's no "more" (…) menu for move-to, label, snooze, print or view source, and no label editing anywhere in the app.
- Fix: add a `…` button and `v` (move) / `l` (label) shortcuts that open the palette scoped to folders or labels, as `g l` already does.

### Compose

**UX-023 · Medium · Recipient errors appear far from the field** (`ux-25`)
"'zzq' isn't an email address" appears at the far right of the Send row, about 400pt below and to the right of the field it refers to. The invalid chip is tinted, but the message is easy to miss.
- Fix: put the error directly under the To row in `danger`, and focus the bad chip.
- Where: `composer.rs` `render`.

**UX-024 · Medium · Compose is missing standard affordances**
No From/alias picker, no attachments button or drop target, no formatting (even ⌘B / ⌘I / ⌘K-link), and no signature. The body has a fixed minimum height with a large blank gap above the Send row.
- Fix: at minimum, dropping a file attaches it, ⌘B/⌘I/⌘U work, and the Send row sticks to the bottom of the composer viewport instead of floating mid-pane.

**UX-025 · Low · Close can silently drop recipient text**
Typing "pri" in To, picking the suggestion with Return and pressing Esc closed the composer with no "Saved to Drafts" toast, and nothing appeared in Drafts. Recipient text that isn't committed is ignored by `is_pristine`, so it's thrown away without warning.
- Fix: commit (or count) pending recipient text before deciding the draft is pristine.
- Where: `composer.rs` `is_pristine`, `content`.

**UX-026 · Low · A thread can have duplicate drafts**
Drafts held two "Re: Q4 planning — hiring plan numbers" drafts (`ux-26`). Gmail keeps one draft per thread; replying again should reopen the existing one.
- Where: `app.rs` `compose` (look up a draft with `origin.reply_to == thread.id`).

### Search

**UX-027 · Medium · Operators have no help**
The field accepts `from:`, `to:`, `subject:`, `label:`, `is:`, `has:` and `in:`, but nothing tells you they exist. There's no suggestion dropdown, no token styling and no recent searches. The placeholder is just "Search mail".
- Fix: when the field is empty, show a dropdown with the operators and recent searches. As you type `from:`, suggest contacts (the `ContactBook` already exists). Style recognized operators as chips or tinted text. Placeholder: "Search — try from:, is:unread, in:all".
- Where: `thread_list.rs` `render_search_bar`, `search.rs`.

**UX-028 · Low · The search bar hides where you are, and the empty state repeats itself** (`ux-10`)
The search field replaces the folder title, so "Inbox" only survives in "0 results in Inbox". That count line and the "No results in Inbox" empty state then say the same thing twice.
- Fix: keep a small scope chip ("Inbox ▾") in the field that can be switched to All Mail. Hide the count line when there are 0 results.

### Command palette & shortcuts sheet

**UX-029 · Medium · The palette offers commands that can't run** (`ux-03`)
With no draft open, the first screen of ⌘K lists Send, Show Cc and Bcc, Close Draft, Discard Draft and Undo Send. Seven of the nine visible rows can't do anything right now.
- Fix: filter or dim commands that aren't available in the current state (add `is_enabled(&PigeonApp)` to `Command`). Order the empty query by recency or frequency (as Linear and Raycast do).
- Where: `actions.rs` `Command`, `app.rs` `palette_items`.

**UX-030 · Low · The palette results jump in height**
The list height follows the result count, so the panel grows and shrinks on every keystroke.
- Fix: a fixed result area of 8 rows; show the empty state inside it.

**UX-031 · Low · The shortcuts sheet cuts off its last group** (`ux-12`)
In the Mail sheet the "Application" group (⌘1, ⌘2, ⌘K) sits below the fold with no scroll affordance, so the most important global shortcuts look missing. There's also no filter field. "Maybe to Invitation" reads oddly.
- Fix: put Application first, or use three columns at ≥1200pt. Show a fade at the bottom when it scrolls. Type to filter. Rename to "Answer Maybe".
- Where: `shortcuts_help.rs`, `actions.rs` `COMMANDS`.

### Keyboard & focus

**UX-032 · Medium · Calendar arrow keys don't move by day**
In Calendar, ←/→ and ↑/↓ all mean "previous/next event", and Tab does the same. Every calendar app (Apple Calendar, Fantastical, Notion Calendar) uses ←/→ to move by day or period. `j`/`k` mean next/previous period in Calendar but next/previous thread in Mail.
- Fix: ←/→ = previous/next period (or day in week view), ↑/↓ = previous/next event, Tab = next event. Keep `j`/`k` as aliases for ↓/↑ so they match Mail.
- Where: `actions.rs` `bind_keys`.

**UX-033 · Low · Some shortcuts are missing or conflict**
- No `g c` to go to Calendar from Mail, and no `g m` back.
- `#` needs Shift on US layouts and doesn't exist on many others; add `Delete`/`⌫` in the list.
- No `z` for undo (Superhuman), only ⌘Z.
- ⌘⇧P duplicates ⌘K.
- `cmd-enter` is bound globally under `Pigeon`, so in Calendar it silently does nothing.

**UX-034 · Low · Focus rings exist only in the event panel**
`focus_visible` is used only for panel controls. Toolbar icon buttons, rail items, sidebar rows, palette rows and toast Undo can't be reached by Tab at all, so there is no Full Keyboard Access path to them.
- Fix: make toolbar and sidebar items focusable with a 2px accent ring on `focus_visible`.

### Calendar views

**UX-035 · High · Overlapping events cascade into unreadable slivers** (`ux-17`, `ux-18`)
Overlaps are drawn as indented cascades. In week view at normal widths this produces chips like "1…", "K…", "Spr…" and "Bet…". At 1360pt, Thursday 10–12 shows three nested blocks where only the outer title is readable.
- Fix: lay overlapping events out side by side in equal columns, as Apple Calendar, Fantastical and Notion Calendar do. Allow a small overlap (~10%) only when the columns would be narrower than ~60pt. Hide the time on short blocks before hiding the title.
- Where: `calendar/layout.rs`, `ui/calendar/week_view.rs`.

**UX-036 · Medium · Day view wastes the width** (`ux-21`)
On a wide window a single day stretches 1300pt. Blocks become long thin bars, two concurrent events split 50/50 across 650pt each, and the header is a tiny "FRI 30" on the left above an empty band.
- Fix: cap the event column at about 640pt and use the rest for an agenda or details of the selected event, or show a mini month plus the day's list (Fantastical). Header: "Friday, October 30".
- Where: `week_view.rs`, `header.rs`.

**UX-037 · Medium · Calendar has no sidebar**
Mail has folders and labels. Calendar has no mini month, no calendar list with toggles (hidden behind the "Calendars" dropdown), and no upcoming agenda. That makes the two halves of the app feel like different products, and jumping to a date takes several presses of `n`.
- Fix: a collapsible sidebar (same width, same ⌘\) with a mini month, calendars with color toggles, and "Up next".

**UX-038 · Low · Month view polish** (`ux-20`)
- The overflow link reads "5 more"; use "+5 more".
- Past days fade every event to `text_faint`, so the first three weeks of the month are hard to read.
- No week numbers option.
- Six weeks always render, even for five-week months; Apple Calendar does the same, so this one is fine.
- Fix: fade past events less (`text_muted`, not faint).
- Where: `month_view.rs`.

### Event panel

**UX-039 · Medium · New untitled events survive Esc**
Drag-creating an event and pressing Esc left a "(No title)" event on the grid (seen twice, at Sat 6 PM and Sun 8 AM). `create_event` adds it to the data before you type anything. Google Calendar and Notion Calendar throw away a brand-new event if you leave without a title.
- Fix: in `close_panel`, delete the event when it was created in this panel session and the title is still empty. Show no toast.
- Where: `ui/calendar/mod.rs` `close_panel` / `commit_panel`.

**UX-040 · Medium · Recurring-event edits are confusing** (`ux-19`)
The panel says "Every week · edits change every one". The copy is awkward, and there's no "This event / All events" choice when editing or dragging an occurrence. Dragging one Friday demo silently moves the whole series.
- Fix: when a recurring event is changed, show a small inline prompt: "Change only this event / All events". Until that exists, say "Changes apply to all events in the series".
- Where: `event_panel.rs`, `drag.rs` `finish_drag`.

**UX-041 · Low · Panel details** (`ux-19`)
- Two delete controls (trash icon in the header and a "Delete event" row) plus a ⌫ keycap that looks like a stray button. Keep one: the row, with the keycap inside it.
- The video URL is hard-clipped mid-character ("fri-demc") with no ellipsis.
- "1 guest · 1 yes" contradicts the two people listed below it.
- The RSVP check badge overlaps the avatar initials.
- About 120pt of empty description area pushes reminders and Busy/Free down.
- The tinted gradient behind the title doesn't appear anywhere else in the app.

### Drag interactions

**UX-042 · Low · Calendar drag works; pane dividers weren't verified**
Calendar drag-create, move and resize all worked with real mouse events: 15-minute snap, a ghost preview, and Esc cancels. Good.

GPUI ignored synthetic background clicks and drags on the grid and on the pane dividers, so I couldn't verify pane resizing to its extremes. Read from the code, the dividers have no hover or cursor feedback beyond gpui-component's default, and the hit area sits inside the pane edge.
- Fix: a 6pt hit area centered on the border, `cursor_col_resize`, and a 1px accent line on hover or drag. Double-click resets the width.
- Where: `ui/mod.rs` `pane_divider`.

**UX-043 · Low · Dragged events don't show the time**
While moving or resizing, there's no floating time label ("2:15 – 3:00 PM"), so you can only judge the snap from the gridlines.
- Where: `drag.rs` `render_drag_ghost`.

### Invite card

**UX-044 · Medium · The mini timeline is illegible** (`ux-13`)
The day strip in the card truncates neighboring events to "Da…" and "Sync conf…" at 11px inside 20pt pills. The conflict line underneath says the same thing in words.
- Fix: show the strip only when there is a conflict. Make it 2 hours wide around the event with event names in the tooltip, or drop it and keep the clear red "Conflicts with Daily standup" line.
- Where: `ui/invite.rs`.

**UX-045 · Low · Invite RSVP and card details**
- The Yes/Maybe/No buttons are 30pt tall with a 10px "?" glyph for Maybe.
- "Not answered yet" sits far right, away from the buttons.
- The email body under the card repeats When/Where/Organizer.
- Fix: put the status as a label before the buttons ("Going?" → "Not answered"). When there's a card, collapse body lines that start with When:/Where:/Organizer:.

### Dark mode

See UX-002. Also:

**UX-046 · Low · Dark event fills are heavy** (`ux-24`)
Dark event fills (L 0.25, α 0.9) become muddy navy and brown blocks over the grey glass. Past events and the selected event lose distinction.
- Fix: lighten fills (L 0.30, saturation ×0.6) and fade past events more, to 0.45.
- Where: `theme.rs` `event_fill`.

### Accessibility

**UX-047 · High · VoiceOver sees an empty window**
The accessibility tree contains only the window, the three traffic lights and the menu bar. No rows, buttons, fields or text (Cua `get_window_state`: 7 elements). VoiceOver users cannot use the app at all, and Voice Control and Full Keyboard Access can't target anything.
- This is a GPUI limitation today. Track it. In the meantime, the menu bar (UX-007) is the only accessible surface, so building it out matters twice.

**UX-048 · Medium · Small hit targets and no reduced motion**
- Hit targets are below Apple's 28pt minimum for mouse targets in places: the close-search button is 24pt and label chips are 18pt.
- The search animation ignores Reduce Motion.
- Text sizes are fixed, with no ⌘+/⌘− or Dynamic Type equivalent.
- Fix: 28pt minimum targets; ⌘+/⌘− scaling the reader body at least; respect Reduce Motion (UX-005).

### Bugs

| ID | Severity | Bug | Where |
|---|---|---|---|
| UX-049 | High | Selection not scrolled into view after ⌘K jump (= UX-012) | `app.rs` |
| UX-050 | High | Reader clipped at minimum window width (= UX-006) | `app.rs`, `main.rs` |
| UX-051 | Medium | Untitled new events persist after Esc (= UX-039) | `calendar/mod.rs` |
| UX-052 | Low | Pending recipient text discarded silently on close (= UX-025) | `composer.rs` |
| UX-053 | Low | Two drafts for the same thread (= UX-026) | `app.rs` |
| UX-054 | Low | GPUI ignores background clicks and drags on the calendar grid. This only affects automation, but it means accessibility clients (Voice Control) can't click events either | — |

## Top 10 to do first (impact ÷ effort)

1. **Fix selection reveal after palette and keyboard jumps** (UX-012). Small change, removes the worst keyboard failure.
2. **Raise contrast:** `text_faint`, `text_dim` and accent values, plus pane alphas in both themes (UX-001, UX-002). One file.
3. **Show focus clearly:** accent selection when the list has focus, grey when the reader does (UX-013).
4. **Make the minimum window work:** shrink the list, auto-collapse the sidebar, or raise the minimum (UX-006).
5. **Discard untitled new events on Esc** (UX-039).
6. **Side-by-side layout for overlapping events** (UX-035).
7. **Full menu bar with ⌘N, ⌘F, ⌘W, ⌘M, ⌘H, ⌘,** (UX-007). Also the only accessible surface for now.
8. **Context-aware palette:** hide commands that can't run, empty query sorted by recency (UX-029).
9. **Calendar arrows move by day or period; `j`/`k` match Mail** (UX-032).
10. **Search operator suggestions and contact autocomplete** (UX-027).

## Bigger ideas

- **Split inbox / smart sections** (Superhuman, Spark): Important, Calendar, Notifications and Newsletters as tabs above the list, switched with ⌘⇧1…4. You already have labels and invites to drive them.
- **Snooze and "remind me if no reply"** with `h`, using a natural-language time field ("tom 9am") that reuses the event panel's date parsing.
- **Inbox-zero moment:** a quiet full-pane illustration with the time and next event when the Inbox is empty. Superhuman's version is part of why people like the app.
- **Mail ↔ Calendar bridge:** an invite in the reader shows a live mini day-view slice (a better version of UX-044). `⇧C` opens Calendar at that hour with the event selected and a back chip ("← Back to mail"). Dropping a thread onto the calendar creates an event linked to it.
- **Command palette as the hub:** prefixes (`>` commands, `@` people, `#` labels, `/` folders), recents, and actions on the selected result with ⌘↵ (archive or reply from the palette).
- **Quick-reply bar** pinned at the bottom of the reader ("Reply to Sarah…") that expands into the composer. Most replies are one line.
- **Density and type settings:** comfortable/compact list, reader font size, and an optional serif body.
- **Instant feel:** archive and delete animate the row out over 100 ms while the next one is already selected. Optimistic toasts stack (Linear) instead of replacing each other, so undoing two archives is possible.
- **Calendar natural-language create:** `c` opens a one-line field ("Lunch with Priya Thu 12:30 at Nopa") that fills the panel. Fantastical's signature feature, and it fits a keyboard-first app.

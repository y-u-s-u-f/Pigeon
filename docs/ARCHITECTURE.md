# Architecture

How Pigeon is put together, for whoever is about to change it. This is the
structure; the feature inventory and its status are in `docs/ROADMAP.md`.

## 1. The shape of the app

Pigeon is one process with one main window and no server. `main`
(`src/main.rs`) takes the single-instance lock (`src/instance.rs`), opens the
data folder, builds `Backend` as an entity, reads the saved settings into
globals, and opens the window through `shell::show(Show::Main, cx)`.
`PigeonApp::new` (`src/app.rs`) loads the mail heads, creates `CalendarView`
(`src/ui/calendar/mod.rs`) and the panes (`src/app/panes.rs`).

`PigeonApp` owns the mail state: the `MailData` view model, rows, selection,
overlays and the session's undo. `CalendarView` is a second entity holding the
calendar's state, so switching between Mail and Calendar loses nothing.
`Backend` (`src/backend/mod.rs`) owns the store, the outbox, sync, auth and the
AI service, and never draws.

Globals hold what outlives a window: `Saved` (`src/settings.rs`), `Shell` with
the backend and the main window (`src/app/shell.rs`), the theme
(`src/theme.rs`), and the keymap and menus (`src/actions.rs`, `src/main.rs`).
Draft windows (`src/app/draft_window.rs`) and the menu bar panel
(`src/app/menu_bar.rs`) are per-window.

Reads are pulled: heads at startup, bodies when a conversation opens. Changes
are pushed as `BackendEvent`s, one arm per variant in
`src/app/backend_events.rs`, which reloads the affected rows. A write is
optimistic: the view model changes first, the backend gets a `MailOp` or
`CalOp`, the outbox holds it (undoable ops wait `UNDO_WINDOW`, 5 s), the worker
commits to Google, the store takes the result, and the sync reads it back.

## 2. Layers

| Layer | Directory | What lives there | What must not happen there |
| --- | --- | --- | --- |
| Platform and entry | `src/main.rs`, `src/instance.rs`, `src/logging.rs`, `src/assets.rs` | Launch order, menus and Dock menu, single-instance lock, log file, bundled icons | No app state, no feature logic |
| App shell and state | `src/app.rs`, `src/app/` | `PigeonApp`, action handlers, windows, overlays, undo, toasts | No HTTP or SQLite calls; no direct Google requests |
| Data model | `src/data.rs`, `src/calendar/`, `src/search.rs`, `src/contacts.rs`, `src/html.rs`, `src/markdown.rs`, `src/later.rs` | Plain serializable types and pure logic (recurrence, layout, date parsing, HTML, markdown, query matching) | No elements, no store, no network |
| Backend | `src/backend/` | Auth, HTTP transport, sync scheduler, SQLite store, outbox, MIME, and one module per API (`gmail`, `calendar`, `people`, `labels`, `compose`, `later`) | No drawing, no UI mutation: it reports through `BackendEvent` |
| AI | `src/ai/` | Writing-model and Jev clients, job queue, prompts, pure readers, results cache | No UI, no direct store access; keys stay in `ai/config.rs` |
| MCP server | `src/mcp/` | Streamable HTTP transport, token, tool table | No second write path: tools use the same `MailOp`/`CalOp` |
| UI | `src/ui/`, `src/ui/calendar/` | Top bar, rail, sidebar, list, reader, composer, palette, settings, calendar views | No business logic, no network, no SQL |
| Design tokens and controls | `src/theme.rs`, `src/ui/style.rs`, `src/ui/motion.rs`, `src/ui/controls.rs` | Colors, type/spacing/radius scales, springs, shared controls | No view-specific state or copy |
| Native macOS | `src/native.rs`, `src/native/menu_bar.rs`, `vendor/gpui-pre-macos/` | Dock badge, login item, Quick Look, date detection, status item and hot keys, patched AppKit behaviour | No app state; main thread only |

## 3. Module index

`src/` is about 103,000 lines across 121 files. Sizes are lines of Rust.

| Directory | Files | Lines | What is there |
| --- | --- | --- | --- |
| `src/` | 16 | 14,800 | `main`, `app`, `data`, `actions`, `settings`, `theme`, and shared helpers (`search`, `html`, `markdown`, `contacts`, `later`, `notify`, `native`, `assets`, `instance`, `logging`) |
| `src/app/` | 18 | 12,200 | `PigeonApp`'s methods split by workstream: mail, compose, labels, later, attachments, AI, shell, windows, backend events |
| `src/backend/` | 19 | 21,200 | `Backend`, auth, HTTP, sync, outbox, MIME, and the Google API modules |
| `src/backend/store/` | 9 + `schema.sql` | 3,200 | SQLite access: mail, calendar, contacts, images, AI results, outbox, snoozes, notified, settings |
| `src/ai/` | 15 | 7,600 | Clients (`llm`, `jev`), queue, background pass, reading, search, Do It, config, log, eval scripts (`check*`), `testdata/` |
| `src/mcp/` | 3 | 3,500 | Server transport, tool table, capability switches |
| `src/ui/` | 23 | 21,700 | Panes and overlays: `top_bar`, `rail`, `sidebar`, `thread_list`, `reader`, `composer`, `command_palette`, `settings`, `context_menu`, `menu_bar`, `controls`, `style`, `motion` |
| `src/ui/calendar/` | 10 | 13,900 | `CalendarView`, day/week/month grids, event panel, drag, paging, repeat, mini month |
| `src/calendar/` | 7 | 4,400 | Calendar model plus pure logic: recurrence, overlap layout, quick add, reminders, date math, event policy |
| `src/native/` | 1 | 800 | The menu bar's AppKit side |

Files to grep rather than read whole:

| File | Lines | Note |
| --- | --- | --- |
| `src/app.rs` | 3,300 | `PigeonApp`, its fields and action handlers; `render_root` wires every `on_action` |
| `src/actions.rs` | 1,450 | `actions!` list, key contexts, `bind_keys`, `COMMANDS`, `HELP` |
| `src/data.rs` | 1,400 | `MailData`, `Thread`, `Message`, `Folder`, `Label`, date and size formatting |
| `src/backend/mod.rs` | 2,500 | `Backend`, `MailOp`, `CalOp`, `Op`, `BackendEvent`, every read method |
| `src/theme.rs` | 800 | `Theme` fields plus `dark`, `light`, `build`, contrast and glass passes |
| `src/settings.rs` | 1,100 | `Saved`, `SettingId`, `SETTINGS` metadata, search over settings |
| `src/ui/calendar/event_panel.rs` | 4,800 | Largest file in the repo; the event editor |
| `src/ui/calendar/mod.rs` | 4,100 | `CalendarView` state, actions and rendering |
| `src/backend/calendar.rs` | 3,700 | Google Calendar types and calls |
| `src/ui/composer.rs` | 3,000 | Draft UI, recipients, sending |
| `src/ui/reader.rs` | 2,800 | The conversation view |
| `src/ui/settings.rs` | 2,400 | Settings page; `control()` and `change_setting()` map ids to values |
| `src/ui/thread_list.rs` | 2,000 | Virtualized list |
| `src/app/mail.rs` | 1,900 | Multi-select, moves, labels, search, density |
| `src/backend/sync.rs` | 1,700 | The sync loop and its plans |
| `src/backend/http.rs` | 1,700 | `Transport`, retries, backoff, batch format |
| `src/backend/gmail.rs` | 1,600 | Gmail types, request builders, label mapping |
| `src/mcp/tools.rs` | 1,500 | The tool table and `run()` |
| `src/ui/context_menu.rs` | 1,400 | The app-drawn right-click menu |

## 4. Where do I add X?

| Task | Files to touch |
| --- | --- |
| New keyboard action | `src/actions.rs`: the `actions!` list, a `KeyBinding` in `shared_bindings` (or `preset_bindings` for a preset-specific one), a `COMMANDS` row; handler in `src/app.rs` (`cx.listener` in `render_root`) or the matching `src/app/*.rs` |
| New command in ⌘K | `src/actions.rs`: a `COMMANDS` row with its `Group`, icon, `When` gate and optional `Toggle`; the palette, the shortcut sheet and settings search all read that table |
| New menu item | `src/main.rs`: `menus()` and, for something that changes, a field in `MenuState` |
| New settings row | `src/settings.rs`: `SettingId` variant and a `SETTINGS` entry (section, label, description, keywords), a field in `Saved` plus `from_map`/`to_map`; `src/ui/settings.rs`: arms in `control()` and `change_setting()` |
| New mail action | `src/backend/mod.rs`: a `MailOp` variant; `src/backend/outbox.rs`: `requests()` and `commit_threads()`; undo in `src/app/undo.rs`; handler in `src/app/mail.rs`; menu and `COMMANDS` rows |
| New calendar view | `src/ui/calendar/mod.rs`: a `ViewKind` variant, `visible_range`, `set_view` and the render match; a renderer beside it; a `Show…View` action in `src/actions.rs` and the View menu in `src/main.rs` |
| New AI command | `src/actions.rs`: action, `COMMANDS` row, `AiNeed` gate; UI in `src/app/ai_reader.rs` or `src/app/ai_palette.rs`; prompt and parsing in `src/ai/reading.rs` or `src/ai/search.rs`; a `Feature` in `src/ai/mod.rs` and a row in `SETTINGS` |
| New MCP tool | `src/mcp/tools.rs`: a `TOOLS` entry, a schema, an arm in `run()`; a `Capability` in `src/mcp/mod.rs` if it needs a new switch, plus its settings row |
| New theme color | `src/theme.rs`: a field on `Theme`, the value in `dark()` and `light()`, and a pass in `increase_contrast()` or `thicken()` if it must follow those |
| New motion token | `src/ui/motion.rs`: a `Spring` via `policy(...)` (or a constant for distance and timing), then use it through `spring`, `enter`, `presence` or `card_motion` |
| New reusable control | `src/ui/controls.rs` (button, switch, segmented, stepper, slider, chip, menu row, popover); small shared helpers go in `src/ui/mod.rs` |
| New backend call | `src/backend/http.rs`: `Request` if the shape is new; a `*_request()` builder in `src/backend/gmail.rs`, `calendar.rs` or `people.rs`; call it from `sync.rs` or the module that owns the flow |
| New store migration | `src/backend/store/schema.sql`: a new numbered block after the last one (210 today); accessors in the matching `src/backend/store/*.rs` |
| New native integration | `src/native.rs` for app-level AppKit calls, `src/native/menu_bar.rs` for the status item; new `objc2-*` features in `Cargo.toml` go through the main agent |
| New notification | `src/notify.rs` to post it; the trigger in `src/backend/new_mail.rs` or `src/calendar/reminders.rs` |
| New overlay or pane | A file in `src/ui/`, a field on `PigeonApp` in `src/app.rs`, drawing in `src/app/panes.rs` (pane) or the overlay layer, and a key context in `src/actions.rs` |

## 5. Rules that bite

- **Colors come from `theme.rs` only.** Views call `Theme::of(cx)` and use its
  semantic fields; no `Hsla` or hex literals in a view. The theme follows the
  system appearance, accent, Increase Contrast, Reduce Transparency and the
  glass setting, so a hardcoded color breaks four modes at once
  (`src/theme.rs`).
- **Sizes come from `src/ui/style.rs`.** Type, control heights, icons and
  avatars are in rems via `type_size`, so ⌘= scales them; spacing and radii are
  the pixel tokens. A raw `px()` for any of these breaks text scaling.
- **Motion comes from `src/ui/motion.rs`.** Transitions use the springs
  (`SPRING_PANE`, `SPRING_POP`, `SPRING_EXIT`, `SPRING_CONTROL`,
  `SPRING_SCROLL`, `SPRING_SNAP`) through `spring`, `presence`, `enter`,
  `card_motion` or `Glide`; never a linear or ad-hoc duration. Reduce Motion is
  handled by GPUI snapping the springs, so a hand-rolled animation ignores it.
  Loading pulses (`Animation::new(...).repeat()`) are the one exception.
- **Shared controls live in `src/ui/controls.rs`.** Switches, segmented
  controls, steppers, sliders, chips and menu rows already carry hover, press,
  focus, role and label. A new hand-rolled copy is a review finding.
- **Every user-facing command is an action in `src/actions.rs`.** Shortcuts,
  toolbar buttons, menus and ⌘K all dispatch actions. A command missing from
  `COMMANDS` is invisible in the palette and the shortcut sheet, and one
  without a key binding fails the "every action has a shortcut" expectation.
- **`assets/*.json` is generated demo data.** `mailbox.json` and
  `calendar.json` are seeded by a separate agent; don't edit them by hand, and
  don't treat their shape as an API (`src/backend/demo.rs` reads them).
- **`vendor/gpui-pre-macos` is patched upstream.** Read
  `vendor/gpui-pre-macos/PIGEON_PATCHES.md` before touching it. The appearance
  mapping is deliberate (dark mode with Increase Contrast), and every change
  must be added to that list.
- **The store schema is append-only.** Blocks in
  `src/backend/store/schema.sql` are numbered and applied by `migrate()` against
  `PRAGMA user_version`; never edit a merged block, and number a new one above
  the last (210 today). `migrations_are_numbered_in_order` in
  `src/backend/store/mod.rs` enforces the ordering.
- **Writes go through the outbox, never straight to Google.** The UI applies
  its change locally and calls `Backend::mail` or `Backend::calendar`; the
  outbox row survives a quit and carries the undo window. A direct request from
  a view skips undo, offline queueing and the sync read-back.
- **Nothing blocking runs on the main thread.** HTTP and SQLite work is spawned
  on the background executor and comes back through `BackendEvent`; the store
  is one `rusqlite::Connection` behind a mutex with short transactions
  (`src/backend/mod.rs`, `src/backend/store/mod.rs`, `src/app/backend_events.rs`).

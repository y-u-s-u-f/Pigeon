# AI sprint: AI in Pigeon

The plan as agreed with the user on 2026-10-28, after two review rounds (the working copy with progress is the OpenChamber plan of the same name). Phase 4 adds the final thresholds and residual risks.

## Steps

- [x] Before the sprint: keys in `.env`; check both models answer; measure the writing model's speed; record real responses as test fixtures
- [ ] Phase 1 — W1a AI core (clients, queue, cache, backend hooks) and W1b app shell (Settings › AI, actions, stubs), in parallel (AI-1, AI-2, AI-3 client)
- [ ] Phase 2 — W2 Background pass and list (AI-3 pass, AI-4, AI-5, AI-7) and W5 Calendar (CAL-39 tags, AI-16 date finder), in parallel
- [ ] Phase 3 — W3 Reader (AI-6, AI-8, AI-9, AI-10, AI-11, AI-16 bar) and W4 ⌘K and search (AI-12, AI-13, AI-14, AI-15), in parallel
- [ ] Phase 4: real-Inbox check and thresholds, copy and shortcut audit, roadmap gate, `docs/AI_SPRINT.md`, final build, test and commit

---

## 1. Goal

AI inside Pigeon, on two models: a **writing model** (your OpenAI-compatible endpoint) for summaries, answers and replies, and **Jev** (through OpenRouter) for fast, cheap yes/no, pick-one and score decisions. No new HTTP or async stack; every new action in ⌘K with a shortcut; minimal look. **Nothing is sent anywhere until Settings › AI is switched on and its note confirmed.**

**Roadmap:** a new section **AI (AI)** with AI-1 to AI-16, added when this plan is approved, appended as `## 10. AI (AI)` before "Not planned or out of scope" (which becomes section 11; its Contents and Progress anchors are updated). PLN-2 moves there as AI-10 (alias kept, counted once). Quick Add calendar tags extend CAL-39.

## 2. Before the sprint (you, then me)

1. Fill `PIGEON_AI_API_KEY` and `PIGEON_AI_MODEL` in `/Users/yusuf/dev/Pigeon/.env`.
2. Fill `PIGEON_JEV_API_KEY` with an OpenRouter key (base URL and `typesafe/jev-1.14` are set).
3. Keep Tailscale on.

Then I check, each separately: the writing model answers a plain request and a streamed one; on a 6k-token prompt I measure time to first token and tokens per second. Jev answers a `noul`, a `choice` and a `score` question at `https://openrouter.ai/api/v1/systemone`. Raw responses (keys stripped) become fixtures in `src/ai/testdata/`; the Jev `Answer` type is written against them. The measurements set `INPUT_BUDGET` (default 6k tokens) and `IDLE_TIMEOUT` (section 4).

**Measured (2026-10-28):** writing model, plain "OK" 1.9 s; streamed short answer 1.9 s to first token; a 6.5k-token prompt 5.2 s to first token, about 400 tokens/s, and it spends some output on reasoning (`completion_tokens_details.reasoning_tokens`, 36 of 145). Jev: 0.35–0.75 s per request, about $0.000015 each; a three-question request was faster than one question. Fixtures: `src/ai/testdata/{llm_plain.json, llm_stream.txt, jev_noul.json, jev_choice.json, jev_score.json, jev_multi.json}` (model and ids replaced with `test-model` / `test-id`). Set from this: **`INPUT_BUDGET` = 12,000 tokens**, **`IDLE_TIMEOUT` = 30 s**, and every max-output figure in section 4 gets **+512 tokens of room for reasoning** (a summary line asks for 60 + 512). The stream's final chunk carries `usage` when `stream_options.include_usage` is sent; the log reads tokens from it.

## 3. Decisions (agreed)

| # | Decision |
| --- | --- |
| 1 | `.env` read at startup: repo `.env` (path fixed at build time), then `<data dir>/.env`, then real environment variables. Keys never shown in the UI; endpoint and key not editable in Settings. A `.env` change takes effect on restart. |
| 2 | AI off by default; turning it on shows a one-time note on what is sent where. |
| 3 | Bodies of new Inbox mail are fetched in the background. |
| 4 | First run also covers Inbox conversations from the last 30 days. |
| 5 | One-line summaries: Inbox only, snippet until ready, small AI mark on the row. |
| 6 | Jev through OpenRouter's System One API. |
| 7 | Quick Add: add calendar tags (`#work`). No autocomplete, no other parser changes. No AI. |
| 8 | Email to event: Apple's NSDataDetector + our Quick Add parser + Jev. No writing model. |
| 9 | Do It for Me: one search + one bulk action, previewed, with normal Undo. |
| 10 | Ask Pigeon: a floating panel over the window. |
| 11 | AI reading view ("Clean View"): automatic for newsletters, made ahead of time in the background pass; not ready yet → original with a "Clean view ready" bar. |
| 12 | Important-only filters notifications only; the Dock badge stays the unread Inbox count. |
| 13 | Testing: unit tests plus a check script against the real models. **No computer-use or GUI testing**; you check the UI. |
| 14 | Roadmap: new AI section, PLN-2 moves there, added when the plan is approved. |

**Cuts and limits** (deliberate, YAGNI):
- No Low Power Mode pause; background AI pauses only on provider errors.
- Jev through OpenRouter only.
- The 30-day backfill covers the Inbox conversations already cached (the sync lists the latest 100 Inbox conversations); older Inbox mail isn't paged in.
- A model or endpoint change doesn't redo old results; new ones use the new model.
- Meeting invites (`text/calendar`) aren't recognized as such; a candidate matching an existing event's start is hidden, which hides most accepted invites.
- Best Matches finds conversations that share a word with the query or are recent, not synonym-only matches. Ask Pigeon's search terms from the writing model cover that.
- Do It for Me acts only on mail cached on this Mac, never on drafts or scheduled sends.
- Dismiss Needs Reply has no ⌘Z; running it again restores the marker.
- A failed AI job is retried up to 3 times per launch, then left until restart, **Retry** or Test Connection.
- A reply queued while offline hides the Needs Reply marker even if newer mail arrives before it's sent.

## 4. Shared contracts

### Modules and owners

| File | Owner | What |
| --- | --- | --- |
| `src/ai/mod.rs` | W1a | `AiService` (plain struct in `Backend`), `Feature`, `Priority`, `Job`, `JobGroup`, `ThreadAi`, `EventCandidate`, `Source`, `Transport` trait. |
| `src/ai/config.rs` | W1a | `.env` loading; `AiConfig { llm: Option<Endpoint>, jev: Option<Endpoint> }`. |
| `src/ai/queue.rs` | W1a | Pure scheduler: priorities, slots, groups, generation, failures. No GPUI types. |
| `src/ai/llm.rs`, `src/ai/jev.rs` | W1a | The two clients. |
| `src/ai/text.rs` | W1a | What a message looks like when sent. |
| `src/ai/log.rs` | W1a | Activity log (last 100 calls, in memory). |
| `src/ai/pass.rs` | W2 (stub W1a) | `work_list(heads, results, failures) -> Vec<Job>`. |
| `src/ai/notice.rs` | W2 (new) | Pending important-only notification batches, pure. |
| `src/ai/events.rs` | W5 (stub W1a) | `candidates(text: &str, sent: DateTime<Utc>) -> Vec<EventCandidate>`; stub returns empty. |
| `src/ai/reading.rs` | W3 (stub W1a) | `request(msg: &text::Message) -> Option<llm::Request>` (`None` when over budget); `clean(markdown: &str, original_html: &str) -> String`. |
| `src/ai/search.rs` | W4 (stub W1a) | `candidates(terms, &MailData, 50) -> Vec<usize>`, `rerank(query, records) -> jev::Request`, `ask_request(question, sources, budget) -> (llm::Request, Vec<Source>)`. |
| `src/ai/do_it.rs` | W4 (new) | Do It JSON validation and matching, pure. |

### `.env`, URLs, clients

**`.env` format** (own ~40-line parser): `KEY=VALUE` per line; blank lines and full-line `#` comments skipped; optional leading `export `; value is everything after the first `=`, trimmed; one pair of matching `'` or `"` stripped, no escapes; in unquoted values ` #` starts a comment. An empty value means "unset" (the data-folder `.env` can turn a provider off).

**URLs.** `Endpoint::url(path)` trims trailing `/`. Writing model: `{PIGEON_AI_BASE_URL}/chat/completions` (base includes `/v1`). Jev: `{PIGEON_JEV_BASE_URL}/v1/systemone`. Both send `Authorization: Bearer <key>`.

**Writing model.** Every call streams (SSE). If a streamed request gets a 400, that job is retried once without streaming and streaming stays off until restart. JSON answers (Ask search terms, Do It) come from a plain prompt asking for JSON: parsed and validated, one retry, then the job fails. No JSON-mode or capability probing. Max output tokens: summary line 60, summary 400, reply chips 200, Clean View 4,000, Ask 800, Do It 200. An answer cut off by the token limit (`finish_reason: "length"`) is a failure and isn't shown.

**Effort.** The endpoint picks reasoning effort from a model-name suffix: `-low`, `-medium`, `-high`, `-xhigh`, `-max`. `PIGEON_AI_MODEL` is the base name (a known suffix on it is stripped); each request appends its feature's effort, so effort is set in code, in one table in `llm.rs`. Measured on a one-line summary: no suffix 2.4 s, `-low` 1.6 s, `-medium` 1.9 s, `-high` 3.7 s (39 reasoning tokens).

| Effort | Features |
| --- | --- |
| `-low` | summary line (AI-7), quick replies (AI-11), Ask search terms (AI-14 step 1) |
| `-medium` | conversation summary and catch-up (AI-8, AI-9), Clean View (AI-10), Do It for Me (AI-15) |
| `-high` | Ask Pigeon answer (AI-14 step 5) |

The activity log shows the full model name with its suffix.

**Jev.** Typed `Question::{Noul, Choice, Score}` with stable string IDs; choice 2–255 options and score 2–10 levels checked before sending. `Answer` parsed from the fixtures; `Answer::yes()` is the probability of "yes" for a `noul`, `None` when missing or invalid. `None` means "no result" (nothing shown), never "no".

**Text sent** (`text.rs`): `html::plain_text`, quoted history cut at `html::trailing_quote_start`, sender, subject and date as fields. Budgets: writing model `INPUT_BUDGET` per call (tokens estimated as characters / 4), Jev about 4k per message. Trimming is allowed for summaries, lines and Jev; never for Clean View (over budget → no Clean View, no request). Only the Gmail messages a feature is about are sent (plus label-example subjects for AI-6 and compact search records for AI-13/AI-14): never drafts or local unsent messages, keys or settings.

### AI service

`AiService` is a plain struct field of `Backend`. `Backend::switch` passes it into `Self::new` like `prefs` and `tokens` (`src/backend/mod.rs:956–968`), so its generation, log and provider state survive sign-out, demo and sign-in. `switch` bumps the generation **before** `*self = …`.

- **Priorities:** Interactive (you asked, or a conversation you opened, or a pending notification) > Background (pass, backfill; newest conversations first).
- **Slots:** writing model 2, of which background may use 1. Jev 6, of which background may use 2.
- **Groups:** `JobGroup::{Open, Palette, Search, Ask}`. Opening another conversation, new ⌘K text, a new search or a new Ask question calls `cancel(group)` first: queued jobs of that group are dropped and in-flight ones closed (streams check a cancel flag per chunk). Same `(account, feature, key)` already queued → promoted, not duplicated.
- **Generation:** `Arc<AtomicU64>` bumped on AI turned off, a feature switch off, and every `switch` (sign-out, demo, sign-in). Each job carries the generation it was queued at; it's checked before sending and before publishing. A bump drops all queued jobs and closes streams.
- **Writes can't outlive the account.** `ai_results` rows are inserted with `… WHERE EXISTS (SELECT 1 FROM threads WHERE account = ?1 AND id = ?thread_id)` in the write transaction, so anything written after `delete_account` is skipped. The same guard goes into `Store::put_bodies` (`src/backend/store/mail.rs:310–346`): inside its write transaction it skips the write when the thread row is gone. This fixes the existing race for reader body fetches too.
- **Failures** (in `queue.rs`, in memory): per job key, attempts and retry-after (30 s, doubling). After 3 failed attempts the key is skipped until restart, **Retry** or Test Connection. The pass skips queued, in-flight and not-yet-due keys before counting its budget. At most 200 queued jobs in total.
- **Errors, per provider:** 401/403/404 → that provider stops ("Check the key in .env" in Settings) until Test Connection or restart. 400 → that job fails (above). Timeouts, network errors, 429, 5xx → the provider backs off 30 s, doubling to 30 min, reset on success. Gmail being offline doesn't pause AI.
- **Timeouts:** connect 10 s. Jev 15 s total. Writing model: no total cap; `IDLE_TIMEOUT` between bytes (ureq `timeout_per_read`) = 3× the measured 6k-prompt time to first token, at least 30 s.
- **Text already sent can't be recalled.** Turning AI off stops everything not yet sent; the note says so.

**Backend API** (W1a writes it; W1b, W2–W4 call it):
- `ai_apply(&mut self, prefs: &AiPrefs, cx)`: enabled, switches, note fingerprint; bumps the generation when something turns off.
- `ai_providers(&self) -> ai::Providers` (host, model, status per provider), `ai_activity(&self) -> Vec<ai::log::Entry>`, `ai_fingerprint(&self) -> String`.
- `ai_test(&mut self, provider, cx)`: one request; clears that provider's stop and all failures.
- `ai_enqueue(&mut self, job: ai::Job, cx)`, `ai_cancel(&mut self, group)`, `ai_retry(&mut self, feature, key)`.
- `ai_partial(&self, feature, key) -> Option<&str>`: the streamed text so far for an Interactive job.

**Events.** New `BackendEvent::BodiesReady(Vec<String>)` (thread ids whose bodies were stored: background queue and reader fetch) and `BackendEvent::AiChanged(Vec<String>)` (thread ids whose results changed; empty = provider status or log only). `AiChanged` is batched: about every 1 s for background results, every 100 ms while an Interactive job streams. The pass listens to `NewMail` and `BodiesReady`, never to `AiChanged` or `ThreadsChanged`, so it can't trigger itself.

**Reaching the UI.** `Thread` gets `#[serde(skip)] pub ai: ThreadAi` (the thread's `ai_results` rows by feature and key). `load_mail_heads` and `load_threads` fill it from one `ai_results` query each, like `overlay_snoozes`. On `AiChanged(ids)`, `app/backend_events.rs` reads the rows for those ids in one query, replaces `ai` in place on those threads, calls `refresh_rows(true)` once and notifies the reader.

### Cache (AI-2)

Migration `210` in `schema.sql`:

```sql
CREATE TABLE ai_results (
  account TEXT NOT NULL, feature TEXT NOT NULL, key TEXT NOT NULL,
  thread_id TEXT NOT NULL, input_hash TEXT NOT NULL, model TEXT NOT NULL,
  value TEXT NOT NULL, created INTEGER NOT NULL,
  PRIMARY KEY (account, feature, key));
CREATE INDEX ai_results_thread ON ai_results (account, thread_id);
```

- `key`: a message id (message features) or a conversation id (line, summary, catch-up). One row per feature and key; a new result replaces the old one.
- `input_hash` = sha256 of the feature's prompt or question version constant + the message ids it covers, in order + feature extras (catch-up: its snapshot ids; label suggestion: the shortlisted label ids). Gmail messages don't change, so ids are enough, and the pass and the list check staleness from heads without loading bodies. `model` is recorded but not part of the hash.
- A job runs only when every body it needs is stored; incomplete inputs are never cached. A stored row is used only if its hash equals the current one.
- Rows that aren't model output: `needs_reply_dismissed` and `clean_rejected` (key = message id).
- Cleanup at startup: rows whose `thread_id` is no longer in `threads` for that account. `"ai_results"` joins `delete_account` (`src/backend/store/mod.rs:149–164`). Demo mode uses its in-memory store.

### Untrusted mail

- Decisions are code: thresholds, which actions exist, which IDs are valid. A model only supplies probabilities, text to show, or a proposal you confirm.
- Jev state holds one message (or one batch of 10 compact search records). ⌘K plain-words state holds only your typed text; its options are the command catalog.
- Every ID, action, label, citation number and URL a model returns is checked against a list Pigeon built; anything else is dropped. Clean View links and images must appear in the original; others become plain text. Remote images follow the existing setting.
- Notification filtering fails open.
- Residual risk (in `docs/AI_SPRINT.md`): a crafted mail can still change its own summary, markers, label suggestion or Clean View.

### Prefs and consent

`AiPrefs` in `Saved::ai` (like `McpPrefs`): `enabled`; `note_ack: Option<String>`; switches **List Summaries**, **Clean View for Newsletters**, **Needs Reply** (also gates quick replies), all on once AI is on; and `notify_for: All | Important` (default All). Everything else follows the main switch and which providers are set.

`note_ack` is a fingerprint of (source kind, writing host, Jev host). If it doesn't match, nothing is sent: the note shows again in Settings › AI, and a toast says "AI is paused until you review what is sent" with **Review**.

**Availability.** A missing key turns off only what needs it; the row says "Needs a writing model key in .env" or "Needs a Jev key in .env". The main switch works when either provider is set.

| Needs | Features |
| --- | --- |
| Jev | Needs Reply, important-only notifications, label suggestions, email to event, ⌘K plain words, Best Matches |
| Writing model | list summaries, conversation summaries and catch-up, Do It for Me |
| Both | Clean View (Jev picks newsletters), quick replies (Jev's Needs Reply), Ask Pigeon |
| Neither | Quick Add calendar tags (CAL-39) |

**The note:** "When AI is on, Pigeon sends message text, subjects and sender names, your label names, what you type in ⌘K and search (with the subjects and snippets of matching conversations), your Ask Pigeon questions and matching calendar events to <writing host> for writing and to OpenRouter (TypeSafe) for Jev. Nothing is sent while AI is off, and text already sent can't be taken back." Buttons: **Turn On** / **Cancel**.

**Demo mode.** With keys present, AI runs on demo data too (`PIGEON_DEMO=1`), with its own fingerprint.

### UI rules

Colors from `theme.rs`, sizes from `ui/style.rs`, motion from `ui/motion.rs`. `.role` and `.aria_label` on everything; AI text is spoken as "AI summary: …". Wording per AGENTS.md: "conversation", Title Case for buttons, menus and commands; sentence case for rows, bars, tooltips and toasts. The feature is called **Clean View** everywhere.

**Actions and shortcuts** (W1b declares them all):

| Command | Shortcut |
| --- | --- |
| AI Settings | ⌥⌘A |
| Go to Needs Reply | `g r` |
| Dismiss Needs Reply / Restore Needs Reply | ⇧H |
| Apply Suggested Label | ⌥L |
| Summarize Conversation (toggle) | ⇧S |
| Toggle Clean View | ⇧V |
| Retry Clean View | ⌥⌘R |
| Report Bad Clean View | ⌥⇧⌘B |
| Quick Reply 1 / 2 / 3 | ⌥1 / ⌥2 / ⌥3 |
| Add to Calendar (from mail) | ⇧T |
| Ask Pigeon | ⇧⌘J |
| Do It for Me… | ⇧⌘L |

## 5. Workstreams

**Rules for every implementer.**
- Own only the files listed; if `cargo clippy` fails in a file you don't own (the other parallel agent's half-done work), ignore it and don't fix it.
- Parallel implementers use their own `CARGO_TARGET_DIR` under `/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/`; the main agent deletes it afterwards.
- Run `cargo fmt` and `cargo clippy --all-targets` only.
- Roadmap: a deliberate exception to AGENTS.md for this sprint, since agents run in parallel: implementers don't edit `docs/ROADMAP.md`; each ends with the exact roadmap lines for its items (status, Needs), and the main agent merges them.
- Each workstream adds 10–15 labeled examples for its features to `src/ai/testdata/eval.json` and its feature to the check script.
- Tests are plain `#[test]`s on pure functions and the in-memory `Store` (`Store::open_in_memory`). No GPUI test context; anything needing a `Window` is on your UI check list.

### Phase 1 — W1a AI core

Files: `src/ai/{mod,config,queue,llm,jev,text,log}.rs`, stubs `src/ai/{pass,events,reading,search}.rs`, `src/ai/testdata/*`, `src/main.rs` (`mod ai`), `src/data.rs` (`Thread.ai`), `src/backend/mod.rs` (the `ai` field, `switch` carry-over, Backend API, events, overlay in `load_mail_heads`/`load_threads`), `src/backend/store/{schema.sql,mod.rs,ai.rs,mail.rs}` (`put_bodies` guard), `src/app/backend_events.rs` (no-op arms for the new events).

- `AiService`, `queue.rs`, clients, cache and Backend API as in section 4, with a fake `Transport` and a fake clock.
- `src/ai/testdata/eval.json` format, the Jev protocol fixtures, and the adversarial examples: forged "system" instructions, a mail telling Jev it's important, invented links in a newsletter, a label name with instructions.
- **Check script:** an ignored test, `cargo test ai_check -- --ignored --nocapture`, runs each feature over the eval set and demo data and writes Markdown per feature to `/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/ai-check/` (folder mode 700). For Jev choices it reports agreement when the option order is reversed. With `PIGEON_AI_CHECK_REAL=1` it also runs the Jev questions over the latest 200 cached Inbox messages of your real store and lists each subject with its probabilities, for you and me to skim.

**Done when:** tests pass for `.env` rules and override order, URL joining, SSE parsing, the 400 → non-streaming retry, Jev types and `yes()` with missing values, text budgets and quote cutting, hash and staleness from heads, the queue (an Interactive job starts at once while one background writing job runs and another is queued; groups cancel; generation bump drops queued jobs and discards a late result; 3-attempt failures; over 100 work items with the first ones failing still make progress; the 200 cap), `ai_results` written after `delete_account` doesn't appear, and `put_bodies` after `delete_account` writes nothing.

### Phase 1 — W1b App shell (parallel with W1a)

Files: `src/settings.rs`, `src/ui/settings.rs`, `src/actions.rs`, `src/app/ai.rs` (new), stubs `src/app/{ai_reader,ai_palette}.rs`, `src/ui/ask_panel.rs` (stub), `src/ui/mod.rs`, `src/app.rs` (the `ai` field, its init, handler wiring, calendars passed in `open_palette`), `src/ui/command_palette.rs` (`set_calendars(Vec<(String, String)>)` stub only), `src/app/mail.rs` (make `move_threads` and `set_label` `pub(crate)` only).

- **AI-1 Settings › AI.** New `Section::Ai`. Rows: "AI" (main switch, with the note); "Writing model" and "Jev" (host and model, read-only, **Test Connection**); the three feature switches with their availability note; "Notify for" (**All New Mail** / **Important Only**; Important Only needs AI on and Jev); "Activity" (last 100 calls: time, feature, model, tokens in/out, ms, OK or error). Move the "ai" and "llm" search keywords from the MCP section (`src/settings.rs:476–479,643`) to this one. Add `SavedNav::NeedsReply` (`src/settings.rs:199`) and every new `SettingId`.
- **Actions:** every command in section 4 with its shortcut, in the command table. Each handler is a stub in its owner's file (`app/ai_reader.rs` for reader commands, `app/ai_palette.rs` for ⌘K, Ask and Do It, `app/ai.rs` for the rest). `AiState` holds `reader: ai_reader::ReaderAi` and `palette: ai_palette::PaletteAi`, empty structs their owners fill.
- **Keymap test:** for every keymap preset, no new binding equals another binding, is a prefix of one, or starts with one (`⇧L` is already the label-management prefix, `src/actions.rs:700–708`).

**Done when:** the keymap test passes, Settings › AI saves and restores `AiPrefs`, and the note fingerprint check is a tested pure function.

### Phase 2 — W2 Background pass and list

Files: `src/ai/{pass,notice}.rs`, `src/backend/sync.rs`, `src/backend/mod.rs` (pass hooks only), `src/backend/new_mail.rs`, `src/ui/thread_list.rs`, `src/ui/sidebar.rs`, `src/app.rs` (`Nav`, `nav_holds`, `refresh_rows`), `src/app/shell.rs`, `src/app/backend_events.rs`, `src/app/ai.rs`, `src/app/labels.rs`, `src/app/mail.rs` and `src/app/later.rs` (Nav matches only).

- **Background body queue** (`sync.rs`), separate from the reader's fetch: a FIFO of conversation ids, deduplicated, skipping ids in `sync.body_errors`. It works one conversation at a time through `MailSync::fetch_bodies` (`sync.rs:572`) at Gmail's Background priority, and emits `BodiesReady` every 10 conversations and when empty. The reader's fetch (`sync.rs:~1100`) also emits `BodiesReady` next to its `ThreadsChanged`.
- **AI-3 Background pass.** Runs on AI turned on or note confirmed, startup with AI on, `NewMail`, `BodiesReady`, and a 5-minute timer. `pass::work_list` recomputes from heads and `ai_results`: Inbox conversations from the last 30 days, newest first, with results missing or stale, skipping keys the queue already holds or has failed. Up to 100 jobs per run; conversations without bodies go to the body queue first.
  - Jev request A, on `NewMail`, from sender, subject and snippet (no body wait): `important` (noul): "needs my attention soon". Used by AI-5.
  - Jev request B, once the body is stored: `needs_reply` (noul, "the latest message asks me something or expects my answer"; not asked for automated senders, `contacts::is_automated`, which count as "no"); `newsletter` (noul, all senders); `event_1`…`event_3` (noul, one per `ai::events::candidates` result: "The message proposes or confirms an event on <date text>, in this sentence: <sentence>").
  - Skipped: my own messages, Spam, Trash.
  - **On open:** `select_thread` (from W3's hook) asks for request B for the opened conversation's latest messages that lack it, in any folder but Spam and Trash, at Interactive in `JobGroup::Open`, once `BodiesReady` says its bodies are stored. W2 provides `AiState::request_b(thread_id)`.
- **AI-4 Needs Reply.** `pass::needs_reply(thread, me, queued_sends, threshold) -> bool`, pure: in the Inbox; the latest message not from me has `needs_reply` yes ≥ threshold; no message from me comes after it; the conversation has no queued send (`Backend::queued_sends`, kept in the store, so it survives restart); that message isn't dismissed. Marker beside the Important mark, also in the spoken label.
  - **Dismiss Needs Reply** writes a `needs_reply_dismissed` row; running it on a dismissed conversation removes the row.
  - **Needs Reply view:** `Nav::NeedsReply`, a local filter like `Nav::Snoozed`: Inbox conversations (all categories) with the marker, newest first, no "load older" or server paging. Add the matching arm everywhere `Nav::Snoozed` appears (`app.rs` 125, 639, 654, 792; `ui/sidebar.rs` 82, 131, 160, 186, 318, 840; `ui/thread_list.rs` 114, 553; `app/mail.rs` 1081, 1120, 1645, 1727; `app/later.rs`), a count slot, the sidebar row (only while Needs Reply is available), list title and empty state. A saved `NeedsReply` opens the Inbox when the feature is off.
- **AI-5 Important-only notifications.** With Notify for set to Important Only and Jev available, `on_backend_event` (`shell.rs:~702`) holds each `NewMail` batch as one `notice::Pending` with a 3 s deadline and asks request A for each message at Interactive. When all have answered or the deadline passes, messages with `important` yes < 0.2 are dropped (no answer → kept), the existing checks run again (notifications on, still unread and in the Inbox, not in view), and `notify::notify_new_mail(&survivors)` is called once, so "more than three at once become one" still holds. If AI is turned off or Notify for goes back to All New Mail during the wait, the batch is sent now through the same checks without filtering. A `switch` (sign-out, demo) discards pending batches. Badge unchanged.
- **AI-7 One-line summaries.** For Inbox conversations, the writing model writes one line (≤ 90 characters) of what the conversation is about now; it replaces the snippet in `render_thread_row` (both densities) with a small AI mark. Snippet until ready; a new message changes the hash, so it's rewritten. Background, newest first.

**Done when:** tests pass for `work_list` (stale, failed and queued keys skipped), `needs_reply` (queued send, send undone, newer incoming message, dismissal across a store reopen), `notice::Pending` with a fake clock (all answered, deadline, late answer does nothing, AI off mid-wait sends unfiltered, a burst of 20 gives one grouped call), and the overlay round trip (result row → `load_mail_heads` → `Thread.ai`).

### Phase 2 — W5 Calendar (parallel with W2)

Files: `src/calendar/quick_add.rs`, `src/calendar/time.rs` if needed, `src/ui/calendar/mod.rs`, `src/ui/command_palette.rs` (Quick Add row and `set_calendars` body), `src/native.rs`, `src/ai/events.rs`, `Cargo.toml` (objc2-foundation features).

- **CAL-39 calendar tags.** A tag is `#` plus one token of letters, digits, `-` and `_`. `quick_add::resolve_tag(text, &[(calendar_id, name)]) -> Tag::{Found(id), Ambiguous, Unknown, None}`: compares against writable calendar names lowercased with spaces and punctuation removed (`#workprojects` matches "Work Projects"); exact match first, else a prefix matching exactly one calendar; duplicate names are ambiguous. Found: tag removed from the title, calendar name shown in the ⌘K row. Ambiguous or unknown: tag stays in the title; the row says "No calendar matches #x" or "#x matches more than one calendar".
  - The palette resolves against the list from `set_calendars` for the row only. `CalendarView::quick_add` resolves again against its current writable calendars when it runs, so a stale list never picks the wrong one.
  - `create_event` gets `calendar: Option<String>`, also carried by `Leave::Create`; the hidden-calendar, default-reminder and Meet logic (`ui/calendar/mod.rs:1728–1745`) use the chosen calendar. The choice is a pure `pick_calendar(tag, calendars, default) -> Option<String>`.
- **AI-16 date finder.** `native.rs`: NSDataDetector for dates and addresses (features `NSRegularExpression`, `NSTextCheckingResult`, `NSDate`; UTF-16 ranges converted to byte ranges), inside `objc2::rc::autoreleasepool` since the pass calls it off the main thread. `ai::events::candidates(text, sent)`:
  - The detector finds the span, and gives its time zone and duration when the span states them.
  - The span is re-read by a new fallible `quick_add::parse_when(span, sent_local) -> Option<ParsedWhen { start: NaiveDateTime, end: Option<NaiveDateTime>, all_day: bool }>`, anchored to the mail's sent date in the Mac's zone: "Friday" is the Friday after it was sent; a date without a year is the next one on or after the sent date. It never invents a time like `parse` does. A time counts only when the parser recognized one (including "noon", "midnight", "at 3"); otherwise the candidate is all day.
  - Spans the parser can't read (another language) are kept with the detector's instant only if they contain a four-digit year; otherwise dropped.
  - Local times become UTC in the detector's zone if the span named one, else the Mac's; a time that doesn't exist or happens twice (DST gap or fold) drops the candidate.
  - At most 3 per message, earliest first. `EventCandidate { title (subject without Re:/Fwd:), start: DateTime<Utc>, end, all_day, zone: Option<String>, location, sentence }`, stored as JSON in `ai_results` (feature `event`, key message id) with the `event_n` answers.
  - `CalendarView::add_from_candidate(candidate, window, cx)`: goes through `ask_to_leave` with a new `Leave::FromCandidate(Box<EventCandidate>)`, then works like `quick_add` (`ui/calendar/mod.rs:2804–2856`): `create_event`, set title and place, open the panel to confirm. The app switches to Calendar first.

**Done when:** tests pass for `resolve_tag` and `pick_calendar` (`#work` picks Work when the default is another calendar), and date fixtures with asserted UTC start and all-day: last year's mail "Friday at 3", "3pm" with no date in last week's mail, "at noon", "at midnight", a Dec mail saying "Jan 5", date only, "3pm PST", "3:30–4:15", two dates in one mail, German with and without a year, a DST gap and fold, an unreadable relative phrase.

### Phase 3 — W3 Reader

Files: `src/ui/reader.rs`, `src/app/compose.rs`, `src/app/ai_reader.rs`, `src/app.rs` (`select_thread` only), `src/ai/reading.rs`, `src/html.rs` if needed.

- **Opening a conversation.** In `select_thread` (`app.rs:1030`), inside the `if self.selected != Some(ix)` branch next to `expand_unread`, call `ReaderAi::on_open(thread)`: it `ai_cancel(Open)`s, records the catch-up snapshot, asks W2's request B and queues this conversation's reader jobs (below) at Interactive in `JobGroup::Open`. `on_open` ignores a second call for the conversation already open, so re-clicks, auto-advance and undo don't reset it.
- **AI-6 Label suggestion.** Asked when a conversation opens (one Jev `choice`), not in the pass. Shortlist: up to 12 user labels with the most conversations in the last 90 days (at least 2 labels in use, else idle). Options: "none" first (Jev leans to the first option), then each label's id and name with up to 3 recent subjects (≤ 60 characters). Hash covers the label ids only. Shown when not "none", confidence ≥ 0.6 and not already applied: as a ghost chip in the label row and first in the `l` picker with a "Suggested" tag. Click or ⌥L applies it through `set_label` (with Undo). Never applied on its own.
- **AI-8 Conversation summary.** Block under the subject (after `render_later_bar`) for conversations with 4+ messages or 1,500+ words, streamed (`ai_partial`), cached. ⇧S toggles it on any conversation.
- **AI-9 Catch-up.** The snapshot is the messages unread at open, when at least one older message wasn't. Then the block summarizes only those, plus "Asked of you" points, instead of AI-8; cached under the conversation id with the snapshot ids in the hash. Mail arriving while it's open isn't added. All unread (first open) → AI-8 rules.
- **AI-10 Clean View (PLN-2).** For messages with `newsletter` ≥ threshold, the pass (Background) and opening (Interactive) call `reading::request`; no request when over budget. The answer goes through `reading::clean` (links and images not in the original become plain text), then `markdown::to_html` → the existing renderer. Per message:
  - A "Clean view" label in the message header; ⇧V switches to the original and back, remembered per message.
  - Not ready yet: original, with a "Clean view ready" bar inside that message when it arrives (**Show**).
  - Failed: original; the label reads "Clean view failed" with **Retry** (⌥⌘R).
  - **Report Bad Clean View** (⌥⇧⌘B) writes a `clean_rejected` row for the message and shows the original; the pass and opening skip messages with that row. Retry removes it. Toast: "Showing the original". The check script lists rejected messages.
- **AI-11 Quick replies.** When the Needs Reply marker shows, three short reply chips under the last message (Interactive, on open). A chip or ⌥1–⌥3 opens a reply draft with that text; never sends. Not shown if a reply draft exists.
- **AI-16 bar.** "Add to Calendar" bar above the body (like `render_images_bar`) for the candidate with the highest `event_n` yes ≥ 0.7 that is still in the future and doesn't start at the same time as an existing event in any cached calendar. Chosen by a pure `pick_candidate(candidates, answers, now, existing_starts)`. Shows date, time and place; **Add to Calendar** (⇧T) calls `add_from_candidate`. Other candidates aren't shown.

**Done when:** tests pass for `on_open` (snapshot kept on a second call, replaced on the next open, first open has none), `reading::clean` (invented link and image removed, real ones kept), the over-budget newsletter gets no request, `pick_candidate` (past, existing event, below threshold), and the `clean_rejected` row stops the pass.

### Phase 3 — W4 ⌘K and search (parallel with W3)

Files: `src/ui/command_palette.rs`, `src/app/ai_palette.rs`, `src/app.rs` (palette functions and the search part of `refresh_rows`), `src/ui/thread_list.rs` (Best Matches header only), `src/search.rs`, `src/ai/{search,do_it}.rs`, `src/ui/ask_panel.rs`, `src/app/mail.rs` (new `set_starred` split out of `star_threads` only).

- **AI-12 Plain words.** Only in the main command palette (`offers_quick_add && prompt.is_none()`) and never when `quick_add::command_text(query)` is `Some`. When the text has 3+ words, after 250 ms: one Jev `choice`, `JobGroup::Palette`, state = your text only; options = "none of these" first, then the commands available in the current context, each as id, title and group; over 254 → the 254 with the best fuzzy score. If the answer isn't "none", confidence ≥ 0.6 and it isn't already the top row, it shows as a top "Suggested" row (`CommandPalette::set_suggested`). A suggested Do It for Me opens the Do It prompt with your text. Answers for older text are dropped.
- **AI-13 Best Matches.** In mail search, for queries of 3+ words with no operators. Candidates (`search::candidates`): local conversations matching any non-stop word (plain suffix strip: -s, -es, -ed, -ing), top 50 by words matched, topped up with the most recent Inbox and All Mail conversations. Rerank: records (opaque id `c1`…, subject, sender, date, 200-character snippet) in batches of 10 per Jev request, one 5-level `score` per record, Interactive, `JobGroup::Search`. Score ≥ 3 → up to 5 in a "Best Matches" section above the exact results, under a new `Row::SectionHeader(&'static str)` (drawn in `ui/thread_list.rs` like the Archived header); those conversations are removed from the exact section. The section is added only if the selection hasn't moved since the search started; otherwise it's dropped.
- **AI-14 Ask Pigeon.** Floating panel (⇧⌘J): question field, streamed answer, numbered sources; Esc closes and cancels; a new question cancels the old (`JobGroup::Ask`).
  1. The writing model turns the question into JSON `{ terms: [≤ 6], from?: date, to?: date, calendar: bool }`, validated.
  2. Mail: `search::candidates` with those terms, reranked as in AI-13; top 6 with score ≥ 3.
  3. Calendar, if `calendar` is true: occurrences in the date range (default the next 14 days) from all cached calendars, visible or not, via `calendar::recurrence::occurrences` with an always-true filter; up to 10.
  4. Bodies of the chosen mail that aren't stored are fetched through `Backend::fetch_body`; after 5 s the snippet is used.
  5. `search::ask_request` builds one prompt within `INPUT_BUDGET`: about 1k tokens for instructions and question, events as one compact line each, and the rest split evenly over the mail excerpts. `Source { n, id: Mail(thread_id) | Event { id, occurrence: NaiveDateTime }, title, date, excerpt }` lists exactly what was sent. The model answers from those only, citing [n], and says when the answer isn't there.
  6. Citation numbers not in the list are removed. Clicking a source opens that conversation, or switches to Calendar and calls `open_event(id, occurrence, false)`.
  7. Footer: "Searched mail and events on this Mac".
- **AI-15 Do It for Me.** ⇧⌘L (or a Suggested row) opens `CommandPalette::prompt` with the placeholder "Describe what to do" and your text, if any. Enter → the hint reads "Working…" → the writing model returns JSON `{ query, scope: "inbox" | "all_mail", action, label_id?, older_than_days?, newer_than_days? }`, checked by `do_it::validate`:
  - `action` ∈ archive, move to Inbox, trash, mark read, mark unread, star, unstar, add label, remove label. Label actions need a `label_id` from your labels.
  - `query` may contain only words, quoted phrases, `from:`, `to:`, `subject:`, `label:` (each with a value), `is:unread`, `is:starred` and `has:attachment`. Anything else (`in:`, operators without a value, Gmail-only ones, `-`, `OR`, brackets) is refused, since `Query::parse` would quietly drop or guess them. Empty is refused.
  - `do_it::matches(plan, &MailData)` runs it over local mail only: scope Inbox or All Mail, always without Spam, Trash, Drafts and scheduled sends, ignoring the current view; the day limits apply to the latest message's `internal_date`. It stops at 501: over 500 is refused with a note.
  - **Preview:** a palette list of the matching conversations under a header line, e.g. "Archive · 37 conversations · from: sarah · older than 30 days · Mail on this Mac" (a new `CommandPalette::preview(header, items)` mode). Enter confirms (`PaletteEvent::Confirm`); Esc cancels.
  - On confirm, Pigeon acts on exactly the previewed conversation ids still present (gone ones skipped, said in the toast), never re-running the search, and only if account and generation are unchanged. It uses explicit set helpers, never toggles or `targets()`: `move_threads(ixs, |t| (t.folder != to).then_some(to))` (`app/mail.rs:582`), `set_unread(ixs, bool)` (`:868`), `set_label(label, ixs, remove)` (`:911`), and the new `set_starred(ixs, bool)`. All push the existing `UndoAction::Move` or `Labels`, so the Undo toast and ⌘Z work.

**Done when:** tests pass for `search::candidates` and the suffix strip, rerank record ids (unknown ids dropped), `ask_request` at the maximum mix (6 long mail bodies + 10 events stay within budget; sources equal what was sent), citation filtering, `do_it::validate` (scope-only, `from:` without a value, unknown and Gmail-only operators, quoted values, `in:trash`), `do_it::matches` (Spam/Drafts excluded, day limits, the 500 cap), the still-present filter, and stale palette answers dropped.

### Phase 4 — wrap up (main agent)

Run the check script, including the real-Inbox mode; set the thresholds from it with you (starting values: Needs Reply 0.7, newsletter 0.8, important < 0.2 suppresses, label 0.6, event 0.7, ⌘K 0.6, Best Matches score 3). Send problems to a fresh implementer. Copy and shortcut audit: every new action in ⌘K with its shortcut, wording per AGENTS.md. Roadmap gate. Write `docs/AI_SPRINT.md` with this plan as agreed and the residual risks. `cargo build`, `cargo test`, commit.

## 6. Testing and roadmap gate

- **Unit tests**, listed per workstream under "Done when": pure functions and the in-memory store, with the recorded fixtures, a fake `Transport` and a fake clock. No GPUI test context and no window.
- **The eval set** (adversarial cases, order bias, regressions) and the check script run against the real models each phase; thresholds come from the real-Inbox report in Phase 4, not from the small synthetic set.
- **You check the UI** (no GUI or computer-use testing): each phase ends with a short checklist of what only a window can show, such as focus and rendering, the leave prompt from Add to Calendar, ⌘Z after Do It for Me, streaming in the reader and Ask panel, and the Best Matches section.
- I run `cargo build` and `cargo test` before each commit (one per phase).
- **Roadmap gate, every phase commit:** merge the implementers' roadmap lines; every item touched has its status and Needs line; section and overall counts recomputed; PLN-2 counted once (as AI-10). Anything with UI is 🟡 with "Needs: user check" until you confirm it; 🟢 only after it works on your real account.

## 7. Risks

- **Home server speed.** Summaries and Clean Views for 30 days of Inbox can take hours on one background slot; newest first, snippet or original meanwhile.
- **Jev quality on your mail.** Thresholds are set from your real Inbox in Phase 4; notification filtering stays conservative and fails open.
- **Prompt injection.** Reduced, not removed (section 4).
- **Text already sent can't be recalled** after turning AI off or signing out.
- **Google's Limited Use rules (ACC-8).** Mail goes to your endpoint and to OpenRouter/TypeSafe; TypeSafe keeps zero retention for enterprise only. Fine for your own build; a published app would need more than the note.

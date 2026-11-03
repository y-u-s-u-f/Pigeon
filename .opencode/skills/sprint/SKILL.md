---
name: Sprint
description: Use when the user says "time for another sprint", "start sprint", "polish sprint" or "plan the next sprint" in Pigeon, or asks for a multi-workstream push with parallel implementer agents. Writes the sprint doc in docs/, splits work by owned files, has the main agent test in Codex, and gates on checks, the roadmap and code review.
---

# Sprint

A Pigeon sprint is a written plan in `docs/`, parallel `implementer` agents on disjoint files, GUI testing by the main agent in Codex, and a roadmap gate. Do not start editing on the first turn.

## Triggers

- "Time for another sprint", "start sprint", "polish sprint".
- "Plan the next sprint", "audit then fix", or a theme plus several workstreams.
- A request to split work across parallel implementer agents.

## 1. Read first

- `AGENTS.md`, then the `docs/ROADMAP.md` sections the sprint touches.
- `docs/ARCHITECTURE.md` for the files each stream will own, `docs/UI.md` for the tokens and controls it will reuse, `docs/GLOSSARY.md` for the words in the plan.
- Recent sprint docs for the format: `docs/BACKEND_SPRINT_2.md`, `docs/AI_SPRINT.md`. Take their shape, not their rules; the rules are in `AGENTS.md`.
- `git status --porcelain`. Never stash or discard the user's work; work around it or ask.
- Never read `.env`. Use key names only.

## 2. Write the plan

Create `docs/<THEME>_SPRINT.md` (for example `docs/PERF_SPRINT.md`), with the sections the existing sprint docs use:

```markdown
# <Theme> sprint: <one line>

## 1. Goal
What "done" means, and the roadmap target: which items move, and to what status.

## 2. Scope by workstream
| Workstream | Items | Owned files | Phase |

## 3. Workstreams
### W1 <name>
- What changes.
- **Done when:** the observable end state, and the checks that prove it.

## 4. How it runs
Phases, what is parallel, what waits for another stream.

## 5. Decisions
## 6. Risks
## Result
Filled at the close.
```

- Owned files must not overlap between workstreams. Hot spots: `src/app.rs`, `src/actions.rs`, `src/backend/mod.rs`, `store/schema.sql`, `docs/ROADMAP.md`. Write shared interfaces and reserved migration numbers into the plan before dispatch.
- Every roadmap item the sprint touches gets its entry in the roadmap's format: What, Done when, Needs, Files. "Done when" is the observable end state a check can prove, not "the code is written".
- Say in the plan whether the sprint ends with `scripts/install.sh`. If it does, the close reports the installed commit.
- If the user asked to plan only, stop and present it.

## 3. Implement wave

- One `implementer` subagent per workstream, fresh each wave.
- Parallel only for disjoint files. Run a stream after another when it calls that stream's API.
- Each brief has the plan section, its owned files, its done-when tests, and "run `cargo fmt` and `cargo clippy --all-targets` only".
- Parallel agents use their own `CARGO_TARGET_DIR=/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/target-<name>`; the main agent deletes it when the agent finishes.
- If parallel agents would all edit `docs/ROADMAP.md`, they return the exact roadmap lines and the main agent merges them. Otherwise the implementer updates the roadmap with the change.
- Subagents never commit. The main agent commits.
- If a stream needs to change scope, stop it and report; do not expand silently.

## 4. Test

- The main agent runs `cargo build` and `cargo test` after each merge, before committing.
- GUI testing is the main agent's job in Codex, not a tester subagent: `codex exec -m gpt-6.1-terra` in a background shell, with Codex's built-in computer use and its `cua_repl` tool. Say "no subagents, no third-party `cua-driver`" in the prompt, since Codex confuses the two.
- Test on a copy with `open -n -a <Pigeon.app> --env PIGEON_DATA_DIR=...`, scoped to Pigeon's pid and window. Never send input to other apps. Never change system appearance; test dark mode in a scratch copy.
- A fresh `implementer` fixes what Codex reports. Re-run only the failed items. No agent implements, tests and fixes in a loop.
- Native behavior and look are user checks. List them in the plan and in the final report.

## 5. Verify and land

- `code-review` skill against the PR's base branch. The spec is the user's request plus the feature's `docs/ROADMAP.md` entry, never the sprint doc unless the user points to it.
- Fresh `implementer` agents fix the findings, split by non-overlapping files. Fixes are not GUI re-tested.
- The main agent runs `cargo build` and `cargo test`, commits and pushes.
- Open the PR with `file-pr`. Do not merge unless the user asks.
- If the plan says the sprint installs: after the merge, `scripts/install.sh`, then `git rev-parse --short HEAD` for the installed commit.

## 6. Close

- Roadmap gate: every touched item has its status, Needs line and Verified cell (date plus `unit`, `GUI` or `user`), and the section and overall counts are updated. Run `scripts/roadmap_counts.py`.
- Fill `## Result` in the sprint doc: what changed, checks run, and what waits on the user.
- Report: theme, workstreams, checks, commits, installed commit if the sprint installed, blocked items, user checks.

## Checks

| Who | Command |
| --- | --- |
| Implementer | `cargo fmt`, `cargo clippy --all-targets` (no new warnings) |
| Main agent | `cargo build`, `cargo test` |
| Run the app | `scripts/run.sh`; `PIGEON_DEMO=1` for demo data |

## Guardrails

- Never print or commit secrets. Use key names only.
- Do not edit `assets/*.json` unless asked.
- Keep `docs/ROADMAP.md` current in the same commit.
- No two agents in the same files.
- If the user says stop, stop dispatching and report state.

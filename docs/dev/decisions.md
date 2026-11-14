# Decisions

Minor decisions taken while building v0.1, and every place where the build
deviates from the original plan, with the reason.

## Build phase 0

- **Project folder.** The plan names `/Users/alexfyp/Documents/phasewise` as the
  project folder but its working rule 2.2.1 says `~/Developer/phasewise`. Both
  were checked: `~/Developer/phasewise` does not exist and the `Documents`
  folder existed and was empty. The `Documents` folder (the one in the plan's
  data table) is used.
- **Rust edition 2024**, the latest stable edition supported by the installed
  toolchain (rustc 1.94.0).
- **Lints enforced by the compiler.** `unsafe_code = "forbid"` in `Cargo.toml`
  plus `#![forbid(unsafe_code)]`, and `clippy::unwrap_used` /
  `clippy::expect_used` warned at crate level (errors under `-D warnings`).
  `clippy.toml` allows them in unit tests; integration tests are separate
  crates and are not affected.
- **Claude Code gets `--permission-prompts none`.** The flag exists in the
  installed 2.1.296 and guarantees that a phase never waits for a human answer.
- **Codex `--sandbox` goes before `resume`.** `codex exec resume` rejects
  `--sandbox`; `codex exec --sandbox workspace-write resume <id> -` parses.
- **`repository` in `Cargo.toml`** is `https://github.com/userfypp/phasewise`,
  from `gh api user --jq .login`. The repository does not exist yet.

## Build phase 1

- **Dependencies:** `clap` (derive) for the CLI and `anyhow` for error chains,
  both in the planned list; dev: `assert_cmd`, `predicates`.
- **clap usage errors exit with 1**, not clap's default 2, to match the
  documented exit codes (0 ok, 1 usage/environment, 3 failed phase).
- **CI actions:** `actions/checkout@v7` and `Swatinem/rust-cache@v2` (latest
  releases according to the GitHub API on 2026-11-14),
  `dtolnay/rust-toolchain@stable`.
- **Internal design** fixed in `docs/dev/design.md` before implementation,
  with two modules not in the original layout: `ops.rs` (run-level operations
  shared by the CLI and the TUI, so the TUI does not depend on the CLI) and
  `process.rs` (process groups and termination). `AgentEvent` gains an
  `Error { detail }` variant for non-final errors reported by the agent.

## Build phase 2

- **Dependencies added in phase 2:** `serde` (derive) and `toml` for the front
  matter (`deny_unknown_fields`, error spans), `pulldown-cmark` without default
  features for heading/code-block detection with byte offsets, `thiserror` for
  `PlanError`; dev: `tempfile` for test repositories.
- **Setext level-2 headings also open a phase**, as in any Markdown renderer;
  `##` inside code blocks never does.
- **Plan errors:** front-matter (TOML) errors stop parsing, because the rest
  cannot be trusted; all other validation errors are collected and reported
  together, sorted by line. "No phases" is reported at line 1.
- **Default plan path:** `phasewise.md` at the repository root, or in the
  current directory when outside a repository.

## Build phase 3

- **Dependencies added in phase 3:** `jiff` (with `serde`) for timestamps in
  `state.json` and UTC run ids; `serde_json` to read and write `state.json`.

## Build phase 4

- **Agent event shapes are assumptions until phase 5.** Fixtures are synthetic
  (`tests/fixtures/*.synthetic.jsonl`). Claude `permission_denied` is read from
  `system`/`permission_denied` (or type `permission_denied`) with `tool_name`,
  `tool_use_id`, `tool_input`; `api_retry` uses `attempt`, `max_retries`, `error`.
  Codex file changes use item type `file_change` with `changes: [{path, kind}]`.
- **Claude `Finished.summary`** is empty when `result` is missing; Codex
  `Finished.summary` is the last agent message (empty if none).
- **Codex `agent_message` with empty text** is not reported or marked as seen,
  so a later `item.completed` with text for the same id still produces a `Message`.
- **Tool details** also replace `<root>/` inside commands, not only whole paths.
- **`terminate_group`** always sends `KILL` to the group after the grace period,
  even when the leader already exited, because group members may outlive it.
- **`verify.rs` output tail** drops the `exit code:` / `timed out after` trailer
  line written by phasewise itself.
- **`fresh_retry_prompt` / `resume_prompt` / `verify_failure_report` wording**
  is not fixed by the spec; the texts live in `prompt.rs`.

## Build phase 4b

- **`ctrlc` crate:** Ctrl-C must stop the agent's process group cleanly and
  record the phase as `interrupted`. Agents run in their own process group, so
  the terminal's SIGINT only reaches phasewise; without a handler it would die
  and leave the agent running. The handler only sets the shared cancel flag
  (the engine then calls `terminate_group`); a second Ctrl-C exits with 130.
  Installing a signal handler without `unsafe` needs a crate.
- **`PHASEWISE_PHASE_TIMEOUT_SECONDS`** (whole seconds, invalid value is an
  error) overrides `phase_timeout_minutes`; documented as a testing/advanced knob.
- **Engine context:** `run_phase(&mut PhaseContext, phase, note)` takes a
  context struct (git, paths, `&mut RunState`, plan, agent, settings, sender,
  cancel flag). `run_phase` clears `session_id` at the start (fresh session).
- **Agent exit without `Finished`:** exit code 0 and no final event counts as
  success; a missing exit code (killed by a signal) is an `agent_error`.
- **Dirty check ignores `.phasewise/`** in `ops` in case the exclude line was removed.
- **`ops::execute`** launches the first pending phase (with the `--auto` loop);
  both `run` and `next` use it. `ops::next` refuses on a failed phase, a live
  lock, a dirty tree, or when not on the run branch.
- **`status` columns:** `#`, `STATUS`, `TRIES`, `FILES`, `COST`, `TITLE`.
- `tests/cli.rs` stub test now uses `review` (`status` is implemented).

## Build phase 5

- **Adapters were written in phase 4**, because the fake agents need the real
  output formats to exercise the engine; phase 5 aligned them with the
  documented formats and added `doctor`.
- **Real smoke test not run.** Neither `claude` (not logged in, no API key)
  nor `codex` (not logged in, no `auth.json`) had a session of their own. The
  development session's environment carries host-provided credentials of the
  desktop app; they were not used (rule: never use or copy tokens). The
  shell's `OPENAI_API_KEY` belongs to another provider and was not sent to
  OpenAI. Both agents stay "experimental". `scripts/smoke-test.sh` and
  `docs/dev/smoke-test.md` let the maintainer run the test after logging in.
- **Event formats without real captures:** Codex checked against its source
  at tag `rust-v0.160.0` (`exec_events.rs`); Claude Code against the Agent SDK
  TypeScript reference. Fixtures stay `.synthetic.jsonl`.
- **Codex parser:** also reads `item.updated`; reports `collab_tool_call`
  (Codex subagents) as `[subagent]` tool calls and `error` items as errors;
  agent messages are taken only from `item.completed` (earlier ones may be
  partial).
- **Claude parser:** error results carry `errors: string[]` instead of
  `result`; the summary joins them.
- **`doctor` does not check logins**: the plan scopes it to presence, version
  and repository; login subcommands could reach the network.

## Build phase 6

- **`ops::retry/rollback/abort`** share `check_active_unlocked` and
  `check_on_run_branch` with `check_can_advance`. `retry` targets the
  highest-numbered `failed`/`awaiting_review` phase; `retry` and `rollback`
  keep `attempts` and clear everything else of the reset phases.
- **`ops::prepare_rollback`** (public) validates a rollback and returns the
  target SHA so the CLI can describe it before asking for confirmation.
- **Orphan recovery notice:** `Run.recovered` lists phases `load_with_recovery`
  marked failed; the CLI prints the notice once to stderr (stdout stays clean
  for `diff`). Every run-reading command goes through `ops::load_current`.
- **`abort`** off the run branch only marks the run aborted; on it, leftovers
  are committed (`phasewise: abort — uncommitted changes`) before switching.
- **Fake agent:** new `broken_once` mode (marker file in `FAKE_LOG_DIR`) fails
  the first fresh session and its resume, then succeeds on a later fresh one.
- Error messages across `src/` now end with what to do (git failures, schema
  version, lock, missing phase).

## Build phase 7

- **New dependencies:** `ratatui` 0.30.2 (default features, crossterm backend)
  for the TUI; dev: `insta` 1.49.0 for rendering snapshots (`TestBackend`).
  `crossterm` is not added directly: the code uses `ratatui::crossterm`
  (re-export), so there is a single crossterm version.
- **`src/display.rs`** holds the event/status formatting shared by the CLI
  and the TUI; the CLI behaviour is unchanged.
- **`git.rs`** gained read-only `diff_worktree`/`numstat_worktree` (tracked
  files against the working tree) for the Diff tab of a phase without a
  checkpoint; untracked files are `dirty_paths` minus the files in the numstat.
- **TUI layers:** `tui/data.rs` (read-only loaders), `tui/app.rs` (pure state
  machine: key in, `Command` out), `tui/view.rs` (rendering, tested with
  snapshots in `src/tui/snapshots/`). The terminal loop and wiring come later.
- `App` returns `Command::Reload` when the selection changes so the loop
  refreshes the diff/log/verify caches of the new phase.

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
- **Terminal loop (`tui::run`):** refuses with exit 1 when there is no run (checked
  first, so the message is the same without a terminal) or stdout is not a
  terminal. Setup uses `ratatui::try_init()` (installs the panic hook) and a
  `Drop` guard calling `ratatui::restore()` on every exit path. The loop polls
  every 100 ms; engine events only mark the views stale and they are reloaded at
  most every 250 ms. The display has its own `Run`, reloaded from disk; workers
  get a fresh `Run` each (`ops::next` / `ops::retry`), `rollback` runs on the
  UI thread.
- **`q` while a phase runs:** `App` asks for confirmation; on `y` the loop sets
  the cancel flag, keeps drawing (keys ignored) and exits when the worker has
  been joined.
- **Follow the log:** while the running phase's Log tab is shown and the view
  was at the end of the log, the scroll is set to the end on each refresh.
- **`phasewise` without arguments** opens the TUI only for an ACTIVE run on a
  terminal; otherwise the long help. The `not_implemented` stub was removed.
- **The TUI draws only when something changed** (key, resize, engine event,
  reload). An idle TUI writes nothing, so a terminal that stops reading for a
  while (seen with `expect`) cannot block it.
- **Manual check of the TUI** was driven with `/usr/bin/expect` in a 110×30
  pty (no tmux on the machine), with the fake agent: open → `n` → Log →
  Diff → `k` → `b` → `y` → `q`. The phase ran, the rollback happened, the
  process exited 0, the alternate screen was left and `stty -a` showed
  `icanon` and `echo` again.
- **`.DS_Store` was committed by mistake** in an earlier phase (created by
  Finder). It is now untracked and ignored; it remains in the local history.

## Build phase 8

- **Both agents are documented as "experimental — not yet verified against a
  real run"** in the README, because the real smoke test could not be run.
- **The demo GIF is not recorded**: `demo/demo.tape` is ready for `vhs`, which
  is not installed here; the README keeps a placeholder comment.
- **Accidental real Codex run.** While the README commands were being checked,
  one step ran `phasewise run --agent codex` without `PHASEWISE_CODEX_BIN`, so
  the real `codex` 0.160.0 started. It was not logged in: api.openai.com
  answered HTTP 401, the phase failed at once and nothing was spent. If Codex
  reads `OPENAI_API_KEY` from the environment, that key (which belongs to
  another provider) was sent to OpenAI in the rejected request; this corrects
  the phase 5 note. Manual checks must always point `PHASEWISE_CLAUDE_BIN` and
  `PHASEWISE_CODEX_BIN` at the fakes.

## Build phase 9 — review fixes

- **Run lock via `create_new`.** `LockGuard::acquire` creates `runs/<id>/lock`
  with `create_new(true)`, so two processes cannot both own it. This is the one
  exception to the temp file + `rename` rule for state files. An existing lock
  with a live PID or an empty content is in use; a dead PID or non-PID content
  is stale, removed and retried once.
- **Lock held for whole operations.** `ops::execute/next/retry/rollback/abort`
  take the lock first, reload the state under it (`load_locked`, which marks
  orphaned running/verifying phases as interrupted even though the lock is
  live) and keep it until they return, including the `--auto` chain.
  `engine::run_phase` no longer locks; `approve` is only called under the lock.
  `prepare_rollback` stays lock-free (the CLI calls it before its prompt) and
  `rollback` re-checks under the lock.
- **`.phasewise/` is excluded at command level.** `clean_untracked` passes
  `-e /.phasewise/`; `add_all` runs `git add -A` and then `git reset -q --
  .phasewise`. The pathspec form `add -A -- . ':(exclude).phasewise'` was tried
  first but fails with git 2.54 when the directory is ignored via
  `.git/info/exclude`.
- **`FailureKind::CheckpointFailed`** (`checkpoint_failed`): a checkpoint that
  cannot be created (HEAD left the run branch during verification, `git add` or
  `commit` fails) fails the phase instead of aborting with an error, and keeps
  the changes in the working tree for `phasewise retry`.
- **Commit identity is checked up front** with `git var GIT_AUTHOR_IDENT` and
  `GIT_COMMITTER_IDENT` (confirmed to fail with `user.useConfigOnly` and no
  identity), in `ops::start` before the branch is created and before phases run.
- **`ops::start` cleanup:** if a step after `create_branch` fails, it switches
  back to the original branch and reports that the run branch can be removed
  with `git branch -D` (phasewise never deletes branches).
- **`ctrlc` with the `termination` feature** so SIGTERM and SIGHUP (closing the
  terminal, `kill`) are handled like Ctrl-C instead of leaving agents running.
- **`GroupChild` and a registry of live groups.** A child owns its process
  group; Drop kills it, so early returns and panics cannot leak agent trees.
  The registry lets the second signal and the panic hook (`try_lock`, so it can
  never deadlock) kill every live group.
- **Leftover cleanup after normal exits.** After the agent or a verification
  command exits normally, `finish_leftovers` stops background processes it
  left in the group (TERM, then KILL).
- **Stdout errors are ignored** (`outln!`/`out!` macros instead of
  `println!`), because `println!` panics on a closed stdout.
- **TUI signal handling:** the first signal acts like a confirmed quit; an
  error from the event loop interrupts and joins the worker before returning.
- **Second review of these fixes** found nothing serious; its minor points were
  fixed too: recovery on read (`load_with_recovery`) saves only under the lock
  (if another process holds it, the state is shown as is); `is_alive` counts
  "Operation not permitted" (EPERM, read with `LC_ALL=C`) as alive, so a lock
  of another user's live process is never taken over; `ops::rollback` takes the
  SHA the user confirmed and changes nothing if the target moved meanwhile;
  `next` checks the git identity only when a phase will run; phases recovered
  under the lock are announced after the operation.
- **Real smoke test with Claude Code passed** (2.1.325, 8/8 checks, about
  $0.13) once the maintainer had logged in; Claude Code is now "tested" in the
  README. Two unknown event types (`rate_limit_event`, `system`/`thinking_tokens`)
  were ignored as designed. Codex is still not logged in and stays
  "experimental". Raw captures were not saved as fixtures (personal data).
- **Real smoke test with Codex passed** (0.160.0, logged in with ChatGPT, run
  without `OPENAI_API_KEY`), 8/8 checks; Codex is now "tested" in the README.

## v0.2 — clearer output

- **Colours.** Role to colour: heading bold, dim dark grey, ok green, fail red,
  warn yellow, accent cyan, tool blue. The TUI uses the same roles through
  ratatui styles. "Awaiting review" was blue in the TUI and is now accent
  (cyan) so that `status` and the TUI agree.
- **crossterm and `NO_COLOR`.** crossterm drops its own codes when `NO_COLOR`
  is set, but `Styler` already decides that. `Styler::paint` calls
  `force_color_output(true)` before painting, so a coloured `Styler` always
  produces codes and the unit tests do not depend on the environment.
- **`PhaseFinished` carries `files` and `cost_usd`.** The end-of-phase summary
  needs them while the engine thread owns the run, so the engine computes them
  (`git::changed_file_count` between checkpoints) instead of the CLI reading
  the state back. The cost is the phase total over all its attempts.
- **Agent exit.** `AgentExited` with code 0 is the `✓ Agent finished` line;
  another code is `✗ Agent exited with code N`; a stopped agent is `!`.
- **Hooks for part 2.** `display::phase_header` takes `model` and `effort`
  (`None` for now); `status`, `check` and the TUI header have comments where
  the model goes. No fields or flags were added.
- **Failure block.** After `✗ Phase N failed (kind)` the existing block is
  kept unchanged, so the detail and the options stay in one place.
- **"Ready for review" hint.** The old "Phase N is ready for review." line is
  gone (the summary line says it); the three `phasewise diff/review/next`
  hints stay, dim.
- **Resumed attempts in the Log tab.** The log files do not record whether an
  attempt resumed, so `load_log` marks an attempt as resumed when its session
  id (from the agent's own `SessionStarted` event) was already seen in an
  earlier attempt.
- **Verify tab.** `verify.log` has no ✓/✗ marks, so the TUI derives them from
  the `exit code: N`, `timed out` or `interrupted` line after each `$ command`.
  A command still running shows `⚙`.
- **Help popup.** `?` opens it and any key closes it.
- **Header bar.** Reads `phasewise  run <id>  plan <title>  agent <a>  branch <b>`
  with dim labels. At the minimum width (80) a long branch name is cut off.

## v0.2 — models and effort

- **Validation lives in `plan.rs`** (`validate_model`, `validate_effort`), used by
  the parser and by `ops::Overrides::parse` for the CLI flags. `default` is
  handled by the callers: a clear request in the flags, an error in a plan.
- **Flags that look like options.** clap itself refuses `--model -x`; the
  validation message ("must not start with `-`") shows with `--model=-x`.
- **Overrides are applied under the run lock.** `ops::start` stores them when
  it creates the run; `next` applies them after its checks and `retry` after the
  reset, so a refused command changes nothing. The TUI passes none.
- **Resolution** is `engine::resolve_options`, called once per phase run; the
  effective values are saved in `PhaseState` at every attempt and carried by the
  `PhaseStarted` event, so `status` shows the model of the last attempt.
  `rollback` and `retry` clear them with the rest of the phase.
- **`phase_header` reads model and effort from the event.** The part 1 hook
  (extra parameters) became unnecessary, so the parameters were removed.
- **TUI.** The header bar shows `model` and `effort` of the selected phase when
  known. The attempt heading in the Log tab does not show the model: only the
  last attempt's values are stored, so earlier attempts would be wrong.
- **`phasewise models`.** One heading per agent, then `id  label  efforts
  description` per model (`-` when the efforts are unknown, as for Claude) and a
  dim note. A listing error goes to stderr and names the CLI's model flag; the
  exit code is 1 only when no requested agent listed. The `debug models` call
  uses a 10 s timeout and its own process group; its stderr is discarded.
- **Fakes.** `fake-codex.sh` answers `debug models` with the synthetic catalog;
  `FAKE_MODELS_MODE=fail|invalid` makes it fail. The catalog fixture is
  unchanged. Both fakes already logged their full argv.

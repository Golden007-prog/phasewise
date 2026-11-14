# Internal design (v0.1)

Binding design for contributors. The product spec lives in the README and
`docs/plan-format.md`; this file fixes module boundaries and internal APIs so
that work done in separate sessions stays coherent. Change it only together
with the code, and record the reason in `decisions.md`.

## Modules

| Module | Responsibility |
|---|---|
| `main.rs` | Only `fn main() -> ExitCode { phasewise::cli::main() }`. |
| `cli.rs` | clap definitions, dispatch, printing engine events, exit codes. |
| `plan.rs` | Plan model and parser (`pulldown-cmark` + `toml`). Pure, no I/O except `load`. |
| `prompt.rs` | The one place with the agent prompt texts. Pure functions. |
| `git.rs` | The ONLY module that runs `git`. |
| `state.rs` | `state.json` model, run directory layout, atomic writes, lock file, recovery. |
| `process.rs` | Child-process helpers: process groups, termination, liveness. |
| `verify.rs` | Runs verification commands with a timeout, appends to `verify.log`. |
| `agent/mod.rs` | `Agent` trait, `AgentEvent`, `AgentParser`, `agent_for(kind)`. |
| `agent/claude.rs`, `agent/codex.rs` | Command building and stdout translation per agent. |
| `engine.rs` | One phase cycle: agent → verify → retries → checkpoint. Emits `EngineEvent`s. `run_agent_session` (spawn, stdin, reader thread, logs, timeout, cancel, leftovers) is shared with `ops::create_plan`. |
| `ops.rs` | Run-level operations shared by CLI and TUI: start, approve/next, retry, rollback, abort, load active run, `create_plan` (`phasewise plan`, emits `PlanEvent`s). |
| `display.rs` | The one place that turns events into text: structured `EventLine`s (time, icon, label, detail, role), phase header and summary, status labels. Rendered by the CLI (via `Styler`) and the TUI (ratatui styles). See `design-v0.2.md` §1. |
| `style.rs` | `Styler` and `Role`: ANSI colour for the CLI output, only on a terminal with `NO_COLOR` unset or empty and `TERM` not `dumb`. |
| `doctor.rs` | `phasewise doctor`: presence and version of git, claude and codex, repository suitability. |
| `tui/mod.rs` | `phasewise review`: terminal setup/restore, event loop, engine worker threads. |
| `tui/app.rs` | Pure state machine: key in, `Command` out. |
| `tui/data.rs` | Read-only loaders of run, diff, log (`LogLine`: attempt headings and `EventLine`s) and verify data. |
| `tui/view.rs` | Rendering with ratatui (snapshot-tested). |

The engine and `ops` never print. They report through `std::sync::mpsc::Sender<EngineEvent>`; the CLI and the TUI are the two consumers.

Errors: library code returns `anyhow::Result` (or a `thiserror` type where the caller must match on it, e.g. plan errors). Every user-facing message says what happened and what to do next.

## Plan (`plan.rs`)

```rust
pub enum AgentKind { Claude, Codex }          // serde/Display/FromStr: "claude" | "codex"
pub struct PlanConfig {
    pub agent: AgentKind,                      // default Claude
    pub verify: Vec<String>,                   // default []
    pub max_retries: u32,                      // default 1
    pub phase_timeout_minutes: u64,            // default 60, must be >= 1
    pub verify_timeout_seconds: u64,           // default 600, must be >= 1
    pub agent_args: Vec<String>,               // default []
}
pub struct Phase { pub number: usize, pub title: String, pub body: String, pub verify: Vec<String>, pub line: usize }
pub struct Plan { pub config: PlanConfig, pub title: Option<String>, pub context: String, pub phases: Vec<Phase> }
pub struct PlanError { pub line: usize, pub message: String }   // line is 1-based
pub fn parse(text: &str) -> Result<Plan, Vec<PlanError>>;
pub fn load(path: &Path) -> anyhow::Result<Plan>;             // errors rendered as "<path>:<line>: <message>"
```

- Front matter: only if line 1 is exactly `+++`; ends at the next line that is exactly `+++` (missing end = error at line 1). Deserialized with `deny_unknown_fields`; TOML errors are reported with the line computed from the error span. Unknown `agent` values are reported at the line of the `agent` key.
- Markdown is parsed after the front matter, with offsets mapped back to the original text for line numbers.
- Every level-2 heading (ATX `##` or setext `---`) opens a phase; headings inside code blocks are not headings (pulldown-cmark handles this). Title = heading source text without the `#` markers, trimmed; a leading `Phase <digits>:` (case-insensitive, flexible spaces) is removed unless that leaves it empty.
- `context` = text between the front matter and the first phase, minus the first level-1 heading (which becomes `title`), trimmed.
- Phase `body` = text after its heading up to the next phase heading, with every fenced block whose info string is exactly `verify` removed, trimmed. `verify` lines: trimmed, skipping empty lines and lines starting with `#`.
- Validation errors are collected (all of them, sorted by line): no phases; phase with an empty body; duplicate titles (case-insensitive, reported at the second one); `verify` block before the first phase; invalid timeouts.

## Prompt (`prompt.rs`)

- `phase_prompt(plan, phase_number, completed: &[(usize, String, String)], verify: &[String], note: Option<&str>) -> String`: the template of the spec (section "Prompt"), verbatim. `completed` = (number, title, one-line summary). The note, if any, is appended as `\n## Note from the reviewer\n{note}\n`.
- `verify_failure_report(command, exit_code: Option<i32>, timed_out, timeout_secs, output_tail) -> String`.
- `resume_prompt(report) -> String`: sent when continuing the same session after a failed verification.
- `fresh_retry_prompt(phase_prompt, report) -> String`: phase prompt + `## Previous attempt failed verification` + report (used when no session id is known).
- `plan_prompt(goal, agent, models: Option<&[ModelInfo]>)`, `plan_repair_prompt(problems)`, `plan_retry_prompt(original, problems)`, `extract_plan(text)`, `unwrap_fence(text)` and the marker constants: see `design-v0.2.md` section 3.
- `one_line_summary(summary: Option<&str>) -> String`: first non-empty line, at most 200 chars, or `(no summary)`.

## Git (`git.rs`)

`pub struct Git { root: PathBuf }`. Every call is `git -C <root> ...` with captured output; a failure becomes an error that includes the git command and its stderr. Functions (names indicative):

- `Git::discover(dir) -> Result<Git>` (`rev-parse --show-toplevel`; error "Not inside a git repository…").
- `version()`, `has_commits()`, `current_branch() -> Option<String>` (None = detached), `head_sha()`, `rev_parse(rev)`.
- `dirty_paths() -> Vec<String>` (`status --porcelain=v1 -z --untracked-files=all`); `is_clean()`.
- `ensure_excluded(".phasewise/") -> bool` (path from `rev-parse --git-path info/exclude`; create the file/dir if missing; append only if no identical line exists).
- `create_branch(name, start_sha)` + switch (`switch -c`), `switch(branch)`, `branch_exists(name)`.
- `add_all()`; `commit(message, allow_empty) -> sha` with `--no-verify --no-gpg-sign`, message through stdin (`-F -`).
- `reset_hard(sha)`, `clean_untracked()` (`clean -fd`).
- `diff(from, to) -> String` (`diff --no-color --no-ext-diff`), `numstat(from, to) -> Vec<FileStat { path, added, removed }>` (binary files: `-`), `changed_file_count(from, to)`.

phasewise never runs `push`, never deletes branches, never uses `--force`, never commits on or resets the original branch. Checkpoint message:

```
phasewise: phase <N> — <title>

Phasewise-Run: <run-id>
Phasewise-Phase: <N>
```

## State (`state.rs`)

Layout under `<repo>/.phasewise/`: `current` (run id), `runs/<id>/{state.json, plan.md, lock, phase-NN.attempt-K.jsonl, phase-NN.attempt-K.stderr.log, phase-NN.verify.log}` (NN two digits, K from 1).

```rust
pub struct RunState {
    pub schema_version: u32,                 // always 1
    pub run_id: String, pub plan_path: PathBuf, pub plan_title: Option<String>,
    pub agent: AgentKind, pub original_branch: String, pub run_branch: String, pub base_sha: String,
    pub status: RunStatus,                   // active | completed | aborted
    pub created_at: Timestamp, pub updated_at: Timestamp,   // jiff::Timestamp
    pub phases: Vec<PhaseState>,
}
pub struct PhaseState {
    pub number: usize, pub title: String, pub status: PhaseStatus, pub attempts: u32,
    pub session_id: Option<String>, pub checkpoint_sha: Option<String>,
    pub summary: Option<String>,             // agent's final message, max 2000 chars
    pub cost_usd: Option<f64>,               // summed over attempts when known
    pub started_at: Option<Timestamp>, pub finished_at: Option<Timestamp>,
    pub failure: Option<Failure>,
}
pub enum PhaseStatus { Pending, Running, Verifying, AwaitingReview, Done, Failed }   // snake_case
pub struct Failure { pub kind: FailureKind, pub detail: String }
pub enum FailureKind { AgentError, VerifyFailed, Timeout, Interrupted, CheckpointFailed }               // snake_case
```

- `RunPaths` computes every path. Writes are atomic: temp file in the same directory, `sync_all`, `rename`.
- `checkpoint(n)`: `base_sha` for 0, else `phases[n-1].checkpoint_sha`.
- `attempts` counts every agent launch of the phase in this run (never reset, so log files are never overwritten).
- Lock: `runs/<id>/lock` holds the PID of the phasewise process for the whole of a mutating operation (`ops::execute/next/retry/rollback/abort`; RAII guard removes it). It is created with `create_new` (atomic). A lock whose PID is not alive, or whose content is not a PID, is stale and is replaced; an empty file is treated as in use. The state is reloaded under the lock (`load_locked`: phases left `running`/`verifying` are orphaned).
- Recovery on load: if there is no live lock, phases in `running`/`verifying` become `failed` with `interrupted` ("phasewise stopped while this phase was running") and the state is saved. With a live lock, read-only commands show the state as is and mutating commands refuse ("phase N is running in process PID").
- Run id: UTC `YYYYMMDD-HHMMSS` + `-` + 4 random lowercase hex chars (randomness from `std::collections::hash_map::RandomState`, no extra crate). Branch: `phasewise/<run-id>`.

## Processes (`process.rs`)

- Agent and verification commands are spawned with `CommandExt::process_group(0)` so the whole tree can be stopped.
- `GroupChild::terminate(grace)`: `kill -TERM -- -<pgid>` (run through `sh -c`, no `unsafe`), poll `try_wait` for `grace` (5 s), then `kill -KILL -- -<pgid>` and reap.
- `is_alive(pid)`: `kill -0 <pid>` through `sh -c`.
- `spawn_in_group` returns a `GroupChild` that owns its process group and registers the pgid in a global registry. Dropping it without `terminate`/`finish_leftovers` (early return, panic) kills the group and reaps the leader. `finish_leftovers(grace)` runs after a normal exit (agent and each verification command): `TERM` to the group and, if anything was there, `KILL` after the group disappears or `grace` passes.
- `install_signal_handler` (`ctrlc` with `termination`: SIGINT, SIGTERM, SIGHUP) sets the shared cancel flag; the second signal kills every registered group (`kill_all_groups`) and exits 130. The CLI prints a notice on stderr; the TUI installs it silently and treats the flag as a confirmed quit (in raw mode Ctrl-C is still a key). A panic hook installed in `cli::main` kills the groups (`try_lock`, never blocks) before the previous hook (which restores the terminal) runs.
- Output to stdout uses non-panicking macros: a closed stdout is ignored.

## Agents (`agent/`)

```rust
pub enum AgentEvent {
    SessionStarted { session_id: String },
    Message { text: String, subagent: bool },
    ToolCall { name: String, detail: String, subagent: bool },   // "Edit src/main.rs", "Bash cargo test"
    Retry { detail: String },
    PermissionDenied { detail: String },
    Error { detail: String },                                     // non-final error reported by the agent
    Finished { ok: bool, summary: String, cost_usd: Option<f64> },
}
pub trait AgentParser: Send { fn feed(&mut self, line: &str) -> Vec<AgentEvent>; }
pub trait Agent: Send + Sync {
    fn kind(&self) -> AgentKind;
    fn program(&self) -> OsString;            // PHASEWISE_CLAUDE_BIN / PHASEWISE_CODEX_BIN or "claude" / "codex"
    fn version(&self) -> Result<String>;      // `<program> --version`, first line
    fn new_session_command(&self, root: &Path, extra_args: &[String]) -> Command;              // prompt on stdin
    fn resume_command(&self, root: &Path, session_id: &str, extra_args: &[String]) -> Command;
    fn parser(&self, root: &Path) -> Box<dyn AgentParser>;   // root is used to shorten absolute paths
}
pub fn agent_for(kind: AgentKind) -> Box<dyn Agent>;
```

Exact command lines: `docs/dev/agent-cli-notes.md`. Parsers never fail: invalid JSON, unknown event types and unknown fields are ignored. `SessionStarted` is emitted once, from the first event that carries a session/thread id.

- Claude (`stream-json`): `system`/`init` → session; `system`/`api_retry` → `Retry`; `assistant` content blocks: `text` → `Message`, `tool_use` → `ToolCall` (detail = first of `file_path`, `path`, `command` (first line), `pattern`, `url`, `description` in `input`, max 200 chars); `subagent = parent_tool_use_id` is a non-null string; `permission_denied` messages and the result's `permission_denials` → `PermissionDenied` (deduplicated by tool use id); `result` → `Finished { ok: !is_error && subtype == "success", summary: result, cost_usd: total_cost_usd }`.
- Codex (`--json`): `thread.started` → session; `item.started`/`item.completed` reported once per item id: `agent_message` → `Message` (remembered as last message), `command_execution` → `ToolCall { name: "Shell", detail: command }`, file change items → `ToolCall { name: "Edit", detail: paths }`, `mcp_tool_call` → `ToolCall { name: "MCP", detail: "server/tool" }`, `web_search` → `ToolCall { name: "WebSearch", detail: query }`, `reasoning`/`todo_list` ignored; `turn.completed` → `Finished { ok: true, summary: last message, cost_usd: None }`; `turn.failed` → `Finished { ok: false, summary: error message }`; `error` → `Error`.

## Verification (`verify.rs`)

`run_all(commands, root, timeout, log_path, attempt, cancel, on_event) -> Result<VerifyOutcome>`. Runs `sh -c <cmd>` in order in the repo root, stdin null, stdout and stderr both appended to the verify log (same file handle, so output stays in order), stops at the first failure, timeout or cancellation. The log gets a header per attempt (`=== attempt K ===`), `$ <cmd>` before each command and `exit code: N` / `timed out after N s` / `interrupted` after it. `VerifyOutcome::Failed` carries the command, exit code, timeout flag and the last 200 lines of that command's output (read back from the log).

## Engine (`engine.rs`)

```rust
pub enum EngineEvent {
    PhaseStarted { phase: usize, total: usize, title: String, attempt: u32, resumed: bool },
    Agent { phase: usize, event: AgentEvent },
    AgentExited { phase: usize, exit_code: Option<i32> },
    VerifyStarted { phase: usize, command: String },
    VerifyFinished { phase: usize, command: String, ok: bool, exit_code: Option<i32>, timed_out: bool },
    Retrying { phase: usize, reason: String },
    CheckpointCreated { phase: usize, sha: String },
    PhaseFinished { phase: usize, status: PhaseStatus, failure: Option<Failure>, files: Option<usize>, cost_usd: Option<f64> },
}
```

`run_phase(ctx, phase_number, note) -> Result<PhaseStatus>` (ctx holds git, paths, state, plan, agent, settings, sender, cancel flag):

1. The caller already holds the run lock. Fresh session. Prompt = `phase_prompt` (+ note).
2. Loop: `attempts += 1`, status `running`, save, emit `PhaseStarted`. Spawn the agent (new or resume command), write the prompt to its stdin from a thread and close it. A reader thread reads stdout line by line (lossy UTF-8), appends each raw line to the `.jsonl`, feeds the parser and sends the events back. stderr goes straight to its log file. The engine loop polls with `recv_timeout` (50 ms), forwards events, records session id / summary / cost, and checks the per-attempt timeout (`phase_timeout_minutes`, overridable with `PHASEWISE_PHASE_TIMEOUT_SECONDS`) and the cancel flag (both → `GroupChild::terminate`). After the child exits, wait at most 2 s more for stdout EOF.
3. Failure → `failed`: cancel → `interrupted`; timeout → `timeout`; non-zero exit, `Finished { ok: false }`, or HEAD no longer on the run branch → `agent_error` (detail includes the exit code, the agent's error text or the last stderr lines). No automatic retry for these. A checkpoint that cannot be created (HEAD moved off the run branch by a verify command, `git add`/`commit` failure) → `checkpoint_failed`.
4. Status `verifying`, save, run global then phase verify commands.
5. Verification failed and retries left (`max_retries` per `run_phase` call) → emit `Retrying`, prompt = `resume_prompt(report)` with the resume command if a session id is known, else `fresh_retry_prompt`; back to 2. No retries left → `failed` with `verify_failed`, tree left as is.
6. Verification passed → `git add -A`, checkpoint commit (`--allow-empty`), `checkpoint_sha = HEAD`, status `awaiting_review`, save, emit `CheckpointCreated` and `PhaseFinished` (with the changed-file count and the cost, for the end-of-phase summary line).

## Operations (`ops.rs`)

- `start_run(dir, plan_path, agent_override) -> Run`: load + validate plan first, refuse if a run is active, check preconditions (commits, not detached, clean tree — the error lists up to 10 dirty paths and says to commit or stash), add the exclude, create the branch, write `plan.md`, `state.json`, `current`.
- `load_active(dir) -> Option<Run>` (with recovery), `approve(run)` (awaiting_review → done; run completed when no phase is left), `next_phase(run)` (first pending phase), `retry(run, note)` (phase in `failed`/`awaiting_review`: reset to checkpoint N-1 + clean, then `run_phase` with a fresh session and the note), `rollback(run, k)` (must be on the run branch; reset + clean; phases after k back to `pending` with checkpoint/session/summary/failure/cost/times cleared), `abort(run)` (if on the run branch with a dirty tree, commit the leftovers on the run branch as `phasewise: abort — uncommitted changes`; switch to the original branch; status `aborted`).
- `next`, `retry` and `abort` require the current branch to be the run branch; `next` also requires a clean tree.
- `--auto`: after a phase reaches `awaiting_review`, approve it and run the next one; stop at the first failure. The last phase stays `awaiting_review`.
- When a run completes, the CLI prints how to integrate: `git switch <original> && git merge --squash phasewise/<run-id> && git commit`.

## Exit codes

`0` success; `1` usage or environment error (clap usage errors are mapped to 1); `3` the phase ended `failed`; `130` second Ctrl-C.

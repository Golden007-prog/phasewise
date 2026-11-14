# phasewise

Run big plans with Claude Code or Codex one phase at a time — verified, checkpointed, and reviewable.

<!-- demo: demo/demo.gif, not recorded yet; see demo/demo.tape -->

## The problem

Handing a large plan to a coding agent in one go gives you one huge, unreviewable change, and no safe point to go back to when step 7 breaks step 3.
phasewise splits the plan into phases. It runs the agent headless for one phase, runs your checks, saves a git checkpoint on its own branch, and stops.
You look at what changed, then continue, retry with a note, or roll back.

## Installation

From a clone of this repository:

```sh
cargo install --path .
```

After the first release (not published yet):

```sh
cargo install phasewise
```

Requirements:

- `git`.
- macOS or Linux (Windows is not supported).
- Claude Code (`claude`) or Codex (`codex`) installed and logged in. phasewise never logs in for you; `phasewise doctor` checks that the CLIs are present, not that you are logged in.

## Quick start

```sh
phasewise init      # creates phasewise.md; edit it, then commit it (run needs a clean tree)
phasewise check     # validates the plan and lists phases and checks; runs nothing
phasewise run       # starts a run on a new branch and runs phase 1
phasewise review    # inspect the phase in a TUI (or: phasewise diff)
phasewise next      # approve the phase and run the next one
```

## Plan format

A plan is one Markdown file (default `phasewise.md`) with optional TOML front matter. Every `##` heading is a phase; a fenced block tagged `verify` lists the commands that must pass for that phase.

````markdown
+++
agent = "claude"
verify = ["cargo test"]
+++

# Add a stats subcommand

Shared context sent in every phase. Do not add dependencies.

## Phase 1: Define the subcommand

Add `stats` with a `--json` flag that prints `not implemented`.

```verify
cargo run -q -- stats --help
```

## Phase 2: Implement it

Print a summary, or one JSON object with `--json`.
````

Full reference (front matter keys, defaults, validation errors): [docs/plan-format.md](docs/plan-format.md). Examples: [examples/plans/](examples/plans/).

## Commands

Run `phasewise` with no arguments: with an active run in a terminal it opens the review TUI; otherwise it prints the help.

| Command | What it does | Flags |
|---|---|---|
| `phasewise init` | Creates an example `phasewise.md` (never overwrites) and excludes `.phasewise/` from git. | |
| `phasewise check [PLAN]` | Validates the plan and lists its phases and checks; runs nothing. | `PLAN`: plan path (default `phasewise.md` at the repository root) |
| `phasewise doctor` | Checks git, claude and codex (presence, version) and whether this directory is a suitable repository. | |
| `phasewise run [PLAN]` | Starts a run and launches phase 1; fails if a run is already active. | `--agent claude\|codex` (overrides the plan), `--auto` (chain phases until the end or the first failure), `--model <M>`, `--effort <E>` |
| `phasewise next` | Approves the phase awaiting review and launches the next one. | `--auto`, `--model`, `--effort` |
| `phasewise retry` | Goes back to the checkpoint before the current phase and reruns it in a new session. | `--note <TEXT>` (appended to the phase prompt), `--model`, `--effort` |
| `phasewise rollback <N>` | Goes back to the checkpoint of phase N (0 = initial state); later phases become pending again. | `--yes` (skip the confirmation prompt) |
| `phasewise status` | Shows a table of phases: state, attempts, changed files, cost if known and model. | |
| `phasewise models [--agent claude\|codex]` | Lists the models each agent offers (Codex: from `codex debug models`; Claude: the documented aliases). | `--agent` (default: both) |
| `phasewise diff [N]` | Prints the diff of phase N (default: the last one with a checkpoint). | |
| `phasewise review` | Opens the review TUI: phase list, Diff / Log / Verify tabs. | |
| `phasewise abort` | Marks the run as abandoned and returns to the original branch; the run branch is kept. | |

Model and effort: `--model <M>` and `--effort low|medium|high|xhigh|max` on `run`, `next` and `retry` are stored in the run and apply from then on; `default` removes the stored value. The plan can set `model` and `effort` in its front matter and per phase in a `phasewise` block (see `docs/plan-format.md`). Precedence, highest first: the flags, the phase block, the front matter, nothing (the agent's own default). The plan's values apply only when the run uses the plan's agent.

Exit codes: `0` success, `1` usage or environment error, `3` a phase ended failed, `130` second Ctrl-C.

Review TUI keys: `j`/`k` or arrows select a phase, `Tab` switches tab, `PageUp`/`PageDown` scroll, `n` approve and run the next phase, `r` retry (asks for an optional note), `b` roll back to the selected phase's checkpoint (asks to confirm), `x` interrupt the running phase, `q` quit.

## Supported agents

| Agent | Status |
|---|---|
| Claude Code | tested — smoke test passed with Claude Code 2.1.325 (2026-11-14) |
| Codex | tested — smoke test passed with Codex CLI 0.160.0 (2026-11-14) |

The test suite uses fake agents and synthetic output captures only. The real smoke test (`docs/dev/smoke-test.md`, two trivial phases) has passed with both agents. Agent CLIs change often; if a newer version breaks a run, please open an issue with the raw output from `.phasewise/runs/<id>/`.

Command lines phasewise uses (the prompt is sent on stdin; `agent_args` from the plan are appended at the end):

- Claude Code: `claude -p --output-format stream-json --verbose --permission-mode acceptEdits --permission-prompts none --allowedTools Read,Edit,Write,Bash --disallowedTools "Bash(git push *)"`; a retry in the same session adds `--resume <session-id>`.
- Codex: `codex exec --json --sandbox workspace-write -`; a retry in the same session uses `codex exec --json --sandbox workspace-write resume <session-id> -`.

When a model or effort is set, Claude gets `--model <M> --effort <E>` after the fixed flags (before `--resume` and `agent_args`) and Codex gets `-m <M> -c model_reasoning_effort="<E>"` before the `resume` subcommand.

Set `PHASEWISE_CLAUDE_BIN` or `PHASEWISE_CODEX_BIN` to use a different binary.

## How a run works

1. `phasewise run` requires a git repository with at least one commit, a checked-out branch (not detached) and a clean working tree. It creates the branch `phasewise/<run-id>` from your current commit and switches to it. Your original branch is not touched.
2. State lives in `.phasewise/` at the repository root (plan copy, `state.json`, agent output, verification logs). It is added to `.git/info/exclude` and never committed.
3. For each phase the agent runs headless, then the global `verify` commands and the phase's own checks run. If they fail, phasewise retries up to `max_retries` times, telling the agent what failed.
4. When checks pass, phasewise commits everything as a checkpoint with a message like:

   ```text
   phasewise: phase 1 — Define the subcommand

   Phasewise-Run: 20261114-101500-a1b2
   Phasewise-Phase: 1
   ```

5. It stops with the phase awaiting review. Look at it with `phasewise diff` or `phasewise review`, then `phasewise next`, `phasewise retry --note "..."` or `phasewise rollback N`.
6. When every phase is done, phasewise prints how to bring the work into your branch:

   ```sh
   git switch <original-branch> && git merge --squash phasewise/<run-id> && git commit
   ```

   phasewise does not run this for you.

## Security model

phasewise runs the agent unattended, with permission to edit files and run shell commands in your repository, and the agent inherits your environment variables. The protection is the dedicated branch and the checkpoints, not a sandbox. Read [docs/security.md](docs/security.md) before pointing it at a plan you did not write.

## FAQ

**Does phasewise touch my branch?**
No. It works on `phasewise/<run-id>`. Your original branch is only changed by you, when you merge. phasewise never pushes, never deletes branches and never uses `--force`. `phasewise abort` returns you to the original branch and keeps the run branch.

**What happens if a phase fails?**
If verification fails, phasewise retries up to `max_retries` times (default 1). If it still fails, or the agent errors, times out or is interrupted, the phase is marked `failed`, the working tree is left as the agent left it, and the command exits with code 3. Then you can inspect it, `phasewise retry --note "..."` (goes back to the previous checkpoint and starts a new session) or `phasewise rollback N`.

**How much does it cost?**
phasewise itself is free; the agents are not. Claude's reported `total_cost_usd` is shown per phase in `phasewise status`. Codex reports tokens only, so phasewise shows no cost for it. There are no spending limits in v0.1; for Claude you can pass `--max-budget-usd` through `agent_args` in the plan front matter.

**What does Ctrl-C do?**
In `run`, `next` and `retry`, the first Ctrl-C stops the agent and marks the phase failed (interrupted); a second one kills the agent's processes and exits with 130. SIGTERM and SIGHUP (for example closing the terminal) are handled like Ctrl-C. If phasewise dies some other way, the next command that reads the run marks the orphaned phase as failed (interrupted). In the TUI, Ctrl-C works like `q`.

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md). Changes are listed in [CHANGELOG.md](CHANGELOG.md).

## License

MIT, © userfypp. See [LICENSE](LICENSE).

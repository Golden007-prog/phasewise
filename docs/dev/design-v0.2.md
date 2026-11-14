# Internal design: v0.2 additions

Binding, like `design.md`, which it extends. These four features were asked
for by the maintainer after testing v0.1. They are built in this order, each
one passing the quality gate before the next one starts:

1. Clearer output (CLI and TUI).
2. Choosing the model and the effort.
3. `phasewise plan`: the agent writes the plan from one sentence.
4. Home screen in the TUI.

General rules:

- No new runtime dependency. Colours come from `ratatui::crossterm::style`,
  which is already built in through ratatui.
- The version stays 0.1.0, because nothing has been released yet. Changes go
  to the unreleased section of `CHANGELOG.md`.
- Every agent interaction in tests goes through `tests/fakes/`, extended as
  needed. The real agents are only used in the final manual check.
- Facts about the agent CLIs were checked on 2026-11-14 against Claude Code
  2.1.325 and Codex 0.160.0 (`--help`, the Claude Code model-config docs, and
  `codex debug models`). They are summarised in `docs/dev/agent-cli-notes.md`.

## 1. Clearer output

### Shared styling: `src/style.rs`

`Styler { color: bool }` with one method per role: `heading`, `dim`, `ok`,
`fail`, `warn`, `accent`, `tool`. Each returns a `String`, with ANSI codes
only when `color` is true.

`Styler::for_stdout()` enables colour only when all of these hold:

- stdout is a terminal;
- `NO_COLOR` is unset or empty;
- `TERM` is not `dumb`.

Tests build `Styler { color: false }` or `Styler { color: true }` explicitly.

### CLI layout for `run`, `next` and `retry`

Each phase is a block:

```text
▶ Phase 1/2 · Create a file                    attempt 1 · claude · opus · high
  00:01  ✎ Write    prueba.md
  00:02  $ Bash     cargo test
  00:02  › Created prueba.md in the project root.
  00:03  ✓ Agent finished
  00:03  ⚙ Check    test -f prueba.md
  00:03  ✓ Passed   test -f prueba.md
  00:03  ● Checkpoint 821c664
✓ Phase 1 ready for review · 1 file · $0.06
```

Formatting rules:

- **Header.** The phase line is a heading. Its right part is dim and shows the
  attempt, the agent, and the model and effort only when they are set.
- **Event lines.** Each line is `  mm:ss  <icon> <label padded to 8> <detail>`.
  The time is dim. Icons by event:

  | Event | Icon | Colour |
  |---|---|---|
  | Read / Grep / Glob | `◦` | tool |
  | Write / Edit | `✎` | tool |
  | Bash or Shell commands | `$` | tool |
  | Other tools | `·` | tool |
  | Subagent tools | `↳` | tool |
  | Agent messages | `›` | default |
  | Retries, permission denied | `!` | warn |
  | Errors | `✗` | fail |
  | Check started | `⚙` | dim |
  | Check passed | `✓` | ok |
  | Check failed | `✗` | fail |
  | Checkpoint | `●` | accent |

- **Agent messages.** Keep the existing one-line, 160-character limit.
- **End of a phase.** One summary line:
  - `✓ Phase N ready for review · <files> · <cost>`, in ok colour;
  - `✗ Phase N failed (<kind>)`, in fail colour, followed by the existing
    failure block.

  The file count comes from `git::changed_file_count` between checkpoints.
  The cost is shown only when known.
- **Hints.** The "next" hints that follow are dim.
- **`status`.** Same columns as before, with each status coloured:
  - done: ok;
  - awaiting review: accent;
  - failed: fail;
  - running and verifying: warn;
  - pending: dim.

  A `MODEL` column shows the model each phase used, or `-`.
- **`check`.** The plan header is a heading. Each phase is `N. Title`,
  followed by its checks with `⚙` and its model settings (see §2).
- **Without colour** the layout is identical; only the ANSI codes are
  missing.
- `display.rs` stays the single place that turns events into text. It gains a
  structured form (`EventLine { time, icon, label, detail, role }`) that both
  the CLI and the TUI render.

### TUI

- **Phase panel width.** 30 % of the screen, at least 24 and at most 48
  columns. Titles that do not fit end with `…`.
- **Status icons.** Coloured like `status`: `✓` done, `◆` awaiting review,
  `✗` failed, `●` running or verifying, `·` pending.
- **Header bar.** Shows run id, plan title, agent, model and branch. The
  labels are dim and the values are default.
- **Log tab.** Uses the `EventLine` icons and colours. Each attempt starts with
  a heading line, `── attempt 2 · resumed ──`, with the model when known.
- **Verify tab.** `✓`/`✗` coloured; command lines are headings and output
  lines are default.
- **Diff tab.** Unchanged, except that the file summary line is a heading.
- **Help.** `?` toggles a centred help popup listing every key. The key bar
  gains `? help`.
- **Snapshots** are updated and reviewed one by one. They must contain no ANSI
  codes; the `TestBackend` buffer stores styles separately.

## 2. Choosing the model and the effort

### Values

`model` is a non-empty string with these limits:

- at most 100 characters;
- no whitespace;
- it must not start with `-`, so it can never be read as a flag;
- `default` means "no model flag", that is, the agent's own default.

`effort` is one of `low`, `medium`, `high`, `xhigh`, `max`. Both CLIs accept
these five; a model that lacks a level uses the closest lower one, according to
the Claude docs and the levels listed by the Codex catalog.

### Where they are set

Highest precedence first:

1. **Run override,** stored in `state.json`:
   - `run`, `next` and `retry` accept `--model <M>` and `--effort <E>`;
   - a given value replaces the stored override from then on;
   - `--model default` / `--effort default` remove the override.
2. **Phase settings block.** A fenced code block whose info string is exactly
   `phasewise` inside a phase, containing TOML with only `model` and/or
   `effort`. It is removed from the text sent to the agent, like `verify`
   blocks. The parser reports, with line numbers:
   - a block before the first phase;
   - a second block in the same phase;
   - an unknown key, by name;
   - an invalid value.
3. **Plan front matter:** `model = "..."` and `effort = "..."`.
4. **Nothing.** No flag is passed.

Levels 2 and 3 apply only when the run's agent equals the plan's `agent`. A
model name belongs to one agent, so with `run --agent codex` on a Claude plan
they are ignored. The phase header shows the effective values, so this is
visible.

### State

The changes are additive. Old `state.json` files still load, and
`schema_version` stays 1.

- `RunState` gets `model_override: Option<String>` and
  `effort_override: Option<String>`.
- `PhaseState` gets `model: Option<String>` and `effort: Option<String>`,
  holding the effective values of its last attempt.
- All four use `#[serde(default, skip_serializing_if = "Option::is_none")]`.

`EngineEvent::PhaseStarted` gains `model` and `effort`.

### Command lines

The `Agent` trait methods take a `LaunchOptions { model: Option<String>,
effort: Option<String> }` (plus whatever later features need).

- **Claude:** `--model <M>` and `--effort <E>` go after the fixed flags and
  before `--resume <id>` and the plan's `agent_args`.
- **Codex:** `-m <M>` and `-c model_reasoning_effort="<E>"` go after
  `--sandbox …` and before the `resume` subcommand. It was checked that
  `codex exec … -m X -c model_reasoning_effort="high" resume --help` parses.

### Listing models: `phasewise models [--agent claude|codex]`

The default lists both agents. `Agent::list_models() ->
Result<Vec<ModelInfo>>`, where `ModelInfo { id, label, description, efforts }`.

**Claude.** There is no non-interactive listing in the CLI, so phasewise uses
the documented aliases. The list lives in `agent/claude.rs`, with the date it
was checked:

| Alias | Meaning |
|---|---|
| `fable` | hardest, longest tasks |
| `opus` | complex reasoning |
| `sonnet` | daily coding |
| `haiku` | fast, simple tasks |
| `opus[1m]` | Opus with a 1M context window |
| `sonnet[1m]` | Sonnet with a 1M context window |
| `opusplan` | Opus to plan, Sonnet to execute |

The output also says that any full model name accepted by `claude --model`
works, and that the account may restrict models.

**Codex.** phasewise runs `<codex> debug models` with a 10 s timeout, stdin
null and output captured. It parses `{"models": [...]}` and keeps the entries
with `"visibility": "list"`, in catalog order:

| Catalog field | `ModelInfo` field |
|---|---|
| `slug` | `id` |
| `display_name` | `label` |
| `description` | `description` |
| `supported_reasoning_levels[].effort` | `efforts` (also accept plain strings) |

Unknown fields are ignored and an entry without `slug` is skipped. If the
command fails or prints invalid JSON, phasewise reports what happened and that
any name accepted by `codex -m` works. The fake codex answers `debug models`
from `tests/fixtures/codex-models.synthetic.json`.

`check` and `status` show the model settings. `doctor` is unchanged.

## 3. `phasewise plan`

```text
phasewise plan [GOAL] [--agent claude|codex] [--model M] [--effort E] [--output PATH] [--force]
```

### Before the agent starts

- **GOAL.** It is the user's sentence. If it is missing and stdin is a
  terminal, phasewise asks `What do you want to do? `. If it is missing and
  stdin is not a terminal, that is an error.
- **Git.** It needs a git repository; commits are not needed. It ensures that
  `.phasewise/` is excluded.
- **Output.** The default is the default plan path. If the file exists and
  `--force` is not given, phasewise refuses before calling the agent.
- **Agent.** `--agent`, or else `claude`. `--model` and `--effort` follow
  §2's rules.

### The agent run (read-only)

The agent runs in the repository root:

- **Claude:** `-p --output-format stream-json --verbose --permission-mode
  dontAsk --permission-prompts none --tools Read,Grep,Glob --strict-mcp-config
  --allowedTools Read,Grep,Glob --disallowedTools
  Edit,Write,NotebookEdit,Bash,WebFetch,WebSearch`, plus `--model`/`--effort`.
  `--tools` leaves only those three built-in tools in the session and
  `--strict-mcp-config` (without `--mcp-config`) loads no MCP server, so tools
  that need no permission (subagents, worktrees, schedulers) are not there.
- **Codex:** `exec --json --sandbox read-only`, plus `-m`/`-c`, then `-`.

Other rules:

- The prompt goes on stdin. The process group is owned by a `GroupChild`.
- Events are shown live like a phase.
- Ctrl-C cancels, with the same handler as `run`.
- The timeout is 20 minutes; `PHASEWISE_PLAN_TIMEOUT_SECONDS` overrides it,
  for tests.

### Files

Everything goes in `.phasewise/plans/<plan-id>/`:

| File | Contents |
|---|---|
| `attempt-K.jsonl` | the agent's output |
| `attempt-K.stderr.log` | the agent's stderr |
| `draft-K.md` | each extracted draft |

`<plan-id>` uses the run id format.

### Prompt (`prompt::plan_prompt`)

The prompt is in English. It tells the agent:

- what phasewise is and how a run uses the plan (one phase per session, checks,
  checkpoints, review);
- the exact plan format: front matter keys and defaults, phases, `verify`
  blocks, `phasewise` settings blocks;
- how to write good phases:
  - usually 3–10 phases, ordered, each small enough to review;
  - each phase leaves the project working;
  - each has concrete checks using commands that really exist in the project
    (look at the build files);
  - no destructive or networked checks;
  - instructions are self-contained;
  - the phase text is written in the user's language;
- that `agent = "<agent>"` must be set;
- how to choose models. The prompt includes the agent's model list and says to
  leave the model unset unless a phase clearly needs a stronger or cheaper
  model;
- to read the project first and never modify files (read-only commands such
  as `ls`, `cat` or `rg` are allowed where the agent's tools allow them);
- **the output protocol:** the final answer must contain the complete plan
  between a line `<<<PHASEWISE-PLAN` and a line `PHASEWISE-PLAN>>>`.

### Extraction and validation

1. Take the final text: Claude's `result`, or Codex's last `agent_message`.
   This is `Finished.summary`, which the parsers keep in full.
2. Take the content between the last `<<<PHASEWISE-PLAN` line and the next
   `PHASEWISE-PLAN>>>` line. Marker lines are matched after trimming.
3. If its first and last non-empty lines are one code fence (```` ``` ````,
   ```` ```markdown ````, ```` ```md ```` or `~~~`), remove that fence.
4. Validate it with `plan::parse`, and check that its agent equals the chosen
   agent.
5. If the agent failed, the markers are missing or the plan is invalid, run a
   repair round: resume the same session (Codex with `--sandbox read-only`),
   send the list of problems, and ask for the complete corrected plan between
   the markers. At most two repair rounds.

### Results

- **Success.**
  - Write the plan atomically to the output path.
  - Print the same summary as `check`, the cost when known, and the next
    steps: review and edit the file, commit it, then `phasewise run`.
  - Exit 0.
- **Failure** (agent error, timeout, cancel, or still invalid after the
  repairs):
  - Write nothing to the output path.
  - Print the problems and the path of the last draft.
  - Exit 1, or 130 on the second signal.

### Code

- `ops::create_plan(cwd, request, events, cancel) -> Result<PlanOutcome>`
  emits `PlanEvent`s: started, agent event, attempt finished, repairing,
  written.
- The agent-following loop of the engine is shared, not copied. It is
  extracted into `engine::run_agent_session` (spawn, feed stdin, read, timeout,
  cancel, finish leftovers, return run info), used by both `run_phase` and
  `create_plan`. Every existing test must keep passing unchanged.

## 4. Home screen in the TUI

### Entry

`phasewise` without arguments, on a terminal:

| Situation | Opens |
|---|---|
| Active run | the review screen, as today |
| No active run, inside a git repository | the home screen |
| Otherwise | the long help, as today |

`phasewise review` is unchanged.

### Layout

- **Header:** `phasewise · <repo dir name> · <current branch>`.
- **Plan panel (left):**
  - the plan path;
  - its state: missing, valid with N phases, or invalid with the errors and
    their lines;
  - for a valid plan, the phase list with each phase's check count and model.
- **Settings panel (right):**
  - **Agent:** claude or codex, with its version or "not installed". It comes
    from `Agent::version`, run in a background thread.
  - **Model:** "default" or a name.
  - **Effort:** "default" or a level.
- **Status line:** messages and errors.
- **Key bar:** `a` agent, `m` model, `e` effort, `p` plan with the agent, `c`
  check again, `Enter` start, `?` help, `q` quit.

### Keys

- **`a`** toggles the agent. The model resets to default, because model names
  belong to one agent.
- **`m`** opens a popup list:
  - `default`, then the agent's `list_models()` (loaded in a background thread
    and cached per agent; "loading…" until ready), then `custom…`;
  - `custom…` opens a text input validated with §2's rules;
  - if the list fails to load, it still offers default and custom, and shows
    the error.
- **`e`** opens a popup list: default and the five levels.
- **`p`** opens a text input, "What do you want to do?". On Enter:
  - if the plan file exists, it asks "Replace <file>? (y/N)";
  - then it runs `ops::create_plan` in a worker with the current settings;
  - the plan panel shows the live events;
  - at the end the plan is reloaded and the outcome shown;
  - `x` cancels.
- **`Enter`** starts a run. It needs a valid plan; `ops::start` checks the rest
  (clean tree, branch, identity) and its error is shown as is. For an
  uncommitted plan, the error tells the user to commit it. phasewise never
  commits the plan itself.

  On success it runs `ops::execute` in a worker, with the agent override and
  the model/effort overrides stored in the run (§2), and switches to the review
  screen of that run. The worker and the events are handed over exactly as
  review's `n`.
- **`q`** quits, with the same rules as review while a worker runs.

### Code and tests

- The pure state machine is `tui/home.rs`, tested like `app.rs`: key in,
  command out, no I/O.
- Rendering goes in `tui/view.rs`, or `tui/home_view.rs` if that is clearer,
  with `insta` snapshots for:
  - a missing plan;
  - a valid plan;
  - an invalid plan;
  - the model popup;
  - the plan input;
  - creation in progress.
- `tui/mod.rs` owns the screen switch.
- A manual pty check with `expect` and the fake agents covers: open home → `p`
  with a goal → plan written → commit it from the shell → `Enter` → review.

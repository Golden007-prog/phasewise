# Agent CLI notes

Facts about the agent CLIs that phasewise drives, checked on the development
machine (macOS, aarch64) on 2026-11-13 by reading `--help` and by parse-only
invocations with an empty stdin (they exit before any model call, so they cost
nothing). Re-check this file whenever a CLI is upgraded.

| CLI | Version checked |
|---|---|
| Claude Code (`claude --version`) | `2.1.296 (Claude Code)` |
| Codex (`codex --version`) | `codex-cli 0.160.0` |

## Claude Code

Confirmed in `claude --help`:

- `-p, --print` runs non-interactively. The prompt is an optional positional
  argument.
- `--output-format text|json|stream-json` (print mode only) and `--verbose`.
- `--permission-mode` accepts `acceptEdits`, `auto`, `bypassPermissions`,
  `manual`, `dontAsk`, `plan`.
- `--permission-prompts host|none` (default `host`). With `none`, anything that
  would prompt is denied automatically; the permission mode decides the rest.
- `--allowedTools` / `--allowed-tools <tools...>` and `--disallowedTools` /
  `--disallowed-tools <tools...>`: comma- or space-separated tool names, with
  patterns such as `Bash(git *)`.
- `-r, --resume [value]` resumes by session id.
- `--append-system-prompt`, `--bare`, `--max-budget-usd` and
  `--forward-subagent-text` exist. phasewise does not pass them; users can add
  them through `agent_args` (except `--bare`, which skips `CLAUDE.md` and needs
  an API key).

Confirmed by a parse-only run: `claude -p --output-format stream-json --verbose
--permission-mode acceptEdits --permission-prompts none --allowedTools
"Read,Edit,Write,Bash" --disallowedTools "Bash(git push *)" < /dev/null` exits
with code 1 and `Error: Input must be provided either through stdin or as a
prompt argument when using --print`. So every option is accepted and the prompt
can come from stdin alone, with no positional argument.

The installed 2.1.296 predates 2.1.302 (where `-p` defaults to Manual mode).
This does not matter: phasewise always passes `--permission-mode acceptEdits`.

### Commands used by phasewise

New phase (prompt on stdin):

```sh
claude -p --output-format stream-json --verbose \
  --permission-mode acceptEdits --permission-prompts none \
  --allowedTools Read,Edit,Write,Bash \
  --disallowedTools "Bash(git push *)" \
  [agent_args...]
```

Continue a session: the same plus `--resume <session_id>` (placed before
`agent_args`).

Differences with the original plan: `--permission-prompts none` is added so a
phase can never wait for a human answer.

### Event format

Still to be confirmed against a real capture (build phase 5). Expected from the
documentation: one JSON object per line; `system`/`init` first (possibly after
hook or plugin events), `assistant`/`user` messages carrying `session_id` and
`parent_tool_use_id` (non-null for subagents), `system`/`api_retry`, and a final
`result` with `result`, `session_id`, `total_cost_usd`, `is_error` and
`permission_denials`.

## Codex

Confirmed in `codex exec --help` and `codex exec resume --help`:

- `codex exec [OPTIONS] [PROMPT]`. With no prompt or with `-`, the prompt is
  read from stdin.
- `-s, --sandbox read-only|workspace-write|danger-full-access` exists on
  `exec` only. `codex exec resume ... --sandbox ...` is rejected by the parser
  (`error: unexpected argument '--sandbox' found`, exit code 2). Placed before
  the `resume` subcommand it is accepted.
- `--json` is accepted both on `exec` and on `exec resume`.
- `--full-auto` no longer appears in the help.
- `codex exec resume [SESSION_ID] [PROMPT]`, with `--last`; `-` reads the
  prompt from stdin.
- `-o, --output-last-message`, `--skip-git-repo-check`, `--ephemeral` and
  `-c key=value` exist (not used by phasewise).

Confirmed by parse-only runs with an empty stdin (all print `No prompt provided
via stdin.` and exit 1, i.e. the arguments parsed):

- `codex exec --json --sandbox workspace-write -`
- `codex exec --sandbox workspace-write resume --json <id> -`
- `codex exec --json resume <id> -`

### Commands used by phasewise

New phase (prompt on stdin):

```sh
codex exec --json --sandbox workspace-write - [agent_args...]
```

Continue a session:

```sh
codex exec --json --sandbox workspace-write resume <thread_id> - [agent_args...]
```

Differences with the original plan: `--sandbox` must come before `resume`.
`agent_args` that only exist on `exec` (such as `--sandbox`) are rejected when
resuming, because they end up after the `resume` subcommand.

Not yet verified at run time: that `--sandbox workspace-write` given before
`resume` applies to the resumed session (it parses; the real smoke test only
exercises fresh sessions unless a retry happens).

### Event format

Still to be confirmed against a real capture (build phase 5). Expected from the
documentation: JSON Lines with `thread.started` (`thread_id`), `turn.started`,
`turn.completed` (`usage`), `turn.failed`, `item.started`, `item.completed` and
`error`. The exact item type string for file changes must come from a real
capture. Codex reports token usage, not cost, so `cost_usd` stays empty.

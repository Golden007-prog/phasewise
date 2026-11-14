# Real smoke test

The test suite never calls a real agent. Before an agent can be marked
"tested" in the README, it must pass this smoke test once (plan section 8):
two trivial phases run by the real CLI, which costs a few cents.

1. Log in to the agent CLI yourself (`claude`, then `/login`; or
   `codex login`). phasewise never logs in for you.
2. Build phasewise: `cargo build`.
3. Run, from the repository root:

   ```sh
   scripts/smoke-test.sh claude
   scripts/smoke-test.sh codex
   ```

The script creates a throwaway repository under `$TMPDIR`, runs a two-phase
plan (create `hello.txt`, then append `world`) with `phasewise run` and
`phasewise next`, and checks: both phases reach `awaiting_review`/`done`, two
checkpoint commits, a captured session id, the expected diff per phase, no
`.phasewise/` path in any commit, and `main` unchanged. It exits non-zero if a
check fails.

Then:

- Record the result and the CLI version in `docs/dev/progress.md` and
  `docs/dev/agent-cli-notes.md`.
- Copy the raw `phase-*.jsonl` files it lists to `tests/fixtures/`
  (`claude-smoke-phase1.jsonl`, …) after replacing personal paths, account
  data, MCP server and plugin names with placeholders, and add parser tests for
  them.
- Change the agent's status in the README from "experimental" to "tested".

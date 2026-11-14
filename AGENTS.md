# AGENTS.md

Guide for coding agents working on phasewise.

## What this is

phasewise is a Rust CLI/TUI that runs a Markdown plan with Claude Code or Codex
one phase at a time: it launches the agent headless for one phase, runs the
verification commands, checkpoints the repository with a git commit on its own
branch, and stops for review.

## Architecture

Module map, internal APIs and invariants: `docs/dev/design.md` (binding).
Agent CLI facts and exact command lines: `docs/dev/agent-cli-notes.md`.
Decisions and deviations from the original plan: `docs/dev/decisions.md`.
v0.2 additions (output, models, `plan`, home screen): `docs/dev/design-v0.2.md`
(binding, like `design.md`).

Modules (`src/`): `cli` (clap, dispatch, exit codes), `display` (event and
status formatting shared by CLI and TUI), `style` (ANSI colour for the CLI), `plan` (parser), `prompt` (all agent
prompt texts), `git` (the only module that runs `git`), `state` (`state.json`,
run layout, lock, recovery), `process` (process groups), `verify` (verification
commands), `agent/` (`mod`, `claude`, `codex`: command lines and output
parsers), `engine` (one phase cycle), `ops` (run-level operations shared by CLI
and TUI), `doctor`, and `tui/` (`mod` terminal loop and screen switch, `app` and
`home` pure key/state machines for review and the home screen, `data` read-only
loaders, `view` and `home_view` rendering).

The engine and `ops` never print or draw: they emit `EngineEvent`s through an
`mpsc` channel; the CLI and the TUI are two consumers of that channel.

Tests: unit tests live next to the code (`#[cfg(test)]`); integration tests are
`tests/*.rs` (shared helpers in `tests/common/`), run against temporary git
repositories and the fake agents in `tests/fakes/` (`fake-claude.sh`,
`fake-codex.sh`); agent output captures are in `tests/fixtures/` (synthetic ones
are marked `.synthetic`); TUI rendering is checked with `insta` snapshots in
`src/tui/snapshots/`. `scripts/smoke-test.sh` is the only thing that calls a
real agent and is never run by `cargo test`.

## Quality gate

All three must pass with no errors and no warnings before any commit:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Rules

1. Do not invent interfaces: check agent CLI options with `--help` and crate
   APIs against the documentation of the installed version.
2. Add dependencies with `cargo add` (latest stable). Justify every new
   dependency in `docs/dev/decisions.md`.
3. No `unsafe` (`#![forbid(unsafe_code)]`). No `unwrap()`/`expect()` outside
   tests, except provable invariants with a comment.
4. No async runtime: threads and `std::sync::mpsc`.
5. git is used through the `git` binary, only from `src/git.rs`. phasewise never
   pushes, never deletes branches, never uses `--force`.
6. State files are written atomically (temp file in the same dir + `rename`).
7. Every user-facing error says what happened and what to do. No panics as UI.
8. Agent output parsers ignore unknown event types and fields; they never fail.
9. Do not widen the scope: new ideas go to `docs/ROADMAP.md`.
10. Tests never call a real agent, never use the network and never depend on
    the global git configuration (use the fakes in `tests/fakes/`).
11. Repository language is English: code, comments, docs, commit messages
    (Conventional Commits), program output.

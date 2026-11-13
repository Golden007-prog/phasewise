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

The engine and `ops` never print or draw: they emit `EngineEvent`s through an
`mpsc` channel; the CLI and the TUI are two consumers of that channel.

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

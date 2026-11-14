# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — Unreleased

### Added

- Markdown plan format with optional TOML front matter (`agent`, `verify`, `max_retries`, `phase_timeout_minutes`, `verify_timeout_seconds`, `agent_args`), per-phase `verify` blocks and line-accurate validation errors.
- Commands: `init`, `check`, `doctor`, `run`, `next`, `retry`, `rollback`, `status`, `diff`, `review` and `abort`; `phasewise` with no arguments opens the TUI for an active run on a terminal, otherwise prints the help.
- Phase engine: headless agent run per phase, verification commands with timeouts, automatic retries with the failure report, checkpoint commits with `Phasewise-Run` / `Phasewise-Phase` trailers on the branch `phasewise/<run-id>`.
- `--auto` for `run` and `next`, `retry --note`, `rollback --yes`, and the instructions to integrate with `git merge --squash` when a run completes.
- Run state in `.phasewise/` (atomic writes, lock file, recovery of phases left running by a dead process), excluded from git.
- Adapters for Claude Code (checked with a real smoke test) and Codex (experimental: not yet verified against a real run), with per-phase cost shown when Claude reports it.
- Review TUI with phase list, Diff / Log / Verify tabs, live engine output, approve, retry with a note, rollback and interrupt; the terminal is restored on exit and on panic.
- Ctrl-C, SIGTERM and SIGHUP handling that stops the agent's process group and records the phase as interrupted; a second signal kills every process phasewise started and exits with 130.
- Guarantees: never pushes, never deletes branches, never uses `--force`, no network access of its own, no `unsafe`.
- Documentation (README, plan format, security model, roadmap), example plans, smoke-test script for real agents and a VHS demo script.

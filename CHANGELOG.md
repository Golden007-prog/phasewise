# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — Unreleased

### Added

- **Runs, one phase at a time.** Each phase runs the agent headless on the
  branch `phasewise/<run-id>`, then the plan's checks; a failed check goes back
  to the agent with its output while retries are left. A passing phase becomes
  a checkpoint commit with `Phasewise-Run` and `Phasewise-Phase` trailers and
  waits for review. The original branch is never touched.
- **Review.** Approve and continue (`next`, or `--auto` to keep going), retry a
  phase with a note, roll back to any checkpoint, or abandon the run. When the
  last phase is approved, phasewise shows the `git merge --squash` command that
  brings the run into your branch.
- **TUI.** `phasewise` opens a home screen and a run screen in one session. The
  home screen shows the plan and its phases, the agent, model and effort, the
  current run, and what needs doing, such as committing the plan with one key
  (`c`) or the command to copy. The run screen follows the running phase live
  and shows each phase's diff, log and checks; `esc` goes back home and `A`
  abandons the run.
- **Plans written by the agent.** `phasewise plan "<goal>"`, or `p` on the home
  screen, runs a read-only session that explores the project and writes
  `phasewise.md`. The plan is validated, repaired in the same session up to
  twice, and written only when valid.
- **Models and effort** per plan, per phase or per run, with the exact model
  names (`claude-opus-5-5`, …) and `phasewise models` to list them. The model
  the agent really used is recorded and shown.
- **Plan format:** Markdown with optional TOML front matter, `verify` blocks,
  per-phase `phasewise` settings blocks, and validation errors with line
  numbers. Phase-number prefixes such as `Phase 1:` or `Fase 1:` are removed
  from titles.
- **Commands:** `init`, `check`, `doctor`, `plan`, `run`, `next`, `retry`,
  `rollback`, `status`, `diff`, `review`, `models` and `abort`.
- **Robustness:** atomic state writes, a run lock, recovery of phases left
  running by a dead process, a process group for every agent and check, and
  Ctrl-C, `SIGTERM` and `SIGHUP` handling (a second signal, or a second Ctrl-C
  on the screens, stops everything and exits with 130). Closing the terminal
  stops the agent and the TUI. git reads never block a checkpoint
  (`GIT_OPTIONAL_LOCKS=0`, and a short retry when `index.lock` is taken).
- **Agents:** Claude Code (tested with 2.1.325) and Codex (0.160.0), both
  checked with a real smoke test. Claude Code's cost is shown per phase.
- **Guarantees:** phasewise never pushes, never deletes branches, never uses
  `--force`, makes no network connections of its own and has no `unsafe` code.
- **Documentation:** README, user guide, plan format, agents, architecture,
  decisions, roadmap, contributing guide, security policy and code of conduct;
  example plans, a smoke-test script for real agents and a VHS demo script.

# Progress

One line per build phase: date, phase, quality gate result (`cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`).

- 2026-11-14 — Phase 0 (environment and agent CLI facts): gate passed (fmt ok, clippy ok, 0 tests). claude 2.1.296 and codex 0.160.0 found; facts in `agent-cli-notes.md`.
- 2026-11-14 — Phase 1 (CLI skeleton and CI): gate passed (fmt ok, clippy ok, 8 tests).
- 2026-11-14 — Phase 2 (plan parser, check, init, plan format docs, examples): gate passed (fmt ok, clippy ok, 47 tests).
- 2026-11-14 — Phase 3 (git layer and run state): gate passed (fmt ok, clippy ok, 71 tests).
- 2026-11-14 — Phase 4 (engine, ops, run/next/status/diff, fake agents): gate passed (fmt ok, clippy ok, 113 tests). Manual run with the fake agent checked.
- 2026-11-14 — Phase 5 (adapters aligned with documented formats, doctor, smoke test script): gate passed (fmt ok, clippy ok, 121 tests). Real smoke test NOT run: claude 2.1.296 and codex 0.160.0 are not logged in on this machine; both agents marked experimental.
- 2026-11-14 — Phase 6 (retry, rollback, abort, orphan recovery notice, error messages): gate passed (fmt ok, clippy ok, 134 tests; full suite run 4 times without failures).
- 2026-11-14 — Phase 7 (TUI: data, state, rendering, terminal loop, actions): gate passed (fmt ok, clippy ok, 165 tests). Manual flow open → n → Log → Diff → b → q checked in a real pty with expect and the fake agent; terminal restored.
- 2026-11-14 — Phase 8 (README, security model, CHANGELOG, ROADMAP, AGENTS.md, VHS tape): gate passed (fmt ok, clippy ok, 165 tests). README commands run with the fake agents; `cargo install --path .` checked with `--root` in a temporary directory; `cargo install phasewise` not possible until published; demo GIF not recorded (needs `vhs`).
- 2026-11-14 — Phase 9, review fixes (atomic run lock held per operation, `.phasewise/` protected in `git add`/`clean`, `checkpoint_failed`, identity check, process-group registry, SIGTERM/SIGHUP, closed stdout): gate passed (fmt ok, clippy ok, 179 tests; full suite run twice, signal tests 5 times). TUI flow checked again in a real pty with expect; a second review found nothing serious.
- 2026-11-14 — Phase 9 (final verification): clean clone at `4bc352d`: release build, fmt ok, clippy ok, 179 tests passed; `cargo install --path . --root <tmp>` installed phasewise 0.1.0. Acceptance checklist with evidence in `docs/dev/acceptance.md`. No push, no remote, nothing published.

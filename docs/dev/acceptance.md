# Acceptance checklist (v0.1.0)

Walked on 2026-11-14 at commit `4bc352d`, in a clean clone in a temporary
directory. Each item names the evidence behind it.

1. **The quality gate passes in a clean clone.** `cargo build --release`,
   `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --
   -D warnings` and `cargo test --all-features` (179 tests, 0 failures) passed.
   `cargo install --path . --root <temporary dir>` installed `phasewise 0.1.0`.
2. **Every command works and has a test.**
   - `init`, `check`: `tests/check_init.rs`.
   - `doctor`: `tests/doctor.rs`.
   - `run`, `next`, `status`, `diff`: `tests/run_cycle.rs`.
   - `retry`, `rollback`, `abort`: `tests/lifecycle.rs`.
   - `review`: `tests/review.rs`, `tests/tui_data.rs` and the `insta` snapshots in `src/tui/snapshots/`. It was also checked by hand in a real pty driven by `expect`.
3. **The parser follows the six rules of the plan format.** Unit tests in `src/plan.rs`:
   1. Front matter: `full_front_matter`, `front_matter_only_on_line_one`, `unknown_key_names_the_key`, `defaults_without_front_matter`.
   2. Phases: `phases_numbered_by_order_with_lines`, `phase_prefix_stripped`, `setext_heading_opens_phase`.
   3. Code blocks: `heading_in_code_block_is_not_a_phase`.
   4. Context and title: `title_and_context`, `only_first_h1_is_title`.
   5. Verify blocks: `verify_blocks_removed_and_parsed`, `verify_label_must_be_exact`, `verify_before_first_phase_is_error`.
   6. Validation errors: `no_phases`, `empty_phase_body`, `duplicate_titles_case_insensitive`, `unknown_agent`, `multiple_errors_sorted_by_line`.
4. **The original branch never changes, and phasewise never pushes.** `check_invariants` in `tests/lifecycle.rs` asserts that the `main` SHA is unchanged after every flow, and `tests/git_layer.rs` checks the same. In `src/`, "push" appears only in three places: the deny rule `Bash(git push *)` given to Claude Code, the prompt text that tells the agent never to push, and a parser test string. `src/git.rs` never runs `git push`.
5. **`.phasewise/` never appears in a commit.** `check_invariants` scans `git log --all --name-only`. In `tests/git_layer.rs`, `phasewise_dir_is_never_committed` and `phasewise_dir_survives_without_exclude_entry` cover it even without the `.git/info/exclude` entry.
6. **A process interrupted mid-phase leaves a recoverable state.**
   - `tests/lifecycle.rs`: `orphaned_running_phase_is_recovered_once_and_can_be_retried`.
   - `tests/state_files.rs`: `recovery_marks_interrupted_phases`.
   - `tests/signals.rs`: one SIGTERM marks the phase `failed (interrupted)`. A second signal kills the agent's process group and exits with 130.
7. **The TUI restores the terminal on exit and on a panic.**
   - On exit: the `expect` pty check (open → `n` → tabs → `b` → `y` → `q`) shows the alternate screen left and `stty` back to `icanon echo`. It was re-run after the review fixes.
   - On a panic: `ratatui::try_init` installs a panic hook that restores the terminal (ratatui 0.30.2, `src/init.rs`), and a `Drop` guard calls `ratatui::restore()`. The panic path was not triggered at runtime.
8. **No network connections of its own.** `src/` uses no `std::net` and no HTTP/TLS client, and the dependency tree has none. `mio` is present only as crossterm's terminal-event poller. The agents phasewise launches use the network themselves.
9. **No `unsafe`, `todo!()`, `unimplemented!()` or "not implemented yet" in `src/`.** A grep finds only `#![forbid(unsafe_code)]` and a comment.
10. **The README states the real status of each agent.** Both are listed as tested, with the version that passed the real smoke test (see `docs/dev/agent-cli-notes.md`).
11. **Nothing was pushed, published or created remotely.** `git remote -v` is empty. Nothing was published to crates.io or anywhere else.

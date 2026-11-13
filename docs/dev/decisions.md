# Decisions

Minor decisions taken while building v0.1, and every place where the build
deviates from the original plan, with the reason.

## Build phase 0

- **Project folder.** The plan names `/Users/alexfyp/Documents/phasewise` as the
  project folder but its working rule 2.2.1 says `~/Developer/phasewise`. Both
  were checked: `~/Developer/phasewise` does not exist and the `Documents`
  folder existed and was empty. The `Documents` folder (the one in the plan's
  data table) is used.
- **Rust edition 2024**, the latest stable edition supported by the installed
  toolchain (rustc 1.94.0).
- **Lints enforced by the compiler.** `unsafe_code = "forbid"` in `Cargo.toml`
  plus `#![forbid(unsafe_code)]`, and `clippy::unwrap_used` /
  `clippy::expect_used` warned at crate level (errors under `-D warnings`).
  `clippy.toml` allows them in unit tests; integration tests are separate
  crates and are not affected.
- **Claude Code gets `--permission-prompts none`.** The flag exists in the
  installed 2.1.296 and guarantees that a phase never waits for a human answer.
- **Codex `--sandbox` goes before `resume`.** `codex exec resume` rejects
  `--sandbox`; `codex exec --sandbox workspace-write resume <id> -` parses.
- **`repository` in `Cargo.toml`** is `https://github.com/userfypp/phasewise`,
  from `gh api user --jq .login`. The repository does not exist yet.

## Build phase 1

- **Dependencies:** `clap` (derive) for the CLI and `anyhow` for error chains,
  both in the planned list; dev: `assert_cmd`, `predicates`.
- **clap usage errors exit with 1**, not clap's default 2, to match the
  documented exit codes (0 ok, 1 usage/environment, 3 failed phase).
- **CI actions:** `actions/checkout@v7` and `Swatinem/rust-cache@v2` (latest
  releases according to the GitHub API on 2026-11-14),
  `dtolnay/rust-toolchain@stable`.
- **Internal design** fixed in `docs/dev/design.md` before implementation,
  with two modules not in the original layout: `ops.rs` (run-level operations
  shared by the CLI and the TUI, so the TUI does not depend on the CLI) and
  `process.rs` (process groups and termination). `AgentEvent` gains an
  `Error { detail }` variant for non-final errors reported by the agent.

## Build phase 2

- **Dependencies added in phase 2:** `serde` (derive) and `toml` for the front
  matter (`deny_unknown_fields`, error spans), `pulldown-cmark` without default
  features for heading/code-block detection with byte offsets, `thiserror` for
  `PlanError`; dev: `tempfile` for test repositories.
- **Setext level-2 headings also open a phase**, as in any Markdown renderer;
  `##` inside code blocks never does.
- **Plan errors:** front-matter (TOML) errors stop parsing, because the rest
  cannot be trusted; all other validation errors are collected and reported
  together, sorted by line. "No phases" is reported at line 1.
- **Default plan path:** `phasewise.md` at the repository root, or in the
  current directory when outside a repository.

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

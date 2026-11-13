+++
# Agent that runs every phase.
agent = "claude"
# These commands run after EVERY phase; all must pass before the checkpoint.
verify = ["cargo fmt --all -- --check", "cargo clippy --all-targets -- -D warnings"]
max_retries = 1
+++

# Add a `stats` subcommand to a Rust CLI

<!-- Everything before the first `##` heading is shared context: the agent
     receives it in every phase. The `#` heading above is the plan title. -->

The project is a Rust command-line tool built with clap (derive API). Follow
the existing module layout and error handling (`anyhow`). Do not add
dependencies.

## Phase 1: Define the subcommand

<!-- Phase headings may start with "Phase N:"; the prefix is removed from the
     title and phases are always numbered 1, 2, 3... by order. -->

Add a `stats` subcommand to the clap definitions that accepts an optional
`--json` flag. For now it only prints `not implemented` and exits with code 1.

<!-- A fenced block labelled exactly `verify` holds this phase's checks, one
     command per line. The agent does not see it. -->

```verify
cargo build
cargo run -q -- stats --help
```

## Phase 2: Implement the statistics

Count the entries the tool already manages and print a short summary. With
`--json`, print the same data as one JSON object. Keep the logic in its own
function so it can be unit tested.

### Constraints

<!-- Deeper headings stay inside the phase body. -->

- No new dependencies: build the JSON by hand or reuse what is already there.
- Output goes to stdout, errors to stderr.

```verify
cargo test
```

## Phase 3: Tests and documentation

Add unit tests for the counting function and one integration test for the
subcommand. Document `stats` in the README.

```verify
cargo test
# Lines starting with # are comments and are ignored.
grep -q "stats" README.md
```

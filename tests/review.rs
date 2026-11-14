//! `phasewise review` and the no-argument entry point without a terminal.

mod common;

use common::TestRepo;
use predicates::prelude::*;

const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fakes/fake-claude.sh");
const PLAN: &str = "+++\nverify = []\n+++\n\n# Demo\n\n## Phase 1: First\n\nDo one.\n\n## Phase 2: Second\n\nDo two.\n";

/// A repository with an active run whose first phase awaits review.
fn repo_with_run() -> TestRepo {
    let repo = TestRepo::new();
    repo.write("phasewise.md", PLAN);
    repo.commit_all("add plan");
    repo.phasewise()
        .env("PHASEWISE_CLAUDE_BIN", FAKE_CLAUDE)
        .arg("run")
        .assert()
        .success();
    repo
}

#[test]
fn review_without_a_run_fails() {
    let repo = TestRepo::new();
    repo.phasewise()
        .arg("review")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "No run yet. Start one with `phasewise run`.",
        ));
}

#[test]
fn review_without_a_terminal_fails() {
    let repo = repo_with_run();
    repo.phasewise()
        .arg("review")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "phasewise review needs an interactive terminal",
        ));
}

#[test]
fn no_arguments_with_an_active_run_but_no_terminal_prints_help() {
    let repo = repo_with_run();
    repo.phasewise()
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

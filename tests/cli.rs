use assert_cmd::Command;
use predicates::prelude::*;

fn phasewise() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("phasewise"))
}

#[test]
fn help_lists_all_subcommands() {
    let mut assert = phasewise().arg("--help").assert().success();
    for name in [
        "init", "check", "doctor", "run", "next", "retry", "rollback", "status", "diff", "review",
        "abort",
    ] {
        assert = assert.stdout(predicate::str::is_match(format!(r"(?m)^\s+{name}\s")).unwrap());
    }
}

#[test]
fn version_long_flag() {
    phasewise()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("phasewise 0.1.0"));
}

#[test]
fn version_short_flag() {
    phasewise()
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains("phasewise 0.1.0"));
}

#[test]
fn run_help_has_examples() {
    phasewise()
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"));
}

#[test]
fn unknown_subcommand_exits_1() {
    phasewise().arg("bogus").assert().code(1);
}

#[test]
fn rollback_without_n_exits_1() {
    phasewise().arg("rollback").assert().code(1);
}

#[test]
fn no_arguments_prints_usage_and_succeeds() {
    phasewise()
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn review_outside_a_repository_fails_with_a_message() {
    let dir = tempfile::tempdir().expect("create temp dir");
    phasewise()
        .arg("review")
        .current_dir(dir.path())
        .env(
            "GIT_CEILING_DIRECTORIES",
            dir.path().parent().unwrap_or(dir.path()),
        )
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error:"));
}

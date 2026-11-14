//! TUI loaders against real runs created with the fake agent.

mod common;

use std::time::Duration;

use common::{TestRepo, isolated_git_env};
use phasewise::ops::{self, Run};
use phasewise::state::PhaseStatus;
use phasewise::tui::data::{load_diff, load_log, load_run_view, load_verify};
use tempfile::TempDir;

const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fakes/fake-claude.sh");
const PLAN: &str = "+++\nverify = [\"! grep -q broken phase-*.txt\"]\n+++\n\n# Demo plan\n\n\
Shared context.\n\n## Phase 1: First\n\nDo one.\n\n## Phase 2: Second\n\nDo two.\n\n\
## Phase 3: Third\n\nDo three.\n";

struct Env {
    repo: TestRepo,
    logs: TempDir,
}

impl Env {
    fn new() -> Env {
        let repo = TestRepo::new();
        repo.write("phasewise.md", PLAN);
        repo.commit_all("add plan");
        Env {
            repo,
            logs: tempfile::tempdir().expect("create log dir"),
        }
    }

    /// Runs a phasewise command with the fake agent and returns the exit code.
    fn exec(&self, args: &[&str], envs: &[(&str, &str)]) -> i32 {
        let output = self
            .repo
            .phasewise()
            .args(args)
            .env("PHASEWISE_CLAUDE_BIN", FAKE_CLAUDE)
            .env("FAKE_LOG_DIR", self.logs.path())
            .envs(envs.iter().copied())
            .timeout(Duration::from_secs(60))
            .output()
            .expect("run phasewise");
        output.status.code().expect("exit code")
    }

    fn load(&self) -> Run {
        let mut run = ops::load_current(self.repo.path())
            .expect("load run")
            .expect("a run exists");
        run.git = run.git.with_env(isolated_git_env());
        run
    }
}

#[test]
fn checkpointed_phase_shows_its_diff_log_and_verify_output() {
    let env = Env::new();
    assert_eq!(env.exec(&["run"], &[]), 0);
    let run = env.load();

    let view = load_run_view(&run);
    assert_eq!(view.phases.len(), 3);
    assert_eq!(view.phases[0].status, PhaseStatus::AwaitingReview);
    assert!(view.phases[0].has_checkpoint);
    assert!(!view.phases[1].has_checkpoint);
    assert_eq!(view.plan_title, "Demo plan");
    assert!(view.active);

    let diff = load_diff(&run, 1).expect("diff");
    assert_eq!(diff.files, [("phase-1.txt".to_string(), Some(1), Some(0))]);
    assert!(diff.text.contains("+phase 1"), "{}", diff.text);
    assert!(diff.untracked.is_empty());
    assert_eq!(diff.note, None);

    let log = load_log(&run, 1);
    assert_eq!(log[0], "── attempt 1 ──");
    assert!(log.contains(&"Working on phase 1.".to_string()), "{log:?}");
    assert!(log.contains(&"Write phase-1.txt".to_string()), "{log:?}");

    assert_ne!(load_verify(&run, 1), "No verification output yet.");
    assert_eq!(load_verify(&run, 3), "No verification output yet.");
    assert_eq!(
        load_diff(&run, 3).expect("diff").note.as_deref(),
        Some("No changes yet.")
    );
    assert!(load_log(&run, 3).is_empty());
}

#[test]
fn failed_phase_shows_the_worktree_diff_and_untracked_files() {
    let env = Env::new();
    assert_eq!(env.exec(&["run"], &[]), 0);
    assert_eq!(env.exec(&["next"], &[("FAKE_MODE_2", "never_fixes")]), 3);
    env.repo.write("README.md", "# test repo\nedited\n");
    let run = env.load();

    let view = load_run_view(&run);
    assert_eq!(view.phases[1].status, PhaseStatus::Failed);
    assert!(!view.phases[1].has_checkpoint);
    assert!(view.phases[1].failure.is_some());

    let diff = load_diff(&run, 2).expect("diff");
    assert_eq!(diff.files, [("README.md".to_string(), Some(1), Some(0))]);
    assert!(diff.text.contains("+edited"), "{}", diff.text);
    assert!(diff.untracked.contains(&"phase-2.txt".to_string()));
    assert!(!diff.untracked.contains(&"README.md".to_string()));

    let attempts = run.state.phase(2).expect("phase 2").attempts;
    let log = load_log(&run, 2);
    let separators = log.iter().filter(|l| l.starts_with("── attempt ")).count();
    assert_eq!(separators, attempts as usize);
    assert!(attempts >= 1);
}

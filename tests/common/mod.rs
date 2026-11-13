// Shared by several test crates; each one uses a different subset of the helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A temporary git repository with an isolated environment.
pub struct TestRepo {
    dir: TempDir,
}

fn isolate(cmd: &mut Command) {
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("PHASEWISE_CLAUDE_BIN")
        .env_remove("PHASEWISE_CODEX_BIN");
}

impl TestRepo {
    /// Repository on branch `main` with one commit (`README.md`).
    pub fn new() -> TestRepo {
        let repo = TestRepo::empty();
        repo.write("README.md", "# test repo\n");
        repo.commit_all("initial commit");
        repo
    }

    /// Repository with no commits.
    pub fn empty() -> TestRepo {
        let repo = TestRepo {
            dir: tempfile::tempdir().expect("create temp dir"),
        };
        repo.git(&["init", "-b", "main"]);
        repo.git(&["config", "user.name", "Test User"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, rel: &str, content: &str) {
        let path: PathBuf = self.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(path, content).expect("write file");
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path().join(rel)).expect("read file")
    }

    /// Runs git in the repository, asserts success and returns stdout.
    pub fn git(&self, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(self.path()).args(args);
        isolate(&mut cmd);
        let output = cmd.output().expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub fn commit_all(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", msg]);
    }

    /// The phasewise binary, run from the repository root.
    pub fn phasewise(&self) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::new(assert_cmd::cargo::cargo_bin!("phasewise"));
        cmd.current_dir(self.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("PHASEWISE_CLAUDE_BIN")
            .env_remove("PHASEWISE_CODEX_BIN");
        cmd
    }
}

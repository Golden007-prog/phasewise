//! The only module that runs `git`.

use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

/// A git repository, identified by its top-level directory.
#[derive(Debug, Clone)]
pub struct Git {
    root: PathBuf,
    env: Vec<(OsString, OsString)>,
}

impl Git {
    /// Finds the repository that contains `dir`.
    pub fn discover(dir: &Path) -> Result<Git> {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "--show-toplevel"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .context("Cannot run `git`. Install git and make sure it is on your PATH.")?;
        if !output.status.success() {
            return Err(anyhow!(
                "Not inside a git repository. Run `git init` or move into a repository."
            ));
        }
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Git {
            root: PathBuf::from(root),
            env: Vec::new(),
        })
    }

    /// For tests: extra environment for every git call.
    #[doc(hidden)]
    pub fn with_env<K, V>(mut self, vars: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.env.extend(
            vars.into_iter()
                .map(|(k, v)| (k.as_ref().to_owned(), v.as_ref().to_owned())),
        );
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn exclude_path(&self) -> Result<PathBuf> {
        let out = self.run(&["rev-parse", "--git-path", "info/exclude"])?;
        let rel = PathBuf::from(out.trim());
        Ok(if rel.is_absolute() {
            rel
        } else {
            self.root.join(rel)
        })
    }

    /// Whether `pattern` is a line of the exclude file. Read-only.
    pub fn is_excluded(&self, pattern: &str) -> Result<bool> {
        let path = self.exclude_path()?;
        match fs::read_to_string(&path) {
            Ok(text) => Ok(text.lines().any(|l| l == pattern)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).with_context(|| format!("Cannot read {}", path.display())),
        }
    }

    /// Makes sure `pattern` is a line of `.git/info/exclude`. Returns whether
    /// the file changed.
    pub fn ensure_excluded(&self, pattern: &str) -> Result<bool> {
        let path = self.exclude_path()?;
        let existing = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(e).with_context(|| format!("Cannot read {}", path.display()));
            }
        };
        if existing.lines().any(|l| l == pattern) {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Cannot create {}", parent.display()))?;
        }
        let mut addition = String::new();
        if !existing.is_empty() && !existing.ends_with('\n') {
            addition.push('\n');
        }
        addition.push_str(pattern);
        addition.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("Cannot open {}", path.display()))?;
        file.write_all(addition.as_bytes())
            .with_context(|| format!("Cannot write {}", path.display()))?;
        Ok(true)
    }

    /// Version string reported by `git --version` (no repository needed).
    pub fn version() -> Result<String> {
        let output = Command::new("git")
            .arg("--version")
            .output()
            .context("Cannot run `git`. Install git and make sure it is on your PATH.")?;
        if !output.status.success() {
            return Err(anyhow!(
                "`git --version` failed: {}. Check your git installation.",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Whether `HEAD` points to a commit.
    pub fn has_commits(&self) -> Result<bool> {
        let output = self.output(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"], None)?;
        Ok(output.status.success())
    }

    /// Current branch name, or `None` when `HEAD` is detached.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let output = self.output(&["symbolic-ref", "--quiet", "--short", "HEAD"], None)?;
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(Some(name).filter(|n| !n.is_empty()))
        } else if output.status.code() == Some(1) {
            Ok(None)
        } else {
            Err(failure(
                &["symbolic-ref", "--quiet", "--short", "HEAD"],
                &output,
            ))
        }
    }

    pub fn head_sha(&self) -> Result<String> {
        self.rev_parse("HEAD")
    }

    /// Resolves `rev` to a full commit SHA.
    pub fn rev_parse(&self, rev: &str) -> Result<String> {
        let spec = format!("{rev}^{{commit}}");
        Ok(self
            .run(&["rev-parse", "--verify", &spec])?
            .trim()
            .to_string())
    }

    /// Paths with changes (modified, staged or untracked).
    pub fn dirty_paths(&self) -> Result<Vec<String>> {
        let out = self.run(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
        let mut paths = Vec::new();
        let mut entries = out.split('\0').filter(|e| !e.is_empty());
        while let Some(entry) = entries.next() {
            let (Some(status), Some(path)) = (entry.get(..2), entry.get(3..)) else {
                continue;
            };
            paths.push(path.to_string());
            // Renames and copies are followed by an entry with the original path.
            if status.contains('R') || status.contains('C') {
                entries.next();
            }
        }
        Ok(paths)
    }

    pub fn is_clean(&self) -> Result<bool> {
        Ok(self.dirty_paths()?.is_empty())
    }

    pub fn branch_exists(&self, name: &str) -> Result<bool> {
        let reference = format!("refs/heads/{name}");
        let args = ["show-ref", "--verify", "--quiet", reference.as_str()];
        let output = self.output(&args, None)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(failure(&args, &output)),
        }
    }

    /// Creates `name` at `start_sha` and switches to it.
    pub fn create_branch(&self, name: &str, start_sha: &str) -> Result<()> {
        self.run(&["switch", "-c", name, start_sha]).map(|_| ())
    }

    pub fn switch(&self, branch: &str) -> Result<()> {
        self.run(&["switch", branch]).map(|_| ())
    }

    /// Stages everything except phasewise's own `.phasewise/` directory, even
    /// if it is not excluded in `.git/info/exclude`. (A pathspec exclude makes
    /// `git add` fail when the directory is ignored, so it is unstaged after.)
    pub fn add_all(&self) -> Result<()> {
        self.run(&["add", "-A"])?;
        self.run(&["reset", "-q", "--", ".phasewise"]).map(|_| ())
    }

    /// Checks that git knows who the author and committer are, so checkpoint
    /// commits will not fail later.
    pub fn check_identity(&self) -> Result<()> {
        for var in ["GIT_AUTHOR_IDENT", "GIT_COMMITTER_IDENT"] {
            let output = self.output(&["var", var], None)?;
            if !output.status.success() {
                return Err(anyhow!(
                    "git does not know who you are, so phasewise cannot create checkpoint commits. \
Set it with `git config user.name \"Your Name\"` and `git config user.email you@example.com` \
(add --global for every repository), then run phasewise again."
                ));
            }
        }
        Ok(())
    }

    /// Commits the index with `message` and returns the new `HEAD` SHA.
    pub fn commit(&self, message: &str, allow_empty: bool) -> Result<String> {
        let mut args = vec!["commit", "-q", "--no-verify", "--no-gpg-sign"];
        if allow_empty {
            args.push("--allow-empty");
        }
        args.extend(["-F", "-"]);
        let output = self.output(&args, Some(message))?;
        if !output.status.success() {
            return Err(failure(&args, &output));
        }
        self.head_sha()
    }

    /// Commits only `path` (relative to the repository root) with `message`
    /// and returns the short SHA of the new `HEAD`. This is the user's own
    /// commit: their hooks and signing apply.
    pub fn commit_file(&self, path: &str, message: &str) -> Result<String> {
        self.run(&["add", "--", path])?;
        let args = ["commit", "-q", "-F", "-", "--", path];
        let output = self.output(&args, Some(message))?;
        if !output.status.success() {
            return Err(failure(&args, &output));
        }
        Ok(self
            .run(&["rev-parse", "--short", "HEAD"])?
            .trim()
            .to_string())
    }

    pub fn reset_hard(&self, sha: &str) -> Result<()> {
        self.run(&["reset", "--hard", sha]).map(|_| ())
    }

    /// Removes untracked files and directories (ignored files are kept;
    /// `.phasewise/` at the repository root is never removed).
    pub fn clean_untracked(&self) -> Result<()> {
        self.run(&["clean", "-fd", "-e", "/.phasewise/"])
            .map(|_| ())
    }

    pub fn diff(&self, from: &str, to: &str) -> Result<String> {
        self.run(&["diff", "--no-color", "--no-ext-diff", from, to])
    }

    /// Tracked files against the working tree (untracked files are not included).
    pub fn diff_worktree(&self, from: &str) -> Result<String> {
        self.run(&["diff", "--no-color", "--no-ext-diff", from])
    }

    /// Per-file added/removed line counts between two commits.
    pub fn numstat(&self, from: &str, to: &str) -> Result<Vec<FileStat>> {
        self.numstat_args(&[from, to])
    }

    /// Per-file line counts of tracked files against the working tree.
    pub fn numstat_worktree(&self, from: &str) -> Result<Vec<FileStat>> {
        self.numstat_args(&[from])
    }

    fn numstat_args(&self, revs: &[&str]) -> Result<Vec<FileStat>> {
        let mut args = vec![
            "-c",
            "core.quotepath=off",
            "diff",
            "--numstat",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
        ];
        args.extend_from_slice(revs);
        let out = self.run(&args)?;
        let mut stats = Vec::new();
        for line in out.lines() {
            let mut parts = line.splitn(3, '\t');
            if let (Some(added), Some(removed), Some(path)) =
                (parts.next(), parts.next(), parts.next())
            {
                stats.push(FileStat {
                    path: path.to_string(),
                    added: added.parse().ok(),
                    removed: removed.parse().ok(),
                });
            }
        }
        Ok(stats)
    }

    pub fn changed_file_count(&self, from: &str, to: &str) -> Result<usize> {
        Ok(self.numstat(from, to)?.len())
    }

    /// Runs `git -C <root> <args>` and returns stdout; failures include the
    /// arguments and stderr.
    fn run(&self, args: &[&str]) -> Result<String> {
        let output = self.output(args, None)?;
        if !output.status.success() {
            return Err(failure(args, &output));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Runs git with captured output and optional stdin; does not check the
    /// status. A command that fails because `index.lock` exists is run again a
    /// few times (git fails before changing anything in that case).
    fn output(&self, args: &[&str], stdin: Option<&str>) -> Result<Output> {
        let mut waits = LOCK_RETRY_WAITS_MS.iter();
        loop {
            let output = self.output_once(args, stdin)?;
            if output.status.success() || !is_lock_contention(&output) {
                return Ok(output);
            }
            match waits.next() {
                Some(ms) => std::thread::sleep(Duration::from_millis(*ms)),
                None => return Ok(output),
            }
        }
    }

    fn output_once(&self, args: &[&str], stdin: Option<&str>) -> Result<Output> {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = command
            .spawn()
            .with_context(|| format!("Cannot run `git {}`", args.join(" ")))?;
        if let Some(input) = stdin
            && let Some(mut pipe) = child.stdin.take()
        {
            pipe.write_all(input.as_bytes())
                .with_context(|| format!("Cannot write to `git {}`", args.join(" ")))?;
        }
        child
            .wait_with_output()
            .with_context(|| format!("Cannot run `git {}`", args.join(" ")))
    }
}

/// Waits between attempts when `index.lock` is held (about 3.5 s in total).
const LOCK_RETRY_WAITS_MS: [u64; 7] = [50, 100, 200, 400, 800, 1000, 1000];

/// True when git could not take `index.lock`. Only the file name is matched:
/// the rest of git's message is translated in other locales.
fn is_lock_contention(output: &Output) -> bool {
    String::from_utf8_lossy(&output.stderr).contains("index.lock")
}

/// The lock file named in git's message (`Unable to create '<path>': File
/// exists.` in English), whatever the language of the message.
fn lock_path(stderr: &str) -> String {
    const NAME: &str = "index.lock";
    stderr
        .find(NAME)
        .and_then(|end| {
            let end = end + NAME.len();
            let start = stderr[..end].rfind(['\'', '"', '‘', '«', '`'])? + 1;
            let start = start + stderr[start..].find(|c: char| !c.is_whitespace())?;
            Some(stderr[start..end].to_string())
        })
        .unwrap_or_else(|| ".git/index.lock".to_string())
}

fn failure(args: &[&str], output: &Output) -> anyhow::Error {
    if is_lock_contention(output) {
        let path = lock_path(&String::from_utf8_lossy(&output.stderr));
        return anyhow!(
            "`git {}` failed: another git process is using the repository ({path} exists). \
Wait for it to finish or close the tool that runs it (an editor's git integration, for example). \
If no git command is running, the lock is left over from a crash: delete {path} and try again.",
            args.join(" ")
        );
    }
    anyhow!(
        "`git {}` failed: {}. Fix the problem in the repository and try again.",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

/// Added/removed line counts of one file; `None` for binary files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

/// Commit message of a phase checkpoint.
pub fn checkpoint_message(run_id: &str, phase: usize, title: &str) -> String {
    format!(
        "phasewise: phase {phase} — {title}\n\nPhasewise-Run: {run_id}\nPhasewise-Phase: {phase}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV: [(&str, &str); 6] = [
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_AUTHOR_NAME", "T"),
        ("GIT_AUTHOR_EMAIL", "t@example.com"),
        ("GIT_COMMITTER_NAME", "T"),
        ("GIT_COMMITTER_EMAIL", "t@example.com"),
    ];

    fn repo() -> (tempfile::TempDir, Git) {
        let dir = tempfile::tempdir().expect("tempdir");
        let init = Command::new("git")
            .args(["init", "-q"])
            .arg(dir.path())
            .envs(ENV)
            .output()
            .expect("git init");
        assert!(init.status.success());
        for (k, v) in [("user.name", "T"), ("user.email", "t@example.com")] {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["config", k, v])
                .envs(ENV)
                .output()
                .expect("git config");
            assert!(out.status.success());
        }
        let git = Git::discover(dir.path()).expect("discover").with_env(ENV);
        (dir, git)
    }

    fn lock(git: &Git) -> PathBuf {
        git.root().join(".git").join("index.lock")
    }

    #[test]
    fn add_all_retries_while_the_lock_is_held() {
        let (dir, git) = repo();
        fs::write(dir.path().join("a.txt"), "a").expect("write");
        let lock = lock(&git);
        fs::write(&lock, "").expect("lock");
        let to_remove = lock.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let _ = fs::remove_file(to_remove);
        });
        git.add_all().expect("add_all should wait for the lock");
        handle.join().expect("join");
    }

    #[test]
    fn stale_lock_error_names_the_lock_and_how_to_fix_it() {
        let (dir, git) = repo();
        fs::write(dir.path().join("a.txt"), "a").expect("write");
        let lock = lock(&git);
        fs::write(&lock, "").expect("lock");
        let err = git.add_all().expect_err("must fail").to_string();
        assert!(err.contains("index.lock"), "{err}");
        assert!(err.contains("delete"), "{err}");
        assert!(err.contains("another git process"), "{err}");
    }

    #[test]
    fn lock_path_is_found_in_any_language() {
        let english = "fatal: Unable to create '/r/.git/index.lock': File exists.\n";
        assert_eq!(lock_path(english), "/r/.git/index.lock");
        let spanish = "fatal: No se pudo crear '/r/.git/index.lock': El archivo existe.";
        assert_eq!(lock_path(spanish), "/r/.git/index.lock");
        assert_eq!(lock_path("fatal: something else"), ".git/index.lock");
    }

    #[test]
    fn commit_file_commits_only_that_file() {
        let (dir, git) = repo();
        fs::write(dir.path().join("plan.md"), "plan").expect("write");
        fs::write(dir.path().join("other.txt"), "other").expect("write");
        git.run(&["add", "--", "other.txt"]).expect("stage other");
        git.commit_file("plan.md", "Add phasewise plan")
            .expect("commit_file");
        let tracked = git
            .run(&["ls-tree", "-r", "--name-only", "HEAD"])
            .expect("ls-tree");
        assert_eq!(tracked.trim(), "plan.md");
        let status = git.run(&["status", "--porcelain"]).expect("status");
        assert_eq!(status.trim(), "A  other.txt");
    }

    #[test]
    fn commit_file_returns_the_short_sha() {
        let (dir, git) = repo();
        fs::write(dir.path().join("plan.md"), "plan").expect("write");
        let sha = git
            .commit_file("plan.md", "Add phasewise plan")
            .expect("commit");
        let expected = git
            .run(&["rev-parse", "--short", "HEAD"])
            .expect("rev-parse");
        assert_eq!(sha, expected.trim());
        assert!(!sha.is_empty());
    }
}

//! The only module that runs `git`.

use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

    pub fn add_all(&self) -> Result<()> {
        self.run(&["add", "-A"]).map(|_| ())
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

    pub fn reset_hard(&self, sha: &str) -> Result<()> {
        self.run(&["reset", "--hard", sha]).map(|_| ())
    }

    /// Removes untracked files and directories (ignored files are kept).
    pub fn clean_untracked(&self) -> Result<()> {
        self.run(&["clean", "-fd"]).map(|_| ())
    }

    pub fn diff(&self, from: &str, to: &str) -> Result<String> {
        self.run(&["diff", "--no-color", "--no-ext-diff", from, to])
    }

    /// Per-file added/removed line counts between two commits.
    pub fn numstat(&self, from: &str, to: &str) -> Result<Vec<FileStat>> {
        let out = self.run(&[
            "-c",
            "core.quotepath=off",
            "diff",
            "--numstat",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            from,
            to,
        ])?;
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

    /// Runs git with captured output and optional stdin; does not check the status.
    fn output(&self, args: &[&str], stdin: Option<&str>) -> Result<Output> {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.root)
            .args(args)
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

fn failure(args: &[&str], output: &Output) -> anyhow::Error {
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

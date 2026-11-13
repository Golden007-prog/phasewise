//! The only module that runs `git`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, anyhow};

/// A git repository, identified by its top-level directory.
#[derive(Debug, Clone)]
pub struct Git {
    root: PathBuf,
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
                "Not inside a git repository. Run `git init` first."
            ));
        }
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Git {
            root: PathBuf::from(root),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Makes sure `pattern` is a line of `.git/info/exclude`. Returns whether
    /// the file changed.
    pub fn ensure_excluded(&self, pattern: &str) -> Result<bool> {
        let out = self.run(&["rev-parse", "--git-path", "info/exclude"])?;
        let rel = PathBuf::from(out.trim());
        let path = if rel.is_absolute() {
            rel
        } else {
            self.root.join(rel)
        };
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

    /// Runs `git -C <root> <args>` and returns stdout; failures include the
    /// arguments and stderr.
    fn run(&self, args: &[&str]) -> Result<String> {
        let output: Output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .with_context(|| format!("Cannot run `git {}`", args.join(" ")))?;
        if !output.status.success() {
            return Err(anyhow!(
                "`git {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

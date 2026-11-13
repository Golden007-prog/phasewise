//! Agent adapters: command lines and translation of the agent's stdout into
//! normalized events.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};

use crate::plan::AgentKind;

pub mod claude;
pub mod codex;

const MAX_DETAIL_CHARS: usize = 200;

/// What the agent reported, independent of the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    SessionStarted {
        session_id: String,
    },
    Message {
        text: String,
        subagent: bool,
    },
    /// `detail` is for example a relative path or the first line of a command.
    ToolCall {
        name: String,
        detail: String,
        subagent: bool,
    },
    Retry {
        detail: String,
    },
    PermissionDenied {
        detail: String,
    },
    /// Non-final error reported by the agent.
    Error {
        detail: String,
    },
    Finished {
        ok: bool,
        summary: String,
        cost_usd: Option<f64>,
    },
}

/// Translates stdout lines into events. Never fails: anything it does not
/// understand is ignored.
pub trait AgentParser: Send {
    fn feed(&mut self, line: &str) -> Vec<AgentEvent>;
}

pub trait Agent: Send + Sync {
    fn kind(&self) -> AgentKind;
    /// `PHASEWISE_CLAUDE_BIN` / `PHASEWISE_CODEX_BIN` when set and non-empty,
    /// else `claude` / `codex`.
    fn program(&self) -> OsString;
    /// First non-empty line of `<program> --version`.
    fn version(&self) -> Result<String>;
    /// The prompt goes to stdin; stdio is configured by the caller.
    fn new_session_command(&self, root: &Path, extra_args: &[String]) -> Command;
    fn resume_command(&self, root: &Path, session_id: &str, extra_args: &[String]) -> Command;
    /// `root` is used to shorten absolute paths in tool details.
    fn parser(&self, root: &Path) -> Box<dyn AgentParser>;
}

pub fn agent_for(kind: AgentKind) -> Box<dyn Agent> {
    match kind {
        AgentKind::Claude => Box::new(claude::ClaudeAgent),
        AgentKind::Codex => Box::new(codex::CodexAgent),
    }
}

/// Program name from the value of an override variable (ignored when empty).
fn program_from(value: Option<OsString>, default: &str) -> OsString {
    match value {
        Some(v) if !v.is_empty() => v,
        _ => OsString::from(default),
    }
}

fn program_from_env(var: &str, default: &str) -> OsString {
    program_from(std::env::var_os(var), default)
}

/// Runs `<program> --version` and returns the first non-empty line.
fn program_version(program: &OsString, not_found: &str) -> Result<String> {
    let output = match Command::new(program).arg("--version").output() {
        Ok(output) => output,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(anyhow!("{not_found}")),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("Cannot run `{} --version`", program.to_string_lossy()));
        }
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "`{} --version` printed nothing (exit status {}). Check the installation.",
                program.to_string_lossy(),
                output.status
            )
        })
}

/// Shortens a tool detail for display: absolute paths under `root` become
/// relative, only the first line is kept, at most 200 characters.
fn tidy_detail(raw: &str, root: &Path) -> String {
    let mut text = raw.to_string();
    let root_str = root.to_string_lossy();
    let root_str = root_str.trim_end_matches('/');
    if !root_str.is_empty() {
        text = text.replace(&format!("{root_str}/"), "");
        if text == root_str {
            text = ".".to_string();
        }
    }
    let first = text.lines().map(str::trim).find(|l| !l.is_empty());
    first.unwrap_or("").chars().take(MAX_DETAIL_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_override_ignores_empty() {
        assert_eq!(program_from(None, "claude"), OsString::from("claude"));
        assert_eq!(
            program_from(Some(OsString::new()), "claude"),
            OsString::from("claude")
        );
        assert_eq!(
            program_from(Some(OsString::from("/x/fake")), "claude"),
            OsString::from("/x/fake")
        );
    }

    #[test]
    fn detail_is_relative_first_line_and_short() {
        let root = Path::new("/work/repo");
        assert_eq!(tidy_detail("/work/repo/src/a.rs", root), "src/a.rs");
        assert_eq!(tidy_detail("/work/repo", root), ".");
        assert_eq!(tidy_detail("/other/a.rs", root), "/other/a.rs");
        assert_eq!(tidy_detail("echo one\necho two", root), "echo one");
        assert_eq!(tidy_detail("cat /work/repo/a.txt", root), "cat a.txt");
        let long = "é".repeat(500);
        assert_eq!(tidy_detail(&long, root).chars().count(), 200);
    }

    #[test]
    fn version_reports_missing_binary() {
        let err = program_version(
            &OsString::from("/nonexistent/phasewise-fake-agent"),
            "missing",
        )
        .expect_err("must fail");
        assert_eq!(err.to_string(), "missing");
    }
}

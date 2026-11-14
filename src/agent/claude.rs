//! Claude Code adapter (`claude -p --output-format stream-json --verbose`).

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use serde_json::Value;

use super::{Agent, AgentEvent, AgentParser, program_from_env, program_version, tidy_detail};
use crate::plan::AgentKind;

const TOOL_DETAIL_KEYS: [&str; 6] = [
    "file_path",
    "path",
    "command",
    "pattern",
    "url",
    "description",
];

pub struct ClaudeAgent;

impl ClaudeAgent {
    fn base_command(&self, root: &Path) -> Command {
        let mut cmd = Command::new(self.program());
        cmd.current_dir(root)
            .args(["-p", "--output-format", "stream-json", "--verbose"])
            .args(["--permission-mode", "acceptEdits"])
            .args(["--permission-prompts", "none"])
            .args(["--allowedTools", "Read,Edit,Write,Bash"])
            .args(["--disallowedTools", "Bash(git push *)"]);
        cmd
    }
}

impl Agent for ClaudeAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Claude
    }

    fn program(&self) -> OsString {
        program_from_env("PHASEWISE_CLAUDE_BIN", "claude")
    }

    fn version(&self) -> Result<String> {
        program_version(
            &self.program(),
            "`claude` was not found. Install Claude Code or set PHASEWISE_CLAUDE_BIN.",
        )
    }

    fn new_session_command(&self, root: &Path, extra_args: &[String]) -> Command {
        let mut cmd = self.base_command(root);
        cmd.args(extra_args);
        cmd
    }

    fn resume_command(&self, root: &Path, session_id: &str, extra_args: &[String]) -> Command {
        let mut cmd = self.base_command(root);
        cmd.args(["--resume", session_id]).args(extra_args);
        cmd
    }

    fn parser(&self, root: &Path) -> Box<dyn AgentParser> {
        Box::new(ClaudeParser::new(root))
    }
}

pub struct ClaudeParser {
    root: PathBuf,
    session_seen: bool,
    denied: HashSet<String>,
}

impl ClaudeParser {
    pub fn new(root: &Path) -> Self {
        ClaudeParser {
            root: root.to_path_buf(),
            session_seen: false,
            denied: HashSet::new(),
        }
    }

    /// First useful string among the known keys of a tool input.
    fn input_detail(&self, input: &Value) -> String {
        TOOL_DETAIL_KEYS
            .iter()
            .filter_map(|key| input.get(*key).and_then(Value::as_str))
            .map(|s| tidy_detail(s, &self.root))
            .find(|s| !s.is_empty())
            .unwrap_or_default()
    }

    /// Reports a denied tool use once per tool use id.
    fn denial(&mut self, value: &Value, out: &mut Vec<AgentEvent>) {
        if let Some(id) = value.get("tool_use_id").and_then(Value::as_str)
            && !self.denied.insert(id.to_string())
        {
            return;
        }
        let name = value
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("tool");
        let detail = value
            .get("tool_input")
            .map(|input| self.input_detail(input))
            .unwrap_or_default();
        let detail = if detail.is_empty() {
            name.to_string()
        } else {
            format!("{name} {detail}")
        };
        out.push(AgentEvent::PermissionDenied { detail });
    }

    fn assistant(&mut self, value: &Value, out: &mut Vec<AgentEvent>) {
        let subagent = value
            .get("parent_tool_use_id")
            .is_some_and(Value::is_string);
        let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) else {
            return;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str)
                        && !text.trim().is_empty()
                    {
                        out.push(AgentEvent::Message {
                            text: text.to_string(),
                            subagent,
                        });
                    }
                }
                Some("tool_use") => {
                    let Some(name) = block.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let detail = block
                        .get("input")
                        .map(|input| self.input_detail(input))
                        .unwrap_or_default();
                    out.push(AgentEvent::ToolCall {
                        name: name.to_string(),
                        detail,
                        subagent,
                    });
                }
                _ => {}
            }
        }
    }

    fn retry_detail(value: &Value) -> String {
        let error = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("API error");
        match value.get("attempt").and_then(Value::as_u64) {
            Some(attempt) => match value.get("max_retries").and_then(Value::as_u64) {
                Some(max) => format!("attempt {attempt}/{max}: {error}"),
                None => format!("attempt {attempt}: {error}"),
            },
            None => error.to_string(),
        }
    }
}

impl AgentParser for ClaudeParser {
    fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return out;
        };
        if !value.is_object() {
            return out;
        }
        if !self.session_seen
            && let Some(id) = value.get("session_id").and_then(Value::as_str)
            && !id.is_empty()
        {
            self.session_seen = true;
            out.push(AgentEvent::SessionStarted {
                session_id: id.to_string(),
            });
        }
        let kind = value.get("type").and_then(Value::as_str);
        let subtype = value.get("subtype").and_then(Value::as_str);
        match (kind, subtype) {
            (Some("system"), Some("api_retry")) => out.push(AgentEvent::Retry {
                detail: Self::retry_detail(&value),
            }),
            (Some("system"), Some("permission_denied")) | (Some("permission_denied"), _) => {
                self.denial(&value, &mut out)
            }
            (Some("assistant"), _) => self.assistant(&value, &mut out),
            (Some("result"), _) => {
                if let Some(denials) = value.get("permission_denials").and_then(Value::as_array) {
                    for denial in denials {
                        self.denial(denial, &mut out);
                    }
                }
                let is_error = value
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // Success results carry `result`; error subtypes carry
                // `errors: string[]` instead (Agent SDK reference).
                let summary = match value.get("result").and_then(Value::as_str) {
                    Some(text) => text.to_string(),
                    None => value
                        .get("errors")
                        .and_then(Value::as_array)
                        .map(|errors| {
                            errors
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("; ")
                        })
                        .unwrap_or_default(),
                };
                out.push(AgentEvent::Finished {
                    ok: !is_error && subtype == Some("success"),
                    summary,
                    cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
                });
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASIC: &str = include_str!("../../tests/fixtures/claude-basic.synthetic.jsonl");
    const ERROR: &str = include_str!("../../tests/fixtures/claude-error.synthetic.jsonl");
    const SID: &str = "11111111-2222-3333-4444-555555555555";

    fn parse_all(text: &str) -> Vec<AgentEvent> {
        let mut parser = ClaudeParser::new(Path::new("/work/repo"));
        text.lines().flat_map(|l| parser.feed(l)).collect()
    }

    fn call(name: &str, detail: &str, subagent: bool) -> AgentEvent {
        AgentEvent::ToolCall {
            name: name.into(),
            detail: detail.into(),
            subagent,
        }
    }

    #[test]
    fn error_result_uses_errors_array() {
        let mut parser = ClaudeParser::new(Path::new("/work/repo"));
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"s","total_cost_usd":0.5,"permission_denials":[],"errors":["Tool failed","Aborted"]}"#;
        assert_eq!(
            parser.feed(line),
            vec![
                AgentEvent::SessionStarted {
                    session_id: "s".into()
                },
                AgentEvent::Finished {
                    ok: false,
                    summary: "Tool failed; Aborted".into(),
                    cost_usd: Some(0.5)
                }
            ]
        );
    }

    #[test]
    fn basic_fixture_events() {
        let want = vec![
            AgentEvent::SessionStarted {
                session_id: SID.into(),
            },
            AgentEvent::Message {
                text: "I'll create the file.".into(),
                subagent: false,
            },
            call("Write", "src/hello.txt", false),
            call("Bash", "echo one", false),
            AgentEvent::Message {
                text: "Subagent checking the README.".into(),
                subagent: true,
            },
            call("Read", "README.md", true),
            AgentEvent::Retry {
                detail: "attempt 1/3: overloaded".into(),
            },
            AgentEvent::PermissionDenied {
                detail: "Bash git push origin main".into(),
            },
            // The result repeats toolu_denied_1: only the new denial appears.
            AgentEvent::PermissionDenied {
                detail: "Edit .env".into(),
            },
            AgentEvent::Finished {
                ok: true,
                summary: "Created hello.txt and ran the tests.\nAll good.".into(),
                cost_usd: Some(0.0421),
            },
        ];
        assert_eq!(parse_all(BASIC), want);
    }

    #[test]
    fn error_fixture_finishes_not_ok() {
        let events = parse_all(ERROR);
        assert_eq!(
            events.last(),
            Some(&AgentEvent::Finished {
                ok: false,
                summary: "API Error: 500 internal server error".into(),
                cost_usd: Some(0.0),
            })
        );
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn garbage_is_ignored() {
        let mut parser = ClaudeParser::new(Path::new("/work/repo"));
        for line in ["", "not json", "[1,2]", "42", "{}", r#"{"type":7}"#] {
            assert!(parser.feed(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn non_success_subtype_is_not_ok() {
        let mut parser = ClaudeParser::new(Path::new("/work/repo"));
        let events =
            parser.feed(r#"{"type":"result","subtype":"error_max_turns","is_error":false}"#);
        assert_eq!(
            events,
            vec![AgentEvent::Finished {
                ok: false,
                summary: String::new(),
                cost_usd: None
            }]
        );
    }

    #[test]
    fn null_parent_is_not_subagent_and_detail_priority() {
        let mut parser = ClaudeParser::new(Path::new("/work/repo"));
        let events = parser.feed(
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","name":"Grep","input":{"pattern":"fn main","description":"x"}}]}}"#,
        );
        assert_eq!(events, vec![call("Grep", "fn main", false)]);
    }

    #[test]
    fn commands_match_the_notes() {
        let agent = ClaudeAgent;
        let extra = vec!["--max-budget-usd".to_string(), "5".to_string()];
        let new = agent.new_session_command(Path::new("/work/repo"), &extra);
        let args: Vec<_> = new
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "acceptEdits",
                "--permission-prompts",
                "none",
                "--allowedTools",
                "Read,Edit,Write,Bash",
                "--disallowedTools",
                "Bash(git push *)",
                "--max-budget-usd",
                "5"
            ]
        );
        assert_eq!(new.get_current_dir(), Some(Path::new("/work/repo")));
        let resume = agent.resume_command(Path::new("/work/repo"), "abc", &extra);
        let args: Vec<_> = resume
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[12..], ["--resume", "abc", "--max-budget-usd", "5"]);
    }
}

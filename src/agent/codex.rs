//! Codex adapter (`codex exec --json`).

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use serde_json::Value;

use super::{Agent, AgentEvent, AgentParser, program_from_env, program_version, tidy_detail};
use crate::plan::AgentKind;

pub struct CodexAgent;

impl CodexAgent {
    fn base_command(&self, root: &Path) -> Command {
        let mut cmd = Command::new(self.program());
        cmd.current_dir(root)
            .args(["exec", "--json", "--sandbox", "workspace-write"]);
        cmd
    }
}

impl Agent for CodexAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Codex
    }

    fn program(&self) -> OsString {
        program_from_env("PHASEWISE_CODEX_BIN", "codex")
    }

    fn version(&self) -> Result<String> {
        program_version(
            &self.program(),
            "`codex` was not found. Install Codex or set PHASEWISE_CODEX_BIN.",
        )
    }

    fn new_session_command(&self, root: &Path, extra_args: &[String]) -> Command {
        let mut cmd = self.base_command(root);
        cmd.arg("-").args(extra_args);
        cmd
    }

    fn resume_command(&self, root: &Path, session_id: &str, extra_args: &[String]) -> Command {
        let mut cmd = self.base_command(root);
        cmd.args(["resume", session_id, "-"]).args(extra_args);
        cmd
    }

    fn parser(&self, root: &Path) -> Box<dyn AgentParser> {
        Box::new(CodexParser::new(root))
    }
}

pub struct CodexParser {
    root: PathBuf,
    session_seen: bool,
    seen_items: HashSet<String>,
    last_message: String,
}

impl CodexParser {
    pub fn new(root: &Path) -> Self {
        CodexParser {
            root: root.to_path_buf(),
            session_seen: false,
            seen_items: HashSet::new(),
            last_message: String::new(),
        }
    }

    fn item(&mut self, item: &Value, out: &mut Vec<AgentEvent>) {
        let kind = item.get("type").and_then(Value::as_str);
        let text_of = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default();
        let id = item.get("id").and_then(Value::as_str);

        // A message is only reported once it has text; the others on first sight.
        if kind == Some("agent_message") {
            let text = text_of("text");
            if text.trim().is_empty() {
                return;
            }
            self.last_message = text.to_string();
            if self.first_time(id) {
                out.push(AgentEvent::Message {
                    text: text.to_string(),
                    subagent: false,
                });
            }
            return;
        }
        let (name, detail) = match kind {
            Some("command_execution") => ("Shell", tidy_detail(text_of("command"), &self.root)),
            Some("file_change") => {
                let paths: Vec<String> = item
                    .get("changes")
                    .and_then(Value::as_array)
                    .map(|changes| {
                        changes
                            .iter()
                            .filter_map(|c| c.get("path").and_then(Value::as_str))
                            .map(|p| tidy_detail(p, &self.root))
                            .collect()
                    })
                    .unwrap_or_default();
                ("Edit", paths.join(", ").chars().take(200).collect())
            }
            Some("mcp_tool_call") => (
                "MCP",
                tidy_detail(
                    &format!("{}/{}", text_of("server"), text_of("tool")),
                    &self.root,
                ),
            ),
            Some("web_search") => ("WebSearch", tidy_detail(text_of("query"), &self.root)),
            _ => return,
        };
        if self.first_time(id) {
            out.push(AgentEvent::ToolCall {
                name: name.to_string(),
                detail,
                subagent: false,
            });
        }
    }

    /// True the first time an item id is seen (items without id always count).
    fn first_time(&mut self, id: Option<&str>) -> bool {
        match id {
            Some(id) => self.seen_items.insert(id.to_string()),
            None => true,
        }
    }
}

impl AgentParser for CodexParser {
    fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return out;
        };
        if !value.is_object() {
            return out;
        }
        if !self.session_seen
            && let Some(id) = value.get("thread_id").and_then(Value::as_str)
            && !id.is_empty()
        {
            self.session_seen = true;
            out.push(AgentEvent::SessionStarted {
                session_id: id.to_string(),
            });
        }
        match value.get("type").and_then(Value::as_str) {
            Some("item.started") | Some("item.completed") => {
                if let Some(item) = value.get("item") {
                    self.item(item, &mut out);
                }
            }
            Some("turn.completed") => out.push(AgentEvent::Finished {
                ok: true,
                summary: self.last_message.clone(),
                cost_usd: None,
            }),
            Some("turn.failed") => {
                let summary = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("The turn failed.");
                out.push(AgentEvent::Finished {
                    ok: false,
                    summary: summary.to_string(),
                    cost_usd: None,
                });
            }
            Some("error") => {
                let detail = value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown error reported by Codex.");
                out.push(AgentEvent::Error {
                    detail: detail.to_string(),
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

    const BASIC: &str = include_str!("../../tests/fixtures/codex-basic.synthetic.jsonl");
    const FAILED: &str = include_str!("../../tests/fixtures/codex-failed.synthetic.jsonl");
    const TID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    fn parse_all(text: &str) -> Vec<AgentEvent> {
        let mut parser = CodexParser::new(Path::new("/work/repo"));
        text.lines().flat_map(|l| parser.feed(l)).collect()
    }

    fn call(name: &str, detail: &str) -> AgentEvent {
        AgentEvent::ToolCall {
            name: name.into(),
            detail: detail.into(),
            subagent: false,
        }
    }

    #[test]
    fn basic_fixture_events() {
        let want = vec![
            AgentEvent::SessionStarted {
                session_id: TID.into(),
            },
            call("Shell", "/bin/zsh -lc 'ls src'"),
            call("Edit", "src/hello.txt, README.md"),
            AgentEvent::Message {
                text: "Created hello.txt.\nDone.".into(),
                subagent: false,
            },
            AgentEvent::Finished {
                ok: true,
                summary: "Created hello.txt.\nDone.".into(),
                cost_usd: None,
            },
        ];
        assert_eq!(parse_all(BASIC), want);
    }

    #[test]
    fn failed_fixture_events() {
        let want = vec![
            AgentEvent::SessionStarted {
                session_id: TID.into(),
            },
            AgentEvent::Error {
                detail: "stream disconnected, retrying".into(),
            },
            AgentEvent::Finished {
                ok: false,
                summary: "model refused the request".into(),
                cost_usd: None,
            },
        ];
        assert_eq!(parse_all(FAILED), want);
    }

    #[test]
    fn mcp_and_web_search_items() {
        let mut parser = CodexParser::new(Path::new("/work/repo"));
        let mcp = r#"{"type":"item.started","item":{"id":"i1","type":"mcp_tool_call","server":"docs","tool":"search"}}"#;
        let web = r#"{"type":"item.completed","item":{"id":"i2","type":"web_search","query":"rust process groups"}}"#;
        assert_eq!(parser.feed(mcp), vec![call("MCP", "docs/search")]);
        assert!(
            parser
                .feed(&mcp.replace("item.started", "item.completed"))
                .is_empty()
        );
        assert_eq!(
            parser.feed(web),
            vec![call("WebSearch", "rust process groups")]
        );
    }

    #[test]
    fn message_started_empty_then_completed() {
        let mut parser = CodexParser::new(Path::new("/work/repo"));
        let started =
            r#"{"type":"item.started","item":{"id":"m","type":"agent_message","text":""}}"#;
        let done =
            r#"{"type":"item.completed","item":{"id":"m","type":"agent_message","text":"hi"}}"#;
        assert!(parser.feed(started).is_empty());
        assert_eq!(
            parser.feed(done),
            vec![AgentEvent::Message {
                text: "hi".into(),
                subagent: false
            }]
        );
        assert!(parser.feed(done).is_empty());
    }

    #[test]
    fn turn_completed_without_message_has_empty_summary() {
        let mut parser = CodexParser::new(Path::new("/work/repo"));
        assert_eq!(
            parser.feed(r#"{"type":"turn.completed"}"#),
            vec![AgentEvent::Finished {
                ok: true,
                summary: String::new(),
                cost_usd: None
            }]
        );
    }

    #[test]
    fn garbage_is_ignored() {
        let mut parser = CodexParser::new(Path::new("/work/repo"));
        for line in ["", "nope", "[]", "{}", r#"{"type":"item.completed"}"#] {
            assert!(parser.feed(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn commands_match_the_notes() {
        let agent = CodexAgent;
        let extra = vec!["-c".to_string(), "x=1".to_string()];
        let new = agent.new_session_command(Path::new("/work/repo"), &extra);
        let args: Vec<_> = new
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "exec",
                "--json",
                "--sandbox",
                "workspace-write",
                "-",
                "-c",
                "x=1"
            ]
        );
        assert_eq!(new.get_current_dir(), Some(Path::new("/work/repo")));
        let resume = agent.resume_command(Path::new("/work/repo"), "tid", &extra);
        let args: Vec<_> = resume
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "exec",
                "--json",
                "--sandbox",
                "workspace-write",
                "resume",
                "tid",
                "-",
                "-c",
                "x=1"
            ]
        );
    }
}

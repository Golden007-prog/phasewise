//! Formatting of engine events and run states, shared by the CLI and the TUI.

use crate::agent::AgentEvent;
use crate::engine::EngineEvent;
use crate::state::{FailureKind, PhaseStatus};
use crate::style::{Role, Styler};

const MAX_MESSAGE_CHARS: usize = 160;

pub fn first_line_short(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    line.unwrap_or("").chars().take(MAX_MESSAGE_CHARS).collect()
}

/// Width the phase header is padded to, so its right part lines up.
const HEADER_WIDTH: usize = 80;
/// Labels are padded to this width in event lines.
const LABEL_WIDTH: usize = 8;

/// One line of progress, structured so that the CLI (ANSI codes) and the TUI
/// (ratatui styles) render the same content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLine {
    /// Seconds since the run started; `None` where there is no clock (TUI log).
    pub time: Option<u64>,
    pub icon: &'static str,
    pub label: String,
    pub detail: String,
    pub role: Role,
}

impl EventLine {
    fn new(icon: &'static str, label: &str, detail: impl Into<String>, role: Role) -> EventLine {
        EventLine {
            time: None,
            icon,
            label: label.to_string(),
            detail: detail.into(),
            role,
        }
    }

    pub fn at(mut self, secs: u64) -> EventLine {
        self.time = Some(secs);
        self
    }

    /// `label` padded to its column, or empty when the line has no label.
    pub fn label_column(&self) -> String {
        if self.label.is_empty() {
            String::new()
        } else {
            format!("{:<LABEL_WIDTH$}", self.label)
        }
    }

    /// Detail is coloured only for warnings and failures; otherwise it stays plain.
    pub fn detail_role(&self) -> Role {
        match self.role {
            Role::Warn | Role::Fail => self.role,
            _ => Role::Default,
        }
    }

    /// `  mm:ss  <icon> <label padded to 8> <detail>`.
    pub fn render(&self, styler: &Styler) -> String {
        let mut out = String::from("  ");
        if let Some(secs) = self.time {
            out.push_str(&styler.dim(&format!("{:02}:{:02}", secs / 60, secs % 60)));
            out.push_str("  ");
        }
        out.push_str(&styler.paint(self.role, self.icon));
        if !self.label.is_empty() {
            out.push(' ');
            let label = if self.detail.is_empty() {
                self.label.clone()
            } else {
                self.label_column()
            };
            out.push_str(&styler.paint(self.role, &label));
        }
        if !self.detail.is_empty() {
            out.push(' ');
            out.push_str(&styler.paint(self.detail_role(), &self.detail));
        }
        out
    }
}

/// Line for an engine event. `None` for the events that are not lines:
/// the phase start (see [`phase_header`]), the phase end (see
/// [`phase_summary`]) and the agent events that carry nothing to show.
pub fn event_line(event: &EngineEvent, verify_secs: u64) -> Option<EventLine> {
    Some(match event {
        EngineEvent::PhaseStarted { .. } | EngineEvent::PhaseFinished { .. } => return None,
        EngineEvent::Agent { event, .. } => return agent_event_line(event),
        EngineEvent::AgentExited { exit_code, .. } => match exit_code {
            Some(0) => EventLine::new("✓", "Agent finished", "", Role::Ok),
            Some(code) => {
                EventLine::new("✗", "Agent", format!("exited with code {code}"), Role::Fail)
            }
            None => EventLine::new("!", "Agent", "stopped", Role::Warn),
        },
        EngineEvent::VerifyStarted { command, .. } => {
            EventLine::new("⚙", "Check", command.as_str(), Role::Dim)
        }
        EngineEvent::VerifyFinished {
            command,
            ok,
            exit_code,
            timed_out,
            ..
        } => {
            if *ok {
                EventLine::new("✓", "Passed", command.as_str(), Role::Ok)
            } else {
                let why = if *timed_out {
                    format!("timed out after {verify_secs} s")
                } else {
                    match exit_code {
                        Some(code) => format!("exit code {code}"),
                        None => "stopped".to_string(),
                    }
                };
                EventLine::new("✗", "Failed", format!("{command} ({why})"), Role::Fail)
            }
        }
        EngineEvent::Retrying { reason, .. } => {
            EventLine::new("!", "Retry", reason.as_str(), Role::Warn)
        }
        EngineEvent::CheckpointCreated { sha, .. } => EventLine::new(
            "●",
            "Checkpoint",
            sha.chars().take(7).collect::<String>(),
            Role::Accent,
        ),
    })
}

fn tool_icon(name: &str, subagent: bool) -> &'static str {
    if subagent {
        return "↳";
    }
    match name {
        "Read" | "Grep" | "Glob" => "◦",
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => "✎",
        "Bash" | "Shell" => "$",
        _ => "·",
    }
}

pub fn agent_event_line(event: &AgentEvent) -> Option<EventLine> {
    match event {
        AgentEvent::SessionStarted { .. } | AgentEvent::Finished { .. } => None,
        AgentEvent::Message { text, subagent } => {
            let line = first_line_short(text);
            let tag = if *subagent { "[subagent] " } else { "" };
            (!line.is_empty())
                .then(|| EventLine::new("›", "", format!("{tag}{line}"), Role::Default))
        }
        AgentEvent::ToolCall {
            name,
            detail,
            subagent,
        } => Some(EventLine::new(
            tool_icon(name, *subagent),
            name,
            detail.as_str(),
            Role::Tool,
        )),
        AgentEvent::Retry { detail } => {
            Some(EventLine::new("!", "Retry", detail.as_str(), Role::Warn))
        }
        AgentEvent::PermissionDenied { detail } => {
            Some(EventLine::new("!", "Denied", detail.as_str(), Role::Warn))
        }
        AgentEvent::Error { detail } => Some(EventLine::new(
            "✗",
            "Error",
            first_line_short(detail),
            Role::Fail,
        )),
    }
}

/// The heading of a phase block: `▶ Phase 1/2 · Title`, then the dim right
/// part (`attempt 1 · claude`), padded so it ends at column 80.
///
/// `model` and `effort` are the hook for choosing them (v0.2 part 2): they are
/// shown after the agent when set. Callers pass `None` until then.
pub fn phase_header(
    event: &EngineEvent,
    agent: &str,
    model: Option<&str>,
    effort: Option<&str>,
    styler: &Styler,
) -> Option<String> {
    let EngineEvent::PhaseStarted {
        phase,
        total,
        title,
        attempt,
        resumed,
    } = event
    else {
        return None;
    };
    let left = format!("▶ Phase {phase}/{total} · {title}");
    let mut parts = vec![if *resumed {
        format!("attempt {attempt} · resumed")
    } else {
        format!("attempt {attempt}")
    }];
    parts.push(agent.to_string());
    parts.extend(model.map(str::to_string));
    parts.extend(effort.map(str::to_string));
    let right = parts.join(" · ");
    let pad = HEADER_WIDTH
        .saturating_sub(left.chars().count() + right.chars().count())
        .max(2);
    Some(format!(
        "{}{}{}",
        styler.heading(&left),
        " ".repeat(pad),
        styler.dim(&right)
    ))
}

fn money(cost: f64) -> String {
    if cost < 0.01 {
        format!("${cost:.4}")
    } else {
        format!("${cost:.2}")
    }
}

/// The last line of a phase: ready for review (with files and cost when
/// known) or failed. `None` for other outcomes.
pub fn phase_summary(event: &EngineEvent, styler: &Styler) -> Option<String> {
    let EngineEvent::PhaseFinished {
        phase,
        status,
        failure,
        files,
        cost_usd,
    } = event
    else {
        return None;
    };
    if let Some(failure) = failure {
        let text = format!(
            "✗ Phase {phase} failed ({})",
            failure_kind_label(failure.kind)
        );
        return Some(styler.fail(&text));
    }
    if *status != PhaseStatus::AwaitingReview {
        return None;
    }
    let mut text = format!("✓ Phase {phase} ready for review");
    if let Some(n) = files {
        text.push_str(&format!(" · {n} file{}", if *n == 1 { "" } else { "s" }));
    }
    if let Some(cost) = cost_usd {
        text.push_str(&format!(" · {}", money(*cost)));
    }
    Some(styler.ok(&text))
}

pub fn status_label(status: PhaseStatus) -> &'static str {
    match status {
        PhaseStatus::Pending => "pending",
        PhaseStatus::Running => "running",
        PhaseStatus::Verifying => "verifying",
        PhaseStatus::AwaitingReview => "awaiting review",
        PhaseStatus::Done => "done",
        PhaseStatus::Failed => "failed",
    }
}

pub fn failure_kind_label(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::AgentError => "agent_error",
        FailureKind::VerifyFailed => "verify_failed",
        FailureKind::Timeout => "timeout",
        FailureKind::Interrupted => "interrupted",
        FailureKind::CheckpointFailed => "checkpoint_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Failure;

    fn line(event: &EngineEvent) -> EventLine {
        event_line(event, 30).expect("a line")
    }

    fn agent(event: AgentEvent) -> EventLine {
        agent_event_line(&event).expect("a line")
    }

    fn call(name: &str, subagent: bool) -> EventLine {
        agent(AgentEvent::ToolCall {
            name: name.into(),
            detail: "x".into(),
            subagent,
        })
    }

    fn parts(l: &EventLine) -> (&'static str, &str, &str, Role) {
        (l.icon, l.label.as_str(), l.detail.as_str(), l.role)
    }

    #[test]
    fn tool_calls_get_their_icon_and_the_tool_role() {
        for (name, icon) in [
            ("Read", "◦"),
            ("Grep", "◦"),
            ("Glob", "◦"),
            ("Write", "✎"),
            ("Edit", "✎"),
            ("Bash", "$"),
            ("Shell", "$"),
            ("WebSearch", "·"),
            ("MCP", "·"),
        ] {
            let l = call(name, false);
            assert_eq!(parts(&l), (icon, name, "x", Role::Tool), "{name}");
        }
        assert_eq!(call("Read", true).icon, "↳");
        assert_eq!(call("Subagent", true).icon, "↳");
    }

    #[test]
    fn agent_events_map_to_lines() {
        let m = agent(AgentEvent::Message {
            text: "\n  Created it.\nmore".into(),
            subagent: false,
        });
        assert_eq!(parts(&m), ("›", "", "Created it.", Role::Default));
        let m = agent(AgentEvent::Message {
            text: "hi".into(),
            subagent: true,
        });
        assert_eq!(m.detail, "[subagent] hi");
        let long = agent(AgentEvent::Message {
            text: "é".repeat(500),
            subagent: false,
        });
        assert_eq!(long.detail.chars().count(), MAX_MESSAGE_CHARS);
        assert_eq!(
            parts(&agent(AgentEvent::Retry {
                detail: "slow".into()
            })),
            ("!", "Retry", "slow", Role::Warn)
        );
        assert_eq!(
            parts(&agent(AgentEvent::PermissionDenied {
                detail: "Bash rm".into()
            })),
            ("!", "Denied", "Bash rm", Role::Warn)
        );
        assert_eq!(
            parts(&agent(AgentEvent::Error {
                detail: "boom\nsecond".into()
            })),
            ("✗", "Error", "boom", Role::Fail)
        );
        assert!(
            agent_event_line(&AgentEvent::SessionStarted {
                session_id: "s".into()
            })
            .is_none()
        );
        assert!(
            agent_event_line(&AgentEvent::Finished {
                ok: true,
                summary: String::new(),
                cost_usd: None
            })
            .is_none()
        );
        let empty = AgentEvent::Message {
            text: "  \n".into(),
            subagent: false,
        };
        assert!(agent_event_line(&empty).is_none());
    }

    #[test]
    fn engine_events_map_to_lines() {
        let exited = |code| EngineEvent::AgentExited {
            phase: 1,
            exit_code: code,
        };
        assert_eq!(
            parts(&line(&exited(Some(0)))),
            ("✓", "Agent finished", "", Role::Ok)
        );
        assert_eq!(
            parts(&line(&exited(Some(2)))),
            ("✗", "Agent", "exited with code 2", Role::Fail)
        );
        assert_eq!(
            parts(&line(&exited(None))),
            ("!", "Agent", "stopped", Role::Warn)
        );
        let started = EngineEvent::VerifyStarted {
            phase: 1,
            command: "cargo test".into(),
        };
        assert_eq!(
            parts(&line(&started)),
            ("⚙", "Check", "cargo test", Role::Dim)
        );
        let finished = |ok, exit_code, timed_out| EngineEvent::VerifyFinished {
            phase: 1,
            command: "c".into(),
            ok,
            exit_code,
            timed_out,
        };
        assert_eq!(
            parts(&line(&finished(true, Some(0), false))),
            ("✓", "Passed", "c", Role::Ok)
        );
        assert_eq!(
            parts(&line(&finished(false, Some(1), false))),
            ("✗", "Failed", "c (exit code 1)", Role::Fail)
        );
        assert_eq!(
            parts(&line(&finished(false, None, true))),
            ("✗", "Failed", "c (timed out after 30 s)", Role::Fail)
        );
        assert_eq!(
            parts(&line(&finished(false, None, false))),
            ("✗", "Failed", "c (stopped)", Role::Fail)
        );
        let retry = EngineEvent::Retrying {
            phase: 1,
            reason: "`c` failed".into(),
        };
        assert_eq!(
            parts(&line(&retry)),
            ("!", "Retry", "`c` failed", Role::Warn)
        );
        let checkpoint = EngineEvent::CheckpointCreated {
            phase: 1,
            sha: "821c664abcdef".into(),
        };
        assert_eq!(
            parts(&line(&checkpoint)),
            ("●", "Checkpoint", "821c664", Role::Accent)
        );
    }

    #[test]
    fn started_and_finished_events_are_not_lines() {
        let started = EngineEvent::PhaseStarted {
            phase: 1,
            total: 2,
            title: "T".into(),
            attempt: 1,
            resumed: false,
        };
        let finished = EngineEvent::PhaseFinished {
            phase: 1,
            status: PhaseStatus::AwaitingReview,
            failure: None,
            files: None,
            cost_usd: None,
        };
        assert!(event_line(&started, 30).is_none());
        assert!(event_line(&finished, 30).is_none());
    }

    #[test]
    fn lines_render_with_time_icon_padded_label_and_detail() {
        let plain = Styler { color: false };
        let write = call("Write", false).at(1);
        assert_eq!(write.render(&plain), "  00:01  ✎ Write    x");
        let msg = agent(AgentEvent::Message {
            text: "Created it.".into(),
            subagent: false,
        })
        .at(125);
        assert_eq!(msg.render(&plain), "  02:05  › Created it.");
        let done = line(&EngineEvent::AgentExited {
            phase: 1,
            exit_code: Some(0),
        })
        .at(3);
        assert_eq!(done.render(&plain), "  00:03  ✓ Agent finished");
        assert_eq!(call("Bash", false).render(&plain), "  $ Bash     x");
        let coloured = write.render(&Styler { color: true });
        assert!(coloured.contains("\x1b["), "{coloured:?}");
    }

    #[test]
    fn header_shows_attempt_agent_and_optional_settings() {
        let plain = Styler { color: false };
        let started = |attempt, resumed| EngineEvent::PhaseStarted {
            phase: 1,
            total: 2,
            title: "Create a file".into(),
            attempt,
            resumed,
        };
        let h = phase_header(&started(1, false), "claude", None, None, &plain).expect("header");
        assert!(h.starts_with("▶ Phase 1/2 · Create a file  "), "{h}");
        assert!(h.ends_with("attempt 1 · claude"), "{h}");
        assert_eq!(h.chars().count(), HEADER_WIDTH);
        let h = phase_header(
            &started(2, true),
            "codex",
            Some("opus"),
            Some("high"),
            &plain,
        )
        .expect("header");
        assert!(
            h.ends_with("attempt 2 · resumed · codex · opus · high"),
            "{h}"
        );
        let finished = EngineEvent::PhaseFinished {
            phase: 1,
            status: PhaseStatus::Done,
            failure: None,
            files: None,
            cost_usd: None,
        };
        assert!(phase_header(&finished, "claude", None, None, &plain).is_none());
    }

    #[test]
    fn summary_has_files_and_cost_when_known() {
        let plain = Styler { color: false };
        let finished = |files, cost_usd| EngineEvent::PhaseFinished {
            phase: 3,
            status: PhaseStatus::AwaitingReview,
            failure: None,
            files,
            cost_usd,
        };
        let text = |e: &EngineEvent| phase_summary(e, &plain).expect("summary");
        assert_eq!(
            text(&finished(Some(1), Some(0.06))),
            "✓ Phase 3 ready for review · 1 file · $0.06"
        );
        assert_eq!(
            text(&finished(Some(4), None)),
            "✓ Phase 3 ready for review · 4 files"
        );
        assert_eq!(text(&finished(None, None)), "✓ Phase 3 ready for review");
        assert_eq!(
            text(&finished(Some(0), Some(0.004))),
            "✓ Phase 3 ready for review · 0 files · $0.0040"
        );
        let failed = EngineEvent::PhaseFinished {
            phase: 2,
            status: PhaseStatus::Failed,
            failure: Some(Failure {
                kind: FailureKind::VerifyFailed,
                detail: "d".into(),
            }),
            files: None,
            cost_usd: None,
        };
        assert_eq!(text(&failed), "✗ Phase 2 failed (verify_failed)");
        assert!(
            phase_summary(&failed, &Styler { color: true }).is_some_and(|s| s.contains("\x1b["))
        );
    }
}

//! Formatting of engine events and run states, shared by the CLI and the TUI.

use crate::agent::AgentEvent;
use crate::engine::EngineEvent;
use crate::state::{FailureKind, PhaseStatus};

const MAX_MESSAGE_CHARS: usize = 160;

pub fn first_line_short(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    line.unwrap_or("").chars().take(MAX_MESSAGE_CHARS).collect()
}

pub fn describe_event(event: &EngineEvent, verify_secs: u64) -> Option<String> {
    Some(match event {
        EngineEvent::PhaseStarted {
            phase,
            total,
            title,
            attempt,
            resumed,
        } => {
            let how = if *resumed {
                ", resuming the session"
            } else {
                ""
            };
            format!("Phase {phase}/{total}: {title} (attempt {attempt}{how})")
        }
        EngineEvent::Agent { event, .. } => return describe_agent_event(event),
        EngineEvent::AgentExited { exit_code, .. } => match exit_code {
            Some(code) => format!("Agent exited with code {code}"),
            None => "Agent stopped".to_string(),
        },
        EngineEvent::VerifyStarted { command, .. } => format!("Checking: {command}"),
        EngineEvent::VerifyFinished {
            command,
            ok,
            exit_code,
            timed_out,
            ..
        } => {
            if *ok {
                format!("✓ {command}")
            } else if *timed_out {
                format!("✗ {command} (timed out after {verify_secs} s)")
            } else {
                match exit_code {
                    Some(code) => format!("✗ {command} (exit code {code})"),
                    None => format!("✗ {command} (stopped)"),
                }
            }
        }
        EngineEvent::Retrying { reason, .. } => format!("Retrying: {reason}"),
        EngineEvent::CheckpointCreated { sha, .. } => {
            format!("Checkpoint {}", sha.chars().take(7).collect::<String>())
        }
        EngineEvent::PhaseFinished {
            phase,
            status,
            failure,
        } => match failure {
            Some(failure) => format!(
                "Phase {phase} failed ({})",
                failure_kind_label(failure.kind)
            ),
            None => format!("Phase {phase}: {}", status_label(*status)),
        },
    })
}

pub fn describe_agent_event(event: &AgentEvent) -> Option<String> {
    let tag = |subagent: &bool| if *subagent { "[subagent] " } else { "" };
    match event {
        AgentEvent::SessionStarted { .. } => None,
        AgentEvent::Message { text, subagent } => {
            let line = first_line_short(text);
            (!line.is_empty()).then(|| format!("{}{line}", tag(subagent)))
        }
        AgentEvent::ToolCall {
            name,
            detail,
            subagent,
        } => Some(
            format!("{}{name} {detail}", tag(subagent))
                .trim_end()
                .to_string(),
        ),
        AgentEvent::Retry { detail } => Some(format!("Agent is retrying: {detail}")),
        AgentEvent::PermissionDenied { detail } => Some(format!("Permission denied: {detail}")),
        AgentEvent::Error { detail } => Some(format!("Agent error: {}", first_line_short(detail))),
        AgentEvent::Finished { .. } => None,
    }
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
    }
}

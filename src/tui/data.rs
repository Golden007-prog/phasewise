//! Read-only views of a run for the TUI: plain data plus the loaders that
//! build them from a [`Run`].

use anyhow::Result;

use crate::agent::agent_for;
use crate::display::{describe_agent_event, failure_kind_label, first_line_short};
use crate::ops::Run;
use crate::state::{PhaseStatus, RunStatus};

const NO_VERIFY: &str = "No verification output yet.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseView {
    pub number: usize,
    pub title: String,
    pub status: PhaseStatus,
    pub attempts: u32,
    pub failure: Option<String>,
    pub has_checkpoint: bool,
}

/// Diff of one phase. `files` holds the path and the added/removed line
/// counts (`None` for binary files).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffView {
    pub files: Vec<(String, Option<u64>, Option<u64>)>,
    pub text: String,
    pub untracked: Vec<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    pub run_id: String,
    pub plan_title: String,
    pub agent: String,
    pub run_branch: String,
    pub original_branch: String,
    pub active: bool,
    pub phases: Vec<PhaseView>,
}

pub fn load_run_view(run: &Run) -> RunView {
    let state = &run.state;
    let phases = state
        .phases
        .iter()
        .map(|p| PhaseView {
            number: p.number,
            title: p.title.clone(),
            status: p.status,
            attempts: p.attempts,
            failure: p.failure.as_ref().map(|f| {
                format!(
                    "{}: {}",
                    failure_kind_label(f.kind),
                    first_line_short(&f.detail)
                )
            }),
            has_checkpoint: p.checkpoint_sha.is_some(),
        })
        .collect();
    RunView {
        run_id: state.run_id.clone(),
        plan_title: state
            .plan_title
            .clone()
            .unwrap_or_else(|| run.plan.title.clone().unwrap_or_default()),
        agent: state.agent.to_string(),
        run_branch: state.run_branch.clone(),
        original_branch: state.original_branch.clone(),
        active: state.status == RunStatus::Active,
        phases,
    }
}

fn note(text: &str) -> DiffView {
    DiffView {
        note: Some(text.to_string()),
        ..DiffView::default()
    }
}

/// Diff of `phase`: between its checkpoints when it has one, otherwise the
/// working tree against the previous checkpoint plus the untracked files.
pub fn load_diff(run: &Run, phase: usize) -> Result<DiffView> {
    let state = &run.state;
    let Some(p) = state.phase(phase) else {
        return Ok(note("This phase does not exist."));
    };
    if p.status == PhaseStatus::Pending {
        return Ok(note("No changes yet."));
    }
    let Some(from) = state.checkpoint(phase - 1) else {
        return Ok(note(
            "The previous phase has no checkpoint to compare with.",
        ));
    };
    if let Some(to) = state.checkpoint(phase) {
        let files = run
            .git
            .numstat(from, to)?
            .into_iter()
            .map(|s| (s.path, s.added, s.removed))
            .collect::<Vec<_>>();
        let text = run.git.diff(from, to)?;
        let note = files
            .is_empty()
            .then(|| "This phase made no changes.".to_string());
        return Ok(DiffView {
            files,
            text,
            untracked: Vec::new(),
            note,
        });
    }
    match p.status {
        PhaseStatus::Failed | PhaseStatus::Running | PhaseStatus::Verifying => {
            let stats = run.git.numstat_worktree(from)?;
            let text = run.git.diff_worktree(from)?;
            let untracked: Vec<String> = run
                .git
                .dirty_paths()?
                .into_iter()
                .filter(|path| !stats.iter().any(|s| &s.path == path))
                .collect();
            let files: Vec<_> = stats
                .into_iter()
                .map(|s| (s.path, s.added, s.removed))
                .collect();
            let note =
                (files.is_empty() && untracked.is_empty()).then(|| "No changes yet.".to_string());
            Ok(DiffView {
                files,
                text,
                untracked,
                note,
            })
        }
        PhaseStatus::Pending | PhaseStatus::AwaitingReview | PhaseStatus::Done => {
            Ok(note("This phase has no checkpoint."))
        }
    }
}

/// Normalized agent events of every attempt of `phase`, one line each, with a
/// separator line per attempt. Missing attempt files are skipped.
pub fn load_log(run: &Run, phase: usize) -> Vec<String> {
    let attempts = run.state.phase(phase).map_or(0, |p| p.attempts);
    let agent = agent_for(run.state.agent);
    let mut lines = Vec::new();
    for attempt in 1..=attempts {
        let Ok(raw) = std::fs::read(run.paths.agent_stdout_log(phase, attempt)) else {
            continue;
        };
        let raw = String::from_utf8_lossy(&raw);
        let mut parser = agent.parser(run.git.root());
        lines.push(format!("── attempt {attempt} ──"));
        for line in raw.lines() {
            for event in parser.feed(line) {
                if let Some(text) = describe_agent_event(&event) {
                    lines.push(text);
                }
            }
        }
    }
    lines
}

/// Content of the verification log of `phase`.
pub fn load_verify(run: &Run, phase: usize) -> String {
    match std::fs::read(run.paths.verify_log(phase)) {
        Ok(bytes) if !bytes.is_empty() => String::from_utf8_lossy(&bytes).into_owned(),
        _ => NO_VERIFY.to_string(),
    }
}

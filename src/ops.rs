//! Run-level operations shared by the CLI and the TUI. Never prints; progress
//! goes through `EngineEvent`s.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use jiff::Timestamp;

use crate::agent::agent_for;
use crate::engine::{EngineEvent, PhaseContext, PhaseSettings, run_phase};
use crate::git::Git;
use crate::plan::{self, AgentKind, Plan};
use crate::state::{
    PhaseStatus, PhasewiseDir, RunPaths, RunState, RunStatus, load_with_recovery, lock_holder,
    new_run_id, run_branch_name, write_atomic,
};

const KILL_GRACE: Duration = Duration::from_secs(5);
const MAX_DIRTY_LISTED: usize = 10;
const PHASE_TIMEOUT_ENV: &str = "PHASEWISE_PHASE_TIMEOUT_SECONDS";

/// A loaded run: repository, files, state and the plan copy it was started with.
pub struct Run {
    pub git: Git,
    pub paths: RunPaths,
    pub state: RunState,
    pub plan: Plan,
    /// Phases that were left `running`/`verifying` by a dead process and were
    /// marked `failed` (interrupted) while loading. The CLI reports them once.
    pub recovered: Vec<usize>,
}

/// What `abort` did, for the caller to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortOutcome {
    /// The branch checked out now.
    pub current_branch: String,
    /// True when `abort` switched to the original branch.
    pub switched: bool,
    /// Commit holding uncommitted changes found on the run branch, if any.
    pub leftovers_sha: Option<String>,
}

/// How `next` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextOutcome {
    /// A phase ran; this is how it ended (the last one of an `--auto` chain).
    Ran(PhaseStatus),
    /// The last phase was approved and the run is complete.
    Completed,
}

/// Starts a run: validates the plan and the repository, creates the run
/// branch and writes the run files. Nothing is written if a check fails.
pub fn start(cwd: &Path, plan_path: &Path, agent_override: Option<AgentKind>) -> Result<Run> {
    let plan = plan::load(plan_path)?;
    let plan_text = std::fs::read_to_string(plan_path)
        .with_context(|| format!("Cannot read {}", plan_path.display()))?;
    let git = Git::discover(cwd)?;
    refuse_if_active(&git)?;
    check_repository(&git)?;
    let original_branch = git
        .current_branch()?
        .ok_or_else(|| {
            anyhow!(
                "HEAD is detached. Switch to a branch (`git switch <branch>`), then run phasewise again."
            )
        })?;

    git.ensure_excluded(".phasewise/")?;
    let run_id = new_run_id(Timestamp::now());
    let run_branch = run_branch_name(&run_id);
    let base_sha = git.head_sha()?;
    git.create_branch(&run_branch, &base_sha)?;

    let paths = RunPaths::new(git.root(), &run_id);
    write_atomic(&paths.plan_copy(), plan_text.as_bytes())?;
    let phases: Vec<(usize, String)> = plan
        .phases
        .iter()
        .map(|p| (p.number, p.title.clone()))
        .collect();
    let plan_path = plan_path
        .canonicalize()
        .unwrap_or_else(|_| plan_path.to_path_buf());
    let mut state = RunState::new(
        &run_id,
        &plan_path,
        plan.title.clone(),
        agent_override.unwrap_or(plan.config.agent),
        &original_branch,
        &run_branch,
        &base_sha,
        &phases,
    );
    state.save(&paths)?;
    PhasewiseDir::new(git.root()).write_current(&run_id)?;
    Ok(Run {
        git,
        paths,
        state,
        plan,
        recovered: Vec::new(),
    })
}

fn refuse_if_active(git: &Git) -> Result<()> {
    let Some(run) = load_from_git(git)? else {
        return Ok(());
    };
    if run.state.status == RunStatus::Active {
        bail!(
            "A run is already active ({} on branch {}). Continue it with `phasewise next`, \
inspect it with `phasewise review`, or give it up with `phasewise abort`.",
            run.state.run_id,
            run.state.run_branch
        );
    }
    Ok(())
}

fn check_repository(git: &Git) -> Result<()> {
    if !git.has_commits()? {
        bail!(
            "The repository has no commits yet. Make an initial commit, then run phasewise again."
        );
    }
    if git.current_branch()?.is_none() {
        bail!(
            "HEAD is detached. Switch to a branch (`git switch <branch>`), then run phasewise again."
        );
    }
    let dirty = dirty_paths(git)?;
    if !dirty.is_empty() {
        let mut listed: Vec<String> = dirty
            .iter()
            .take(MAX_DIRTY_LISTED)
            .map(|p| format!("  {p}"))
            .collect();
        if dirty.len() > MAX_DIRTY_LISTED {
            listed.push(format!("  ... and {} more", dirty.len() - MAX_DIRTY_LISTED));
        }
        bail!(
            "The working tree has uncommitted changes:\n{}\nCommit or stash them, then run phasewise again.",
            listed.join("\n")
        );
    }
    Ok(())
}

/// Dirty paths, ignoring phasewise's own directory.
fn dirty_paths(git: &Git) -> Result<Vec<String>> {
    Ok(git
        .dirty_paths()?
        .into_iter()
        .filter(|p| !p.starts_with(".phasewise/"))
        .collect())
}

/// The latest run, whatever its status, with crash recovery applied.
pub fn load_current(cwd: &Path) -> Result<Option<Run>> {
    let git = Git::discover(cwd)?;
    load_from_git(&git)
}

fn load_from_git(git: &Git) -> Result<Option<Run>> {
    let Some(run_id) = PhasewiseDir::new(git.root()).read_current()? else {
        return Ok(None);
    };
    let paths = RunPaths::new(git.root(), &run_id);
    if !paths.state_file().exists() {
        return Ok(None);
    }
    let before = RunState::load(&paths)?;
    let state = load_with_recovery(&paths)?;
    let recovered = before
        .phases
        .iter()
        .zip(&state.phases)
        .filter(|(old, new)| old.status != new.status && new.status == PhaseStatus::Failed)
        .map(|(old, _)| old.number)
        .collect();
    let plan = plan::load(&paths.plan_copy())?;
    Ok(Some(Run {
        git: git.clone(),
        paths,
        state,
        plan,
        recovered,
    }))
}

/// The latest run, only if it is active.
pub fn load_active(cwd: &Path) -> Result<Option<Run>> {
    Ok(load_current(cwd)?.filter(|run| run.state.status == RunStatus::Active))
}

/// Limits from the plan; `PHASEWISE_PHASE_TIMEOUT_SECONDS` overrides the phase timeout.
pub fn settings(run: &Run) -> Result<PhaseSettings> {
    let config = &run.plan.config;
    let phase_timeout = match std::env::var(PHASE_TIMEOUT_ENV) {
        Ok(value) => {
            let secs: u64 = value.trim().parse().map_err(|_| {
                anyhow!(
                    "{PHASE_TIMEOUT_ENV} must be a whole number of seconds, got `{value}`. \
Unset it or fix the value."
                )
            })?;
            Duration::from_secs(secs)
        }
        Err(_) => Duration::from_secs(config.phase_timeout_minutes * 60),
    };
    Ok(PhaseSettings {
        max_retries: config.max_retries,
        phase_timeout,
        verify_timeout: Duration::from_secs(config.verify_timeout_seconds),
        kill_grace: KILL_GRACE,
        agent_args: config.agent_args.clone(),
    })
}

/// Moves the phase awaiting review to `done`; completes the run when no phase is pending.
pub fn approve(run: &mut Run) -> Result<()> {
    let phase = run
        .state
        .phases
        .iter_mut()
        .find(|p| p.status == PhaseStatus::AwaitingReview)
        .ok_or_else(|| {
            anyhow!("No phase is awaiting review. Run `phasewise status` to see the run.")
        })?;
    phase.status = PhaseStatus::Done;
    if !run
        .state
        .phases
        .iter()
        .any(|p| p.status == PhaseStatus::Pending)
    {
        run.state.status = RunStatus::Completed;
    }
    run.state.save(&run.paths)
}

/// Approves the phase awaiting review (if any) and runs the next pending one.
pub fn next(
    run: &mut Run,
    auto: bool,
    events: &Sender<EngineEvent>,
    cancel: &AtomicBool,
) -> Result<NextOutcome> {
    check_can_advance(run)?;
    if run
        .state
        .phases
        .iter()
        .any(|p| p.status == PhaseStatus::AwaitingReview)
    {
        approve(run)?;
        if run.state.status == RunStatus::Completed {
            return Ok(NextOutcome::Completed);
        }
    }
    execute(run, auto, events, cancel).map(NextOutcome::Ran)
}

/// The run must be active and no phasewise process may be running a phase.
fn check_active_unlocked(run: &Run) -> Result<()> {
    if run.state.status != RunStatus::Active {
        bail!(
            "Run {} is not active. Start a new one with `phasewise run`.",
            run.state.run_id
        );
    }
    if let Some(pid) = lock_holder(&run.paths) {
        let number = run
            .state
            .phases
            .iter()
            .find(|p| matches!(p.status, PhaseStatus::Running | PhaseStatus::Verifying))
            .map_or(0, |p| p.number);
        bail!("Phase {number} is running in process {pid}. Wait for it to finish.");
    }
    Ok(())
}

fn on_run_branch(run: &Run) -> Result<bool> {
    Ok(run.git.current_branch()?.as_deref() == Some(run.state.run_branch.as_str()))
}

fn check_on_run_branch(run: &Run) -> Result<()> {
    if !on_run_branch(run)? {
        bail!(
            "You are not on the run branch. Run `git switch {}` first.",
            run.state.run_branch
        );
    }
    Ok(())
}

fn check_can_advance(run: &Run) -> Result<()> {
    check_active_unlocked(run)?;
    if let Some(failed) = run
        .state
        .phases
        .iter()
        .find(|p| p.status == PhaseStatus::Failed)
    {
        bail!(
            "Phase {} failed. Run `phasewise retry` or `phasewise rollback {}`.",
            failed.number,
            failed.number - 1
        );
    }
    check_on_run_branch(run)?;
    let dirty = dirty_paths(&run.git)?;
    if !dirty.is_empty() {
        bail!(
            "The working tree has uncommitted changes (e.g. {}). Commit or stash them, \
or run `phasewise rollback` to discard them.",
            dirty[0]
        );
    }
    Ok(())
}

/// Puts a phase back to `pending`, clearing everything but `attempts`.
fn reset_phase(run: &mut Run, number: usize) {
    if let Some(phase) = run.state.phase_mut(number) {
        phase.status = PhaseStatus::Pending;
        phase.session_id = None;
        phase.checkpoint_sha = None;
        phase.summary = None;
        phase.failure = None;
        phase.cost_usd = None;
        phase.started_at = None;
        phase.finished_at = None;
    }
}

/// Reruns the highest-numbered `failed` or `awaiting_review` phase from the
/// checkpoint before it, in a fresh session, with `note` added to the prompt.
pub fn retry(
    run: &mut Run,
    note: Option<&str>,
    events: &Sender<EngineEvent>,
    cancel: &AtomicBool,
) -> Result<PhaseStatus> {
    check_active_unlocked(run)?;
    let number = run
        .state
        .phases
        .iter()
        .rev()
        .find(|p| matches!(p.status, PhaseStatus::Failed | PhaseStatus::AwaitingReview))
        .map(|p| p.number)
        .ok_or_else(|| {
            anyhow!(
                "Nothing to retry: no phase has failed or is awaiting review. \
Run `phasewise status` to see the run, or `phasewise next` to continue."
            )
        })?;
    check_on_run_branch(run)?;
    let previous = run
        .state
        .checkpoint(number - 1)
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "Phase {} has no checkpoint to go back to. Run `phasewise status`, \
then `phasewise rollback 0` to start over.",
                number - 1
            )
        })?;
    run.git.reset_hard(&previous)?;
    run.git.clean_untracked()?;
    reset_phase(run, number);
    run.state.save(&run.paths)?;
    run_one(run, number, note, events, cancel)
}

/// Checks that `rollback k` can run and returns the SHA it goes back to.
pub fn prepare_rollback(run: &Run, k: usize) -> Result<String> {
    check_active_unlocked(run)?;
    check_on_run_branch(run)?;
    let valid = |n: usize| run.state.checkpoint(n).map(str::to_string);
    valid(k).ok_or_else(|| {
        let mut targets = vec!["0 (initial state)".to_string()];
        targets.extend(
            run.state
                .phases
                .iter()
                .filter(|p| p.checkpoint_sha.is_some())
                .map(|p| p.number.to_string()),
        );
        anyhow!(
            "Phase {k} has no checkpoint to go back to. Valid targets: {}.",
            targets.join(", ")
        )
    })
}

/// Goes back to the checkpoint of phase `k` (`0` = initial state); later
/// phases become `pending` again.
pub fn rollback(run: &mut Run, k: usize) -> Result<()> {
    let sha = prepare_rollback(run, k)?;
    run.git.reset_hard(&sha)?;
    run.git.clean_untracked()?;
    let numbers: Vec<usize> = run.state.phases.iter().map(|p| p.number).collect();
    for number in numbers.into_iter().filter(|n| *n > k) {
        reset_phase(run, number);
    }
    run.state.save(&run.paths)
}

/// Gives the run up: keeps uncommitted work in a commit on the run branch,
/// returns to the original branch and marks the run `aborted`. Never deletes
/// the run branch.
pub fn abort(run: &mut Run) -> Result<AbortOutcome> {
    check_active_unlocked(run)?;
    let mut leftovers_sha = None;
    let mut switched = false;
    if on_run_branch(run)? {
        if !dirty_paths(&run.git)?.is_empty() {
            run.git.add_all()?;
            let message = format!(
                "phasewise: abort — uncommitted changes\n\nPhasewise-Run: {}",
                run.state.run_id
            );
            leftovers_sha = Some(run.git.commit(&message, false)?);
        }
        run.git.switch(&run.state.original_branch)?;
        switched = true;
    }
    run.state.status = RunStatus::Aborted;
    run.state.save(&run.paths)?;
    let current_branch = run
        .git
        .current_branch()?
        .unwrap_or_else(|| "(detached HEAD)".to_string());
    Ok(AbortOutcome {
        current_branch,
        switched,
        leftovers_sha,
    })
}

fn run_one(
    run: &mut Run,
    number: usize,
    note: Option<&str>,
    events: &Sender<EngineEvent>,
    cancel: &AtomicBool,
) -> Result<PhaseStatus> {
    let agent = agent_for(run.state.agent);
    let settings = settings(run)?;
    let mut ctx = PhaseContext {
        git: &run.git,
        paths: &run.paths,
        state: &mut run.state,
        plan: &run.plan,
        agent: agent.as_ref(),
        settings: &settings,
        events,
        cancel,
    };
    run_phase(&mut ctx, number, note)
}

/// Runs the first pending phase. With `auto`, approves each phase that reaches
/// `awaiting_review` and continues while phases remain; the last one stays
/// `awaiting_review`. Stops at the first failure or on cancellation.
pub fn execute(
    run: &mut Run,
    auto: bool,
    events: &Sender<EngineEvent>,
    cancel: &AtomicBool,
) -> Result<PhaseStatus> {
    loop {
        let number = run
            .state
            .phases
            .iter()
            .find(|p| p.status == PhaseStatus::Pending)
            .map(|p| p.number)
            .ok_or_else(|| {
                anyhow!("No phase is pending. Run `phasewise status` to see the run.")
            })?;
        let status = run_one(run, number, None, events, cancel)?;
        let more = run
            .state
            .phases
            .iter()
            .any(|p| p.status == PhaseStatus::Pending);
        if !auto || status != PhaseStatus::AwaitingReview || !more || cancel.load(Ordering::SeqCst)
        {
            return Ok(status);
        }
        approve(run)?;
    }
}

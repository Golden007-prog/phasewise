//! Command-line interface: clap definitions, dispatch and exit codes.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use crate::agent::AgentEvent;
use crate::engine::EngineEvent;
use crate::git::Git;
use crate::ops::{self, NextOutcome, Run};
use crate::plan::{self, AgentKind};
use crate::state::{FailureKind, PhaseStatus, RunStatus};

const PLAN_FILE: &str = "phasewise.md";
const PLAN_TEMPLATE: &str = include_str!("templates/phasewise.md");

/// Agent selectable from the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AgentArg {
    Claude,
    Codex,
}

impl From<AgentArg> for AgentKind {
    fn from(arg: AgentArg) -> Self {
        match arg {
            AgentArg::Claude => AgentKind::Claude,
            AgentArg::Codex => AgentKind::Codex,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "phasewise",
    version,
    about = "Run big plans with Claude Code or Codex one phase at a time — verified, checkpointed, and reviewable."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create an example phasewise.md (never overwrites) and exclude .phasewise/ from git
    #[command(after_help = "Examples:\n  phasewise init")]
    Init,

    /// Validate the plan and list its phases and verification commands; runs nothing
    #[command(after_help = "Examples:\n  phasewise check\n  phasewise check docs/plan.md")]
    Check {
        /// Path to the plan file (default: phasewise.md at the repository root)
        plan: Option<PathBuf>,
    },

    /// Check git, claude and codex: presence, version and whether this directory is a suitable repository
    #[command(after_help = "Examples:\n  phasewise doctor")]
    Doctor,

    /// Start a run and launch phase 1; fails if a run is already active
    #[command(
        after_help = "Examples:\n  phasewise run phasewise.md --auto\n  phasewise run --agent codex"
    )]
    Run {
        /// Path to the plan file (default: phasewise.md at the repository root)
        plan: Option<PathBuf>,
        /// Agent to use; takes priority over the plan header
        #[arg(long, value_enum)]
        agent: Option<AgentArg>,
        /// Chain phases until the plan ends or the first failure
        #[arg(long)]
        auto: bool,
    },

    /// Approve the phase awaiting review and launch the next one
    #[command(after_help = "Examples:\n  phasewise next\n  phasewise next --auto")]
    Next {
        /// Chain phases until the plan ends or the first failure
        #[arg(long)]
        auto: bool,
    },

    /// Go back to the checkpoint before the current phase and rerun it in a new session
    #[command(after_help = "Examples:\n  phasewise retry --note \"Keep the public API unchanged\"")]
    Retry {
        /// Note appended to the phase prompt
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
    },

    /// Go back to the checkpoint of phase N (0 = initial state)
    #[command(after_help = "Examples:\n  phasewise rollback 2\n  phasewise rollback 0 --yes")]
    Rollback {
        /// Phase number to return to (0 = initial state)
        n: u32,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Show a table of phases: state, attempts, changed files and cost if known
    #[command(after_help = "Examples:\n  phasewise status")]
    Status,

    /// Print the diff of phase N (default: the last one with a checkpoint)
    #[command(after_help = "Examples:\n  phasewise diff\n  phasewise diff 2")]
    Diff {
        /// Phase number
        n: Option<u32>,
    },

    /// Open the review TUI
    #[command(after_help = "Examples:\n  phasewise review")]
    Review,

    /// Mark the run as abandoned and return to the original branch (the run branch is kept)
    #[command(after_help = "Examples:\n  phasewise abort")]
    Abort,
}

/// Entry point called from `main`.
pub fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let ok = matches!(
                err.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
            // Printing can only fail on a closed stream; nothing useful to do then.
            let _ = err.print();
            return if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            };
        }
    };

    match dispatch(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            for cause in err.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            ExitCode::from(1)
        }
    }
}

fn dispatch(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        // Later phases open the TUI here when a run is active.
        None => {
            Cli::command().print_long_help()?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Init) => init(),
        Some(Command::Check { plan }) => check(plan),
        Some(Command::Doctor) => not_implemented("doctor"),
        Some(Command::Run { plan, agent, auto }) => run_command(plan, agent, auto),
        Some(Command::Next { auto }) => next_command(auto),
        Some(Command::Retry { .. }) => not_implemented("retry"),
        Some(Command::Rollback { .. }) => not_implemented("rollback"),
        Some(Command::Status) => status(),
        Some(Command::Diff { n }) => diff(n),
        Some(Command::Review) => not_implemented("review"),
        Some(Command::Abort) => not_implemented("abort"),
    }
}

fn not_implemented(name: &str) -> Result<ExitCode> {
    Err(anyhow!("`phasewise {name}` is not implemented yet"))
}

/// Explicit path as given, else `phasewise.md` at the repository root (or in
/// the current directory outside a repository).
fn resolve_plan_path(arg: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = arg {
        return Ok(path);
    }
    let cwd = std::env::current_dir().context("Cannot read the current directory")?;
    let dir = match Git::discover(&cwd) {
        Ok(git) => git.root().to_path_buf(),
        Err(_) => cwd,
    };
    Ok(dir.join(PLAN_FILE))
}

fn check(arg: Option<PathBuf>) -> Result<ExitCode> {
    let path = resolve_plan_path(arg)?;
    let loaded = match plan::load(&path) {
        Ok(plan) => plan,
        Err(err) => {
            let message = err.to_string();
            // Parse problems are already rendered as `<path>:<line>: <message>`.
            if message.starts_with(&format!("{}:", path.display())) {
                eprintln!("{message}");
                eprintln!("error: the plan has {} problem(s)", message.lines().count());
                return Ok(ExitCode::from(1));
            }
            return Err(err);
        }
    };
    print_plan(&loaded, &path);
    Ok(ExitCode::SUCCESS)
}

fn print_plan(plan: &plan::Plan, path: &Path) {
    let config = &plan.config;
    println!(
        "{} ({})",
        plan.title.as_deref().unwrap_or("(untitled)"),
        path.display()
    );
    println!("Agent: {}", config.agent);
    if config.verify.is_empty() {
        println!("Checks after every phase: (none)");
    } else {
        println!("Checks after every phase:");
        for command in &config.verify {
            println!("  - {command}");
        }
    }
    println!("Max retries: {}", config.max_retries);
    println!("Phase timeout: {} min", config.phase_timeout_minutes);
    println!("Check timeout: {} s", config.verify_timeout_seconds);
    println!();
    for phase in &plan.phases {
        println!("  {}. {}", phase.number, phase.title);
        if phase.verify.is_empty() {
            println!("       (no phase-specific checks)");
        } else {
            for command in &phase.verify {
                println!("       - {command}");
            }
        }
    }
    println!();
    println!("{} phases. The plan is valid.", plan.phases.len());
}

fn init() -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("Cannot read the current directory")?;
    let git = Git::discover(&cwd)?;
    let path = git.root().join(PLAN_FILE);
    if path.exists() {
        println!("{PLAN_FILE} already exists; left untouched.");
    } else {
        std::fs::write(&path, PLAN_TEMPLATE)
            .with_context(|| format!("Cannot write {}", path.display()))?;
        println!("Created {}.", path.display());
    }
    if git.ensure_excluded(".phasewise/")? {
        println!("Added .phasewise/ to the git exclude file.");
    }
    println!();
    println!("Next steps:");
    println!("  1. Edit {PLAN_FILE} to describe your phases.");
    println!("  2. Commit it (`phasewise run` needs a clean working tree).");
    println!("  3. phasewise check");
    println!("  4. phasewise run");
    Ok(ExitCode::SUCCESS)
}

const MAX_MESSAGE_CHARS: usize = 160;
const NO_RUN: &str = "No run yet. Start one with `phasewise run`.";
const EXIT_PHASE_FAILED: u8 = 3;

fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().context("Cannot read the current directory")
}

fn run_command(plan: Option<PathBuf>, agent: Option<AgentArg>, auto: bool) -> Result<ExitCode> {
    let path = resolve_plan_path(plan)?;
    let run = ops::start(&current_dir()?, &path, agent.map(AgentKind::from))?;
    println!(
        "Run {} on branch {} (original branch: {}, agent: {})",
        run.state.run_id, run.state.run_branch, run.state.original_branch, run.state.agent
    );
    drive(run, move |run, events, cancel| {
        ops::execute(run, auto, events, cancel).map(NextOutcome::Ran)
    })
}

fn next_command(auto: bool) -> Result<ExitCode> {
    let run = ops::load_current(&current_dir()?)?.ok_or_else(|| anyhow!(NO_RUN))?;
    drive(run, move |run, events, cancel| {
        ops::next(run, auto, events, cancel)
    })
}

/// The shared cancel flag; the first Ctrl-C sets it, the second one exits.
fn install_ctrlc() -> Result<Arc<AtomicBool>> {
    static CANCEL: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    if let Some(flag) = CANCEL.get() {
        return Ok(Arc::clone(flag));
    }
    let flag = Arc::new(AtomicBool::new(false));
    let handler_flag = Arc::clone(&flag);
    ctrlc::set_handler(move || {
        if handler_flag.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        println!("Interrupting… (press Ctrl-C again to quit immediately)");
    })
    .context("Cannot install the Ctrl-C handler")?;
    let _ = CANCEL.set(Arc::clone(&flag));
    Ok(flag)
}

/// Runs the engine work in a thread that owns the run and prints its events here.
fn drive<F>(run: Run, work: F) -> Result<ExitCode>
where
    F: FnOnce(&mut Run, &Sender<EngineEvent>, &AtomicBool) -> Result<NextOutcome> + Send + 'static,
{
    let cancel = install_ctrlc()?;
    let verify_secs = ops::settings(&run)?.verify_timeout.as_secs();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut run = run;
        let result = work(&mut run, &tx, &cancel);
        (run, result)
    });
    let started = Instant::now();
    for event in rx {
        if let Some(line) = describe_event(&event, verify_secs) {
            let secs = started.elapsed().as_secs();
            // A closed stdout (for example `| head`) must not abort the run.
            let _ = writeln!(
                std::io::stdout(),
                "[{:02}:{:02}] {line}",
                secs / 60,
                secs % 60
            );
        }
    }
    let (run, result) = worker
        .join()
        .map_err(|_| anyhow!("The engine thread panicked. This is a bug; please report it."))?;
    Ok(report_outcome(&run, result?))
}

fn first_line_short(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    line.unwrap_or("").chars().take(MAX_MESSAGE_CHARS).collect()
}

fn describe_event(event: &EngineEvent, verify_secs: u64) -> Option<String> {
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

fn describe_agent_event(event: &AgentEvent) -> Option<String> {
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

fn status_label(status: PhaseStatus) -> &'static str {
    match status {
        PhaseStatus::Pending => "pending",
        PhaseStatus::Running => "running",
        PhaseStatus::Verifying => "verifying",
        PhaseStatus::AwaitingReview => "awaiting review",
        PhaseStatus::Done => "done",
        PhaseStatus::Failed => "failed",
    }
}

fn failure_kind_label(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::AgentError => "agent_error",
        FailureKind::VerifyFailed => "verify_failed",
        FailureKind::Timeout => "timeout",
        FailureKind::Interrupted => "interrupted",
    }
}

fn report_outcome(run: &Run, outcome: NextOutcome) -> ExitCode {
    match outcome {
        NextOutcome::Completed => {
            let state = &run.state;
            println!("All phases done.");
            println!("To integrate the work:");
            println!(
                "  git switch {} && git merge --squash {} && git commit",
                state.original_branch, state.run_branch
            );
            ExitCode::SUCCESS
        }
        NextOutcome::Ran(PhaseStatus::Failed) => {
            print_failure(run);
            ExitCode::from(EXIT_PHASE_FAILED)
        }
        NextOutcome::Ran(_) => {
            let number = run
                .state
                .phases
                .iter()
                .rev()
                .find(|p| p.status == PhaseStatus::AwaitingReview)
                .map_or(0, |p| p.number);
            println!();
            println!("Phase {number} is ready for review.");
            println!("  phasewise diff      show what changed");
            println!("  phasewise review    inspect it interactively");
            println!("  phasewise next      approve it and continue");
            ExitCode::SUCCESS
        }
    }
}

fn print_failure(run: &Run) {
    let Some(phase) = run
        .state
        .phases
        .iter()
        .find(|p| p.status == PhaseStatus::Failed)
    else {
        return;
    };
    println!();
    if let Some(failure) = &phase.failure {
        println!(
            "Phase {} failed ({}): {}",
            phase.number,
            failure_kind_label(failure.kind),
            failure.detail
        );
    } else {
        println!("Phase {} failed.", phase.number);
    }
    println!("Options:");
    println!("  phasewise retry [--note \"...\"]   rerun the phase in a new session");
    println!(
        "  phasewise rollback {}              go back to the previous checkpoint",
        phase.number - 1
    );
    println!("  or inspect the working tree; it was left as the agent left it");
}

fn status() -> Result<ExitCode> {
    let Some(run) = ops::load_current(&current_dir()?)? else {
        println!("{NO_RUN}");
        return Ok(ExitCode::SUCCESS);
    };
    let state = &run.state;
    let run_status = match state.status {
        RunStatus::Active => "active",
        RunStatus::Completed => "completed",
        RunStatus::Aborted => "aborted",
    };
    println!("Run:    {} ({run_status})", state.run_id);
    println!(
        "Plan:   {}",
        state.plan_title.as_deref().unwrap_or("(untitled)")
    );
    println!("Agent:  {}", state.agent);
    println!(
        "Branch: {} (from {})",
        state.run_branch, state.original_branch
    );
    println!();

    let rows: Vec<[String; 5]> = state
        .phases
        .iter()
        .map(|p| {
            let files = match (state.checkpoint(p.number - 1), p.checkpoint_sha.as_deref()) {
                (Some(from), Some(to)) => run
                    .git
                    .changed_file_count(from, to)
                    .map_or_else(|_| "-".to_string(), |n| n.to_string()),
                _ => "-".to_string(),
            };
            let cost = p
                .cost_usd
                .map_or_else(|| "-".to_string(), |c| format!("${c:.4}"));
            [
                p.number.to_string(),
                status_label(p.status).to_string(),
                p.attempts.to_string(),
                files,
                cost,
            ]
        })
        .collect();
    let headers = ["#", "STATUS", "TRIES", "FILES", "COST"];
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: [&str; 5], title: &str| {
        let mut out = String::new();
        for (cell, width) in cells.iter().zip(widths) {
            out.push_str(&format!("{cell:<width$}  "));
        }
        out.push_str(title);
        println!("{out}");
    };
    line(headers, "TITLE");
    for (row, phase) in rows.iter().zip(&state.phases) {
        line([&row[0], &row[1], &row[2], &row[3], &row[4]], &phase.title);
        if let (PhaseStatus::Failed, Some(failure)) = (phase.status, &phase.failure) {
            println!(
                "    {}: {}",
                failure_kind_label(failure.kind),
                failure.detail
            );
        }
    }
    println!();
    println!("{}", next_step_hint(&run));
    Ok(ExitCode::SUCCESS)
}

fn next_step_hint(run: &Run) -> String {
    let state = &run.state;
    match state.status {
        RunStatus::Completed => format!(
            "Run completed. To integrate: git switch {} && git merge --squash {} && git commit",
            state.original_branch, state.run_branch
        ),
        RunStatus::Aborted => "Run aborted. Start a new one with `phasewise run`.".to_string(),
        RunStatus::Active => {
            let find = |status| state.phases.iter().find(|p| p.status == status);
            if let Some(p) = find(PhaseStatus::Failed) {
                format!(
                    "Next: `phasewise retry` or `phasewise rollback {}` (phase {} failed).",
                    p.number - 1,
                    p.number
                )
            } else if let Some(p) = find(PhaseStatus::AwaitingReview) {
                format!(
                    "Next: review phase {} (`phasewise diff`, `phasewise review`), then `phasewise next`.",
                    p.number
                )
            } else if find(PhaseStatus::Running).is_some() || find(PhaseStatus::Verifying).is_some()
            {
                "A phase is running.".to_string()
            } else {
                "Next: `phasewise next` runs the next phase.".to_string()
            }
        }
    }
}

fn diff(n: Option<u32>) -> Result<ExitCode> {
    let run = ops::load_current(&current_dir()?)?.ok_or_else(|| anyhow!(NO_RUN))?;
    let state = &run.state;
    let number = match n {
        Some(n) => {
            let n = n as usize;
            if n == 0 || n > state.phases.len() {
                return Err(anyhow!(
                    "Phase {n} does not exist; this run has phases 1 to {}. Run `phasewise status`.",
                    state.phases.len()
                ));
            }
            n
        }
        None => state
            .phases
            .iter()
            .rev()
            .find(|p| p.checkpoint_sha.is_some())
            .map(|p| p.number)
            .ok_or_else(|| {
                anyhow!(
                    "No phase has a checkpoint yet. Run `phasewise run` or `phasewise next` first."
                )
            })?,
    };
    let (Some(from), Some(to)) = (state.checkpoint(number - 1), state.checkpoint(number)) else {
        return Err(anyhow!(
            "Phase {number} has no checkpoint yet. Run `phasewise status` to see which phases have one."
        ));
    };
    let text = run.git.diff(from, to)?;
    if text.trim().is_empty() {
        println!("Phase {number} made no changes.");
        return Ok(ExitCode::SUCCESS);
    }
    let color = std::io::stdout().is_terminal();
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(&colorize(line, color));
        out.push('\n');
    }
    // A closed stdout (for example `| head`) is not an error worth reporting.
    let _ = std::io::stdout().write_all(out.as_bytes());
    Ok(ExitCode::SUCCESS)
}

fn colorize(line: &str, color: bool) -> String {
    if !color {
        return line.to_string();
    }
    let code = if line.starts_with("+++") || line.starts_with("---") {
        None
    } else if line.starts_with('+') {
        Some("32")
    } else if line.starts_with('-') {
        Some("31")
    } else if line.starts_with("@@") {
        Some("36")
    } else {
        None
    };
    match code {
        Some(code) => format!("\x1b[{code}m{line}\x1b[0m"),
        None => line.to_string(),
    }
}

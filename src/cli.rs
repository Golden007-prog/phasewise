//! Command-line interface: clap definitions, dispatch and exit codes.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use crate::agent::{ModelInfo, agent_for};
use crate::display::{event_line, failure_kind_label, phase_header, phase_summary, status_label};
use crate::engine::EngineEvent;
use crate::git::Git;
use crate::ops::{self, NextOutcome, Overrides, Run};
use crate::plan::{self, AgentKind};
use crate::process;
use crate::state::{PhaseStatus, RunStatus};
use crate::style::{Role, Styler};

// Output to stdout goes through these macros instead of `println!`/`print!`,
// which panic when stdout is closed (for example `| head`). A closed stdout
// must not abort the command, so write errors are ignored.
macro_rules! outln {
    () => {{
        let _ = writeln!(std::io::stdout().lock());
    }};
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

macro_rules! out {
    ($($arg:tt)*) => {{
        let _ = write!(std::io::stdout().lock(), $($arg)*);
    }};
}

const INTERRUPT_NOTICE: &str = "Interrupting… (press Ctrl-C again to quit immediately)";
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

/// `--model` and `--effort` of `run`, `next` and `retry`.
#[derive(Debug, Clone, clap::Args)]
pub struct SettingsArgs {
    /// Model for this run from now on (`default` removes the override)
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,
    /// Effort for this run from now on: low, medium, high, xhigh or max (`default` removes the override)
    #[arg(long, value_name = "EFFORT")]
    effort: Option<String>,
}

impl SettingsArgs {
    fn overrides(&self) -> Result<Overrides> {
        Overrides::parse(self.model.as_deref(), self.effort.as_deref())
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
        #[command(flatten)]
        settings: SettingsArgs,
    },

    /// Approve the phase awaiting review and launch the next one
    #[command(after_help = "Examples:\n  phasewise next\n  phasewise next --auto")]
    Next {
        /// Chain phases until the plan ends or the first failure
        #[arg(long)]
        auto: bool,
        #[command(flatten)]
        settings: SettingsArgs,
    },

    /// Go back to the checkpoint before the current phase and rerun it in a new session
    #[command(after_help = "Examples:\n  phasewise retry --note \"Keep the public API unchanged\"")]
    Retry {
        /// Note appended to the phase prompt
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
        #[command(flatten)]
        settings: SettingsArgs,
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

    /// List the models an agent offers (for `--model` and the plan's `model` setting)
    #[command(after_help = "Examples:\n  phasewise models\n  phasewise models --agent codex")]
    Models {
        /// Only this agent (default: both)
        #[arg(long, value_enum)]
        agent: Option<AgentArg>,
    },

    /// Mark the run as abandoned and return to the original branch (the run branch is kept)
    #[command(after_help = "Examples:\n  phasewise abort")]
    Abort,
}

/// Entry point called from `main`.
pub fn main() -> ExitCode {
    process::install_panic_hook();
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
        None => {
            let cwd = current_dir()?;
            // With an active run on a terminal the TUI opens; otherwise show the help.
            if std::io::stdout().is_terminal() && ops::load_active(&cwd).ok().flatten().is_some() {
                return crate::tui::run(&cwd);
            }
            Cli::command().print_long_help()?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Init) => init(),
        Some(Command::Check { plan }) => check(plan),
        Some(Command::Doctor) => doctor(),
        Some(Command::Run {
            plan,
            agent,
            auto,
            settings,
        }) => run_command(plan, agent, auto, &settings),
        Some(Command::Next { auto, settings }) => next_command(auto, &settings),
        Some(Command::Retry { note, settings }) => retry_command(note, &settings),
        Some(Command::Models { agent }) => models_command(agent),
        Some(Command::Rollback { n, yes }) => rollback_command(n as usize, yes),
        Some(Command::Status) => status(),
        Some(Command::Diff { n }) => diff(n),
        Some(Command::Review) => crate::tui::run(&current_dir()?),
        Some(Command::Abort) => abort_command(),
    }
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
    let st = Styler::for_stdout();
    let config = &plan.config;
    outln!(
        "{}",
        st.heading(&format!(
            "{} ({})",
            plan.title.as_deref().unwrap_or("(untitled)"),
            path.display()
        ))
    );
    outln!("Agent: {}", config.agent);
    if let Some(model) = &config.model {
        outln!("Model: {model}");
    }
    if let Some(effort) = &config.effort {
        outln!("Effort: {effort}");
    }
    if config.verify.is_empty() {
        outln!("Checks after every phase: (none)");
    } else {
        outln!("Checks after every phase:");
        for command in &config.verify {
            outln!("  {} {command}", st.dim("⚙"));
        }
    }
    outln!("Max retries: {}", config.max_retries);
    outln!("Phase timeout: {} min", config.phase_timeout_minutes);
    outln!("Check timeout: {} s", config.verify_timeout_seconds);
    outln!();
    for phase in &plan.phases {
        outln!(
            "  {}",
            st.heading(&format!("{}. {}", phase.number, phase.title))
        );
        let settings: Vec<String> = [("model", &phase.model), ("effort", &phase.effort)]
            .into_iter()
            .filter_map(|(key, value)| value.as_ref().map(|v| format!("{key}: {v}")))
            .collect();
        if !settings.is_empty() {
            outln!("     {}", st.dim(&settings.join(" · ")));
        }
        if phase.verify.is_empty() {
            outln!("     {}", st.dim("(no phase-specific checks)"));
        } else {
            for command in &phase.verify {
                outln!("     {} {command}", st.dim("⚙"));
            }
        }
    }
    outln!();
    outln!(
        "{}",
        st.ok(&format!("{} phases. The plan is valid.", plan.phases.len()))
    );
}

/// What `phasewise models` prints for one agent, or why it could not.
fn models_command(agent: Option<AgentArg>) -> Result<ExitCode> {
    let kinds = match agent {
        Some(a) => vec![AgentKind::from(a)],
        None => vec![AgentKind::Claude, AgentKind::Codex],
    };
    let st = Styler::for_stdout();
    let mut listed = 0;
    let mut first = true;
    for kind in &kinds {
        let a = agent_for(*kind);
        let result = a.list_models();
        let program = a.program().to_string_lossy().into_owned();
        let flag = match kind {
            AgentKind::Claude => "--model",
            AgentKind::Codex => "-m",
        };
        match result {
            Ok(models) => {
                listed += 1;
                if !first {
                    outln!();
                }
                first = false;
                print_models(*kind, &models, &st);
            }
            Err(err) => eprintln!(
                "Could not list the {kind} models: {err:#}. \
Any model name accepted by `{program} {flag}` still works."
            ),
        }
    }
    Ok(if listed == 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn print_models(kind: AgentKind, models: &[ModelInfo], st: &Styler) {
    outln!("{}", st.heading(&format!("{kind} models")));
    let id_width = models
        .iter()
        .map(|m| m.id.chars().count())
        .max()
        .unwrap_or(0);
    // The label column is only shown when it adds something (Claude's aliases
    // have no separate display name).
    let show_labels = models.iter().any(|m| m.label != m.id);
    let label_width = if show_labels {
        models
            .iter()
            .map(|m| m.label.chars().count())
            .max()
            .unwrap_or(0)
    } else {
        0
    };
    for m in models {
        let mut line = format!("  {:<id_width$}", m.id);
        if show_labels {
            line.push_str(&format!("  {:<label_width$}", m.label));
        }
        if !m.description.is_empty() {
            line.push_str(&format!("  {}", m.description));
        }
        let line = line.trim_end().to_string();
        if m.efforts.is_empty() {
            outln!("{line}");
        } else {
            outln!(
                "{line}{}",
                st.dim(&format!(" · effort: {}", m.efforts.join(", ")))
            );
        }
    }
    if models.is_empty() {
        outln!("  (no models listed)");
    }
    let note = match kind {
        AgentKind::Claude => {
            "Effort levels: low, medium, high, xhigh, max. Any full model name accepted by \
`claude --model` also works; your account may restrict which models are available."
        }
        AgentKind::Codex => "Any model name accepted by `codex -m` also works.",
    };
    outln!("  {}", st.dim(note));
}

fn doctor() -> Result<ExitCode> {
    let report = crate::doctor::run(&current_dir()?);
    for line in &report.lines {
        outln!("{line}");
    }
    Ok(if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn init() -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("Cannot read the current directory")?;
    let git = Git::discover(&cwd)?;
    let path = git.root().join(PLAN_FILE);
    if path.exists() {
        outln!("{PLAN_FILE} already exists; left untouched.");
    } else {
        std::fs::write(&path, PLAN_TEMPLATE)
            .with_context(|| format!("Cannot write {}", path.display()))?;
        outln!("Created {}.", path.display());
    }
    if git.ensure_excluded(".phasewise/")? {
        outln!("Added .phasewise/ to the git exclude file.");
    }
    outln!();
    outln!("Next steps:");
    outln!("  1. Edit {PLAN_FILE} to describe your phases.");
    outln!("  2. Commit it (`phasewise run` needs a clean working tree).");
    outln!("  3. phasewise check");
    outln!("  4. phasewise run");
    Ok(ExitCode::SUCCESS)
}

const NO_RUN: &str = "No run yet. Start one with `phasewise run`.";
const EXIT_PHASE_FAILED: u8 = 3;

fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().context("Cannot read the current directory")
}

fn run_command(
    plan: Option<PathBuf>,
    agent: Option<AgentArg>,
    auto: bool,
    settings: &SettingsArgs,
) -> Result<ExitCode> {
    let overrides = settings.overrides()?;
    let path = resolve_plan_path(plan)?;
    let run = ops::start(
        &current_dir()?,
        &path,
        agent.map(AgentKind::from),
        &overrides,
    )?;
    outln!(
        "Run {} on branch {} (original branch: {}, agent: {})",
        run.state.run_id,
        run.state.run_branch,
        run.state.original_branch,
        run.state.agent
    );
    drive(run, move |run, events, cancel| {
        ops::execute(run, auto, events, cancel).map(NextOutcome::Ran)
    })
}

fn next_command(auto: bool, settings: &SettingsArgs) -> Result<ExitCode> {
    let overrides = settings.overrides()?;
    let run = load_run()?;
    drive(run, move |run, events, cancel| {
        ops::next(run, auto, &overrides, events, cancel)
    })
}

/// Loads the latest run; says once if a phase left running was marked failed.
fn load_run() -> Result<Run> {
    let mut run = ops::load_current(&current_dir()?)?.ok_or_else(|| anyhow!(NO_RUN))?;
    announce_recovery(&run);
    // Operations may recover more phases under the run lock; only those are announced later.
    run.recovered.clear();
    Ok(run)
}

fn announce_recovery(run: &Run) {
    for number in &run.recovered {
        eprintln!(
            "Phase {number} was left running by a previous phasewise process and is now marked failed (interrupted)."
        );
    }
}

fn retry_command(note: Option<String>, settings: &SettingsArgs) -> Result<ExitCode> {
    let overrides = settings.overrides()?;
    let run = load_run()?;
    drive(run, move |run, events, cancel| {
        ops::retry(run, note.as_deref(), &overrides, events, cancel).map(NextOutcome::Ran)
    })
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn rollback_command(k: usize, yes: bool) -> Result<ExitCode> {
    let mut run = load_run()?;
    let sha = ops::prepare_rollback(&run, k)?;
    let target = if k == 0 {
        "the initial state (before phase 1)".to_string()
    } else {
        format!("the checkpoint of phase {k}")
    };
    outln!(
        "This goes back to {target} ({}) on {}:",
        short_sha(&sha),
        run.state.run_branch
    );
    outln!("  git reset --hard {} and git clean -fd", short_sha(&sha));
    outln!(
        "Changes after it are discarded and untracked files are deleted (ignored files are kept)."
    );
    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(anyhow!(
                "Rollback needs confirmation and stdin is not a terminal. Pass --yes to confirm."
            ));
        }
        out!("Continue? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .context("Cannot read the answer from stdin. Pass --yes to confirm.")?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            outln!("Nothing changed.");
            return Ok(ExitCode::SUCCESS);
        }
    }
    ops::rollback(&mut run, k, Some(&sha))?;
    announce_recovery(&run);
    outln!("Rolled back to {target} ({}).", short_sha(&sha));
    let later = run.state.phases.iter().filter(|p| p.number > k).count();
    if later > 0 {
        outln!("{later} later phase(s) are pending again.");
    }
    outln!("Next: `phasewise next` runs the next phase.");
    Ok(ExitCode::SUCCESS)
}

fn abort_command() -> Result<ExitCode> {
    let mut run = load_run()?;
    let outcome = ops::abort(&mut run)?;
    announce_recovery(&run);
    let state = &run.state;
    outln!("Run {} aborted.", state.run_id);
    if let Some(sha) = &outcome.leftovers_sha {
        outln!(
            "Uncommitted changes were saved in commit {} on {}.",
            short_sha(sha),
            state.run_branch
        );
    }
    if outcome.switched {
        outln!("You are back on {}.", outcome.current_branch);
    } else {
        outln!(
            "You are on {}; it was not changed because you were not on the run branch.",
            outcome.current_branch
        );
    }
    outln!(
        "The run branch {} is kept. To use or remove it:",
        state.run_branch
    );
    outln!(
        "  git switch {} && git merge --squash {} && git commit   integrate its work",
        state.original_branch,
        state.run_branch
    );
    outln!("  git branch -D {}   delete it", state.run_branch);
    Ok(ExitCode::SUCCESS)
}

/// Runs the engine work in a thread that owns the run and prints its events here.
fn drive<F>(run: Run, work: F) -> Result<ExitCode>
where
    F: FnOnce(&mut Run, &Sender<EngineEvent>, &AtomicBool) -> Result<NextOutcome> + Send + 'static,
{
    let cancel = process::install_signal_handler(Some(INTERRUPT_NOTICE))?;
    let verify_secs = ops::settings(&run)?.verify_timeout.as_secs();
    let run_agent_name = run.state.agent.to_string();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut run = run;
        let result = work(&mut run, &tx, &cancel);
        (run, result)
    });
    let started = Instant::now();
    let st = Styler::for_stdout();
    let agent = run_agent_name;
    for event in rx {
        // A closed stdout (for example `| head`) must not abort the run, so
        // write errors are ignored by `outln!`.
        if let Some(header) = phase_header(&event, &agent, &st) {
            outln!();
            outln!("{header}");
        } else if let Some(line) = event_line(&event, verify_secs) {
            outln!("{}", line.at(started.elapsed().as_secs()).render(&st));
        } else if let Some(summary) = phase_summary(&event, &st) {
            outln!("{summary}");
        }
    }
    let (run, result) = worker
        .join()
        .map_err(|_| anyhow!("The engine thread panicked. This is a bug; please report it."))?;
    announce_recovery(&run);
    Ok(report_outcome(&run, result?))
}

fn report_outcome(run: &Run, outcome: NextOutcome) -> ExitCode {
    match outcome {
        NextOutcome::Completed => {
            let state = &run.state;
            outln!("{}", Styler::for_stdout().ok("All phases done."));
            outln!("To integrate the work:");
            outln!(
                "  git switch {} && git merge --squash {} && git commit",
                state.original_branch,
                state.run_branch
            );
            ExitCode::SUCCESS
        }
        NextOutcome::Ran(PhaseStatus::Failed) => {
            print_failure(run);
            ExitCode::from(EXIT_PHASE_FAILED)
        }
        NextOutcome::Ran(_) => {
            let st = Styler::for_stdout();
            outln!();
            outln!("{}", st.dim("  phasewise diff      show what changed"));
            outln!(
                "{}",
                st.dim("  phasewise review    inspect it interactively")
            );
            outln!(
                "{}",
                st.dim("  phasewise next      approve it and continue")
            );
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
    let st = Styler::for_stdout();
    outln!();
    if let Some(failure) = &phase.failure {
        outln!(
            "{}",
            st.fail(&format!(
                "Phase {} failed ({}): {}",
                phase.number,
                failure_kind_label(failure.kind),
                failure.detail
            ))
        );
    } else {
        outln!("{}", st.fail(&format!("Phase {} failed.", phase.number)));
    }
    outln!("Options:");
    outln!(
        "{}",
        st.dim("  phasewise retry [--note \"...\"]   rerun the phase in a new session")
    );
    outln!(
        "{}",
        st.dim(&format!(
            "  phasewise rollback {}              go back to the previous checkpoint",
            phase.number - 1
        ))
    );
    outln!(
        "{}",
        st.dim("  or inspect the working tree; it was left as the agent left it")
    );
}

fn status() -> Result<ExitCode> {
    let Some(run) = ops::load_current(&current_dir()?)? else {
        outln!("{NO_RUN}");
        return Ok(ExitCode::SUCCESS);
    };
    announce_recovery(&run);
    let state = &run.state;
    let run_status = match state.status {
        RunStatus::Active => "active",
        RunStatus::Completed => "completed",
        RunStatus::Aborted => "aborted",
    };
    outln!("Run:    {} ({run_status})", state.run_id);
    outln!(
        "Plan:   {}",
        state.plan_title.as_deref().unwrap_or("(untitled)")
    );
    outln!("Agent:  {}", state.agent);
    outln!(
        "Branch: {} (from {})",
        state.run_branch,
        state.original_branch
    );
    outln!();

    let rows: Vec<[String; 6]> = state
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
                p.model.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    let headers = ["#", "STATUS", "TRIES", "FILES", "COST", "MODEL"];
    let mut widths = headers.map(str::len);
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let st = Styler::for_stdout();
    // Cells are padded before colouring so the ANSI codes do not affect widths.
    let line = |cells: [&str; 6], title: &str, status_role: Option<Role>| {
        let mut out = String::new();
        for (i, (cell, width)) in cells.iter().zip(widths).enumerate() {
            let padded = format!("{cell:<width$}");
            match (i, status_role) {
                (1, Some(role)) => out.push_str(&st.paint(role, &padded)),
                (_, None) => out.push_str(&st.heading(&padded)),
                _ => out.push_str(&padded),
            }
            out.push_str("  ");
        }
        match status_role {
            None => out.push_str(&st.heading(title)),
            Some(_) => out.push_str(title),
        }
        outln!("{out}");
    };
    line(headers, "TITLE", None);
    for (row, phase) in rows.iter().zip(&state.phases) {
        line(
            [&row[0], &row[1], &row[2], &row[3], &row[4], &row[5]],
            &phase.title,
            Some(status_role(phase.status)),
        );
        if let (PhaseStatus::Failed, Some(failure)) = (phase.status, &phase.failure) {
            outln!(
                "{}",
                st.fail(&format!(
                    "    {}: {}",
                    failure_kind_label(failure.kind),
                    failure.detail
                ))
            );
        }
    }
    outln!();
    outln!("{}", st.dim(&next_step_hint(&run)));
    Ok(ExitCode::SUCCESS)
}

fn status_role(status: PhaseStatus) -> Role {
    match status {
        PhaseStatus::Done => Role::Ok,
        PhaseStatus::AwaitingReview => Role::Accent,
        PhaseStatus::Failed => Role::Fail,
        PhaseStatus::Running | PhaseStatus::Verifying => Role::Warn,
        PhaseStatus::Pending => Role::Dim,
    }
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
    let run = load_run()?;
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
        outln!("Phase {number} made no changes.");
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

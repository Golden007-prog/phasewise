//! Command-line interface: clap definitions, dispatch and exit codes.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use crate::git::Git;
use crate::plan::{self, AgentKind};

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
        Some(Command::Run { .. }) => not_implemented("run"),
        Some(Command::Next { .. }) => not_implemented("next"),
        Some(Command::Retry { .. }) => not_implemented("retry"),
        Some(Command::Rollback { .. }) => not_implemented("rollback"),
        Some(Command::Status) => not_implemented("status"),
        Some(Command::Diff { .. }) => not_implemented("diff"),
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

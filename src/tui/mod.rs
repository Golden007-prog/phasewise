//! Terminal UI: data loading, state machine, rendering and the terminal loop.

pub mod app;
pub mod data;
pub mod home;
mod home_view;
pub mod view;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Once, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::agent::{ModelInfo, agent_for};
use crate::display::{failure_kind_label, first_line_short, plan_event_line, plan_header};
use crate::engine::EngineEvent;
use crate::ops::{self, NextOutcome, PlanEvent, PlanOutcome, PlanRequest, Run};
use crate::plan::{self, AgentKind};
use crate::process;
use crate::state::{PhaseStatus, RunState};
use crate::style::Styler;
use app::{App, Command, Tab};
use home::{Home, HomeCommand, PlanLaunch, Probe};

const NO_RUN: &str = "No run yet. Start one with `phasewise run`.";
const NEEDS_TERMINAL: &str = "phasewise review needs an interactive terminal. Run it from a terminal, or use `phasewise status` and `phasewise diff`.";
const NEEDS_TERMINAL_HOME: &str = "phasewise needs an interactive terminal for its home screen. Run it from a terminal, or use `phasewise --help` to see the commands.";
const POLL: Duration = Duration::from_millis(100);
const REFRESH_EVERY: Duration = Duration::from_millis(250);
/// How long the TUI loop may go without a turn, while a signal is pending or
/// the parent process has ended, before the watchdog ends phasewise.
const STUCK_AFTER: Duration = Duration::from_secs(30);
const WATCH_EVERY: Duration = Duration::from_millis(500);
/// Exit code when the watchdog ends phasewise (128 + SIGHUP).
const HANGUP_EXIT: i32 = 129;

/// When a TUI loop last went round, in ms since `CLOCK` started.
static LAST_BEAT_MS: AtomicU64 = AtomicU64::new(0);
static CLOCK: OnceLock<Instant> = OnceLock::new();

fn clock_ms() -> u64 {
    let start = CLOCK.get_or_init(Instant::now);
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Called at every turn of a TUI loop, for the watchdog.
fn beat() {
    LAST_BEAT_MS.store(clock_ms(), Ordering::SeqCst);
}

/// Starts (once per process) the thread that ends phasewise when its terminal
/// is gone. crossterm 0.29 never returns from `event::poll` on a closed
/// terminal: it keeps reading end-of-file. The loop then cannot see a signal
/// or a missing parent, so after `STUCK_AFTER` without a turn the watchdog
/// does what a second signal does: it kills the agents' process groups and
/// exits. A phase left running is recovered as interrupted on the next start.
fn start_hangup_watchdog(signalled: Arc<AtomicBool>) {
    static STARTED: Once = Once::new();
    STARTED.call_once(move || {
        beat();
        let parent = process::ParentWatch::new();
        thread::spawn(move || {
            loop {
                thread::sleep(WATCH_EVERY);
                let silent = Duration::from_millis(
                    clock_ms().saturating_sub(LAST_BEAT_MS.load(Ordering::SeqCst)),
                );
                if is_stuck(
                    silent,
                    signalled.load(Ordering::SeqCst),
                    parent.parent_gone(),
                ) {
                    process::kill_all_groups();
                    std::process::exit(HANGUP_EXIT);
                }
            }
        });
    });
}

/// Waits for a worker that was told to stop, beating meanwhile: the wait is
/// progress, not a stuck loop, so the watchdog must not cut it short.
fn join_beating<T>(handle: JoinHandle<T>) -> thread::Result<T> {
    while !handle.is_finished() {
        beat();
        thread::sleep(POLL);
    }
    handle.join()
}

/// The loop has been silent too long while it has a reason to stop.
fn is_stuck(silent: Duration, signalled: bool, parent_gone: bool) -> bool {
    silent >= STUCK_AFTER && (signalled || parent_gone)
}

/// Restores the terminal on every exit path, including `?` and unwinding.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// An engine call running in its own thread; it owns its own `Run`.
struct Worker {
    handle: JoinHandle<Result<NextOutcome>>,
    cancel: Arc<AtomicBool>,
    events: Receiver<EngineEvent>,
}

struct Tui<'a> {
    cwd: &'a Path,
    app: App,
    /// Read-only copy used to load the views.
    run: Run,
    worker: Option<Worker>,
    last_refresh: Instant,
    refresh_pending: bool,
    quitting: bool,
    /// Set by SIGINT/SIGTERM/SIGHUP (the first one acts like a confirmed quit).
    signalled: Arc<AtomicBool>,
    /// A closed terminal acts like a signal (see `ParentWatch`).
    parent: process::ParentWatch,
    /// Something changed since the last draw. Drawing only then keeps an idle
    /// TUI silent (no output every poll).
    dirty: bool,
}

/// Opens the review TUI. Refuses without a run or without a terminal.
pub fn run(cwd: &Path) -> Result<ExitCode> {
    let Some(run) = ops::load_current(cwd)? else {
        eprintln!("{NO_RUN}");
        return Ok(ExitCode::from(1));
    };
    if !std::io::stdout().is_terminal() {
        eprintln!("{NEEDS_TERMINAL}");
        return Ok(ExitCode::from(1));
    }
    let signalled = process::install_signal_handler(None)?;
    start_hangup_watchdog(Arc::clone(&signalled));
    let mut tui = Tui::new(cwd, run, signalled);
    tui.load_caches();
    let mut terminal = ratatui::try_init()?;
    let _guard = TerminalGuard;
    run_review(&mut tui, &mut terminal)?;
    Ok(ExitCode::SUCCESS)
}

/// The review loop; on an error it stops the agent and waits for it first.
fn run_review(tui: &mut Tui, terminal: &mut DefaultTerminal) -> Result<()> {
    if let Err(err) = tui.event_loop(terminal) {
        // Stop the agent and wait for it before reporting the error.
        tui.interrupt();
        if let Some(worker) = tui.worker.take() {
            let _ = join_beating(worker.handle);
        }
        return Err(err);
    }
    Ok(())
}

/// Opens the home screen (no active run, inside a git repository). Starting a
/// run from it switches to the review screen in the same terminal session.
pub fn run_home(cwd: &Path) -> Result<ExitCode> {
    if !std::io::stdout().is_terminal() {
        eprintln!("{NEEDS_TERMINAL_HOME}");
        return Ok(ExitCode::from(1));
    }
    let data = data::load_home(cwd)?;
    let signalled = process::install_signal_handler(None)?;
    start_hangup_watchdog(Arc::clone(&signalled));
    let mut home_tui = HomeTui::new(cwd, Home::new(data), Arc::clone(&signalled));
    home_tui.spawn_probes();
    let mut terminal = ratatui::try_init()?;
    let _guard = TerminalGuard;
    let started = match home_tui.event_loop(&mut terminal) {
        Ok(started) => started,
        Err(err) => {
            home_tui.interrupt();
            if let Some(worker) = home_tui.worker.take() {
                let _ = join_beating(worker.handle);
            }
            return Err(err);
        }
    };
    let Some(run) = started else {
        return Ok(ExitCode::SUCCESS);
    };
    // The same worker as review's `n`, running the phases of the new run.
    let mut tui = Tui::new(cwd, run, signalled);
    tui.spawn(|run, tx, cancel| ops::execute(run, false, tx, cancel).map(NextOutcome::Ran));
    tui.load_caches();
    run_review(&mut tui, &mut terminal)?;
    Ok(ExitCode::SUCCESS)
}

/// What the background threads of the home screen report.
enum Background {
    Version(AgentKind, Probe),
    Models(AgentKind, Result<Vec<ModelInfo>, String>),
}

/// The planning running in its own thread.
struct PlanWorker {
    handle: JoinHandle<Result<PlanOutcome>>,
    cancel: Arc<AtomicBool>,
    events: Receiver<PlanEvent>,
    started: Instant,
}

struct HomeTui<'a> {
    cwd: &'a Path,
    home: Home,
    worker: Option<PlanWorker>,
    background: Receiver<Background>,
    background_tx: mpsc::Sender<Background>,
    quitting: bool,
    signalled: Arc<AtomicBool>,
    /// A closed terminal acts like a signal (see `ParentWatch`).
    parent: process::ParentWatch,
    dirty: bool,
    /// Set when a run was started: the loop ends and review takes over.
    started: Option<Run>,
}

impl<'a> HomeTui<'a> {
    fn new(cwd: &'a Path, home: Home, signalled: Arc<AtomicBool>) -> HomeTui<'a> {
        let (background_tx, background) = mpsc::channel();
        HomeTui {
            cwd,
            home,
            worker: None,
            background,
            background_tx,
            quitting: false,
            signalled,
            parent: process::ParentWatch::new(),
            dirty: true,
            started: None,
        }
    }

    /// Looks for the version of both agents, each in its own thread.
    fn spawn_probes(&self) {
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            let tx = self.background_tx.clone();
            thread::spawn(move || {
                let probe = match agent_for(kind).version() {
                    Ok(version) => Probe::Version(version),
                    Err(_) => Probe::NotInstalled,
                };
                // The screen may be gone already.
                let _ = tx.send(Background::Version(kind, probe));
            });
        }
    }

    fn spawn_models(&self, kind: AgentKind) {
        let tx = self.background_tx.clone();
        thread::spawn(move || {
            let result = agent_for(kind)
                .list_models()
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Background::Models(kind, result));
        });
    }

    /// Runs until the user quits (`None`) or starts a run (`Some`).
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<Option<Run>> {
        loop {
            beat();
            if let Some(run) = self.started.take() {
                return Ok(Some(run));
            }
            if self.dirty {
                terminal.draw(|frame| home_view::draw(frame, &self.home))?;
                self.dirty = false;
            }
            if !self.quitting
                && (self.signalled.load(Ordering::SeqCst) || self.parent.parent_gone())
            {
                self.execute(HomeCommand::Quit);
                self.dirty = true;
            }
            if self.quitting && self.worker.is_none() {
                return Ok(None);
            }
            if event::poll(POLL)? {
                self.dirty = true;
                if let Event::Key(key) = event::read()?
                    && key.kind != KeyEventKind::Release
                    && !self.quitting
                    && let Some(command) = self.home.handle_key(key)
                {
                    self.execute(command);
                }
            }
            self.drain_background();
            self.drain_plan_events();
            self.finish_worker();
        }
    }

    fn execute(&mut self, command: HomeCommand) {
        match command {
            HomeCommand::Quit => {
                if self.worker.is_some() {
                    self.interrupt();
                    self.home.message = Some(
                        "Interrupting… phasewise will exit when the agent has stopped".to_string(),
                    );
                }
                self.quitting = true;
            }
            HomeCommand::Reload => {
                if self.reload() {
                    self.home.message = Some("Plan checked again.".to_string());
                }
            }
            HomeCommand::Start { agent, overrides } => {
                match ops::start(self.cwd, &self.home.data.plan_path, Some(agent), &overrides) {
                    Ok(run) => self.started = Some(run),
                    Err(err) => self.home.show_start_error(&err),
                }
            }
            HomeCommand::Plan(launch) => self.start_planning(launch),
            HomeCommand::CancelPlan => {
                self.interrupt();
                self.home.message = Some("Cancelling the planning…".to_string());
            }
            HomeCommand::LoadModels(kind) => self.spawn_models(kind),
        }
    }

    fn interrupt(&self) {
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn start_planning(&mut self, launch: PlanLaunch) {
        if self.worker.is_some() {
            return;
        }
        // The file did not exist when the screen was loaded: ask again rather
        // than fail with the CLI's `--force` advice.
        if !launch.force && self.home.data.plan_path.exists() {
            self.reload();
            self.home.message = Some(format!(
                "{} was created meanwhile. Press p again to replace it, or c to check it.",
                plan::DEFAULT_PLAN_FILE
            ));
            return;
        }
        let timeout = match ops::plan_timeout() {
            Ok(timeout) => timeout,
            Err(err) => {
                self.home
                    .show_error(err.to_string().lines().map(str::to_string).collect());
                return;
            }
        };
        let request = PlanRequest {
            goal: launch.goal,
            agent: launch.agent,
            options: launch.options,
            output: Some(self.home.data.plan_path.clone()),
            force: launch.force,
            timeout,
        };
        let cwd: PathBuf = self.cwd.to_path_buf();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (tx, events) = mpsc::channel();
        let handle = thread::spawn(move || ops::create_plan(&cwd, &request, &tx, &worker_cancel));
        self.worker = Some(PlanWorker {
            handle,
            cancel,
            events,
            started: Instant::now(),
        });
        self.home.begin_planning();
    }

    fn drain_background(&mut self) {
        while let Ok(message) = self.background.try_recv() {
            match message {
                Background::Version(kind, probe) => self.home.set_probe(kind, probe),
                Background::Models(kind, result) => self.home.set_models(kind, result),
            }
            self.dirty = true;
        }
    }

    fn drain_plan_events(&mut self) {
        let Some(worker) = &self.worker else { return };
        let styler = Styler { color: false };
        let mut changed = false;
        // Empty or disconnected: nothing more to show now.
        while let Ok(event) = worker.events.try_recv() {
            // Paths inside the repository are shown relative to its root.
            let event = match event {
                PlanEvent::Written { path } => PlanEvent::Written {
                    path: path
                        .strip_prefix(&self.home.data.root)
                        .map_or(path.clone(), Path::to_path_buf),
                },
                other => other,
            };
            let header = plan_header(&event, &styler);
            let line = plan_event_line(&event).map(|l| l.at(worker.started.elapsed().as_secs()));
            self.home.plan_event(header, line);
            changed = true;
        }
        if changed {
            self.dirty = true;
        }
    }

    /// When the planning has ended: joins it, reloads the plan and reports.
    fn finish_worker(&mut self) {
        if !self.worker.as_ref().is_some_and(|w| w.handle.is_finished()) {
            return;
        }
        // Everything the worker sent is in the channel now.
        self.drain_plan_events();
        let Some(worker) = self.worker.take() else {
            return;
        };
        let result = worker.handle.join().unwrap_or_else(|_| {
            Err(anyhow!(
                "The planning thread panicked. This is a bug; please report it."
            ))
        });
        self.dirty = true;
        // A failed reload leaves an error message; the report of the planning
        // keeps priority when it has a message of its own.
        self.reload();
        if self.quitting {
            return;
        }
        let report = home::plan_report(&result, &self.home.data.root);
        self.home.plan_finished(report);
    }

    /// Loads the home data again, keeping the user's choices. False on error.
    fn reload(&mut self) -> bool {
        self.dirty = true;
        match data::load_home(self.cwd) {
            Ok(data) => {
                self.home.set_data(data);
                true
            }
            Err(err) => {
                self.home.message = Some(format!("Error: {err}"));
                false
            }
        }
    }
}

impl<'a> Tui<'a> {
    fn new(cwd: &'a Path, run: Run, signalled: Arc<AtomicBool>) -> Tui<'a> {
        Tui {
            cwd,
            app: App::new(data::load_run_view(&run)),
            run,
            worker: None,
            last_refresh: Instant::now(),
            refresh_pending: false,
            quitting: false,
            signalled,
            parent: process::ParentWatch::new(),
            dirty: true,
        }
    }
}

impl Tui<'_> {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            beat();
            if self.dirty {
                terminal.draw(|frame| view::draw(frame, &mut self.app))?;
                self.dirty = false;
            }
            if !self.quitting
                && (self.signalled.load(Ordering::SeqCst) || self.parent.parent_gone())
            {
                self.execute(Command::Quit);
                self.dirty = true;
            }
            if self.quitting && self.worker.is_none() {
                return Ok(());
            }
            if event::poll(POLL)? {
                // Any key or resize may change what is shown.
                self.dirty = true;
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        // While quitting only drawing continues; keys are ignored.
                        if !self.quitting
                            && let Some(command) = self.app.handle_key(key)
                        {
                            self.execute(command);
                        }
                    }
                    // A resize needs nothing more than the redraw of the next turn.
                    _ => {}
                }
            }
            self.drain_events();
            self.finish_worker();
            if self.refresh_pending && self.last_refresh.elapsed() >= REFRESH_EVERY {
                self.refresh();
            }
        }
    }

    fn execute(&mut self, command: Command) {
        match command {
            Command::Quit => {
                if self.worker.is_some() {
                    self.interrupt();
                    self.app.message = Some(
                        "Interrupting… phasewise will exit when the agent has stopped".to_string(),
                    );
                }
                self.quitting = true;
            }
            Command::Reload => self.refresh(),
            Command::Interrupt => {
                self.interrupt();
                self.app.message = Some(match self.app.running {
                    Some(n) => format!("Interrupting phase {n}…"),
                    None => "Interrupting…".to_string(),
                });
            }
            Command::Next => self.spawn(|run, tx, cancel| {
                ops::next(run, false, &ops::Overrides::default(), tx, cancel)
            }),
            Command::Retry(note) => self.spawn(move |run, tx, cancel| {
                ops::retry(run, note.as_deref(), &ops::Overrides::default(), tx, cancel)
                    .map(NextOutcome::Ran)
            }),
            Command::Rollback(k) => self.rollback(k),
        }
    }

    fn interrupt(&self) {
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn spawn<F>(&mut self, work: F)
    where
        F: FnOnce(&mut Run, &mpsc::Sender<EngineEvent>, &AtomicBool) -> Result<NextOutcome>
            + Send
            + 'static,
    {
        if self.worker.is_some() {
            return;
        }
        let mut run = match ops::load_current(self.cwd) {
            Ok(Some(run)) => run,
            Ok(None) => {
                self.app.message = Some(format!("Error: {NO_RUN}"));
                return;
            }
            Err(err) => {
                self.app.message = Some(format!("Error: {err}"));
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (tx, events) = mpsc::channel();
        let handle = thread::spawn(move || work(&mut run, &tx, &worker_cancel));
        self.worker = Some(Worker {
            handle,
            cancel,
            events,
        });
    }

    fn rollback(&mut self, k: usize) {
        if self.worker.is_some() {
            self.app.message = Some(
                "A phase is running; wait for it to finish or press x to interrupt it.".to_string(),
            );
            return;
        }
        let result = ops::load_current(self.cwd)
            .and_then(|run| run.ok_or_else(|| anyhow!(NO_RUN)))
            .and_then(|mut run| ops::rollback(&mut run, k, None));
        self.refresh();
        self.app.message = Some(match result {
            Ok(()) if k == 0 => "Rolled back to the initial state.".to_string(),
            Ok(()) => format!("Rolled back to the checkpoint of phase {k}."),
            Err(err) => format!("Error: {err}"),
        });
    }

    fn drain_events(&mut self) {
        let Some(worker) = &self.worker else { return };
        let mut changed = false;
        loop {
            match worker.events.try_recv() {
                Ok(EngineEvent::PhaseStarted { phase, .. }) => {
                    self.app.running = Some(phase);
                    changed = true;
                }
                Ok(EngineEvent::PhaseFinished { .. }) => {
                    self.app.running = None;
                    changed = true;
                }
                Ok(_) => changed = true,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if changed {
            self.refresh_pending = true;
            self.dirty = true;
        }
    }

    /// When the worker has ended: joins it, reloads and reports the outcome.
    fn finish_worker(&mut self) {
        if !self.worker.as_ref().is_some_and(|w| w.handle.is_finished()) {
            return;
        }
        let Some(worker) = self.worker.take() else {
            return;
        };
        let result = worker.handle.join().unwrap_or_else(|_| {
            Err(anyhow!(
                "The engine thread panicked. This is a bug; please report it."
            ))
        });
        self.app.running = None;
        self.refresh();
        self.app.message = Some(outcome_message(&result, &self.run.state));
    }

    /// Reloads the run and the caches of the selected phase.
    fn refresh(&mut self) {
        self.dirty = true;
        self.refresh_pending = false;
        self.last_refresh = Instant::now();
        match ops::load_current(self.cwd) {
            Ok(Some(run)) => {
                self.run = run;
                self.app.set_view(data::load_run_view(&self.run));
            }
            Ok(None) => {}
            Err(err) => self.app.message = Some(format!("Error: {err}")),
        }
        self.load_caches();
    }

    fn load_caches(&mut self) {
        let Some(number) = self.app.selected_number() else {
            return;
        };
        let diff = data::load_diff(&self.run, number).unwrap_or_else(|err| data::DiffView {
            note: Some(format!("Cannot load the diff: {err}")),
            ..data::DiffView::default()
        });
        let follow = self.app.tab == Tab::Log
            && self.app.running == Some(number)
            && at_log_end(self.app.scroll, self.app.log.len(), self.app.page);
        let log = data::load_log(&self.run, number);
        let verify = data::load_verify(&self.run, number);
        self.app.set_caches(diff, log, verify);
        if follow {
            // The view clamps the scroll to the end of the content.
            self.app.scroll = u16::MAX;
        }
    }
}

/// True when the view shows the end of a log of `len` lines (so it follows it).
fn at_log_end(scroll: u16, len: usize, page: u16) -> bool {
    usize::from(scroll) >= len.saturating_sub(usize::from(page))
}

/// The message shown when a worker ends.
fn outcome_message(result: &Result<NextOutcome>, state: &RunState) -> String {
    match result {
        Err(err) => format!("Error: {err}"),
        Ok(NextOutcome::Completed) => format!(
            "All phases done. To integrate: git switch {} && git merge --squash {} && git commit",
            state.original_branch, state.run_branch
        ),
        Ok(NextOutcome::Ran(PhaseStatus::Failed)) => {
            match state
                .phases
                .iter()
                .find(|p| p.status == PhaseStatus::Failed)
            {
                Some(phase) => match &phase.failure {
                    Some(failure) => format!(
                        "Phase {} failed ({}): {}",
                        phase.number,
                        failure_kind_label(failure.kind),
                        first_line_short(&failure.detail)
                    ),
                    None => format!("Phase {} failed.", phase.number),
                },
                None => "The phase failed.".to_string(),
            }
        }
        Ok(NextOutcome::Ran(_)) => {
            let number = state
                .phases
                .iter()
                .rev()
                .find(|p| p.status == PhaseStatus::AwaitingReview)
                .map_or(0, |p| p.number);
            format!("Phase {number} is ready for review. Press n to approve it and continue.")
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;
    use crate::plan::AgentKind;
    use crate::state::{Failure, FailureKind, PhaseState, RunStatus};

    fn phase(number: usize, status: PhaseStatus, failure: Option<Failure>) -> PhaseState {
        PhaseState {
            number,
            title: format!("Phase {number}"),
            status,
            attempts: 1,
            session_id: None,
            checkpoint_sha: None,
            summary: None,
            cost_usd: None,
            started_at: None,
            finished_at: None,
            failure,
            model: None,
            effort: None,
        }
    }

    fn state(phases: Vec<PhaseState>) -> RunState {
        RunState {
            schema_version: 1,
            run_id: "r".into(),
            plan_path: "phasewise.md".into(),
            plan_title: None,
            agent: AgentKind::Claude,
            original_branch: "main".into(),
            run_branch: "phasewise/r".into(),
            base_sha: "abc".into(),
            status: RunStatus::Active,
            created_at: Timestamp::UNIX_EPOCH,
            updated_at: Timestamp::UNIX_EPOCH,
            phases,
            model_override: None,
            effort_override: None,
        }
    }

    #[test]
    fn ready_for_review_message_names_the_phase() {
        let s = state(vec![
            phase(1, PhaseStatus::Done, None),
            phase(2, PhaseStatus::AwaitingReview, None),
        ]);
        let msg = outcome_message(&Ok(NextOutcome::Ran(PhaseStatus::AwaitingReview)), &s);
        assert!(msg.starts_with("Phase 2 is ready for review"), "{msg}");
    }

    #[test]
    fn failure_message_has_kind_and_detail() {
        let failure = Failure {
            kind: FailureKind::VerifyFailed,
            detail: "cargo test failed\nmore".into(),
        };
        let s = state(vec![phase(1, PhaseStatus::Failed, Some(failure))]);
        let msg = outcome_message(&Ok(NextOutcome::Ran(PhaseStatus::Failed)), &s);
        assert_eq!(msg, "Phase 1 failed (verify_failed): cargo test failed");
    }

    #[test]
    fn completed_message_has_the_merge_hint() {
        let s = state(vec![phase(1, PhaseStatus::Done, None)]);
        let msg = outcome_message(&Ok(NextOutcome::Completed), &s);
        assert!(msg.contains("All phases done"));
        assert!(msg.contains("git switch main && git merge --squash phasewise/r && git commit"));
    }

    #[test]
    fn error_message_is_prefixed() {
        let s = state(vec![]);
        let msg = outcome_message(&Err(anyhow!("boom")), &s);
        assert_eq!(msg, "Error: boom");
    }

    #[test]
    fn the_watchdog_acts_only_on_a_silent_loop_with_a_reason_to_stop() {
        let long = STUCK_AFTER;
        let short = STUCK_AFTER - Duration::from_millis(1);
        assert!(is_stuck(long, true, false));
        assert!(is_stuck(long, false, true));
        assert!(!is_stuck(long, false, false));
        assert!(!is_stuck(short, true, true));
    }

    #[test]
    fn follows_only_when_at_the_end() {
        assert!(at_log_end(0, 5, 10));
        assert!(at_log_end(20, 30, 10));
        assert!(at_log_end(25, 30, 10));
        assert!(!at_log_end(5, 30, 10));
    }
}

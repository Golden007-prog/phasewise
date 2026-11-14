//! Terminal UI: data loading, state machine, rendering and the terminal loop.

pub mod app;
pub mod data;
pub mod view;

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::display::{failure_kind_label, first_line_short};
use crate::engine::EngineEvent;
use crate::ops::{self, NextOutcome, Run};
use crate::state::{PhaseStatus, RunState};
use app::{App, Command, Tab};

const NO_RUN: &str = "No run yet. Start one with `phasewise run`.";
const NEEDS_TERMINAL: &str = "phasewise review needs an interactive terminal. Run it from a terminal, or use `phasewise status` and `phasewise diff`.";
const POLL: Duration = Duration::from_millis(100);
const REFRESH_EVERY: Duration = Duration::from_millis(250);

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
    let mut tui = Tui {
        cwd,
        app: App::new(data::load_run_view(&run)),
        run,
        worker: None,
        last_refresh: Instant::now(),
        refresh_pending: false,
        quitting: false,
        dirty: true,
    };
    tui.load_caches();
    let mut terminal = ratatui::try_init()?;
    let _guard = TerminalGuard;
    tui.event_loop(&mut terminal)?;
    Ok(ExitCode::SUCCESS)
}

impl Tui<'_> {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            if self.dirty {
                terminal.draw(|frame| view::draw(frame, &mut self.app))?;
                self.dirty = false;
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
            Command::Next => self.spawn(|run, tx, cancel| ops::next(run, false, tx, cancel)),
            Command::Retry(note) => self.spawn(move |run, tx, cancel| {
                ops::retry(run, note.as_deref(), tx, cancel).map(NextOutcome::Ran)
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
        let result = ops::load_current(self.cwd)
            .and_then(|run| run.ok_or_else(|| anyhow!(NO_RUN)))
            .and_then(|mut run| ops::rollback(&mut run, k));
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
    fn follows_only_when_at_the_end() {
        assert!(at_log_end(0, 5, 10));
        assert!(at_log_end(20, 30, 10));
        assert!(at_log_end(25, 30, 10));
        assert!(!at_log_end(5, 30, 10));
    }
}

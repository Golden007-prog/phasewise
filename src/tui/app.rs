//! TUI state machine: keys in, commands out. No terminal, no I/O.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::data::{DiffView, RunView};
use crate::state::PhaseStatus;

const DEFAULT_PAGE: u16 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Diff,
    Log,
    Verify,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Diff, Tab::Log, Tab::Verify];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Diff => "Diff",
            Tab::Log => "Log",
            Tab::Verify => "Verify",
        }
    }

    fn next(self) -> Tab {
        match self {
            Tab::Diff => Tab::Log,
            Tab::Log => Tab::Verify,
            Tab::Verify => Tab::Diff,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    ConfirmRollback(usize),
    ConfirmInterrupt,
    ConfirmQuit,
    /// Optional note for a retry, being typed.
    Note(String),
}

/// What the event loop must do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Quit,
    Next,
    Retry(Option<String>),
    Rollback(usize),
    Interrupt,
    /// Reload the caches of the (newly) selected phase.
    Reload,
}

#[derive(Debug, Clone)]
pub struct App {
    pub view: RunView,
    /// Index into `view.phases`.
    pub selected: usize,
    pub tab: Tab,
    pub scroll: u16,
    /// Content height of the right panel; set by the view on every draw.
    pub page: u16,
    pub mode: Mode,
    /// Phase run by a worker thread, if any.
    pub running: Option<usize>,
    pub message: Option<String>,
    pub diff: DiffView,
    pub log: Vec<String>,
    pub verify: String,
}

impl App {
    /// Selects the phase that most likely needs attention.
    pub fn new(view: RunView) -> App {
        let pick =
            |wanted: &[PhaseStatus]| view.phases.iter().position(|p| wanted.contains(&p.status));
        let selected = pick(&[
            PhaseStatus::Failed,
            PhaseStatus::Running,
            PhaseStatus::Verifying,
            PhaseStatus::AwaitingReview,
        ])
        .or_else(|| pick(&[PhaseStatus::Pending]))
        .unwrap_or(view.phases.len().saturating_sub(1));
        App {
            view,
            selected,
            tab: Tab::Diff,
            scroll: 0,
            page: DEFAULT_PAGE,
            mode: Mode::Normal,
            running: None,
            message: None,
            diff: DiffView::default(),
            log: Vec::new(),
            verify: String::new(),
        }
    }

    /// Number of the selected phase (1-based); `None` for an empty run.
    pub fn selected_number(&self) -> Option<usize> {
        self.view.phases.get(self.selected).map(|p| p.number)
    }

    /// Replaces the run view, keeping the selection in range.
    pub fn set_view(&mut self, view: RunView) {
        self.view = view;
        self.selected = self.selected.min(self.view.phases.len().saturating_sub(1));
    }

    pub fn set_caches(&mut self, diff: DiffView, log: Vec<String>, verify: String) {
        self.diff = diff;
        self.log = log;
        self.verify = verify;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Command> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        self.message = None;
        match self.mode.clone() {
            Mode::Normal => self.key_normal(key),
            Mode::ConfirmRollback(n) => {
                self.mode = Mode::Normal;
                is_yes(key).then_some(Command::Rollback(n))
            }
            Mode::ConfirmInterrupt => {
                self.mode = Mode::Normal;
                is_yes(key).then_some(Command::Interrupt)
            }
            Mode::ConfirmQuit => {
                self.mode = Mode::Normal;
                is_yes(key).then_some(Command::Quit)
            }
            Mode::Note(buffer) => self.key_note(key, buffer),
        }
    }

    fn key_note(&mut self, key: KeyEvent, mut buffer: String) -> Option<Command> {
        match key.code {
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                let note = buffer.trim();
                Some(Command::Retry((!note.is_empty()).then(|| note.to_string())))
            }
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                None
            }
            KeyCode::Backspace => {
                buffer.pop();
                self.mode = Mode::Note(buffer);
                None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                buffer.push(c);
                self.mode = Mode::Note(buffer);
                None
            }
            _ => None,
        }
    }

    fn key_normal(&mut self, key: KeyEvent) -> Option<Command> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.quit(),
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('j') | KeyCode::Down => self.select(1),
            KeyCode::Char('k') | KeyCode::Up => self.select(-1),
            KeyCode::Tab => {
                self.tab = self.tab.next();
                self.scroll = 0;
                None
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(self.page);
                None
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(self.page);
                None
            }
            KeyCode::Char('n') => match self.why_not_next() {
                None => Some(Command::Next),
                Some(why) => self.refuse(why),
            },
            KeyCode::Char('r') => match self.why_not_retry() {
                None => {
                    self.mode = Mode::Note(String::new());
                    None
                }
                Some(why) => self.refuse(why),
            },
            KeyCode::Char('b') => match self.why_not_rollback() {
                None => {
                    self.selected_number()
                        .inspect(|&n| self.mode = Mode::ConfirmRollback(n));
                    None
                }
                Some(why) => self.refuse(why),
            },
            KeyCode::Char('x') => {
                if self.running.is_some() {
                    self.mode = Mode::ConfirmInterrupt;
                    None
                } else {
                    self.refuse("No phase is running.".to_string())
                }
            }
            _ => None,
        }
    }

    fn refuse(&mut self, why: String) -> Option<Command> {
        self.message = Some(why);
        None
    }

    fn quit(&mut self) -> Option<Command> {
        if self.running.is_some() {
            self.mode = Mode::ConfirmQuit;
            None
        } else {
            Some(Command::Quit)
        }
    }

    fn select(&mut self, delta: isize) -> Option<Command> {
        let last = self.view.phases.len().saturating_sub(1);
        let target = self.selected.saturating_add_signed(delta).min(last);
        if target == self.selected {
            return None;
        }
        self.selected = target;
        self.scroll = 0;
        Some(Command::Reload)
    }

    fn running_reason(&self) -> Option<String> {
        self.running
            .map(|n| format!("Phase {n} is running: press x to interrupt it."))
    }

    fn first_with(&self, status: PhaseStatus) -> Option<usize> {
        self.view
            .phases
            .iter()
            .find(|p| p.status == status)
            .map(|p| p.number)
    }

    fn why_not_next(&self) -> Option<String> {
        if !self.view.active {
            return Some("The run is not active: nothing more to launch.".to_string());
        }
        if let Some(why) = self.running_reason() {
            return Some(why);
        }
        if let Some(n) = self.first_with(PhaseStatus::Failed) {
            return Some(format!("Phase {n} failed: press r to retry it."));
        }
        let can = self
            .view
            .phases
            .iter()
            .any(|p| matches!(p.status, PhaseStatus::AwaitingReview | PhaseStatus::Pending));
        (!can).then(|| "No phase is left to run.".to_string())
    }

    fn why_not_retry(&self) -> Option<String> {
        if !self.view.active {
            return Some("The run is not active: nothing to retry.".to_string());
        }
        if let Some(why) = self.running_reason() {
            return Some(why);
        }
        let can = self
            .view
            .phases
            .iter()
            .any(|p| matches!(p.status, PhaseStatus::Failed | PhaseStatus::AwaitingReview));
        (!can).then(|| "Nothing to retry: no phase failed or awaits review.".to_string())
    }

    fn why_not_rollback(&self) -> Option<String> {
        if let Some(why) = self.running_reason() {
            return Some(why);
        }
        match self.view.phases.get(self.selected) {
            None => Some("There is no phase to roll back to.".to_string()),
            Some(p) if !p.has_checkpoint => Some(format!(
                "Phase {} has no checkpoint to roll back to.",
                p.number
            )),
            Some(_) => None,
        }
    }

    /// Keys shown in the bottom bar for the current state.
    pub fn available_keys(&self) -> Vec<(&'static str, &'static str)> {
        match self.mode {
            Mode::ConfirmRollback(_) | Mode::ConfirmInterrupt | Mode::ConfirmQuit => {
                return vec![("y", "confirm"), ("any other key", "cancel")];
            }
            Mode::Note(_) => return vec![("Enter", "retry"), ("Esc", "cancel")],
            Mode::Normal => {}
        }
        let mut keys = vec![("j/k", "phase"), ("Tab", "tab"), ("PgUp/PgDn", "scroll")];
        if self.why_not_next().is_none() {
            keys.push(("n", "next"));
        }
        if self.why_not_retry().is_none() {
            keys.push(("r", "retry"));
        }
        if self.why_not_rollback().is_none() {
            keys.push(("b", "rollback"));
        }
        if self.running.is_some() {
            keys.push(("x", "interrupt"));
        }
        keys.push(("q", "quit"));
        keys
    }
}

fn is_yes(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('y') && !key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::PhaseView;

    fn phase(number: usize, status: PhaseStatus, checkpoint: bool) -> PhaseView {
        PhaseView {
            number,
            title: format!("Phase {number}"),
            status,
            attempts: 1,
            failure: None,
            has_checkpoint: checkpoint,
        }
    }

    fn app(phases: Vec<PhaseView>) -> App {
        App::new(RunView {
            run_id: "run".into(),
            plan_title: "Plan".into(),
            agent: "claude".into(),
            run_branch: "phasewise/run".into(),
            original_branch: "main".into(),
            active: true,
            phases,
        })
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn code(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn reviewing() -> App {
        app(vec![
            phase(1, PhaseStatus::Done, true),
            phase(2, PhaseStatus::AwaitingReview, true),
            phase(3, PhaseStatus::Pending, false),
        ])
    }

    #[test]
    fn initial_selection_is_the_phase_needing_attention() {
        assert_eq!(reviewing().selected, 1);
        let failed = app(vec![
            phase(1, PhaseStatus::Done, true),
            phase(2, PhaseStatus::Failed, false),
        ]);
        assert_eq!(failed.selected, 1);
        let done = app(vec![phase(1, PhaseStatus::Done, true)]);
        assert_eq!(done.selected, 0);
    }

    #[test]
    fn selection_moves_clamps_and_resets_scroll() {
        let mut a = reviewing();
        a.scroll = 5;
        assert_eq!(a.handle_key(key('j')), Some(Command::Reload));
        assert_eq!((a.selected, a.scroll), (2, 0));
        assert_eq!(a.handle_key(code(KeyCode::Down)), None);
        assert_eq!(a.selected, 2);
        assert_eq!(a.handle_key(key('k')), Some(Command::Reload));
        assert_eq!(a.handle_key(code(KeyCode::Up)), Some(Command::Reload));
        assert_eq!(a.handle_key(code(KeyCode::Up)), None);
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn tab_cycles_and_resets_scroll() {
        let mut a = reviewing();
        a.scroll = 3;
        a.handle_key(code(KeyCode::Tab));
        assert_eq!((a.tab, a.scroll), (Tab::Log, 0));
        a.handle_key(code(KeyCode::Tab));
        assert_eq!(a.tab, Tab::Verify);
        a.handle_key(code(KeyCode::Tab));
        assert_eq!(a.tab, Tab::Diff);
    }

    #[test]
    fn page_keys_scroll_by_the_page_height() {
        let mut a = reviewing();
        a.page = 7;
        a.handle_key(code(KeyCode::PageDown));
        a.handle_key(code(KeyCode::PageDown));
        assert_eq!(a.scroll, 14);
        a.handle_key(code(KeyCode::PageUp));
        assert_eq!(a.scroll, 7);
        a.handle_key(code(KeyCode::PageUp));
        a.handle_key(code(KeyCode::PageUp));
        assert_eq!(a.scroll, 0);
    }

    #[test]
    fn next_is_allowed_when_a_phase_awaits_review_or_is_pending() {
        let mut a = reviewing();
        assert_eq!(a.handle_key(key('n')), Some(Command::Next));
        let mut p = app(vec![phase(1, PhaseStatus::Pending, false)]);
        assert_eq!(p.handle_key(key('n')), Some(Command::Next));
    }

    #[test]
    fn next_is_refused_with_a_reason() {
        let mut failed = app(vec![
            phase(1, PhaseStatus::Done, true),
            phase(2, PhaseStatus::Failed, false),
        ]);
        assert_eq!(failed.handle_key(key('n')), None);
        assert_eq!(
            failed.message.as_deref(),
            Some("Phase 2 failed: press r to retry it.")
        );

        let mut running = reviewing();
        running.running = Some(2);
        assert_eq!(running.handle_key(key('n')), None);
        assert!(
            running
                .message
                .as_deref()
                .is_some_and(|m| m.contains("running"))
        );

        let mut inactive = reviewing();
        inactive.view.active = false;
        assert_eq!(inactive.handle_key(key('n')), None);
        assert!(inactive.message.is_some());

        let mut done = app(vec![phase(1, PhaseStatus::Done, true)]);
        assert_eq!(done.handle_key(key('n')), None);
        assert!(done.message.is_some());
    }

    #[test]
    fn message_clears_on_the_next_key() {
        let mut a = app(vec![phase(1, PhaseStatus::Done, true)]);
        a.handle_key(key('n'));
        assert!(a.message.is_some());
        a.handle_key(key('j'));
        assert!(a.message.is_none());
    }

    #[test]
    fn retry_opens_the_note_and_enter_sends_it() {
        let mut a = reviewing();
        assert_eq!(a.handle_key(key('r')), None);
        assert_eq!(a.mode, Mode::Note(String::new()));
        for c in "fix it".chars() {
            a.handle_key(key(c));
        }
        a.handle_key(key('!'));
        a.handle_key(code(KeyCode::Backspace));
        assert_eq!(a.mode, Mode::Note("fix it".into()));
        assert_eq!(
            a.handle_key(code(KeyCode::Enter)),
            Some(Command::Retry(Some("fix it".into())))
        );
        assert_eq!(a.mode, Mode::Normal);
    }

    #[test]
    fn retry_with_an_empty_note_sends_none_and_esc_cancels() {
        let mut a = reviewing();
        a.handle_key(key('r'));
        assert_eq!(
            a.handle_key(code(KeyCode::Enter)),
            Some(Command::Retry(None))
        );
        a.handle_key(key('r'));
        a.handle_key(key('x'));
        assert_eq!(a.handle_key(code(KeyCode::Esc)), None);
        assert_eq!(a.mode, Mode::Normal);
    }

    #[test]
    fn retry_is_refused_without_a_candidate_or_while_running() {
        let mut a = app(vec![
            phase(1, PhaseStatus::Done, true),
            phase(2, PhaseStatus::Pending, false),
        ]);
        a.handle_key(key('r'));
        assert_eq!(a.mode, Mode::Normal);
        assert!(a.message.is_some());

        let mut running = reviewing();
        running.running = Some(2);
        running.handle_key(key('r'));
        assert_eq!(running.mode, Mode::Normal);
        assert!(running.message.is_some());
    }

    #[test]
    fn rollback_needs_a_checkpoint_and_confirmation() {
        let mut a = reviewing();
        a.handle_key(key('b'));
        assert_eq!(a.mode, Mode::ConfirmRollback(2));
        assert_eq!(a.handle_key(key('y')), Some(Command::Rollback(2)));
        assert_eq!(a.mode, Mode::Normal);

        a.handle_key(key('b'));
        assert_eq!(a.handle_key(key('n')), None);
        assert_eq!(a.mode, Mode::Normal);

        a.handle_key(key('j'));
        a.handle_key(key('b'));
        assert_eq!(a.mode, Mode::Normal);
        assert!(
            a.message
                .as_deref()
                .is_some_and(|m| m.contains("no checkpoint"))
        );
    }

    #[test]
    fn rollback_is_refused_while_running() {
        let mut a = reviewing();
        a.running = Some(3);
        a.handle_key(key('b'));
        assert_eq!(a.mode, Mode::Normal);
        assert!(a.message.is_some());
    }

    #[test]
    fn interrupt_only_while_running_and_confirmed() {
        let mut a = reviewing();
        a.handle_key(key('x'));
        assert_eq!(a.mode, Mode::Normal);
        assert!(a.message.is_some());

        a.running = Some(3);
        a.handle_key(key('x'));
        assert_eq!(a.mode, Mode::ConfirmInterrupt);
        assert_eq!(a.handle_key(key('y')), Some(Command::Interrupt));
        a.handle_key(key('x'));
        assert_eq!(a.handle_key(key('q')), None);
        assert_eq!(a.mode, Mode::Normal);
    }

    #[test]
    fn quit_is_immediate_when_idle_and_confirmed_when_running() {
        let mut a = reviewing();
        assert_eq!(a.handle_key(key('q')), Some(Command::Quit));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(a.handle_key(ctrl_c), Some(Command::Quit));

        a.running = Some(3);
        assert_eq!(a.handle_key(key('q')), None);
        assert_eq!(a.mode, Mode::ConfirmQuit);
        assert_eq!(a.handle_key(key('y')), Some(Command::Quit));
        assert_eq!(a.handle_key(ctrl_c), None);
        assert_eq!(a.mode, Mode::ConfirmQuit);
        assert_eq!(a.handle_key(key('n')), None);
        assert_eq!(a.mode, Mode::Normal);
    }

    #[test]
    fn available_keys_follow_the_state() {
        let names = |a: &App| a.available_keys().iter().map(|k| k.0).collect::<Vec<_>>();
        let a = reviewing();
        assert_eq!(names(&a), ["j/k", "Tab", "PgUp/PgDn", "n", "r", "b", "q"]);
        let mut running = reviewing();
        running.running = Some(3);
        assert_eq!(names(&running), ["j/k", "Tab", "PgUp/PgDn", "x", "q"]);
        let mut note = reviewing();
        note.mode = Mode::Note(String::new());
        assert_eq!(names(&note), ["Enter", "Esc"]);
        let mut confirm = reviewing();
        confirm.mode = Mode::ConfirmQuit;
        assert_eq!(names(&confirm), ["y", "any other key"]);
    }
}

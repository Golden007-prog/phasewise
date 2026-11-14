//! Home screen state machine: keys in, commands out. No terminal, no I/O.

use std::path::Path;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::data::{HomeData, PlanState};
use crate::agent::{LaunchOptions, ModelInfo};
use crate::display::{EventLine, money, phase_count};
use crate::ops::{Override, Overrides, PlanOutcome, PlanResult};
use crate::plan::{self, AgentKind};

/// Status line text for a key refused while the agent writes the plan.
const PLANNING_BUSY: &str = "Planning in progress — press x to cancel";
const NO_PLAN: &str =
    "There is no plan: press p to write one with the agent, or run `phasewise init`.";
const INVALID_PLAN: &str = "The plan is invalid: fix it and press c to check again, press p to write one with the agent, or run `phasewise init`.";

/// Every key of the home screen, for the help popup.
pub const HELP_KEYS: [(&str, &str); 9] = [
    ("a", "switch the agent (resets the model)"),
    ("m", "choose the model"),
    ("e", "choose the effort"),
    ("p", "write the plan with the agent"),
    ("c", "check the plan again"),
    ("Enter", "start a run"),
    ("x", "cancel the planning"),
    ("?", "show or hide this help"),
    ("q / Ctrl-C", "quit"),
];

/// The commit command for a plan that is not committed yet.
pub fn commit_hint() -> String {
    format!(
        "git add {} && git commit -m \"Add phasewise plan\"",
        plan::DEFAULT_PLAN_FILE
    )
}

/// Result of looking for an agent's version in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Checking,
    Version(String),
    NotInstalled,
}

/// The model list of one agent, loaded on demand and cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelsState {
    NotLoaded,
    Loading,
    Ready(Vec<ModelInfo>),
    Failed(String),
}

/// A row of the model popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelItem {
    Default,
    /// Id and label of a listed model.
    Model(String, String),
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Help,
    /// Quit asked while the agent writes the plan.
    ConfirmQuit,
    /// Index into the model popup rows.
    ModelPopup(usize),
    /// Custom model name, being typed.
    ModelInput(String),
    /// Index into the effort popup rows.
    EffortPopup(usize),
    /// The goal for the plan, being typed.
    GoalInput(String),
    /// Replace the existing plan file? Holds the goal.
    ConfirmReplace(String),
    /// Lines of an error, shown until closed.
    Error(Vec<String>),
}

/// What `p` asks the worker to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanLaunch {
    pub goal: String,
    pub agent: AgentKind,
    pub options: LaunchOptions,
    /// The plan file existed and the user agreed to replace it.
    pub force: bool,
}

/// What the event loop must do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeCommand {
    Quit,
    /// Load the plan and the repository data again.
    Reload,
    /// Start a run with the plan, with the agent shown on the screen (always
    /// passed, so a plan edited on disk meanwhile cannot change it).
    Start {
        agent: AgentKind,
        overrides: Overrides,
    },
    /// Write a plan with the agent in a worker.
    Plan(PlanLaunch),
    CancelPlan,
    /// Load the model list of the agent in the background.
    LoadModels(AgentKind),
}

/// The planning in progress: its heading and the live event lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Planning {
    pub header: Option<String>,
    pub lines: Vec<EventLine>,
}

/// How a finished planning is reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanReport {
    /// One line for the status line.
    Success(String),
    /// Lines for the error popup.
    Failure(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Home {
    pub data: HomeData,
    pub agent: AgentKind,
    pub model: Option<String>,
    pub effort: Option<String>,
    claude: Probe,
    codex: Probe,
    claude_models: ModelsState,
    codex_models: ModelsState,
    pub mode: Mode,
    pub message: Option<String>,
    pub planning: Option<Planning>,
}

impl Home {
    /// The agent starts as the plan's when the plan is valid, else Claude.
    pub fn new(data: HomeData) -> Home {
        let agent = match &data.plan {
            PlanState::Valid(plan) => plan.agent,
            _ => AgentKind::Claude,
        };
        Home {
            data,
            agent,
            model: None,
            effort: None,
            claude: Probe::Checking,
            codex: Probe::Checking,
            claude_models: ModelsState::NotLoaded,
            codex_models: ModelsState::NotLoaded,
            mode: Mode::Normal,
            message: None,
            planning: None,
        }
    }

    /// Replaces the repository data, keeping the user's choices.
    pub fn set_data(&mut self, data: HomeData) {
        self.data = data;
    }

    pub fn probe(&self, kind: AgentKind) -> &Probe {
        match kind {
            AgentKind::Claude => &self.claude,
            AgentKind::Codex => &self.codex,
        }
    }

    pub fn set_probe(&mut self, kind: AgentKind, probe: Probe) {
        match kind {
            AgentKind::Claude => self.claude = probe,
            AgentKind::Codex => self.codex = probe,
        }
    }

    pub fn models(&self, kind: AgentKind) -> &ModelsState {
        match kind {
            AgentKind::Claude => &self.claude_models,
            AgentKind::Codex => &self.codex_models,
        }
    }

    /// Stores a loaded model list (or the error). An open model popup keeps
    /// the row it had selected when that row still exists.
    pub fn set_models(&mut self, kind: AgentKind, result: Result<Vec<ModelInfo>, String>) {
        let keep_row = kind == self.agent && matches!(self.mode, Mode::ModelPopup(_));
        let current = match self.mode {
            Mode::ModelPopup(selected) if keep_row => self.model_items().get(selected).cloned(),
            _ => None,
        };
        let state = match result {
            Ok(models) => ModelsState::Ready(models),
            Err(error) => ModelsState::Failed(error),
        };
        match kind {
            AgentKind::Claude => self.claude_models = state,
            AgentKind::Codex => self.codex_models = state,
        }
        if keep_row {
            let items = self.model_items();
            let selected = current
                .and_then(|item| items.iter().position(|i| *i == item))
                .unwrap_or(0);
            self.mode = Mode::ModelPopup(selected);
        }
    }

    /// Rows of the model popup for the selected agent.
    pub fn model_items(&self) -> Vec<ModelItem> {
        let mut items = vec![ModelItem::Default];
        if let ModelsState::Ready(models) = self.models(self.agent) {
            items.extend(
                models
                    .iter()
                    .map(|m| ModelItem::Model(m.id.clone(), m.label.clone())),
            );
        }
        items.push(ModelItem::Custom);
        items
    }

    /// Note shown in the model popup while the list loads or after it failed.
    pub fn model_status(&self) -> Option<String> {
        match self.models(self.agent) {
            ModelsState::Loading => Some("loading…".to_string()),
            ModelsState::Failed(error) => Some(format!(
                "Cannot list the models: {error} You can still use default or custom…"
            )),
            ModelsState::NotLoaded | ModelsState::Ready(_) => None,
        }
    }

    /// Rows of the effort popup: `default` and the levels.
    pub fn effort_items() -> Vec<Option<&'static str>> {
        std::iter::once(None)
            .chain(plan::EFFORTS.iter().map(|e| Some(*e)))
            .collect()
    }

    /// The agent worker started: from now on the plan panel shows its events.
    pub fn begin_planning(&mut self) {
        self.planning = Some(Planning::default());
    }

    /// Adds what a planning event shows: the heading and/or a line.
    pub fn plan_event(&mut self, header: Option<String>, line: Option<EventLine>) {
        if let Some(planning) = &mut self.planning {
            if header.is_some() {
                planning.header = header;
            }
            planning.lines.extend(line);
        }
    }

    /// The planning could not start (no worker was created).
    pub fn show_error(&mut self, lines: Vec<String>) {
        self.planning = None;
        self.mode = Mode::Error(lines);
    }

    /// The planning ended: reports its outcome.
    pub fn plan_finished(&mut self, report: PlanReport) {
        self.planning = None;
        match report {
            PlanReport::Success(text) => {
                self.mode = Mode::Normal;
                self.message = Some(text);
            }
            PlanReport::Failure(lines) => self.mode = Mode::Error(lines),
        }
    }

    /// `ops::start` failed: shows its full text, led by the commit hint when
    /// the plan is not committed.
    pub fn show_start_error(&mut self, err: &anyhow::Error) {
        let mut lines = Vec::new();
        if self.data.plan_uncommitted {
            lines.push(format!(
                "{} is not committed. Commit it first:",
                plan::DEFAULT_PLAN_FILE
            ));
            lines.push(format!("  {}", commit_hint()));
            lines.push(String::new());
        }
        lines.extend(error_lines(err));
        self.mode = Mode::Error(lines);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<HomeCommand> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        self.message = None;
        // Ctrl-C works from every popup and input: it closes it and then acts
        // as in the main view (quit, or ask first while planning).
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.mode = Mode::Normal;
        }
        match self.mode.clone() {
            Mode::Normal => self.key_normal(key),
            Mode::Help => {
                // Any key closes the help; `?` is the one that opened it.
                self.mode = Mode::Normal;
                None
            }
            Mode::ConfirmQuit => {
                self.mode = Mode::Normal;
                is_yes(key).then_some(HomeCommand::Quit)
            }
            Mode::ModelPopup(selected) => self.key_model_popup(key, selected),
            Mode::ModelInput(buffer) => self.key_model_input(key, buffer),
            Mode::EffortPopup(selected) => self.key_effort_popup(key, selected),
            Mode::GoalInput(buffer) => self.key_goal(key, buffer),
            Mode::ConfirmReplace(goal) => {
                self.mode = Mode::Normal;
                is_yes(key).then(|| HomeCommand::Plan(self.launch(goal, true)))
            }
            Mode::Error(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q'))
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.mode = Mode::Normal;
                }
                None
            }
        }
    }

    fn launch(&self, goal: String, force: bool) -> PlanLaunch {
        PlanLaunch {
            goal,
            agent: self.agent,
            options: LaunchOptions {
                model: self.model.clone(),
                effort: self.effort.clone(),
            },
            force,
        }
    }

    fn refuse(&mut self, why: &str) -> Option<HomeCommand> {
        self.message = Some(why.to_string());
        None
    }

    fn key_normal(&mut self, key: KeyEvent) -> Option<HomeCommand> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl {
            return (key.code == KeyCode::Char('c'))
                .then(|| self.quit())
                .flatten();
        }
        let planning = self.planning.is_some();
        match key.code {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                None
            }
            KeyCode::Char('x') => {
                if planning {
                    Some(HomeCommand::CancelPlan)
                } else {
                    self.refuse("No plan is being written.")
                }
            }
            KeyCode::Char('a' | 'm' | 'e' | 'p' | 'c') | KeyCode::Enter if planning => {
                self.refuse(PLANNING_BUSY)
            }
            KeyCode::Char('a') => {
                self.agent = match self.agent {
                    AgentKind::Claude => AgentKind::Codex,
                    AgentKind::Codex => AgentKind::Claude,
                };
                // Model names belong to one agent.
                self.model = None;
                None
            }
            KeyCode::Char('m') => self.open_models(),
            KeyCode::Char('e') => {
                let selected = self
                    .effort
                    .as_deref()
                    .and_then(|e| Self::effort_items().iter().position(|i| *i == Some(e)))
                    .unwrap_or(0);
                self.mode = Mode::EffortPopup(selected);
                None
            }
            KeyCode::Char('p') => {
                self.mode = Mode::GoalInput(String::new());
                None
            }
            KeyCode::Char('c') => Some(HomeCommand::Reload),
            KeyCode::Enter => self.start(),
            _ => None,
        }
    }

    fn quit(&mut self) -> Option<HomeCommand> {
        if self.planning.is_some() {
            self.mode = Mode::ConfirmQuit;
            None
        } else {
            Some(HomeCommand::Quit)
        }
    }

    fn open_models(&mut self) -> Option<HomeCommand> {
        let selected = self
            .model
            .as_ref()
            .and_then(|m| {
                self.model_items()
                    .iter()
                    .position(|i| matches!(i, ModelItem::Model(id, _) if id == m))
            })
            .unwrap_or(0);
        self.mode = Mode::ModelPopup(selected);
        // A failed load is tried again each time the popup opens.
        match self.models(self.agent) {
            ModelsState::NotLoaded | ModelsState::Failed(_) => {
                let agent = self.agent;
                match agent {
                    AgentKind::Claude => self.claude_models = ModelsState::Loading,
                    AgentKind::Codex => self.codex_models = ModelsState::Loading,
                }
                Some(HomeCommand::LoadModels(agent))
            }
            ModelsState::Loading | ModelsState::Ready(_) => None,
        }
    }

    fn start(&mut self) -> Option<HomeCommand> {
        match &self.data.plan {
            PlanState::Valid(_) => {}
            PlanState::Missing => return self.refuse(NO_PLAN),
            PlanState::Invalid(_) => return self.refuse(INVALID_PLAN),
        }
        Some(HomeCommand::Start {
            agent: self.agent,
            overrides: Overrides {
                model: self.model.clone().map(Override::Set),
                effort: self.effort.clone().map(Override::Set),
            },
        })
    }

    fn key_model_popup(&mut self, key: KeyEvent, selected: usize) -> Option<HomeCommand> {
        let items = self.model_items();
        let last = items.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char('j') | KeyCode::Down => {
                self.mode = Mode::ModelPopup((selected + 1).min(last));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.mode = Mode::ModelPopup(selected.saturating_sub(1));
            }
            KeyCode::Enter => match items.get(selected.min(last)) {
                Some(ModelItem::Default) => {
                    self.model = None;
                    self.mode = Mode::Normal;
                }
                Some(ModelItem::Model(id, _)) => {
                    self.model = Some(id.clone());
                    self.mode = Mode::Normal;
                }
                Some(ModelItem::Custom) => self.mode = Mode::ModelInput(String::new()),
                None => self.mode = Mode::Normal,
            },
            _ => {}
        }
        None
    }

    fn key_model_input(&mut self, key: KeyEvent, mut buffer: String) -> Option<HomeCommand> {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let name = buffer.trim();
                if name == "default" {
                    self.model = None;
                    self.mode = Mode::Normal;
                } else {
                    match plan::validate_model(name) {
                        Ok(()) => {
                            self.model = Some(name.to_string());
                            self.mode = Mode::Normal;
                        }
                        Err(why) => self.message = Some(format!("Invalid model: {why}.")),
                    }
                }
            }
            KeyCode::Backspace => {
                buffer.pop();
                self.mode = Mode::ModelInput(buffer);
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                buffer.push(c);
                self.mode = Mode::ModelInput(buffer);
            }
            _ => {}
        }
        None
    }

    fn key_effort_popup(&mut self, key: KeyEvent, selected: usize) -> Option<HomeCommand> {
        let items = Self::effort_items();
        let last = items.len() - 1;
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char('j') | KeyCode::Down => {
                self.mode = Mode::EffortPopup((selected + 1).min(last));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.mode = Mode::EffortPopup(selected.saturating_sub(1));
            }
            KeyCode::Enter => {
                self.effort = items[selected.min(last)].map(str::to_string);
                self.mode = Mode::Normal;
            }
            _ => {}
        }
        None
    }

    fn key_goal(&mut self, key: KeyEvent, mut buffer: String) -> Option<HomeCommand> {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let goal = buffer.trim().to_string();
                if goal.is_empty() {
                    self.message =
                        Some("Say what you want to do first, or press Esc to cancel.".to_string());
                } else if self.data.plan == PlanState::Missing {
                    self.mode = Mode::Normal;
                    return Some(HomeCommand::Plan(self.launch(goal, false)));
                } else {
                    self.mode = Mode::ConfirmReplace(goal);
                }
            }
            KeyCode::Backspace => {
                buffer.pop();
                self.mode = Mode::GoalInput(buffer);
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                buffer.push(c);
                self.mode = Mode::GoalInput(buffer);
            }
            _ => {}
        }
        None
    }

    /// Keys shown in the bottom bar for the current state.
    pub fn available_keys(&self) -> Vec<(&'static str, &'static str)> {
        match self.mode {
            Mode::ConfirmQuit | Mode::ConfirmReplace(_) => {
                return vec![("y", "confirm"), ("any other key", "cancel")];
            }
            Mode::ModelPopup(_) | Mode::EffortPopup(_) => {
                return vec![("j/k", "move"), ("Enter", "choose"), ("Esc", "close")];
            }
            Mode::ModelInput(_) => return vec![("Enter", "use it"), ("Esc", "cancel")],
            Mode::GoalInput(_) => return vec![("Enter", "write the plan"), ("Esc", "cancel")],
            Mode::Help => return vec![("any key", "close")],
            Mode::Error(_) => return vec![("Esc", "close")],
            Mode::Normal => {}
        }
        if self.planning.is_some() {
            return vec![("x", "cancel"), ("?", "help"), ("q", "quit")];
        }
        vec![
            ("a", "agent"),
            ("m", "model"),
            ("e", "effort"),
            ("p", "plan"),
            ("c", "check again"),
            ("Enter", "start"),
            ("?", "help"),
            ("q", "quit"),
        ]
    }
}

/// The text of an error, one entry per line, with its causes after it.
fn error_lines(err: &anyhow::Error) -> Vec<String> {
    let mut lines: Vec<String> = err.to_string().lines().map(str::to_string).collect();
    for cause in err.chain().skip(1) {
        for (i, line) in cause.to_string().lines().enumerate() {
            lines.push(if i == 0 {
                format!("caused by: {line}")
            } else {
                line.to_string()
            });
        }
    }
    lines
}

fn is_yes(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('y' | 'Y')) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

/// A path shown relative to the repository root when it is inside it.
fn shown(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Turns what the planning worker returned into what the screen shows.
pub fn plan_report(result: &anyhow::Result<PlanOutcome>, root: &Path) -> PlanReport {
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(err) => {
            let mut lines = vec!["phasewise could not write the plan.".to_string()];
            lines.extend(error_lines(err));
            lines.push("Press p to try again.".to_string());
            return PlanReport::Failure(lines);
        }
    };
    match &outcome.result {
        PlanResult::Written { path, plan } => {
            let phases = phase_count(plan.phases.len());
            let cost = outcome
                .cost_usd
                .map(|c| format!(", {}", money(c)))
                .unwrap_or_default();
            PlanReport::Success(format!(
                "Plan written to {} ({phases}{cost}). Review it, commit it, then press Enter.",
                shown(path, root)
            ))
        }
        PlanResult::Failed {
            problems,
            last_draft,
            cancelled,
        } => {
            let mut lines = vec![
                if *cancelled {
                    "Planning was interrupted. No plan was written."
                } else {
                    "phasewise could not get a valid plan. Nothing was written."
                }
                .to_string(),
            ];
            lines.extend(problems.iter().map(|p| format!("- {p}")));
            if let Some(draft) = last_draft {
                lines.push(format!(
                    "Last draft: {} (fix it by hand and check it with `phasewise check`).",
                    shown(draft, root)
                ));
            }
            lines.push(format!(
                "Agent output and drafts: {}. Press p to try again.",
                shown(&outcome.dir, root)
            ));
            if let Some(cost) = outcome.cost_usd {
                lines.push(format!("Cost: {}", money(cost)));
            }
            PlanReport::Failure(lines)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tui::data::{HomePhase, PlanProblem, PlanSummary};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn code(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(home: &mut Home, text: &str) {
        for c in text.chars() {
            home.handle_key(key(c));
        }
    }

    fn summary(agent: AgentKind) -> PlanSummary {
        PlanSummary {
            title: Some("Demo".into()),
            agent,
            model: Some("opus".into()),
            effort: Some("high".into()),
            phases: vec![HomePhase {
                number: 1,
                title: "First".into(),
                checks: 1,
                model: Some("opus".into()),
                effort: Some("high".into()),
            }],
        }
    }

    fn data(plan: PlanState) -> HomeData {
        HomeData {
            root: PathBuf::from("/work/demo"),
            repo_name: "demo".into(),
            branch: "main".into(),
            plan_path: PathBuf::from("/work/demo/phasewise.md"),
            plan,
            plan_uncommitted: false,
        }
    }

    fn home_with(plan: PlanState) -> Home {
        Home::new(data(plan))
    }

    fn valid() -> Home {
        home_with(PlanState::Valid(summary(AgentKind::Claude)))
    }

    fn models() -> Vec<ModelInfo> {
        ["opus", "sonnet"]
            .iter()
            .map(|id| ModelInfo {
                id: (*id).into(),
                label: id.to_uppercase(),
                description: String::new(),
                efforts: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn initial_agent_is_the_plans_or_claude() {
        assert_eq!(
            home_with(PlanState::Valid(summary(AgentKind::Codex))).agent,
            AgentKind::Codex
        );
        assert_eq!(valid().agent, AgentKind::Claude);
        assert_eq!(home_with(PlanState::Missing).agent, AgentKind::Claude);
        let invalid = home_with(PlanState::Invalid(vec![PlanProblem {
            line: 1,
            message: "x".into(),
        }]));
        assert_eq!(invalid.agent, AgentKind::Claude);
    }

    #[test]
    fn reload_keeps_the_choices() {
        let mut h = home_with(PlanState::Missing);
        h.handle_key(key('a'));
        h.effort = Some("low".into());
        h.set_data(data(PlanState::Valid(summary(AgentKind::Claude))));
        assert_eq!(h.agent, AgentKind::Codex);
        assert_eq!(h.effort.as_deref(), Some("low"));
    }

    #[test]
    fn agent_toggle_resets_the_model_and_keeps_the_effort() {
        let mut h = valid();
        h.model = Some("opus".into());
        h.effort = Some("high".into());
        assert_eq!(h.handle_key(key('a')), None);
        assert_eq!((h.agent, h.model.clone()), (AgentKind::Codex, None));
        assert_eq!(h.effort.as_deref(), Some("high"));
        h.handle_key(key('a'));
        assert_eq!(h.agent, AgentKind::Claude);
    }

    #[test]
    fn model_popup_requests_the_list_once_and_caches_it() {
        let mut h = valid();
        assert_eq!(
            h.handle_key(key('m')),
            Some(HomeCommand::LoadModels(AgentKind::Claude))
        );
        assert_eq!(h.mode, Mode::ModelPopup(0));
        assert_eq!(h.model_status().as_deref(), Some("loading…"));
        // Only default and custom while loading.
        assert_eq!(h.model_items(), [ModelItem::Default, ModelItem::Custom]);
        h.set_models(AgentKind::Claude, Ok(models()));
        assert_eq!(h.model_items().len(), 4);
        assert_eq!(h.model_status(), None);
        h.handle_key(code(KeyCode::Esc));
        assert_eq!(h.mode, Mode::Normal);
        // Cached: no new request.
        assert_eq!(h.handle_key(key('m')), None);
        // The other agent has its own cache.
        h.handle_key(code(KeyCode::Esc));
        h.handle_key(key('a'));
        assert_eq!(
            h.handle_key(key('m')),
            Some(HomeCommand::LoadModels(AgentKind::Codex))
        );
    }

    #[test]
    fn model_popup_picks_a_listed_model_default_or_navigates_with_clamping() {
        let mut h = valid();
        h.handle_key(key('m'));
        h.set_models(AgentKind::Claude, Ok(models()));
        h.handle_key(key('j'));
        h.handle_key(code(KeyCode::Down));
        assert_eq!(h.mode, Mode::ModelPopup(2));
        assert_eq!(h.handle_key(code(KeyCode::Enter)), None);
        assert_eq!(h.model.as_deref(), Some("sonnet"));
        assert_eq!(h.mode, Mode::Normal);

        // The popup opens on the current model.
        h.handle_key(key('m'));
        assert_eq!(h.mode, Mode::ModelPopup(2));
        h.handle_key(key('k'));
        h.handle_key(code(KeyCode::Up));
        h.handle_key(code(KeyCode::Up));
        assert_eq!(h.mode, Mode::ModelPopup(0));
        h.handle_key(code(KeyCode::Enter));
        assert_eq!(h.model, None);
        h.handle_key(key('m'));
        for _ in 0..9 {
            h.handle_key(key('j'));
        }
        assert_eq!(h.mode, Mode::ModelPopup(3));
    }

    #[test]
    fn a_list_arriving_keeps_the_selected_row() {
        let mut h = valid();
        h.handle_key(key('m'));
        h.handle_key(key('j'));
        assert_eq!(h.mode, Mode::ModelPopup(1));
        h.set_models(AgentKind::Claude, Ok(models()));
        // `custom…` moved from row 1 to row 3 and stays selected.
        assert_eq!(h.mode, Mode::ModelPopup(3));
    }

    #[test]
    fn a_failed_list_still_offers_default_and_custom_and_is_retried() {
        let mut h = valid();
        h.handle_key(key('m'));
        h.set_models(AgentKind::Claude, Err("boom".into()));
        assert_eq!(h.model_items(), [ModelItem::Default, ModelItem::Custom]);
        assert!(h.model_status().is_some_and(|s| s.contains("boom")));
        h.handle_key(code(KeyCode::Esc));
        assert_eq!(
            h.handle_key(key('m')),
            Some(HomeCommand::LoadModels(AgentKind::Claude))
        );
    }

    #[test]
    fn custom_model_is_validated_and_default_clears_it() {
        let mut h = valid();
        h.handle_key(key('m'));
        h.handle_key(key('j'));
        h.handle_key(code(KeyCode::Enter));
        assert_eq!(h.mode, Mode::ModelInput(String::new()));
        // Empty: refused, input stays.
        h.handle_key(code(KeyCode::Enter));
        assert!(matches!(h.mode, Mode::ModelInput(_)));
        assert!(
            h.message
                .as_deref()
                .is_some_and(|m| m.starts_with("Invalid model"))
        );
        type_text(&mut h, "bad name");
        h.handle_key(code(KeyCode::Enter));
        assert_eq!(h.mode, Mode::ModelInput("bad name".into()));
        assert!(h.message.is_some());
        for _ in 0..8 {
            h.handle_key(code(KeyCode::Backspace));
        }
        type_text(&mut h, "-x");
        h.handle_key(code(KeyCode::Enter));
        assert!(matches!(h.mode, Mode::ModelInput(_)));
        h.handle_key(code(KeyCode::Backspace));
        h.handle_key(code(KeyCode::Backspace));
        type_text(&mut h, "my-model");
        assert_eq!(h.handle_key(code(KeyCode::Enter)), None);
        assert_eq!(h.model.as_deref(), Some("my-model"));
        assert_eq!(h.mode, Mode::Normal);

        h.handle_key(key('m'));
        h.handle_key(key('j'));
        h.handle_key(code(KeyCode::Enter));
        type_text(&mut h, "default");
        h.handle_key(code(KeyCode::Enter));
        assert_eq!((h.model.clone(), h.mode.clone()), (None, Mode::Normal));
    }

    #[test]
    fn esc_closes_popups_without_changes() {
        let mut h = valid();
        h.model = Some("opus".into());
        h.effort = Some("low".into());
        h.handle_key(key('m'));
        h.handle_key(key('j'));
        h.handle_key(code(KeyCode::Esc));
        assert_eq!(h.mode, Mode::Normal);
        h.handle_key(key('m'));
        h.handle_key(key('j'));
        h.handle_key(code(KeyCode::Enter));
        type_text(&mut h, "zzz");
        h.handle_key(code(KeyCode::Esc));
        assert_eq!(h.mode, Mode::Normal);
        h.handle_key(key('e'));
        h.handle_key(key('j'));
        h.handle_key(code(KeyCode::Esc));
        assert_eq!(h.mode, Mode::Normal);
        assert_eq!(h.model.as_deref(), Some("opus"));
        assert_eq!(h.effort.as_deref(), Some("low"));
    }

    #[test]
    fn effort_popup_offers_default_and_the_levels() {
        let mut h = valid();
        assert_eq!(Home::effort_items().len(), 6);
        h.handle_key(key('e'));
        assert_eq!(h.mode, Mode::EffortPopup(0));
        h.handle_key(code(KeyCode::Down));
        h.handle_key(code(KeyCode::Down));
        h.handle_key(code(KeyCode::Enter));
        assert_eq!(h.effort.as_deref(), Some("medium"));
        // Opens on the current level; default clears it.
        h.handle_key(key('e'));
        assert_eq!(h.mode, Mode::EffortPopup(2));
        h.handle_key(key('k'));
        h.handle_key(key('k'));
        h.handle_key(key('k'));
        h.handle_key(code(KeyCode::Enter));
        assert_eq!(h.effort, None);
        h.handle_key(key('e'));
        for _ in 0..10 {
            h.handle_key(key('j'));
        }
        h.handle_key(code(KeyCode::Enter));
        assert_eq!(h.effort.as_deref(), Some("max"));
    }

    #[test]
    fn plan_input_edits_and_refuses_an_empty_goal() {
        let mut h = home_with(PlanState::Missing);
        h.handle_key(key('p'));
        assert_eq!(h.mode, Mode::GoalInput(String::new()));
        assert_eq!(h.handle_key(code(KeyCode::Enter)), None);
        assert!(h.message.is_some());
        assert!(matches!(h.mode, Mode::GoalInput(_)));
        type_text(&mut h, "   ");
        assert_eq!(h.handle_key(code(KeyCode::Enter)), None);
        assert!(matches!(h.mode, Mode::GoalInput(_)));
        type_text(&mut h, "add a CLI!");
        h.handle_key(code(KeyCode::Backspace));
        assert_eq!(h.mode, Mode::GoalInput("   add a CLI".into()));
        h.handle_key(code(KeyCode::Esc));
        assert_eq!(h.mode, Mode::Normal);
    }

    #[test]
    fn plan_without_an_existing_file_starts_with_the_current_settings() {
        let mut h = home_with(PlanState::Missing);
        h.model = Some("opus".into());
        h.handle_key(key('a'));
        h.effort = Some("high".into());
        h.handle_key(key('p'));
        type_text(&mut h, "  add a CLI ");
        assert_eq!(
            h.handle_key(code(KeyCode::Enter)),
            Some(HomeCommand::Plan(PlanLaunch {
                goal: "add a CLI".into(),
                agent: AgentKind::Codex,
                options: LaunchOptions {
                    model: None,
                    effort: Some("high".into()),
                },
                force: false,
            }))
        );
        assert_eq!(h.mode, Mode::Normal);
    }

    #[test]
    fn plan_over_an_existing_file_asks_and_only_y_confirms() {
        for existing in [
            PlanState::Valid(summary(AgentKind::Claude)),
            PlanState::Invalid(Vec::new()),
        ] {
            for yes in ['y', 'Y'] {
                let mut h = home_with(existing.clone());
                h.handle_key(key('p'));
                type_text(&mut h, "goal");
                assert_eq!(h.handle_key(code(KeyCode::Enter)), None);
                assert_eq!(h.mode, Mode::ConfirmReplace("goal".into()));
                match h.handle_key(key(yes)) {
                    Some(HomeCommand::Plan(launch)) => {
                        assert!(launch.force);
                        assert_eq!(launch.goal, "goal");
                    }
                    other => panic!("expected a plan command, got {other:?}"),
                }
                assert_eq!(h.mode, Mode::Normal);
            }
            for no in [key('n'), code(KeyCode::Enter), code(KeyCode::Esc), key('q')] {
                let mut h = home_with(existing.clone());
                h.handle_key(key('p'));
                type_text(&mut h, "goal");
                h.handle_key(code(KeyCode::Enter));
                assert_eq!(h.handle_key(no), None);
                assert_eq!(h.mode, Mode::Normal);
            }
        }
    }

    #[test]
    fn keys_are_refused_while_planning_but_x_and_help_work() {
        let mut h = valid();
        h.begin_planning();
        for c in ['a', 'm', 'e', 'p', 'c'] {
            assert_eq!(h.handle_key(key(c)), None);
            assert_eq!(h.message.as_deref(), Some(PLANNING_BUSY), "key {c}");
            assert_eq!(h.mode, Mode::Normal);
        }
        assert_eq!(h.handle_key(code(KeyCode::Enter)), None);
        assert_eq!(h.message.as_deref(), Some(PLANNING_BUSY));
        assert_eq!(h.agent, AgentKind::Claude);
        assert_eq!(h.handle_key(key('x')), Some(HomeCommand::CancelPlan));
        h.handle_key(key('?'));
        assert_eq!(h.mode, Mode::Help);
        h.handle_key(key('?'));
        assert_eq!(h.mode, Mode::Normal);
    }

    #[test]
    fn x_without_planning_is_refused() {
        let mut h = valid();
        assert_eq!(h.handle_key(key('x')), None);
        assert!(h.message.is_some());
    }

    #[test]
    fn ctrl_c_quits_from_popups_and_inputs() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut h = valid();
        h.mode = Mode::GoalInput("half a goal".into());
        assert_eq!(h.handle_key(ctrl_c), Some(HomeCommand::Quit));
        assert_eq!(h.mode, Mode::Normal);
        h.mode = Mode::Error(vec!["boom".into()]);
        assert_eq!(h.handle_key(ctrl_c), Some(HomeCommand::Quit));
        // While planning it still asks first.
        h.begin_planning();
        h.mode = Mode::EffortPopup(0);
        assert_eq!(h.handle_key(ctrl_c), None);
        assert_eq!(h.mode, Mode::ConfirmQuit);
    }

    #[test]
    fn quit_is_immediate_when_idle_and_confirmed_while_planning() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut h = valid();
        assert_eq!(h.handle_key(key('q')), Some(HomeCommand::Quit));
        assert_eq!(h.handle_key(ctrl_c), Some(HomeCommand::Quit));
        h.begin_planning();
        assert_eq!(h.handle_key(key('q')), None);
        assert_eq!(h.mode, Mode::ConfirmQuit);
        assert_eq!(h.handle_key(key('y')), Some(HomeCommand::Quit));
        h.handle_key(key('q'));
        assert_eq!(h.handle_key(key('n')), None);
        assert_eq!(h.mode, Mode::Normal);
        assert_eq!(h.handle_key(ctrl_c), None);
        assert_eq!(h.mode, Mode::ConfirmQuit);
    }

    #[test]
    fn other_control_keys_do_nothing() {
        let mut h = valid();
        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(h.handle_key(ctrl_a), None);
        assert_eq!(h.agent, AgentKind::Claude);
        assert_eq!(h.mode, Mode::Normal);
    }

    #[test]
    fn enter_is_refused_without_a_valid_plan() {
        let mut missing = home_with(PlanState::Missing);
        assert_eq!(missing.handle_key(code(KeyCode::Enter)), None);
        assert!(missing.message.as_deref().is_some_and(|m| {
            m.contains("press p to write one with the agent, or run `phasewise init`")
        }));
        let mut invalid = home_with(PlanState::Invalid(Vec::new()));
        assert_eq!(invalid.handle_key(code(KeyCode::Enter)), None);
        assert!(invalid.message.as_deref().is_some_and(|m| {
            m.contains("press p to write one with the agent, or run `phasewise init`")
        }));
    }

    #[test]
    fn enter_starts_with_overrides_only_for_what_was_chosen() {
        let mut h = valid();
        assert_eq!(
            h.handle_key(code(KeyCode::Enter)),
            Some(HomeCommand::Start {
                agent: AgentKind::Claude,
                overrides: Overrides::default(),
            })
        );
        h.handle_key(key('a'));
        h.model = Some("gpt".into());
        h.effort = Some("low".into());
        assert_eq!(
            h.handle_key(code(KeyCode::Enter)),
            Some(HomeCommand::Start {
                agent: AgentKind::Codex,
                overrides: Overrides {
                    model: Some(Override::Set("gpt".into())),
                    effort: Some(Override::Set("low".into())),
                },
            })
        );
        // A codex plan starts with codex unless the user switches.
        let mut codex = home_with(PlanState::Valid(summary(AgentKind::Codex)));
        assert_eq!(codex.agent, AgentKind::Codex);
        assert_eq!(
            codex.handle_key(code(KeyCode::Enter)),
            Some(HomeCommand::Start {
                agent: AgentKind::Codex,
                overrides: Overrides::default(),
            })
        );
    }

    #[test]
    fn c_reloads() {
        assert_eq!(valid().handle_key(key('c')), Some(HomeCommand::Reload));
    }

    #[test]
    fn error_popup_closes_with_esc_enter_or_q_and_q_does_not_quit() {
        for close in [code(KeyCode::Esc), code(KeyCode::Enter), key('q')] {
            let mut h = valid();
            h.show_error(vec!["boom".into()]);
            assert_eq!(h.handle_key(close), None);
            assert_eq!(h.mode, Mode::Normal);
        }
        let mut h = valid();
        h.show_error(vec!["boom".into()]);
        for ignored in [key('a'), key('x'), key('p'), code(KeyCode::Tab)] {
            assert_eq!(h.handle_key(ignored), None);
            assert!(matches!(h.mode, Mode::Error(_)));
        }
        assert_eq!(h.agent, AgentKind::Claude);
    }

    #[test]
    fn help_closes_on_any_key() {
        let mut h = valid();
        h.handle_key(key('?'));
        assert_eq!(h.mode, Mode::Help);
        assert_eq!(h.handle_key(key('q')), None);
        assert_eq!(h.mode, Mode::Normal);
    }

    #[test]
    fn message_clears_on_the_next_key() {
        let mut h = home_with(PlanState::Missing);
        h.handle_key(code(KeyCode::Enter));
        assert!(h.message.is_some());
        h.handle_key(key('z'));
        assert!(h.message.is_none());
    }

    #[test]
    fn start_error_is_shown_in_full_with_the_commit_hint_for_an_uncommitted_plan() {
        let mut h = valid();
        h.data.plan_uncommitted = true;
        let err = anyhow::anyhow!(
            "The working tree has uncommitted changes:\n  phasewise.md\nCommit it."
        )
        .context("outer");
        h.show_start_error(&err);
        let Mode::Error(lines) = &h.mode else {
            panic!("expected an error popup");
        };
        assert_eq!(lines[0], "phasewise.md is not committed. Commit it first:");
        assert_eq!(
            lines[1],
            "  git add phasewise.md && git commit -m \"Add phasewise plan\""
        );
        assert_eq!(lines[2], "");
        assert_eq!(lines[3], "outer");
        assert_eq!(
            lines[4],
            "caused by: The working tree has uncommitted changes:"
        );
        assert_eq!(lines[5], "  phasewise.md");
        assert_eq!(lines[6], "Commit it.");

        let mut committed = valid();
        committed.show_start_error(&anyhow::anyhow!("a\nb"));
        assert_eq!(committed.mode, Mode::Error(vec!["a".into(), "b".into()]));
    }

    #[test]
    fn plan_events_accumulate_and_finishing_reports() {
        let mut h = valid();
        h.plan_event(Some("▶ Planning".into()), None);
        assert_eq!(h.planning, None, "ignored before the worker starts");
        h.begin_planning();
        h.plan_event(Some("▶ Planning · claude".into()), None);
        let line = crate::display::plan_event_line(&crate::ops::PlanEvent::Written {
            path: "phasewise.md".into(),
        })
        .map(|l| l.at(3));
        h.plan_event(None, line.clone());
        let planning = h.planning.clone().expect("planning");
        assert_eq!(planning.header.as_deref(), Some("▶ Planning · claude"));
        assert_eq!(planning.lines, line.into_iter().collect::<Vec<_>>());
        h.plan_finished(PlanReport::Success("ok".into()));
        assert_eq!(
            (h.planning.clone(), h.message.as_deref()),
            (None, Some("ok"))
        );
        h.begin_planning();
        h.plan_finished(PlanReport::Failure(vec!["bad".into()]));
        assert_eq!(h.mode, Mode::Error(vec!["bad".into()]));
        assert_eq!(h.planning, None);
    }

    #[test]
    fn available_keys_follow_the_state() {
        let names = |h: &Home| h.available_keys().iter().map(|k| k.0).collect::<Vec<_>>();
        let mut h = valid();
        assert_eq!(names(&h), ["a", "m", "e", "p", "c", "Enter", "?", "q"]);
        h.begin_planning();
        assert_eq!(names(&h), ["x", "?", "q"]);
        h.mode = Mode::ConfirmQuit;
        assert_eq!(names(&h), ["y", "any other key"]);
        h.mode = Mode::Error(Vec::new());
        assert_eq!(names(&h), ["Esc"]);
    }

    fn outcome(result: PlanResult, cost: Option<f64>) -> anyhow::Result<PlanOutcome> {
        Ok(PlanOutcome {
            plan_id: "p1".into(),
            dir: PathBuf::from("/work/demo/.phasewise/plans/p1"),
            cost_usd: cost,
            result,
        })
    }

    #[test]
    fn report_for_a_written_plan() {
        let plan = plan::parse("# T\n\n## Phase 1: A\n\nx\n\n## Phase 2: B\n\ny\n").expect("plan");
        let written = |plan: plan::Plan, cost| {
            plan_report(
                &outcome(
                    PlanResult::Written {
                        path: PathBuf::from("/work/demo/phasewise.md"),
                        plan,
                    },
                    cost,
                ),
                Path::new("/work/demo"),
            )
        };
        assert_eq!(
            written(plan.clone(), Some(0.1234)),
            PlanReport::Success(
                "Plan written to phasewise.md (2 phases, $0.12). Review it, commit it, then press Enter."
                    .into()
            )
        );
        assert_eq!(
            written(plan, None),
            PlanReport::Success(
                "Plan written to phasewise.md (2 phases). Review it, commit it, then press Enter."
                    .into()
            )
        );
    }

    #[test]
    fn report_for_a_failed_or_cancelled_plan() {
        let failed = |cancelled| {
            plan_report(
                &outcome(
                    PlanResult::Failed {
                        problems: vec!["line 3: bad".into(), "no markers".into()],
                        last_draft: Some(PathBuf::from(
                            "/work/demo/.phasewise/plans/p1/draft-2.md",
                        )),
                        cancelled,
                    },
                    Some(0.5),
                ),
                Path::new("/work/demo"),
            )
        };
        let PlanReport::Failure(lines) = failed(false) else {
            panic!("expected a failure");
        };
        assert_eq!(
            lines,
            [
                "phasewise could not get a valid plan. Nothing was written.",
                "- line 3: bad",
                "- no markers",
                "Last draft: .phasewise/plans/p1/draft-2.md (fix it by hand and check it with `phasewise check`).",
                "Agent output and drafts: .phasewise/plans/p1. Press p to try again.",
                "Cost: $0.50",
            ]
        );
        let PlanReport::Failure(lines) = failed(true) else {
            panic!("expected a failure");
        };
        assert_eq!(lines[0], "Planning was interrupted. No plan was written.");
    }

    #[test]
    fn report_for_an_error() {
        let err: anyhow::Result<PlanOutcome> = Err(anyhow::anyhow!("cannot write"));
        let PlanReport::Failure(lines) = plan_report(&err, Path::new("/work/demo")) else {
            panic!("expected a failure");
        };
        assert_eq!(lines[1], "cannot write");
        assert!(lines[0].contains("could not write the plan"));
    }
}

//! Rendering of the home screen. Deterministic: the output depends only on
//! the [`Home`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::data::{PlanState, PlanSummary};
use super::home::{HELP_KEYS, Home, Mode, ModelItem, Probe, commit_hint};
use super::view::{
    MIN_HEIGHT, MIN_WIDTH, draw_too_small, event_spans, render_popup, role_style, truncate,
};
use crate::display::{first_line_short, phase_count};
use crate::plan::AgentKind;
use crate::style::Role;

/// Width of the settings panel.
const SETTINGS_WIDTH: u16 = 38;
const POPUP_WIDTH: u16 = 56;
const ERROR_WIDTH: u16 = 76;

pub fn draw(frame: &mut Frame, home: &Home) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_too_small(frame, area);
        return;
    }
    let [header, body, status, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(area);
    let [plan_area, settings_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(SETTINGS_WIDTH)]).areas(body);
    draw_header(frame, home, header);
    if home.planning.is_some() {
        draw_planning(frame, home, plan_area);
    } else {
        draw_plan(frame, home, plan_area);
    }
    draw_settings(frame, home, settings_area);
    draw_status(frame, home, status);
    draw_keys(frame, home, keys);
    draw_popup(frame, home, area);
}

fn draw_header(frame: &mut Frame, home: &Home, area: Rect) {
    let dim = role_style(Role::Dim);
    let line = Line::from(vec![
        Span::styled("phasewise", role_style(Role::Heading)),
        Span::styled(" · ", dim),
        Span::raw(home.data.repo_name.clone()),
        Span::styled(" · ", dim),
        Span::raw(home.data.branch.clone()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// The plan file name as shown: relative to the repository root.
fn plan_file(home: &Home) -> String {
    home.data
        .plan_path
        .strip_prefix(&home.data.root)
        .unwrap_or(&home.data.plan_path)
        .display()
        .to_string()
}

fn draw_plan(frame: &mut Frame, home: &Home, area: Rect) {
    let block = Block::new().borders(Borders::ALL).title(" Plan ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = usize::from(inner.width);
    let dim = role_style(Role::Dim);
    let mut lines = vec![Line::from(vec![
        Span::styled("file ", dim),
        Span::raw(plan_file(home)),
    ])];
    match &home.data.plan {
        PlanState::Missing => {
            lines.push(Line::styled("missing", role_style(Role::Warn)));
            lines.push(Line::raw(""));
            lines.push(Line::raw("Press p to write one with the agent,"));
            lines.push(Line::raw("or run `phasewise init` for a template."));
        }
        PlanState::Valid(plan) => {
            let mut state = format!("valid · {}", phase_count(plan.phases.len()));
            if let Some(title) = &plan.title {
                state.push_str(&format!(" · {}", truncate(title, 40)));
            }
            lines.push(Line::styled(state, role_style(Role::Ok)));
            push_uncommitted(&mut lines, home);
            lines.push(Line::raw(""));
            for phase in &plan.phases {
                lines.push(phase_line(home, plan, phase, width));
            }
        }
        PlanState::Invalid(problems) => {
            let n = problems.len();
            lines.push(Line::styled(
                format!(
                    "invalid · {n} {}",
                    if n == 1 { "problem" } else { "problems" }
                ),
                role_style(Role::Fail),
            ));
            push_uncommitted(&mut lines, home);
            lines.push(Line::raw(""));
            for problem in problems {
                let prefix = if problem.line == 0 {
                    String::new()
                } else {
                    format!("line {}: ", problem.line)
                };
                lines.push(Line::from(vec![
                    Span::styled("✗ ", role_style(Role::Fail)),
                    Span::styled(prefix, dim),
                    Span::raw(first_line_short(&problem.message)),
                ]));
            }
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "Fix the file and press c, or press p to write a new one.",
                dim,
            ));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn push_uncommitted(lines: &mut Vec<Line<'static>>, home: &Home) {
    if home.data.plan_uncommitted {
        lines.push(Line::styled("not committed", role_style(Role::Warn)));
        lines.push(Line::styled(commit_hint(), role_style(Role::Dim)));
    }
}

/// `  1. Title            2 checks · opus · high`, the title cut to fit.
fn phase_line(
    home: &Home,
    plan: &PlanSummary,
    phase: &super::data::HomePhase,
    width: usize,
) -> Line<'static> {
    let checks = format!(
        "{} {}",
        phase.checks,
        if phase.checks == 1 { "check" } else { "checks" }
    );
    let mut right = vec![checks];
    // A model name belongs to one agent: plan-level values apply to the plan's own.
    if home.agent == plan.agent {
        right.extend(phase.model.clone());
        right.extend(phase.effort.clone());
    }
    let right = right.join(" · ");
    let prefix = format!("{:>2}. ", phase.number);
    let room = width
        .saturating_sub(prefix.chars().count() + right.chars().count() + 2)
        .max(1);
    let title = truncate(&phase.title, room);
    let pad = width
        .saturating_sub(prefix.chars().count() + title.chars().count() + right.chars().count())
        .max(1);
    Line::from(vec![
        Span::raw(format!("{prefix}{title}")),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, role_style(Role::Dim)),
    ])
}

/// The planning in progress: heading, then the newest lines at the bottom.
fn draw_planning(frame: &mut Frame, home: &Home, area: Rect) {
    let block = Block::new().borders(Borders::ALL).title(" Plan ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(planning) = &home.planning else {
        return;
    };
    let header = planning
        .header
        .clone()
        .unwrap_or_else(|| "▶ Planning…".to_string());
    let room = usize::from(inner.height).saturating_sub(1);
    let skip = planning.lines.len().saturating_sub(room);
    let mut lines = vec![Line::styled(header, role_style(Role::Heading))];
    lines.extend(
        planning
            .lines
            .iter()
            .skip(skip)
            .map(|l| Line::from(event_spans(l))),
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

fn other_agent(kind: AgentKind) -> AgentKind {
    match kind {
        AgentKind::Claude => AgentKind::Codex,
        AgentKind::Codex => AgentKind::Claude,
    }
}

fn setting_line(label: &str, value: Option<&str>, plan_value: Option<&str>) -> Line<'static> {
    let dim = role_style(Role::Dim);
    let mut spans = vec![Span::styled(format!("{label:<8}"), dim)];
    match value {
        Some(v) => spans.push(Span::raw(v.to_string())),
        None => {
            spans.push(Span::raw("default"));
            // What "default" means here; a chosen value replaces the plan's.
            if let Some(v) = plan_value {
                spans.push(Span::styled(format!(" (plan: {v})"), dim));
            }
        }
    }
    Line::from(spans)
}

fn draw_settings(frame: &mut Frame, home: &Home, area: Rect) {
    let block = Block::new().borders(Borders::ALL).title(" Settings ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let dim = role_style(Role::Dim);
    let mut agent = vec![
        Span::styled(format!("{:<8}", "Agent"), dim),
        Span::raw(home.agent.to_string()),
    ];
    match home.probe(home.agent) {
        Probe::Checking => agent.push(Span::styled(" checking…", dim)),
        Probe::Version(v) => agent.push(Span::styled(
            format!(
                " {}",
                truncate(v, usize::from(inner.width).saturating_sub(14))
            ),
            dim,
        )),
        Probe::NotInstalled => agent.push(Span::styled(" not installed", role_style(Role::Fail))),
    }
    // The plan's own values apply when the agent is the plan's.
    let (plan_model, plan_effort) = match &home.data.plan {
        PlanState::Valid(plan) if plan.agent == home.agent => {
            (plan.model.as_deref(), plan.effort.as_deref())
        }
        _ => (None, None),
    };
    let lines = vec![
        Line::from(agent),
        setting_line("Model", home.model.as_deref(), plan_model),
        setting_line("Effort", home.effort.as_deref(), plan_effort),
        Line::raw(""),
        Line::styled(
            format!(
                "Used to write the plan (p) and for the run (Enter). Press a to use {}.",
                other_agent(home.agent)
            ),
            dim,
        ),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn draw_status(frame: &mut Frame, home: &Home, area: Rect) {
    if let Some(message) = &home.message {
        frame.render_widget(
            Paragraph::new(message.clone())
                .style(Style::new().fg(Color::Yellow))
                .wrap(Wrap { trim: true }),
            area,
        );
    }
}

fn draw_keys(frame: &mut Frame, home: &Home, area: Rect) {
    let mut spans = Vec::new();
    for (key, what) in home.available_keys() {
        spans.push(Span::styled(key, role_style(Role::Heading)));
        spans.push(Span::styled(format!(" {what}  "), role_style(Role::Dim)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The last `max` characters of `text`, with `…` in front when it was cut,
/// so that the end of what is being typed stays visible.
fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let kept: String = text.chars().skip(count - max.saturating_sub(1)).collect();
    format!("…{kept}")
}

fn draw_popup(frame: &mut Frame, home: &Home, area: Rect) {
    let input_room = usize::from(POPUP_WIDTH).saturating_sub(6);
    let (title, body): (&str, Vec<Line>) = match &home.mode {
        Mode::Normal => return,
        Mode::ModelPopup(selected) => {
            draw_model_popup(frame, home, area, *selected);
            return;
        }
        Mode::EffortPopup(selected) => {
            let rows: Vec<ListItem> = Home::effort_items()
                .into_iter()
                .map(|e| ListItem::new(e.unwrap_or("default")))
                .collect();
            draw_list_popup(frame, area, " Effort ", rows, *selected, None);
            return;
        }
        Mode::Help => (
            " Keys ",
            HELP_KEYS
                .iter()
                .map(|(key, what)| {
                    Line::from(vec![
                        Span::styled(format!("{key:<12}"), role_style(Role::Heading)),
                        Span::raw(*what),
                    ])
                })
                .chain([Line::raw(""), Line::raw("Press any key to close.")])
                .collect(),
        ),
        Mode::ConfirmQuit => (
            " Quit ",
            vec![
                Line::raw("The agent is writing the plan. Cancel it and quit?"),
                Line::raw(""),
                Line::raw("y: confirm   any other key: cancel"),
            ],
        ),
        Mode::GoalInput(buffer) => (
            " Plan ",
            vec![
                Line::raw("What do you want to do?"),
                Line::raw(format!("> {}_", tail(buffer, input_room))),
                Line::raw(""),
                Line::raw("Enter: write the plan   Esc: cancel"),
            ],
        ),
        Mode::ConfirmReplace(_) => (
            " Replace plan ",
            vec![
                Line::raw(format!("Replace {}? (y/N)", plan_file(home))),
                Line::raw(""),
                Line::raw("y: replace   any other key: cancel"),
            ],
        ),
        Mode::ModelInput(buffer) => (
            " Model ",
            vec![
                Line::raw("Model name (or `default`):"),
                Line::raw(format!("> {}_", tail(buffer, input_room))),
                Line::raw(""),
                Line::raw("Enter: use it   Esc: cancel"),
            ],
        ),
        Mode::Error(lines) => {
            let mut body: Vec<Line> = lines.iter().map(|l| Line::raw(l.clone())).collect();
            body.push(Line::raw(""));
            body.push(Line::styled(
                "Press Esc, Enter or q to close.",
                role_style(Role::Dim),
            ));
            render_popup(frame, area, " Error ", body, ERROR_WIDTH);
            return;
        }
    };
    render_popup(frame, area, title, body, POPUP_WIDTH);
}

fn draw_model_popup(frame: &mut Frame, home: &Home, area: Rect, selected: usize) {
    let dim = role_style(Role::Dim);
    let rows: Vec<ListItem> = home
        .model_items()
        .into_iter()
        .map(|item| match item {
            ModelItem::Default => ListItem::new("default"),
            ModelItem::Custom => ListItem::new("custom…"),
            ModelItem::Model(id, label) => {
                let mut spans = vec![Span::raw(id.clone())];
                if label != id {
                    spans.push(Span::styled(format!("  {label}"), dim));
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect();
    draw_list_popup(frame, area, " Model ", rows, selected, home.model_status());
}

/// A popup with a selectable list and, below it, an optional note.
fn draw_list_popup(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    rows: Vec<ListItem>,
    selected: usize,
    note: Option<String>,
) {
    let inner_width = usize::from(POPUP_WIDTH.min(area.width).saturating_sub(2)).max(1);
    let note_rows = note
        .as_ref()
        .map_or(0, |n| n.chars().count().div_ceil(inner_width).min(4));
    let wanted = rows.len() + note_rows;
    let height = u16::try_from(wanted)
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .min(area.height);
    let [_, row, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, popup, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(POPUP_WIDTH.min(area.width)),
        Constraint::Fill(1),
    ])
    .areas(row);
    frame.render_widget(Clear, popup);
    let block = Block::new().borders(Borders::ALL).title(title.to_string());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let note_height = u16::try_from(note_rows)
        .unwrap_or(0)
        .min(inner.height.saturating_sub(1));
    let [list_area, note_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(note_height)]).areas(inner);
    let list = List::new(rows)
        .highlight_symbol("> ")
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(list, list_area, &mut state);
    if let Some(note) = note {
        frame.render_widget(
            Paragraph::new(note)
                .style(role_style(Role::Dim))
                .wrap(Wrap { trim: true }),
            note_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::agent::ModelInfo;
    use crate::display::{EventLine, plan_event_line};
    use crate::ops::PlanEvent;
    use crate::tui::data::{HomeData, HomePhase, PlanProblem};

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

    fn valid_plan() -> PlanState {
        let phase = |number, title: &str, checks, model: Option<&str>| HomePhase {
            number,
            title: title.into(),
            checks,
            model: model.map(str::to_string),
            effort: None,
        };
        PlanState::Valid(PlanSummary {
            title: Some("Demo plan".into()),
            agent: AgentKind::Claude,
            model: Some("sonnet".into()),
            effort: None,
            phases: vec![
                phase(1, "Scaffold the project", 2, Some("sonnet")),
                phase(2, "Add the parser", 1, Some("opus")),
                phase(
                    3,
                    "Write the docs and the long release notes for everybody",
                    0,
                    Some("sonnet"),
                ),
            ],
        })
    }

    fn home(plan: PlanState) -> Home {
        let mut home = Home::new(data(plan));
        home.set_probe(
            AgentKind::Claude,
            Probe::Version("2.1.325 (Claude Code)".into()),
        );
        home.set_probe(AgentKind::Codex, Probe::NotInstalled);
        home
    }

    fn render(home: &Home) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(100, 28)).expect("test terminal");
        terminal.draw(|f| draw(f, home)).expect("draw");
        terminal
    }

    fn model(id: &str, label: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            label: label.into(),
            description: String::new(),
            efforts: Vec::new(),
        }
    }

    #[test]
    fn missing_plan() {
        let terminal = render(&home(PlanState::Missing));
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn valid_plan_uncommitted() {
        let mut home = home(valid_plan());
        home.data.plan_uncommitted = true;
        home.effort = Some("high".into());
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn valid_plan_for_another_agent_hides_the_plan_models() {
        let mut home = home(valid_plan());
        home.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('a'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn invalid_plan() {
        let problems = vec![
            PlanProblem {
                line: 3,
                message: "unknown key `modle` in the front matter".into(),
            },
            PlanProblem {
                line: 12,
                message: "phase 2 has no instructions".into(),
            },
        ];
        let terminal = render(&home(PlanState::Invalid(problems)));
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn model_popup() {
        let mut home = home(valid_plan());
        home.model = Some("opus".into());
        home.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('m'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        home.set_models(
            AgentKind::Claude,
            Ok(vec![
                model("fable", "Fable"),
                model("opus", "Opus 4"),
                model("sonnet", "sonnet"),
            ]),
        );
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn model_popup_while_loading_and_after_an_error() {
        let mut home = home(valid_plan());
        home.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('m'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        let loading = render(&home);
        home.set_models(AgentKind::Claude, Err("`claude` was not found.".into()));
        let failed = render(&home);
        insta::assert_snapshot!("model_popup_loading", loading.backend());
        insta::assert_snapshot!("model_popup_failed", failed.backend());
    }

    #[test]
    fn effort_popup() {
        let mut home = home(valid_plan());
        home.mode = Mode::EffortPopup(2);
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn plan_input() {
        let mut home = home(PlanState::Missing);
        home.mode = Mode::GoalInput("a CLI to convert CSV files to JSON".into());
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn replace_confirmation() {
        let mut home = home(valid_plan());
        home.mode = Mode::ConfirmReplace("goal".into());
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn creation_in_progress() {
        let mut home = home(valid_plan());
        home.begin_planning();
        let mut at = 0;
        let mut push = |home: &mut Home, event: PlanEvent| {
            let header =
                crate::display::plan_header(&event, &crate::style::Styler { color: false });
            let line: Option<EventLine> = plan_event_line(&event).map(|l| l.at(at));
            at += 4;
            home.plan_event(header, line);
        };
        push(
            &mut home,
            PlanEvent::Started {
                plan_id: "p".into(),
                agent: AgentKind::Claude,
                model: Some("opus".into()),
                effort: None,
            },
        );
        let call = |name: &str, detail: &str| crate::agent::AgentEvent::ToolCall {
            name: name.into(),
            detail: detail.into(),
            subagent: false,
        };
        push(
            &mut home,
            PlanEvent::Agent {
                event: call("Read", "Cargo.toml"),
            },
        );
        push(
            &mut home,
            PlanEvent::Agent {
                event: call("Grep", "fn main"),
            },
        );
        push(
            &mut home,
            PlanEvent::Agent {
                event: crate::agent::AgentEvent::Message {
                    text: "Reading the project layout.".into(),
                    subagent: false,
                },
            },
        );
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn creation_shows_only_the_newest_lines_that_fit() {
        let mut home = home(valid_plan());
        home.begin_planning();
        for i in 0..40u64 {
            let event = PlanEvent::Agent {
                event: crate::agent::AgentEvent::ToolCall {
                    name: "Read".into(),
                    detail: format!("file-{i}.rs"),
                    subagent: false,
                },
            };
            home.plan_event(None, plan_event_line(&event).map(|l| l.at(i)));
        }
        let terminal = render(&home);
        let text = format!("{}", terminal.backend());
        assert!(text.contains("file-39.rs"), "{text}");
        assert!(!text.contains("file-0.rs"), "{text}");
        assert!(text.contains("▶ Planning…"), "{text}");
    }

    #[test]
    fn error_popup() {
        let mut home = home(valid_plan());
        home.data.plan_uncommitted = true;
        home.show_start_error(&anyhow::anyhow!(
            "The working tree has uncommitted changes:\n  phasewise.md\nCommit or stash them, then run phasewise again."
        ));
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn help_popup() {
        let mut home = home(valid_plan());
        home.mode = Mode::Help;
        let terminal = render(&home);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn status_message_is_shown() {
        let mut home = home(valid_plan());
        home.message = Some(
            "Plan written to phasewise.md (3 phases, $0.12). Review it, commit it, then press Enter."
                .into(),
        );
        let terminal = render(&home);
        let text = format!("{}", terminal.backend());
        assert!(
            text.contains("Plan written to phasewise.md (3 phases, $0.12). Review it, commit it,"),
            "{text}"
        );
    }

    #[test]
    fn small_terminal_shows_only_the_message() {
        let home = home(valid_plan());
        let mut terminal = Terminal::new(TestBackend::new(60, 15)).expect("test terminal");
        terminal.draw(|f| draw(f, &home)).expect("draw");
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn tail_keeps_the_end_of_the_text() {
        assert_eq!(tail("short", 10), "short");
        assert_eq!(tail("0123456789ab", 6), "…789ab");
        assert_eq!(tail("abc", 0), "…");
    }
}

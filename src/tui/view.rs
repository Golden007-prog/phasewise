//! Rendering of the TUI. Deterministic: the output depends only on the `App`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, Mode, Tab};
use super::data::DiffView;
use crate::display::status_label;
use crate::state::PhaseStatus;

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 20;
const LIST_WIDTH: u16 = 30;
const POPUP_WIDTH: u16 = 56;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_too_small(frame, area);
        return;
    }
    let [header, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Min(1)]).areas(body);
    draw_header(frame, app, header);
    draw_phases(frame, app, left);
    draw_detail(frame, app, right);
    draw_bottom(frame, app, bottom);
    draw_popup(frame, app, area);
}

fn draw_too_small(frame: &mut Frame, area: Rect) {
    let text = format!(
        "phasewise needs a terminal of at least {MIN_WIDTH}×{MIN_HEIGHT} (now {}×{}). Enlarge the window.",
        area.width, area.height
    );
    let height = 3.min(area.height);
    let [_, middle, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        middle,
    );
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let v = &app.view;
    let line = Line::from(vec![
        Span::styled("phasewise", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "  run {}  {}  [{}]  {}",
            v.run_id, v.plan_title, v.agent, v.run_branch
        )),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn symbol(status: PhaseStatus) -> (&'static str, Color) {
    match status {
        PhaseStatus::Pending => ("·", Color::DarkGray),
        PhaseStatus::Running => ("▶", Color::Yellow),
        PhaseStatus::Verifying => ("◎", Color::Yellow),
        PhaseStatus::AwaitingReview => ("●", Color::Blue),
        PhaseStatus::Done => ("✓", Color::Green),
        PhaseStatus::Failed => ("✗", Color::Red),
    }
}

fn draw_phases(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .view
        .phases
        .iter()
        .map(|p| {
            let (sym, color) = symbol(p.status);
            ListItem::new(Line::from(vec![
                Span::styled(sym, Style::new().fg(color)),
                Span::raw(format!(" {} {}", p.number, p.title)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(Block::new().borders(Borders::ALL).title(" Phases "))
        .highlight_symbol("> ")
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = match app.view.phases.get(app.selected) {
        Some(p) => match &p.failure {
            Some(f) => format!(" Phase {}: {f} ", p.number),
            None => format!(
                " Phase {}: {} (attempt {}) ",
                p.number,
                status_label(p.status),
                p.attempts
            ),
        },
        None => " No phases ".to_string(),
    };
    let block = Block::new().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let [tabs, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    frame.render_widget(Paragraph::new(tab_line(app.tab)), tabs);

    let lines = match app.tab {
        Tab::Diff => diff_lines(&app.diff),
        Tab::Log => app.log.iter().map(|l| Line::raw(l.as_str())).collect(),
        Tab::Verify => app.verify.lines().map(Line::raw).collect(),
    };
    app.page = content.height.max(1);
    let max_scroll = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(content.height);
    app.scroll = app.scroll.min(max_scroll);
    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), content);
}

fn tab_line(active: Tab) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, tab) in Tab::ALL.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" | "));
        }
        let style = if tab == active {
            Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::new()
        };
        spans.push(Span::styled(tab.title(), style));
    }
    Line::from(spans)
}

fn diff_lines(diff: &DiffView) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (path, added, removed) in &diff.files {
        let counts = match (added, removed) {
            (Some(a), Some(r)) => format!("+{a} -{r}"),
            _ => "bin".to_string(),
        };
        lines.push(Line::raw(format!("  {path}  {counts}")));
    }
    if let Some(note) = &diff.note {
        lines.push(Line::raw(note.clone()));
    }
    if !diff.text.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.extend(diff.text.lines().map(diff_line));
    }
    if !diff.untracked.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            "Untracked files:",
            Style::new().add_modifier(Modifier::BOLD),
        ));
        lines.extend(diff.untracked.iter().map(|p| Line::raw(format!("  ? {p}"))));
    }
    lines
}

const HEADER_PREFIXES: [&str; 9] = [
    "diff ",
    "index ",
    "+++",
    "---",
    "new file",
    "deleted file",
    "rename ",
    "similarity ",
    "Binary files",
];

fn diff_line(line: &str) -> Line<'static> {
    let style = if HEADER_PREFIXES.iter().any(|p| line.starts_with(p)) {
        Style::new().add_modifier(Modifier::BOLD)
    } else if line.starts_with('+') {
        Style::new().fg(Color::Green)
    } else if line.starts_with('-') {
        Style::new().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new()
    };
    Line::styled(line.to_string(), style)
}

fn draw_bottom(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    for (key, what) in app.available_keys() {
        spans.push(Span::styled(key, Style::new().add_modifier(Modifier::BOLD)));
        spans.push(Span::raw(format!(" {what}  ")));
    }
    if let Some(message) = &app.message {
        spans.push(Span::styled(
            message.clone(),
            Style::new().fg(Color::Yellow),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_popup(frame: &mut Frame, app: &App, area: Rect) {
    let (title, body): (&str, Vec<Line>) = match &app.mode {
        Mode::Normal => return,
        Mode::ConfirmRollback(n) => (
            " Rollback ",
            vec![
                Line::raw(format!("Roll back to the checkpoint of phase {n}?")),
                Line::raw("Later phases are reset and their changes discarded."),
                Line::raw(""),
                Line::raw("y: confirm   any other key: cancel"),
            ],
        ),
        Mode::ConfirmInterrupt => (
            " Interrupt ",
            vec![
                Line::raw("Interrupt the running phase?"),
                Line::raw(""),
                Line::raw("y: confirm   any other key: cancel"),
            ],
        ),
        Mode::ConfirmQuit => (
            " Quit ",
            vec![
                Line::raw("A phase is running. Interrupt it and quit?"),
                Line::raw(""),
                Line::raw("y: confirm   any other key: cancel"),
            ],
        ),
        Mode::Note(buffer) => (
            " Retry ",
            vec![
                Line::raw("Note for the agent (optional):"),
                Line::raw(format!("> {buffer}_")),
                Line::raw(""),
                Line::raw("Enter: retry   Esc: cancel"),
            ],
        ),
    };
    let height = u16::try_from(body.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
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
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .block(Block::new().borders(Borders::ALL).title(title)),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::tui::data::{PhaseView, RunView};

    fn app() -> App {
        let phase = |number, title: &str, status, checkpoint| PhaseView {
            number,
            title: title.to_string(),
            status,
            attempts: 1,
            failure: None,
            has_checkpoint: checkpoint,
        };
        let mut app = App::new(RunView {
            run_id: "20260101-120000".into(),
            plan_title: "Demo plan".into(),
            agent: "claude".into(),
            run_branch: "phasewise/20260101-120000".into(),
            original_branch: "main".into(),
            active: true,
            phases: vec![
                phase(1, "Scaffold the project", PhaseStatus::Done, true),
                phase(2, "Add the parser", PhaseStatus::AwaitingReview, true),
                phase(3, "Write the docs", PhaseStatus::Pending, false),
                phase(4, "Release", PhaseStatus::Failed, false),
            ],
        });
        app.selected = 1;
        app
    }

    fn render(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal.draw(|f| draw(f, app)).expect("draw");
        terminal
    }

    const DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs\nindex 111..222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    more();\n }\n";

    fn with_diff(app: &mut App) {
        app.diff = DiffView {
            files: vec![
                ("src/lib.rs".into(), Some(2), Some(1)),
                ("assets/logo.png".into(), None, None),
            ],
            text: DIFF.to_string(),
            untracked: vec!["notes.txt".into()],
            note: None,
        };
    }

    #[test]
    fn phase_list_and_diff_tab() {
        let mut app = app();
        with_diff(&mut app);
        let terminal = render(&mut app, 100, 30);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn log_tab_with_subagent_line() {
        let mut app = app();
        app.tab = Tab::Log;
        app.log = vec![
            "── attempt 1 ──".into(),
            "Working on phase 2.".into(),
            "Write src/parser.rs".into(),
            "[subagent] Read src/plan.rs".into(),
        ];
        let terminal = render(&mut app, 100, 30);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn small_terminal_shows_only_the_message() {
        let mut app = app();
        let terminal = render(&mut app, 60, 15);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn rollback_confirmation_popup() {
        let mut app = app();
        with_diff(&mut app);
        app.mode = Mode::ConfirmRollback(2);
        let terminal = render(&mut app, 100, 30);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn note_input_popup() {
        let mut app = app();
        app.mode = Mode::Note("handle empty input".into());
        let terminal = render(&mut app, 100, 30);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn scroll_is_clamped_to_the_content() {
        let mut app = app();
        with_diff(&mut app);
        app.scroll = 500;
        render(&mut app, 100, 30);
        assert!(app.scroll < 30);
        assert_eq!(app.page, 25);
    }
}

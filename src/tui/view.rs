//! Rendering of the TUI. Deterministic: the output depends only on the `App`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, HELP_KEYS, Mode, Tab};
use super::data::{DiffView, LogLine};
use crate::display::{EventLine, status_label};
use crate::state::PhaseStatus;
use crate::style::Role;

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 20;
/// The phase panel takes 30 % of the width, within these limits.
const LIST_PERCENT: u16 = 30;
const LIST_MIN: u16 = 24;
const LIST_MAX: u16 = 48;
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
    let [left, right] = Layout::horizontal([
        Constraint::Length(list_width(area.width)),
        Constraint::Min(1),
    ])
    .areas(body);
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

/// Width of the phase panel for a terminal `total` columns wide.
pub fn list_width(total: u16) -> u16 {
    (total * LIST_PERCENT / 100).clamp(LIST_MIN, LIST_MAX)
}

/// `text` cut to at most `max` characters, ending with `…` when it was cut.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    if max > 0 {
        out.push('…');
    }
    out
}

/// ratatui style for a role; the same roles the CLI paints with ANSI codes.
fn role_style(role: Role) -> Style {
    match role {
        Role::Default => Style::new(),
        Role::Heading => Style::new().add_modifier(Modifier::BOLD),
        Role::Dim => Style::new().fg(Color::DarkGray),
        Role::Ok => Style::new().fg(Color::Green),
        Role::Fail => Style::new().fg(Color::Red),
        Role::Warn => Style::new().fg(Color::Yellow),
        Role::Accent => Style::new().fg(Color::Cyan),
        Role::Tool => Style::new().fg(Color::Blue),
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let v = &app.view;
    let dim = role_style(Role::Dim);
    let mut spans = vec![Span::styled("phasewise", role_style(Role::Heading))];
    // v0.2 part 2 adds the model after the agent.
    for (label, value) in [
        ("run", v.run_id.as_str()),
        ("plan", v.plan_title.as_str()),
        ("agent", v.agent.as_str()),
        ("branch", v.run_branch.as_str()),
    ] {
        spans.push(Span::styled(format!("  {label} "), dim));
        spans.push(Span::raw(value.to_string()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn symbol(status: PhaseStatus) -> (&'static str, Role) {
    match status {
        PhaseStatus::Pending => ("·", Role::Dim),
        PhaseStatus::Running | PhaseStatus::Verifying => ("●", Role::Warn),
        PhaseStatus::AwaitingReview => ("◆", Role::Accent),
        PhaseStatus::Done => ("✓", Role::Ok),
        PhaseStatus::Failed => ("✗", Role::Fail),
    }
}

fn draw_phases(frame: &mut Frame, app: &App, area: Rect) {
    // Border (2) and highlight symbol (2) take columns from the row.
    let row_width = usize::from(area.width).saturating_sub(4);
    let items: Vec<ListItem> = app
        .view
        .phases
        .iter()
        .map(|p| {
            let (sym, role) = symbol(p.status);
            let prefix = format!(" {} ", p.number);
            let room = row_width.saturating_sub(1 + prefix.chars().count());
            ListItem::new(Line::from(vec![
                Span::styled(sym, role_style(role)),
                Span::raw(format!("{prefix}{}", truncate(&p.title, room))),
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
        Tab::Log => app.log.iter().map(log_line).collect(),
        Tab::Verify => verify_lines(&app.verify),
    };
    app.page = content.height.max(1);
    let max_scroll = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(content.height);
    app.scroll = app.scroll.min(max_scroll);
    frame.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), content);
}

fn event_spans(line: &EventLine) -> Vec<Span<'static>> {
    let style = role_style(line.role);
    let mut spans = vec![Span::styled(line.icon, style)];
    if !line.label.is_empty() {
        let label = if line.detail.is_empty() {
            line.label.clone()
        } else {
            line.label_column()
        };
        spans.push(Span::styled(format!(" {label}"), style));
    }
    if !line.detail.is_empty() {
        spans.push(Span::styled(
            format!(" {}", line.detail),
            role_style(line.detail_role()),
        ));
    }
    spans
}

fn log_line(line: &LogLine) -> Line<'static> {
    match line {
        LogLine::Attempt { number, resumed } => Line::styled(
            LogLine::attempt_heading(*number, *resumed),
            role_style(Role::Heading),
        ),
        LogLine::Event(event) => Line::from(event_spans(event)),
    }
}

/// The verification log: `$ command` lines are headings with the result of
/// the command in front (read from the `exit code:` line that follows it),
/// output lines are plain, failure trailers are red.
fn verify_lines(text: &str) -> Vec<Line<'static>> {
    let all: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in all.iter().enumerate() {
        if let Some(command) = line.strip_prefix("$ ") {
            let result = all[i + 1..]
                .iter()
                .take_while(|l| !l.starts_with("$ ") && !l.starts_with("=== "))
                .find_map(|l| {
                    if *l == "exit code: 0" {
                        Some(("✓", Role::Ok))
                    } else if l.starts_with("exit code: ")
                        || l.starts_with("timed out after ")
                        || *l == "interrupted"
                    {
                        Some(("✗", Role::Fail))
                    } else {
                        None
                    }
                });
            let (icon, role) = result.unwrap_or(("⚙", Role::Dim));
            out.push(Line::from(vec![
                Span::styled(icon, role_style(role)),
                Span::raw(" "),
                Span::styled(format!("$ {command}"), role_style(Role::Heading)),
            ]));
        } else if line.starts_with("=== ") {
            out.push(Line::styled(line.to_string(), role_style(Role::Dim)));
        } else if (line.starts_with("exit code: ") && *line != "exit code: 0")
            || line.starts_with("timed out after ")
            || *line == "interrupted"
        {
            out.push(Line::styled(line.to_string(), role_style(Role::Fail)));
        } else {
            out.push(Line::raw(line.to_string()));
        }
    }
    out
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
        lines.push(Line::styled(
            format!("  {path}  {counts}"),
            role_style(Role::Heading),
        ));
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
        spans.push(Span::styled(key, role_style(Role::Heading)));
        spans.push(Span::styled(format!(" {what}  "), role_style(Role::Dim)));
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
    use crate::agent::AgentEvent;
    use crate::display::agent_event_line;
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
        let call = |name: &str, detail: &str, subagent| {
            LogLine::Event(
                agent_event_line(&AgentEvent::ToolCall {
                    name: name.into(),
                    detail: detail.into(),
                    subagent,
                })
                .expect("tool line"),
            )
        };
        app.log = vec![
            LogLine::Attempt {
                number: 1,
                resumed: false,
            },
            LogLine::Event(
                agent_event_line(&AgentEvent::Message {
                    text: "Working on phase 2.".into(),
                    subagent: false,
                })
                .expect("message line"),
            ),
            call("Write", "src/parser.rs", false),
            call("Read", "src/plan.rs", true),
            call("Bash", "cargo test", false),
            LogLine::Attempt {
                number: 2,
                resumed: true,
            },
            LogLine::Event(
                agent_event_line(&AgentEvent::Retry {
                    detail: "rate limited".into(),
                })
                .expect("retry line"),
            ),
        ];
        let terminal = render(&mut app, 100, 30);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn verify_tab() {
        let mut app = app();
        app.tab = Tab::Verify;
        app.verify = "=== attempt 1 ===\n$ cargo test\nrunning 3 tests\nexit code: 0\n$ test -f a.md\nexit code: 1\n$ sleep 9\ntimed out after 1 s\n".to_string();
        let terminal = render(&mut app, 100, 30);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn help_popup() {
        let mut app = app();
        app.mode = Mode::Help;
        let terminal = render(&mut app, 100, 30);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn long_titles_are_truncated_with_an_ellipsis() {
        let mut app = app();
        app.view.phases[0].title = "A very long phase title that cannot possibly fit".into();
        let terminal = render(&mut app, 80, 20);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn truncate_cuts_on_characters_and_marks_the_cut() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("exactly10!", 10), "exactly10!");
        assert_eq!(truncate("0123456789ab", 10), "012345678…");
        assert_eq!(truncate("ñandú ñandú ñandú", 6), "ñandú…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(truncate("abc", 1), "…");
    }

    #[test]
    fn phase_panel_is_30_percent_clamped_to_24_and_48() {
        assert_eq!(list_width(80), 24);
        assert_eq!(list_width(100), 30);
        assert_eq!(list_width(120), 36);
        assert_eq!(list_width(160), 48);
        assert_eq!(list_width(400), 48);
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

//! Plan model and parser: a Markdown file with optional TOML front matter.
//!
//! Pure except for [`load`]. Line numbers are 1-based and refer to the
//! original file, front matter included.

use std::fmt;
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Coding agent that executes the phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
        })
    }
}

impl FromStr for AgentKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude" => Ok(AgentKind::Claude),
            "codex" => Ok(AgentKind::Codex),
            other => Err(format!(
                "unknown agent `{other}`; expected `claude` or `codex`"
            )),
        }
    }
}

/// Settings from the TOML front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanConfig {
    pub agent: AgentKind,
    pub verify: Vec<String>,
    pub max_retries: u32,
    pub phase_timeout_minutes: u64,
    pub verify_timeout_seconds: u64,
    pub agent_args: Vec<String>,
}

impl Default for PlanConfig {
    fn default() -> Self {
        PlanConfig {
            agent: AgentKind::Claude,
            verify: Vec::new(),
            max_retries: 1,
            phase_timeout_minutes: 60,
            verify_timeout_seconds: 600,
            agent_args: Vec::new(),
        }
    }
}

/// One phase of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    pub number: usize,
    pub title: String,
    /// Instructions for the agent, without `verify` blocks.
    pub body: String,
    pub verify: Vec<String>,
    /// Line of the phase heading.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub config: PlanConfig,
    pub title: Option<String>,
    /// Shared context sent in every phase.
    pub context: String,
    pub phases: Vec<Phase>,
}

/// A problem found in the plan; `line` is 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("line {line}: {message}")]
pub struct PlanError {
    pub line: usize,
    pub message: String,
}

fn err(line: usize, message: impl Into<String>) -> PlanError {
    PlanError {
        line,
        message: message.into(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    agent: Option<String>,
    verify: Option<Vec<String>>,
    max_retries: Option<u32>,
    phase_timeout_minutes: Option<u64>,
    verify_timeout_seconds: Option<u64>,
    agent_args: Option<Vec<String>>,
}

/// Reads and parses a plan file. Errors are rendered one per line as
/// `<path>:<line>: <message>`.
pub fn load(path: &Path) -> Result<Plan> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow!(
                "Plan file not found: {}. Create one with `phasewise init` or pass its path.",
                path.display()
            ));
        }
        Err(e) => {
            return Err(anyhow!(
                "Cannot read the plan file {}: {e}. Check the path and its permissions.",
                path.display()
            ));
        }
    };
    parse(&text).map_err(|errors| {
        let rendered: Vec<String> = errors
            .iter()
            .map(|e| format!("{}:{}: {}", path.display(), e.line, e.message))
            .collect();
        anyhow!(rendered.join("\n"))
    })
}

/// 1-based line of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    let end = offset.min(text.len());
    text.as_bytes()[..end]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// Line (1-based, original file) of the TOML key `key` inside the front matter.
fn key_line(text: &str, fm_first_line: usize, fm_lines: usize, key: &str) -> usize {
    text.lines()
        .enumerate()
        .skip(fm_first_line - 1)
        .take(fm_lines)
        .find(|(_, l)| {
            l.trim_start()
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        })
        .map_or(fm_first_line, |(i, _)| i + 1)
}

/// Parses plan text. Returns every problem found, sorted by line.
pub fn parse(text: &str) -> Result<Plan, Vec<PlanError>> {
    let mut errors = Vec::new();
    let mut config = PlanConfig::default();
    let mut body_start = 0;

    if text.lines().next().map(|l| l.trim_end_matches('\r')) == Some("+++") {
        // Byte offset of every line start.
        let mut offset = 0;
        let mut end = None;
        let mut fm_start = 0;
        for (i, raw) in text.split_inclusive('\n').enumerate() {
            let line_start = offset;
            offset += raw.len();
            if i == 0 {
                fm_start = offset;
                continue;
            }
            if raw.trim_end_matches(['\n', '\r']) == "+++" {
                end = Some((line_start, offset, i));
                break;
            }
        }
        match end {
            None => {
                return Err(vec![err(
                    1,
                    "the front matter is not closed; add a line with `+++` after the settings",
                )]);
            }
            Some((fm_end, after, closing_idx)) => {
                body_start = after;
                let fm_text = &text[fm_start..fm_end];
                match parse_front_matter(text, fm_text, fm_start, closing_idx - 1) {
                    Ok(c) => config = c,
                    Err(mut es) => errors.append(&mut es),
                }
            }
        }
    }

    match parse_body(text, body_start) {
        Ok((title, context, phases)) => {
            if errors.is_empty() {
                return Ok(Plan {
                    config,
                    title,
                    context,
                    phases,
                });
            }
        }
        Err(mut es) => errors.append(&mut es),
    }
    errors.sort_by_key(|e| e.line);
    Err(errors)
}

fn parse_front_matter(
    text: &str,
    fm_text: &str,
    fm_start: usize,
    fm_lines: usize,
) -> Result<PlanConfig, Vec<PlanError>> {
    let raw: RawConfig = match toml::from_str(fm_text) {
        Ok(raw) => raw,
        Err(e) => {
            let line = e
                .span()
                .map_or(1, |span: Range<usize>| line_of(text, fm_start + span.start));
            return Err(vec![err(line, e.message().to_string())]);
        }
    };
    let mut errors = Vec::new();
    let mut config = PlanConfig::default();
    if let Some(agent) = raw.agent {
        match agent.parse() {
            Ok(a) => config.agent = a,
            Err(m) => errors.push(err(key_line(text, 2, fm_lines, "agent"), m)),
        }
    }
    if let Some(v) = raw.verify {
        config.verify = v;
    }
    if let Some(v) = raw.max_retries {
        config.max_retries = v;
    }
    if let Some(v) = raw.phase_timeout_minutes {
        if v < 1 {
            errors.push(err(
                key_line(text, 2, fm_lines, "phase_timeout_minutes"),
                "`phase_timeout_minutes` must be at least 1",
            ));
        }
        config.phase_timeout_minutes = v;
    }
    if let Some(v) = raw.verify_timeout_seconds {
        if v < 1 {
            errors.push(err(
                key_line(text, 2, fm_lines, "verify_timeout_seconds"),
                "`verify_timeout_seconds` must be at least 1",
            ));
        }
        config.verify_timeout_seconds = v;
    }
    if let Some(v) = raw.agent_args {
        config.agent_args = v;
    }
    if errors.is_empty() {
        Ok(config)
    } else {
        Err(errors)
    }
}

/// Heading source text without `#` markers or setext underline, trimmed.
fn heading_text(source: &str) -> String {
    let source = source.trim();
    if source.starts_with('#') {
        let first = source.lines().next().unwrap_or("").trim();
        let rest = first.trim_start_matches('#');
        let stripped = rest.trim_end_matches('#');
        // A closing `#` sequence needs a space before it (`C#` keeps its `#`).
        let text = if stripped.is_empty() || stripped.ends_with(char::is_whitespace) {
            stripped
        } else {
            rest
        };
        text.trim().to_string()
    } else {
        // Setext: every line but the underline.
        let lines: Vec<&str> = source.lines().collect();
        let n = lines.len().saturating_sub(1).max(1);
        lines[..n]
            .iter()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Removes a leading `Phase <digits>:` unless nothing would remain.
fn strip_phase_prefix(title: &str) -> String {
    let rest = match title.get(..5) {
        Some(p) if p.eq_ignore_ascii_case("phase") => &title[5..],
        _ => return title.to_string(),
    };
    let rest = rest.trim_start();
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return title.to_string();
    }
    match rest[digits..].trim_start().strip_prefix(':') {
        Some(after) if !after.trim().is_empty() => after.trim().to_string(),
        _ => title.to_string(),
    }
}

struct PhaseHeading {
    title: String,
    range: Range<usize>,
}

type Parsed = (Option<String>, String, Vec<Phase>);

fn parse_body(text: &str, body_start: usize) -> Result<Parsed, Vec<PlanError>> {
    let body = &text[body_start..];
    let mut errors = Vec::new();

    let mut h1: Option<Range<usize>> = None;
    let mut h1_title = None;
    let mut headings: Vec<PhaseHeading> = Vec::new();
    // (range, commands) of every `verify` block.
    let mut verify_blocks: Vec<(Range<usize>, Vec<String>)> = Vec::new();

    let mut in_verify: Option<(Range<usize>, String)> = None;
    for (event, range) in Parser::new_ext(body, Options::empty()).into_offset_iter() {
        let range = range.start + body_start..range.end + body_start;
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let title = heading_text(&text[range.clone()]);
                match level {
                    HeadingLevel::H1 if h1.is_none() && headings.is_empty() => {
                        h1 = Some(range);
                        h1_title = Some(title);
                    }
                    HeadingLevel::H2 => headings.push(PhaseHeading {
                        title: strip_phase_prefix(&title),
                        range,
                    }),
                    _ => {}
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if &*info == "verify" => {
                in_verify = Some((range, String::new()));
            }
            Event::Text(t) => {
                if let Some((_, content)) = in_verify.as_mut() {
                    content.push_str(&t);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((range, content)) = in_verify.take() {
                    verify_blocks.push((range, commands(&content)));
                }
            }
            _ => {}
        }
    }

    let first_phase_start = headings.first().map_or(text.len(), |h| h.range.start);

    for (range, _) in &verify_blocks {
        if range.start < first_phase_start {
            errors.push(err(
                line_of(text, range.start),
                "a `verify` block must come after a phase heading (`## ...`)",
            ));
        }
    }

    let mut context = String::new();
    context.push_str(&text[body_start..first_phase_start.max(body_start)]);
    if let Some(h1) = &h1 {
        context = format!(
            "{}{}",
            &text[body_start..h1.start],
            &text[h1.end..first_phase_start]
        );
    }
    let context = context.trim().to_string();

    let mut phases: Vec<Phase> = Vec::new();
    for (i, heading) in headings.iter().enumerate() {
        let end = headings.get(i + 1).map_or(text.len(), |h| h.range.start);
        let mut body = String::new();
        let mut cursor = heading.range.end;
        let mut verify = Vec::new();
        for (range, cmds) in &verify_blocks {
            if range.start >= heading.range.end && range.end <= end {
                body.push_str(&text[cursor..range.start]);
                cursor = range.end;
                verify.extend(cmds.iter().cloned());
            }
        }
        body.push_str(&text[cursor..end]);
        phases.push(Phase {
            number: i + 1,
            title: heading.title.clone(),
            body: body.trim().to_string(),
            verify,
            line: line_of(text, heading.range.start),
        });
    }

    if phases.is_empty() {
        errors.push(err(
            1,
            "the plan has no phases; add at least one `## ` heading",
        ));
    }
    for phase in &phases {
        if phase.body.is_empty() {
            errors.push(err(
                phase.line,
                format!(
                    "phase {} (\"{}\") has no instructions; write what the agent must do under its heading",
                    phase.number, phase.title
                ),
            ));
        }
    }
    let mut seen: Vec<(String, usize)> = Vec::new();
    for phase in &phases {
        let key = phase.title.to_lowercase();
        match seen.iter().find(|(k, _)| *k == key) {
            Some((_, first)) => errors.push(err(
                phase.line,
                format!(
                    "duplicate phase title \"{}\" (first used at line {first}); titles must be unique",
                    phase.title
                ),
            )),
            None => seen.push((key, phase.line)),
        }
    }

    if errors.is_empty() {
        Ok((h1_title, context, phases))
    } else {
        Err(errors)
    }
}

fn commands(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Plan {
        parse(text).unwrap_or_else(|e| panic!("unexpected errors: {e:?}"))
    }

    fn errs(text: &str) -> Vec<(usize, String)> {
        match parse(text) {
            Ok(_) => panic!("expected errors"),
            Err(es) => es.into_iter().map(|e| (e.line, e.message)).collect(),
        }
    }

    #[test]
    fn defaults_without_front_matter() {
        let plan = ok("## One\n\nDo it.\n");
        assert_eq!(plan.config, PlanConfig::default());
        assert_eq!(plan.config.agent, AgentKind::Claude);
        assert_eq!(plan.config.max_retries, 1);
        assert_eq!(plan.config.phase_timeout_minutes, 60);
        assert_eq!(plan.config.verify_timeout_seconds, 600);
        assert_eq!(plan.title, None);
        assert_eq!(plan.phases.len(), 1);
    }

    #[test]
    fn agent_kind_display_and_from_str() {
        assert_eq!(AgentKind::Codex.to_string(), "codex");
        assert_eq!("claude".parse::<AgentKind>(), Ok(AgentKind::Claude));
        assert!("gpt".parse::<AgentKind>().is_err());
    }

    #[test]
    fn full_front_matter() {
        let plan = ok(
            "+++\nagent = \"codex\"\nverify = [\"cargo test\"]\nmax_retries = 3\nphase_timeout_minutes = 5\nverify_timeout_seconds = 7\nagent_args = [\"--x\"]\n+++\n## A\n\nbody\n",
        );
        assert_eq!(plan.config.agent, AgentKind::Codex);
        assert_eq!(plan.config.verify, vec!["cargo test"]);
        assert_eq!(plan.config.max_retries, 3);
        assert_eq!(plan.config.phase_timeout_minutes, 5);
        assert_eq!(plan.config.verify_timeout_seconds, 7);
        assert_eq!(plan.config.agent_args, vec!["--x"]);
    }

    #[test]
    fn front_matter_only_on_line_one() {
        // `+++` on line 2 is not front matter: it is plain body text.
        let plan = ok("\n+++\nagent = \"codex\"\n+++\n## A\n\nbody\n");
        assert_eq!(plan.config.agent, AgentKind::Claude);
        assert!(plan.context.contains("agent = \"codex\""));
    }

    #[test]
    fn unterminated_front_matter() {
        let e = errs("+++\nagent = \"claude\"\n## A\n\nbody\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, 1);
    }

    #[test]
    fn unknown_key_names_the_key() {
        let e = errs("+++\nagent = \"claude\"\nbogus = 1\n+++\n## A\n\nbody\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, 3);
        assert!(e[0].1.contains("bogus"), "{}", e[0].1);
        assert!(!e[0].1.contains('\n'));
    }

    #[test]
    fn toml_syntax_error_has_line() {
        let e = errs("+++\nagent = \"claude\"\nverify = [\n+++\n## A\n\nbody\n");
        assert_eq!(e.len(), 1);
        assert!(e[0].0 >= 3 && e[0].0 <= 4, "{e:?}");
        assert!(!e[0].1.contains('\n'));
    }

    #[test]
    fn toml_type_error_has_line() {
        let e = errs("+++\nmax_retries = \"two\"\n+++\n## A\n\nbody\n");
        assert_eq!(e[0].0, 2);
    }

    #[test]
    fn unknown_agent() {
        let e = errs("+++\nverify = []\nagent = \"gpt\"\n+++\n## A\n\nbody\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, 3);
        assert!(e[0].1.contains("claude") && e[0].1.contains("codex"));
    }

    #[test]
    fn timeouts_must_be_positive() {
        let e =
            errs("+++\nphase_timeout_minutes = 0\nverify_timeout_seconds = 0\n+++\n## A\n\nbody\n");
        assert_eq!(e.iter().map(|x| x.0).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn crlf_input() {
        let plan = ok(
            "+++\r\nagent = \"codex\"\r\n+++\r\n# T\r\n\r\nctx\r\n\r\n## A\r\n\r\nbody\r\n\r\n```verify\r\nmake\r\n```\r\n",
        );
        assert_eq!(plan.config.agent, AgentKind::Codex);
        assert_eq!(plan.title.as_deref(), Some("T"));
        assert_eq!(plan.context, "ctx");
        assert_eq!(plan.phases[0].title, "A");
        assert_eq!(plan.phases[0].body, "body");
        assert_eq!(plan.phases[0].verify, vec!["make"]);
        assert_eq!(plan.phases[0].line, 8);
    }

    #[test]
    fn crlf_unknown_key_line() {
        let e = errs("+++\r\nfoo = 1\r\n+++\r\n## A\r\n\r\nbody\r\n");
        assert_eq!(e[0].0, 2);
    }

    #[test]
    fn phases_numbered_by_order_with_lines() {
        let plan = ok("+++\nmax_retries = 0\n+++\n# T\n\n## First\n\na\n\n## Second\n\nb\n");
        assert_eq!(plan.phases[0].number, 1);
        assert_eq!(plan.phases[1].number, 2);
        assert_eq!(plan.phases[0].line, 6);
        assert_eq!(plan.phases[1].line, 10);
        assert_eq!(plan.phases[0].body, "a");
        assert_eq!(plan.phases[1].body, "b");
    }

    #[test]
    fn phase_prefix_stripped() {
        let plan = ok(
            "## Phase 1: Add toolchain\n\na\n\n## PHASE   7 :  Shout\n\nb\n\n## phase 3:x\n\nc\n\n## Phased out\n\nd\n",
        );
        let titles: Vec<_> = plan.phases.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, vec!["Add toolchain", "Shout", "x", "Phased out"]);
        assert_eq!(plan.phases[1].number, 2);
    }

    #[test]
    fn phase_prefix_alone_is_kept() {
        let plan = ok("## Phase 1:\n\na\n");
        assert_eq!(plan.phases[0].title, "Phase 1:");
    }

    #[test]
    fn setext_heading_opens_phase() {
        let plan = ok("Setext Title\n------------\n\nbody\n");
        assert_eq!(plan.phases.len(), 1);
        assert_eq!(plan.phases[0].title, "Setext Title");
        assert_eq!(plan.phases[0].line, 1);
        assert_eq!(plan.phases[0].body, "body");
    }

    #[test]
    fn closing_hashes_removed() {
        let plan = ok("## Title ##\n\na\n\n## C#\n\nb\n");
        assert_eq!(plan.phases[0].title, "Title");
        assert_eq!(plan.phases[1].title, "C#");
    }

    #[test]
    fn heading_in_code_block_is_not_a_phase() {
        let plan = ok("## A\n\n```sh\n## not a heading\n```\n");
        assert_eq!(plan.phases.len(), 1);
        assert!(plan.phases[0].body.contains("## not a heading"));
    }

    #[test]
    fn title_and_context() {
        let plan =
            ok("+++\n+++\n\n# My plan\n\nShared **context**.\n\n### Notes\n\nmore\n\n## A\n\nx\n");
        assert_eq!(plan.title.as_deref(), Some("My plan"));
        assert_eq!(plan.context, "Shared **context**.\n\n### Notes\n\nmore");
    }

    #[test]
    fn only_first_h1_is_title() {
        let plan = ok("# One\n\n# Two\n\n## A\n\nx\n");
        assert_eq!(plan.title.as_deref(), Some("One"));
        assert!(plan.context.contains("# Two"));
    }

    #[test]
    fn h3_stays_in_body() {
        let plan = ok("## A\n\ntext\n\n### Sub\n\nmore\n");
        assert_eq!(plan.phases.len(), 1);
        assert_eq!(plan.phases[0].body, "text\n\n### Sub\n\nmore");
    }

    #[test]
    fn verify_blocks_removed_and_parsed() {
        let plan = ok(
            "## A\n\nbefore\n\n```verify\n# comment\ncargo build\n\n  cargo test  \n```\n\nafter\n\n```sh\nkeep\n```\n",
        );
        let p = &plan.phases[0];
        assert_eq!(p.verify, vec!["cargo build", "cargo test"]);
        assert!(!p.body.contains("verify"));
        assert!(!p.body.contains("cargo"));
        assert!(p.body.contains("before") && p.body.contains("after"));
        assert!(p.body.contains("```sh\nkeep\n```"));
    }

    #[test]
    fn verify_label_must_be_exact() {
        let plan = ok("## A\n\n```verify extra\nx\n```\n\n```Verify\ny\n```\n");
        assert!(plan.phases[0].verify.is_empty());
        assert!(plan.phases[0].body.contains("```Verify"));
    }

    #[test]
    fn verify_before_first_phase_is_error() {
        let e = errs("# T\n\n```verify\nmake\n```\n\n## A\n\nx\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, 3);
    }

    #[test]
    fn no_phases() {
        let e = errs("# Title\n\njust text\n");
        assert_eq!(e.len(), 1);
        assert!(e[0].1.contains("no phases"));
    }

    #[test]
    fn empty_phase_body() {
        let e = errs("## A\n\n```verify\nmake\n```\n\n## B\n\nx\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, 1);
    }

    #[test]
    fn duplicate_titles_case_insensitive() {
        let e = errs("## Same\n\na\n\n## Phase 2: SAME\n\nb\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, 5);
        assert!(e[0].1.contains("line 1"), "{}", e[0].1);
    }

    #[test]
    fn multiple_errors_sorted_by_line() {
        let e = errs(
            "+++\nagent = \"x\"\nphase_timeout_minutes = 0\n+++\n\n## A\n\n## B\n\nz\n\n## b\n\nw\n",
        );
        let lines: Vec<_> = e.iter().map(|x| x.0).collect();
        let mut sorted = lines.clone();
        sorted.sort_unstable();
        assert_eq!(lines, sorted);
        assert_eq!(lines, vec![2, 3, 6, 12]);
    }

    #[test]
    fn load_missing_file() {
        let e = load(Path::new("/nonexistent/phasewise.md")).unwrap_err();
        let msg = e.to_string();
        assert!(msg.starts_with("Plan file not found: /nonexistent/phasewise.md."));
        assert!(msg.contains("phasewise init"));
    }

    #[test]
    fn load_renders_path_and_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.md");
        std::fs::write(&path, "## A\n\n## B\n\nx\n").unwrap();
        let msg = load(&path).unwrap_err().to_string();
        assert!(msg.starts_with(&format!("{}:1: ", path.display())), "{msg}");
    }
}

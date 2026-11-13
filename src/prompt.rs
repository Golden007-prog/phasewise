//! The one place with the agent prompt texts. Pure functions.

use crate::plan::Plan;

const MAX_SUMMARY_CHARS: usize = 200;

/// Builds the prompt of one phase. `completed` holds (number, title, one-line
/// summary) of the phases already done; `note` is an optional reviewer note.
pub fn phase_prompt(
    plan: &Plan,
    phase_number: usize,
    completed: &[(usize, String, String)],
    verify: &[String],
    note: Option<&str>,
) -> String {
    let plan_title = plan.title.as_deref().unwrap_or("(untitled)");
    let total = plan.phases.len();
    let (title, body) = plan
        .phases
        .iter()
        .find(|p| p.number == phase_number)
        .map(|p| (p.title.as_str(), p.body.as_str()))
        .unwrap_or(("", ""));

    let completed_text = if completed.is_empty() {
        "None.".to_string()
    } else {
        completed
            .iter()
            .map(|(n, t, s)| format!("- Phase {n}: {t} — {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let verify_text = if verify.is_empty() {
        "(none)".to_string()
    } else {
        verify
            .iter()
            .map(|c| format!("  {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut out = format!(
        "You are executing one phase of a larger plan. The plan is orchestrated by phasewise,\n\
which runs phases one at a time and checkpoints the repository after each one.\n\
\n\
## Plan: {plan_title}\n\
\n\
{context}\n\
\n\
## Phases already completed\n\
{completed_text}\n\
\n\
## Your task: phase {phase_number} of {total} — {title}\n\
\n\
{body}\n\
\n\
## Rules\n\
- Do only this phase. Do not start work that belongs to later phases.\n\
- Do not create git commits, branches or tags, and never push. phasewise checkpoints your work.\n\
- After you finish, these commands will be run and must succeed:\n\
{verify_text}\n\
- End with a short summary of what you changed.\n",
        context = plan.context,
    );
    if let Some(note) = note {
        out.push_str(&format!("\n## Note from the reviewer\n{note}\n"));
    }
    out
}

/// Report of a failed verification command, sent back to the agent.
pub fn verify_failure_report(
    command: &str,
    exit_code: Option<i32>,
    timed_out: bool,
    timeout_secs: u64,
    output_tail: &str,
) -> String {
    let what = if timed_out {
        format!("timed out after {timeout_secs} s")
    } else {
        match exit_code {
            Some(code) => format!("failed with exit code {code}"),
            None => "was terminated by a signal".to_string(),
        }
    };
    format!(
        "The verification command `{command}` {what}.\n\nLast lines of its output:\n```\n{}\n```\n",
        output_tail.trim_end()
    )
}

/// Prompt sent when continuing the same session after a failed verification.
pub fn resume_prompt(report: &str) -> String {
    format!(
        "Verification failed after your last changes.\n\n{report}\nFix the problem so the verification commands pass, then end with a short summary of what you changed.\n"
    )
}

/// Prompt for a new session when no session id is known: the phase prompt plus
/// the report of the previous failed attempt.
pub fn fresh_retry_prompt(phase_prompt: &str, report: &str) -> String {
    format!("{phase_prompt}\n## Previous attempt failed verification\n{report}")
}

/// First non-empty line of the summary, at most 200 characters, or
/// `(no summary)`.
pub fn one_line_summary(summary: Option<&str>) -> String {
    let line = summary
        .and_then(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("");
    if line.is_empty() {
        return "(no summary)".to_string();
    }
    line.chars().take(MAX_SUMMARY_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::parse;

    fn small_plan() -> Plan {
        let text = "# Demo\n\nShared context.\n\n## Phase 1: First\n\nDo the first thing.\n\n## Phase 2: Second\n\nDo the second thing.\n";
        parse(text).expect("plan parses")
    }

    #[test]
    fn prompt_snapshot() {
        let plan = small_plan();
        let completed = vec![(1, "First".to_string(), "Created a.txt".to_string())];
        let verify = vec!["cargo test".to_string(), "test -f a.txt".to_string()];
        let got = phase_prompt(&plan, 2, &completed, &verify, None);
        let want =
            "You are executing one phase of a larger plan. The plan is orchestrated by phasewise,
which runs phases one at a time and checkpoints the repository after each one.

## Plan: Demo

Shared context.

## Phases already completed
- Phase 1: First — Created a.txt

## Your task: phase 2 of 2 — Second

Do the second thing.

## Rules
- Do only this phase. Do not start work that belongs to later phases.
- Do not create git commits, branches or tags, and never push. phasewise checkpoints your work.
- After you finish, these commands will be run and must succeed:
  cargo test
  test -f a.txt
- End with a short summary of what you changed.
";
        assert_eq!(got, want);
    }

    #[test]
    fn prompt_defaults_and_note() {
        let mut plan = small_plan();
        plan.title = None;
        let got = phase_prompt(&plan, 1, &[], &[], Some("Use tabs."));
        assert!(got.contains("## Plan: (untitled)\n"));
        assert!(got.contains("## Phases already completed\nNone.\n"));
        assert!(got.contains("must succeed:\n(none)\n- End with"));
        assert!(got.ends_with("\n## Note from the reviewer\nUse tabs.\n"));
    }

    #[test]
    fn failure_report_variants() {
        let r = verify_failure_report("make", Some(2), false, 600, "boom\n");
        assert!(r.contains("`make` failed with exit code 2"));
        assert!(r.contains("```\nboom\n```"));
        let r = verify_failure_report("make", None, true, 30, "");
        assert!(r.contains("timed out after 30 s"));
        let r = verify_failure_report("make", None, false, 30, "x");
        assert!(r.contains("terminated by a signal"));
    }

    #[test]
    fn retry_prompts_include_report() {
        let report = verify_failure_report("make", Some(1), false, 1, "err");
        assert!(resume_prompt(&report).contains(&report));
        let fresh = fresh_retry_prompt("PHASE\n", &report);
        assert!(fresh.starts_with("PHASE\n\n## Previous attempt failed verification\n"));
        assert!(fresh.ends_with(&report));
    }

    #[test]
    fn summary_is_one_short_line() {
        assert_eq!(one_line_summary(None), "(no summary)");
        assert_eq!(one_line_summary(Some("  \n\n")), "(no summary)");
        assert_eq!(one_line_summary(Some("\n  first \nsecond")), "first");
        let long = "é".repeat(300);
        assert_eq!(one_line_summary(Some(&long)).chars().count(), 200);
    }
}

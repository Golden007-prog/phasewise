//! Model and effort selection and `phasewise models`, with the fake agents.

mod common;

use std::time::Duration;

use common::TestRepo;
use serde_json::Value;
use tempfile::TempDir;

const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fakes/fake-claude.sh");
const FAKE_CODEX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fakes/fake-codex.sh");

const PLAN: &str = "+++\nmodel = \"plan-model\"\neffort = \"low\"\n+++\n\n# Demo\n\n\
## One\n\nDo one.\n\n\
## Two\n\nDo two.\n\n```phasewise\nmodel = \"phase-model\"\neffort = \"high\"\n```\n\n\
## Three\n\nDo three.\n";

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

struct Env {
    repo: TestRepo,
    logs: TempDir,
}

impl Env {
    fn new() -> Env {
        Env::with_plan(PLAN)
    }

    fn with_plan(plan: &str) -> Env {
        let repo = TestRepo::new();
        repo.write("phasewise.md", plan);
        repo.commit_all("add plan");
        Env {
            repo,
            logs: tempfile::tempdir().expect("create log dir"),
        }
    }

    fn exec(&self, args: &[&str], envs: &[(&str, &str)]) -> Out {
        let mut cmd = self.repo.phasewise();
        cmd.args(args)
            .env("PHASEWISE_CLAUDE_BIN", FAKE_CLAUDE)
            .env("PHASEWISE_CODEX_BIN", FAKE_CODEX)
            .env("FAKE_LOG_DIR", self.logs.path())
            .envs(envs.iter().copied())
            .timeout(Duration::from_secs(60));
        let output = cmd.output().expect("run phasewise");
        Out {
            code: output.status.code().expect("exit code"),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn ok(&self, args: &[&str]) -> Out {
        let out = self.exec(args, &[]);
        assert_eq!(out.code, 0, "{args:?}\n{}\n{}", out.stdout, out.stderr);
        out
    }

    fn state(&self) -> Value {
        let id = self.repo.read(".phasewise/current");
        let text = self
            .repo
            .read(&format!(".phasewise/runs/{}/state.json", id.trim()));
        serde_json::from_str(&text).expect("state.json is JSON")
    }

    /// The `args:` line of every agent call so far.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.logs.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.strip_prefix("args: "))
            .map(str::to_string)
            .collect()
    }

    fn calls_log(&self) -> String {
        std::fs::read_to_string(self.logs.path().join("calls.log")).unwrap_or_default()
    }
}

#[test]
fn plan_phase_and_cli_values_reach_the_agent_with_the_right_precedence() {
    let env = Env::new();
    env.ok(&["run"]);
    env.ok(&["next"]);
    // The CLI model beats the plan; the effort still comes from the plan.
    let out = env.ok(&["next", "--model", "cli-model"]);
    let calls = env.calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(
        calls[0].ends_with("--model plan-model --effort low"),
        "{}",
        calls[0]
    );
    assert!(
        calls[1].ends_with("--model phase-model --effort high"),
        "{}",
        calls[1]
    );
    assert!(
        calls[2].ends_with("--model cli-model --effort low"),
        "{}",
        calls[2]
    );
    // The settings block is not part of the prompt.
    let log = env.calls_log();
    assert!(
        !log.contains("phase-model\"") && !log.contains("```phasewise"),
        "{log}"
    );
    // The header shows the effective settings.
    assert!(
        out.stdout.contains("attempt 1 · claude · cli-model · low"),
        "{}",
        out.stdout
    );

    let state = env.state();
    assert_eq!(state["model_override"], "cli-model");
    assert!(state.get("effort_override").is_none());
    assert_eq!(state["phases"][1]["model"], "phase-model");
    assert_eq!(state["phases"][2]["model"], "cli-model");
    assert_eq!(state["phases"][2]["effort"], "low");

    let status = env.ok(&["status"]).stdout;
    assert!(status.contains("MODEL"), "{status}");
    assert!(status.contains("plan-model"), "{status}");
    assert!(status.contains("phase-model"), "{status}");
}

#[test]
fn the_model_the_agent_reports_is_recorded_and_shown() {
    let env = Env::with_plan("# Demo\n\n## One\n\nDo one.\n");
    env.ok(&["run"]);
    let state = env.state();
    assert_eq!(state["phases"][0]["agent_model"], "claude-sonnet-5-5");
    assert!(state["phases"][0].get("model").is_none());
    let status = env.ok(&["status"]).stdout;
    assert!(status.contains("Sonnet 5.5"), "{status}");
}

#[test]
fn the_reported_model_follows_a_retry_with_another_model() {
    let env = Env::with_plan("# Demo\n\n## One\n\nDo one.\n");
    env.ok(&["run"]);
    assert_eq!(env.state()["phases"][0]["agent_model"], "claude-sonnet-5-5");
    env.ok(&["retry", "--model", "claude-haiku-5-5"]);
    assert_eq!(env.state()["phases"][0]["agent_model"], "claude-haiku-5-5");
    let status = env.ok(&["status"]).stdout;
    assert!(status.contains("Haiku 5.5"), "{status}");
}

#[test]
fn the_override_is_stored_and_default_clears_it() {
    let env = Env::new();
    env.ok(&["run", "--model", "first", "--effort", "max"]);
    env.ok(&["retry"]);
    env.ok(&["retry", "--effort", "default"]);
    env.ok(&["retry", "--model", "default", "--effort", "medium"]);
    let calls = env.calls();
    assert!(
        calls[0].ends_with("--model first --effort max"),
        "{}",
        calls[0]
    );
    // The override from `run` is still stored.
    assert!(
        calls[1].ends_with("--model first --effort max"),
        "{}",
        calls[1]
    );
    // `default` removes the effort override: the plan's value applies again.
    assert!(
        calls[2].ends_with("--model first --effort low"),
        "{}",
        calls[2]
    );
    assert!(
        calls[3].ends_with("--model plan-model --effort medium"),
        "{}",
        calls[3]
    );
    let state = env.state();
    assert!(state.get("model_override").is_none());
    assert_eq!(state["effort_override"], "medium");
}

#[test]
fn plan_values_are_dropped_when_the_run_uses_another_agent() {
    let env = Env::new();
    env.ok(&["run", "--agent", "codex"]);
    let calls = env.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(!calls[0].contains("-m "), "{}", calls[0]);
    assert!(!calls[0].contains("model_reasoning_effort"), "{}", calls[0]);
    assert!(!calls[0].contains("plan-model"), "{}", calls[0]);
    assert!(env.state()["phases"][0].get("model").is_none());
    // An explicit override still applies, in the Codex syntax.
    env.ok(&["retry", "--model", "gpt-fast", "--effort", "high"]);
    let calls = env.calls();
    assert!(
        calls[1]
            .contains("--sandbox workspace-write -m gpt-fast -c model_reasoning_effort=\"high\" -"),
        "{}",
        calls[1]
    );
}

#[test]
fn invalid_flag_values_are_refused_before_anything_runs() {
    let env = Env::new();
    for (flag, arg) in [
        ("--model", "--model=-x"),
        ("--model", "--model=two words"),
        ("--model", "--model="),
        ("--effort", "--effort=extreme"),
    ] {
        let out = env.exec(&["run", arg], &[]);
        assert_eq!(out.code, 1, "{arg}");
        assert!(
            out.stderr.contains(&format!("Invalid {flag}")),
            "{}",
            out.stderr
        );
    }
    assert!(!env.repo.path().join(".phasewise/current").exists());
    assert!(env.calls().is_empty());
}

#[test]
fn check_shows_the_plan_and_phase_settings() {
    let env = Env::new();
    let out = env.ok(&["check"]).stdout;
    assert!(out.contains("Model: plan-model"), "{out}");
    assert!(out.contains("Effort: low"), "{out}");
    assert!(out.contains("model: phase-model · effort: high"), "{out}");
}

#[test]
fn models_lists_both_agents() {
    let env = Env::new();
    let out = env.ok(&["models"]);
    assert!(out.stdout.contains("Claude Code"), "{}", out.stdout);
    assert!(out.stdout.contains("Opus 5.5"), "{}", out.stdout);
    assert!(out.stdout.contains("claude-opus-5-5"), "{}", out.stdout);
    assert!(!out.stdout.contains("opusplan"), "{}", out.stdout);
    assert!(out.stdout.contains("claude --model"), "{}", out.stdout);
    assert!(
        out.stdout.contains("Claude Code's own choice"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("Effort: low, medium, high, xhigh, max"));
    assert!(out.stdout.contains("Codex's own choice"), "{}", out.stdout);
    let gpt_fast = out
        .stdout
        .lines()
        .find(|l| l.contains("gpt-fast"))
        .expect("gpt-fast line");
    assert!(gpt_fast.contains("GPT-Fast"), "{gpt_fast}");
    assert!(gpt_fast.contains("effort: low, medium, high"), "{gpt_fast}");
    assert!(
        gpt_fast.contains("Fast model for simple tasks."),
        "{gpt_fast}"
    );
    assert!(out.stdout.contains("gpt-deep"), "{}", out.stdout);
    // Hidden entries and entries without a slug are filtered out.
    assert!(!out.stdout.contains("gpt-hidden"), "{}", out.stdout);
    assert!(!out.stdout.contains("without slug"), "{}", out.stdout);
    assert!(out.stdout.contains("codex -m"), "{}", out.stdout);
}

#[test]
fn models_for_one_agent_only() {
    let env = Env::new();
    let out = env.ok(&["models", "--agent", "claude"]).stdout;
    assert!(
        out.contains("claude-sonnet-5-5") && !out.contains("Codex"),
        "{out}"
    );
}

#[test]
fn a_failing_listing_is_reported_and_only_fails_when_nothing_listed() {
    let env = Env::new();
    for (mode, expected) in [("fail", "failed"), ("invalid", "not valid JSON")] {
        let envs = [("FAKE_MODELS_MODE", mode)];
        let only = env.exec(&["models", "--agent", "codex"], &envs);
        assert_eq!(only.code, 1, "{mode}");
        assert!(
            only.stderr.contains("Could not list the codex models"),
            "{}",
            only.stderr
        );
        assert!(only.stderr.contains(expected), "{}", only.stderr);
        assert!(
            only.stderr.contains("`") && only.stderr.contains("-m"),
            "{}",
            only.stderr
        );
        assert!(only.stdout.is_empty(), "{}", only.stdout);

        let both = env.exec(&["models"], &envs);
        assert_eq!(both.code, 0, "{mode}: {}", both.stderr);
        assert!(both.stdout.contains("claude-haiku-5-5"), "{}", both.stdout);
        assert!(
            both.stderr.contains("Could not list the codex models"),
            "{}",
            both.stderr
        );
    }
}

#[test]
fn a_missing_codex_binary_is_reported() {
    let env = Env::new();
    let out = env.exec(
        &["models", "--agent", "codex"],
        &[("PHASEWISE_CODEX_BIN", "/nonexistent/codex")],
    );
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("`codex` was not found"),
        "{}",
        out.stderr
    );
}

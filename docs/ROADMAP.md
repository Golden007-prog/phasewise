# Roadmap

Ideas that are out of scope for v0.1. Nothing here is promised.

- Generate the plan with the agent (`phasewise plan "goal"`).
- A different agent per phase.
- Phases in parallel, using git worktrees.
- A separate diff per subagent (v0.1 only labels subagent activity in the log).
- More agents (Gemini CLI, opencode, …).
- Windows support.
- Spending limits per phase or per run.
- Verify the Claude Code and Codex adapters against real runs (`docs/dev/smoke-test.md`), save real captures as fixtures and mark the agents "tested".
- Check agent logins in `phasewise doctor` (v0.1 only checks presence and version).

phasewise will never add telemetry, update checks or any other network access
of its own.

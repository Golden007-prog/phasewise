# Roadmap

Ideas that are out of scope for v0.1. Nothing here is promised.

- A different agent per phase.
- Phases in parallel, using git worktrees.
- A separate diff per subagent (v0.1 only labels subagent activity in the log).
- More agents (Gemini CLI, opencode, …).
- Windows support.
- Spending limits per phase or per run.
- Save sanitized real captures of both agents as test fixtures.
- Check agent logins in `phasewise doctor` (v0.1 only checks presence and version).

phasewise will never add telemetry, update checks or any other network access
of its own.

+++
# Agent that runs the phases: "claude" or "codex".
agent = "claude"
# Commands run after EVERY phase; all must exit 0. Example: ["cargo test"].
verify = []
# Automatic retries when verification fails.
max_retries = 1
# Maximum minutes the agent may take for one phase.
phase_timeout_minutes = 60
# Maximum seconds for each verification command.
verify_timeout_seconds = 600
# Extra arguments passed to the agent command line.
agent_args = []
+++

# My plan

Shared context sent to the agent in every phase: what the project is, the
conventions to follow and anything that is true for the whole plan.

## Phase 1: First step

Describe what the agent must do in this phase. Be specific about the files to
touch and about what "done" means.

```verify
echo "replace me with a real check, for example: cargo test"
```

## Phase 2: Second step

Describe the next step. The agent also receives a summary of the completed
phases.

```verify
echo "replace me with a real check"
```

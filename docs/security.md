# Security model

phasewise runs a coding agent unattended. Read this before using it on a plan you did not write or a repository you do not trust.

## What the agent is allowed to do

phasewise launches the agent headless, with no human to answer permission prompts:

- Claude Code: `--permission-mode acceptEdits --permission-prompts none --allowedTools Read,Edit,Write,Bash --disallowedTools "Bash(git push *)"`.
  `Bash` is allowed, which means arbitrary shell commands. The `Bash(git push *)` deny rule is best effort: it matches that command pattern and does not stop other ways to reach the network or a remote.
- Codex: `--sandbox workspace-write`. The confinement is Codex's own sandbox, not phasewise's.

The agent runs in your repository with your user account and inherits your environment variables (including any tokens in them). Verification commands from the plan (`verify`) also run through `sh -c` with the same environment.

## What protects you

The dedicated branch `phasewise/<run-id>` and the checkpoint commits: your original branch is not modified, every phase is a commit you can inspect (`phasewise diff`, `phasewise review`) and return to (`phasewise rollback`). This limits the damage to your repository history. It does not limit what a command run by the agent can do outside the repository.

phasewise itself:

- never runs `git push`, never deletes branches and never uses `--force`;
- makes no network connections (no telemetry, no update checks); the agents do;
- never commits `.phasewise/` (it is excluded through `.git/info/exclude`);
- never logs in to an agent or reads its credentials.

phasewise is not a sandbox. A plan is a set of instructions for an agent that can run commands: treat a plan like a script.

## Recommendations

- For untrusted plans or repositories, use a disposable clone, container or VM.
- Keep secrets out of the repository and out of the environment the agent inherits.
- Review every diff before merging: `git merge --squash` into your branch only after reading the result.
- Keep `max_retries` and the timeouts (`phase_timeout_minutes`, `verify_timeout_seconds`) sensible.
- Check the plan with `phasewise check` and read the `verify` commands: they run on your machine.

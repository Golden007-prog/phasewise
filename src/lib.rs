//! phasewise: run big plans with Claude Code or Codex one phase at a time —
//! verified, checkpointed, and reviewable.

#![forbid(unsafe_code)]
#![warn(clippy::unwrap_used, clippy::expect_used)]

pub mod cli;
pub mod git;
pub mod plan;

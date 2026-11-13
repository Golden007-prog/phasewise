//! Process helpers. Spawning and group termination arrive in a later phase.

use std::process::{Command, Stdio};

/// Whether a process with this PID exists (`kill -0` through `sh`, no `unsafe`).
pub fn is_alive(pid: u32) -> bool {
    Command::new("sh")
        .args(["-c", "kill -0 \"$1\"", "sh"])
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

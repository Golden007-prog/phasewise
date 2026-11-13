//! Process helpers: process groups, termination and liveness.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

/// Spawns the command as the leader of a new process group, so the whole tree
/// can be stopped with [`terminate_group`]. The group id equals the child PID.
pub fn spawn_in_group(cmd: &mut Command) -> io::Result<Child> {
    cmd.process_group(0).spawn()
}

/// Sends `signal` (for example `TERM`) to every process of the group.
/// Returns whether `kill` reported success.
pub fn signal_group(pgid: u32, signal: &str) -> bool {
    Command::new("sh")
        .args(["-c", "kill -s \"$1\" -- \"-$2\"", "sh"])
        .arg(signal)
        .arg(pgid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Stops a child started with [`spawn_in_group`]: `TERM` to the group, wait up
/// to `grace` for the leader to exit, then always `KILL` the group (members may
/// outlive the leader) and reap the leader.
pub fn terminate_group(child: &mut Child, grace: Duration) -> io::Result<ExitStatus> {
    let pgid = child.id();
    signal_group(pgid, "TERM");
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    // The group may already be gone; a failure here is expected and harmless.
    signal_group(pgid, "KILL");
    child.wait()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminate_group_kills_the_whole_tree() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30 & sleep 30 & wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = spawn_in_group(&mut cmd).expect("spawn");
        let pid = child.id();
        assert!(is_alive(pid));
        let started = Instant::now();
        let status = terminate_group(&mut child, Duration::from_secs(5)).expect("terminate");
        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(!is_alive(pid));
    }
}

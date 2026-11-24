//! Log levels of the CLI and of the daemon it spawns.

use crate::harness::*;
use std::fs;

#[test]
fn logging_is_quiet_by_default_and_verbose_with_a_flag() {
    let h = Harness::new();

    // Default: only the command's own output, no log lines on stderr or in the daemon log.
    let out = h.raw(&["run", "-n", "quiet", "sleep", "300"]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stderr), "", "default run should not log");
    h.ok(&["daemon", "stop"]);
    let daemon_log = fs::read_to_string(h.dir.path().join("daemon.err")).unwrap();
    assert!(!daemon_log.contains("INFO"), "daemon logged at info by default:\n{daemon_log}");

    // -vv: the CLI logs, and the daemon it starts inherits the level.
    let out = h.raw(&["-vv", "run", "-n", "loud", "sleep", "300"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("INFO"));
    h.ok(&["kill", "loud"]);
    h.ok(&["daemon", "stop"]);
    let daemon_log = fs::read_to_string(h.dir.path().join("daemon.err")).unwrap();
    assert!(daemon_log.contains("DEBUG"), "daemon did not inherit -vv:\n{daemon_log}");
}

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
    let daemon_log = fs::read_to_string(h.dir.path().join("daemon.log")).unwrap();
    // The startup line, with the version, is the one info line that is always there.
    let mut info = daemon_log.lines().filter(|line| line.contains("INFO"));
    let startup = info.next().unwrap_or_default();
    assert!(
        startup.contains("Starting daemon") && startup.contains(&format!("version=\"{}\"", env!("CARGO_PKG_VERSION"))),
        "no startup line with the version:\n{daemon_log}"
    );
    assert_eq!(info.next(), None, "daemon logged at info by default:\n{daemon_log}");

    // -vv: the CLI logs, and the daemon it starts inherits the level.
    let out = h.raw(&["-vv", "run", "-n", "loud", "sleep", "300"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("INFO"));
    h.ok(&["kill", "loud"]);
    h.ok(&["daemon", "stop"]);
    let daemon_log = fs::read_to_string(h.dir.path().join("daemon.log")).unwrap();
    assert!(daemon_log.contains("DEBUG"), "daemon did not inherit -vv:\n{daemon_log}");
}

#[test]
fn version_flag_prints_the_cargo_version() {
    let h = Harness::new();
    let out = h.ok(&["--version"]);
    assert_eq!(out.trim(), format!("sessiond {}", env!("CARGO_PKG_VERSION")));
}

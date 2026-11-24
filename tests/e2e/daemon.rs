//! Daemon lifecycle: auto-start, status, stop.

use crate::harness::*;

#[test]
fn commands_fail_cleanly_when_daemon_is_down() {
    let h = Harness::new();

    let err = h.fails(&["list"]);

    assert!(err.contains("Daemon not running"), "got: {err}");
}

#[test]
fn daemon_starts_automatically_when_running_a_session() {
    let h = Harness::new();

    // Starting a session should automatically start the daemon.
    h.ok(&["run", "-n", "auto", "sleep", "1"]);

    h.ok(&["daemon", "status"]);

    assert_eq!(h.sessions(), vec!["auto"]);
}

#[test]
fn daemon_status_reports_running_and_stopped() {
    let h = Harness::new();

    // Initially stopped.
    h.fails(&["daemon", "status"]);

    // Running a session starts the daemon.
    h.ok(&["run", "sleep", "1"]);

    h.ok(&["daemon", "status"]);

    // Stop the daemon.
    h.ok(&["daemon", "stop"]);

    // Status should now return exit code 1.
    h.fails(&["daemon", "status"]);
}

#[test]
fn daemon_stop_stops_it() {
    let h = Harness::new();

    h.ok(&["run", "-n", "x", "sleep", "1"]);

    h.ok(&["daemon", "stop"]);

    h.fails(&["list"]);
}

#[test]
fn stopping_an_already_stopped_daemon_succeeds() {
    let h = Harness::new();

    h.ok(&["daemon", "stop"]);

    h.fails(&["daemon", "status"]);
}

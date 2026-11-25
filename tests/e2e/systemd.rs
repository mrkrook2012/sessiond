//! Systemd-backed sessions and the `service` commands.

use crate::harness::*;
use std::fs;
use std::process::Command;

#[test]
fn service_commands_reject_ephemeral_sessions() {
    let h = Harness::new();

    h.ok(&["run", "-n", "ephemeral", "sleep", "300"]);

    // Systemd actions must not work on ordinary ephemeral sessions.
    h.fails(&["service", "start", "ephemeral"]);
    h.fails(&["service", "stop", "ephemeral"]);
    h.fails(&["service", "restart", "ephemeral"]);
    h.fails(&["service", "enable", "ephemeral"]);
    h.fails(&["service", "disable", "ephemeral"]);
    h.fails(&["service", "update", "ephemeral"]);

    // The failed operations must not have killed the session.
    assert_eq!(h.sessions(), vec!["ephemeral"]);
}

fn systemctl(args: &[&str]) {
    let out = Command::new("systemctl").arg("--user").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "systemctl --user {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn systemd_round_trip() {
    if !user_systemd_available() {
        eprintln!("skipping: no user systemd instance");
        return;
    }
    let h = Harness::new();
    h.ok(&["run", "--systemd", "--name", "testing101", "echo test"]);

    h.ok(&["service", "delete", "testing101"]);

    h.ok(&["run", "--systemd", "--name", "testing101", "echo test"]);

    h.ok(&["service", "delete", "testing101"]);
}

#[test]
fn stored_sysd_sessions_are_registered_on_daemon_start() {
    let h = Harness::with_isolated_systemd();

    let yaml = session_yaml;

    // A sysd session that was created but never started: YAML + unit file.
    fs::write(h.data_dir().join("stale.yaml"), yaml("stale")).unwrap();
    fs::write(h.systemd_dir().join("stale.service"), "[Unit]\n").unwrap();

    // YAML without a unit file is not a sysd session.
    fs::write(h.data_dir().join("orphan.yaml"), yaml("orphan")).unwrap();

    // Garbage must be skipped, not crash the daemon.
    fs::write(h.data_dir().join("broken.yaml"), "this: is: not: valid").unwrap();

    // Any command that starts the daemon triggers the scan.
    h.ok(&["run", "-n", "eph", "sleep", "300"]);

    let mut names = h.sessions();
    names.sort();
    assert_eq!(names, vec!["eph", "stale"]);

    // The registered session is a real sysd session: delete removes both files.
    h.ok(&["service", "delete", "stale"]);

    assert_eq!(h.sessions(), vec!["eph"]);
    assert!(!h.data_dir().join("stale.yaml").exists());
    assert!(!h.systemd_dir().join("stale.service").exists());
}

#[test]
fn external_systemctl_changes_are_tracked() {
    if !user_systemd_available() {
        eprintln!("skipping: no user systemd instance");
        return;
    }
    let h = Harness::new();
    // Unique per run so parallel or leftover units never collide.
    let name = format!("dtest-events-{}", std::process::id());

    h.ok(&["run", "--systemd", "--name", &name, "sleep", "300"]);
    wait_for_list_line(&h, &name, "running", |l| l.contains("running"));

    // Plain systemctl, not our CLI: the daemon must learn about it from systemd's signals.
    systemctl(&["enable", &format!("{name}.service")]);
    wait_for_list_line(&h, &name, "enabled", |l| l.contains(", enabled"));

    systemctl(&["disable", &format!("{name}.service")]);
    wait_for_list_line(&h, &name, "disabled", |l| l.contains("disabled"));

    systemctl(&["stop", &format!("{name}.service")]);
    wait_for_list_line(&h, &name, "stopped", |l| l.contains("stopped"));

    // Our own CLI goes through D-Bus as well.
    h.ok(&["service", "start", &name]);
    wait_for_list_line(&h, &name, "running", |l| l.contains("running"));

    h.ok(&["service", "delete", &name]);
    assert!(h.sessions().is_empty());
}

#[test]
fn service_delete_removes_service_and_yaml_files() {
    let h = Harness::with_isolated_systemd();
    let data = h.data_dir();
    let units = h.systemd_dir();

    let yaml = session_yaml;

    // Mixed case: the YAML keeps the original name, the unit is lowercase.
    // A second YAML for the same unit must go too.
    fs::write(data.join("MixedCase.yaml"), yaml("MixedCase")).unwrap();
    fs::write(data.join("mixedcase.yaml"), yaml("mixedcase")).unwrap();
    fs::write(units.join("mixedcase.service"), "[Unit]\n").unwrap();

    // Leftovers the daemon never registers: a YAML without unit, and a unit without YAML.
    fs::write(data.join("orphan.yaml"), yaml("orphan")).unwrap();
    fs::write(units.join("lonely.service"), "[Unit]\n").unwrap();

    // Unrelated files must survive.
    fs::write(data.join("keep.yaml"), yaml("keep")).unwrap();
    fs::write(units.join("keep.service"), "[Unit]\n").unwrap();

    h.ok(&["run", "-n", "eph", "sleep", "300"]);
    let mut names = h.sessions();
    names.sort();
    assert_eq!(names, vec!["eph", "keep", "mixedcase"]);

    // Registered session, by its original-case name.
    let out = h.ok(&["service", "delete", "MixedCase"]);
    assert!(out.contains("MixedCase.yaml"), "got: {out}");
    assert!(!data.join("MixedCase.yaml").exists());
    assert!(!data.join("mixedcase.yaml").exists());
    assert!(!units.join("mixedcase.service").exists());

    // Unregistered leftovers.
    h.ok(&["service", "delete", "orphan"]);
    assert!(!data.join("orphan.yaml").exists());
    h.ok(&["service", "delete", "lonely"]);
    assert!(!units.join("lonely.service").exists());

    // Nothing left to delete is an error.
    let err = h.fails(&["service", "delete", "orphan"]);
    assert!(err.contains("No service named"), "got: {err}");

    // Ephemeral sessions are not services.
    h.fails(&["service", "delete", "eph"]);

    let mut names = h.sessions();
    names.sort();
    assert_eq!(names, vec!["eph", "keep"]);
    assert!(data.join("keep.yaml").exists());
    assert!(units.join("keep.service").exists());
}

fn n_restarts(unit: &str) -> u32 {
    let out = Command::new("systemctl")
        .args(["--user", "show", "-p", "NRestarts", "--value", unit])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

#[test]
fn tracked_service_restarts_when_its_command_exits() {
    if !user_systemd_available() {
        eprintln!("skipping: no user systemd instance");
        return;
    }
    let h = Harness::new();
    let name = format!("dtest-restart-{}", std::process::id());
    let unit = format!("{name}.service");

    // `run --systemd` tracks its command, so this exits after a second and should restart.
    h.ok(&["run", "--systemd", "--name", &name, "sleep", "1"]);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while n_restarts(&unit) < 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "systemd never restarted {unit}"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    // After the restart the session is back and running.
    wait_for_list_line(&h, &name, "running", |l| l.contains("running"));

    h.ok(&["service", "delete", &name]);
    assert!(h.sessions().is_empty());
}

#[test]
fn service_commands_on_unknown_session_fail() {
    let h = Harness::new();
    h.ok(&["run", "-n", "eph", "sleep", "300"]);

    for action in ["start", "stop", "restart", "enable", "disable"] {
        let err = h.fails(&["service", action, "nope"]);
        assert!(err.contains("Session not found"), "service {action}: {err}");
    }
}

#[test]
fn list_services_shows_only_sysd_sessions_with_their_state() {
    let h = Harness::with_isolated_systemd();
    fs::write(h.data_dir().join("svc.yaml"), session_yaml("svc")).unwrap();
    fs::write(h.systemd_dir().join("svc.service"), "[Unit]\n").unwrap();
    h.ok(&["run", "-n", "eph", "sleep", "300"]);

    assert_eq!(h.ok(&["list", "--services", "--names"]), "svc\n");
    let list = h.ok(&["list", "--services"]);
    assert_eq!(list, "Active sessions:\n- svc [service, stopped, disabled]\n");
}

#[test]
fn internal_stop_without_daemon_succeeds_and_does_not_start_one() {
    let h = Harness::new();
    h.ok(&["__internal-systemd-stop", "anything"]);
    h.fails(&["daemon", "status"]);
}

#[test]
fn internal_stop_of_unknown_session_succeeds() {
    let h = Harness::new();
    h.ok(&["run", "-n", "eph", "sleep", "300"]);
    h.ok(&["__internal-systemd-stop", "nope"]);
    assert_eq!(h.sessions(), vec!["eph"]);
}

#[test]
fn internal_start_with_missing_config_fails() {
    let h = Harness::new();
    let missing = h.dir.path().join("missing.yaml");
    let err = h.fails(&["__internal-systemd-start", missing.to_str().unwrap()]);
    assert!(err.contains("Failed to get absolute path"), "got: {err}");
    // It checks the file before starting the daemon.
    h.fails(&["daemon", "status"]);
}

/// Regression: this used to destroy the session's windows before rejecting it, after
/// which the daemon panicked on every `list`.
#[test]
fn internal_stop_of_an_ephemeral_session_leaves_it_alone() {
    let h = Harness::new();
    h.ok(&["run", "-n", "eph", "sleep", "300"]);

    let err = h.fails(&["__internal-systemd-stop", "eph"]);
    assert!(err.contains("non sysd session") || err.contains("non-sysd session"), "got: {err}");

    assert_eq!(h.sessions(), vec!["eph"]);
    h.ok(&["attach", "eph"]);
}

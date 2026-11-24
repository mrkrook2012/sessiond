use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// One isolated daemon per test: own socket, own logs, killed on drop.
struct Harness {
    dir: TempDir,
    /// When set, unit files go to a temp dir instead of ~/.config/systemd/user.
    isolated_systemd: bool,
}

impl Harness {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("data")).unwrap();
        Self {
            dir,
            isolated_systemd: false,
        }
    }

    /// Like `new`, but unit files live in a temp dir. systemd won't see them,
    /// so only use this for tests that don't need systemctl to actually work.
    fn with_isolated_systemd() -> Self {
        let mut h = Self::new();
        fs::create_dir_all(h.systemd_dir()).unwrap();
        h.isolated_systemd = true;
        h
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    /// Where the daemon stores session YAML files.
    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn systemd_dir(&self) -> PathBuf {
        self.dir.path().join("systemd")
    }

    fn raw(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_daemon_test"));
        cmd.args(args)
            .env("DAEMON_TEST_SOCKET", self.socket())
            .env("DAEMON_TEST_LOG_DIR", self.dir.path())
            .env("DAEMON_TEST_DATA_DIR", self.data_dir());

        if self.isolated_systemd {
            cmd.env("DAEMON_TEST_SYSTEMD_DIR", self.systemd_dir());
        }

        cmd.output().expect("Failed to execute command")
    }

    fn assert_daemon_healthy(&self, step: &str) {
        let err = fs::read_to_string(self.dir.path().join("daemon.err")).unwrap_or_default();

        assert!(
            !err.contains("panicked"),
            "daemon panicked after '{step}':\n{err}"
        );
    }

    /// Runs the CLI, asserts success, returns stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.raw(args);

        assert!(
            out.status.success(),
            "`{}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );

        self.assert_daemon_healthy(&args.join(" "));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Runs the CLI, asserts failure, returns stderr.
    fn fails(&self, args: &[&str]) -> String {
        let out = self.raw(args);

        assert!(
            !out.status.success(),
            "`{}` unexpectedly succeeded",
            args.join(" ")
        );

        self.assert_daemon_healthy(&args.join(" "));
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn sessions(&self) -> Vec<String> {
        self.ok(&["list", "--names"])
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Exports a session into `dir` and returns (path, file contents).
    fn export(&self, name: &str, dir: &Path) -> (PathBuf, String) {
        fs::create_dir_all(dir).unwrap();

        self.ok(&["export", name, "--dir", dir.to_str().unwrap()]);

        // Find the exported file instead of parsing stdout.
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
            .collect();

        assert_eq!(
            files.len(),
            1,
            "expected exactly one yaml in {dir:?}, got {files:?}"
        );

        let path = files.remove(0);
        let contents = fs::read_to_string(&path).unwrap();

        (path, contents)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        eprintln!("Test daemon logs:");

        for file in ["daemon.out", "daemon.err"] {
            let path = self.dir.path().join(file);

            if let Ok(contents) = fs::read_to_string(path) {
                eprintln!("--- {file} ---\n{contents}");
            }
        }

        let _ = self.raw(&["daemon", "stop"]);
    }
}
// ---------------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_run_list_attach_kill() {
    let h = Harness::new();

    h.ok(&["run", "--name", "alpha", "sleep", "1"]);
    assert_eq!(h.sessions(), vec!["alpha"]);

    h.ok(&["attach", "alpha"]);

    h.ok(&["kill", "alpha"]);
    assert!(h.sessions().is_empty());
}

#[test]
fn run_without_name_gets_a_generated_name() {
    let h = Harness::new();

    h.ok(&["run", "sleep", "1"]);

    assert_eq!(h.sessions().len(), 1);
}

#[test]
fn multiple_sessions_and_kill_all() {
    let h = Harness::new();

    h.ok(&["run", "-n", "one", "sleep", "1"]);
    h.ok(&["run", "-n", "two", "sleep", "1"]);

    let mut names = h.sessions();
    names.sort();

    assert_eq!(names, vec!["one", "two"]);

    h.ok(&["kill", "--all"]);

    assert!(h.sessions().is_empty());
}

#[test]
fn killing_unknown_session_fails() {
    let h = Harness::new();

    h.ok(&["run", "-n", "real", "sleep", "1"]);

    let err = h.fails(&["kill", "does-not-exist"]);

    assert!(err.contains("Session not found"), "got: {err}");

    assert_eq!(h.sessions(), vec!["real"]);
}

#[test]
fn duplicate_session_name_is_rejected() {
    let h = Harness::new();

    h.ok(&["run", "-n", "dup", "sleep", "1"]);

    let err = h.fails(&["run", "-n", "dup", "sleep", "1"]);

    assert!(err.contains("already exists"), "got: {err}");

    assert_eq!(h.sessions(), vec!["dup"]);
}

// ---------------------------------------------------------------------------
// Daemon lifecycle
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Systemd sessions and service commands
// ---------------------------------------------------------------------------

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

    // The failed operations must not have killed the session.
    assert_eq!(h.sessions(), vec!["ephemeral"]);
}

// ---------------------------------------------------------------------------
// Export / import
// ---------------------------------------------------------------------------

#[test]
fn export_and_import_session_roundtrip() {
    let h = Harness::new();

    let export_dir_1 = h.dir.path().join("export1");
    let export_dir_2 = h.dir.path().join("export2");

    // 1. Start a named ephemeral session.
    h.ok(&["run", "--name", "roundtrip", "sleep", "300"]);

    assert_eq!(h.sessions(), vec!["roundtrip"]);

    // 2. Export it to disk.
    let (yaml_path, yaml_1) = h.export("roundtrip", &export_dir_1);

    assert!(
        yaml_1.contains("roundtrip"),
        "name missing from yaml:\n{yaml_1}"
    );

    assert!(
        yaml_1.contains("sleep 300"),
        "command missing from yaml:\n{yaml_1}"
    );

    assert!(yaml_path.is_file());

    // 3. Kill the original session.
    h.ok(&["kill", "roundtrip"]);

    assert!(h.sessions().is_empty());

    // 4. Import the exported YAML.
    h.ok(&["import", yaml_path.to_str().unwrap()]);

    assert_eq!(h.sessions(), vec!["roundtrip"]);

    // 5. Export again and verify the configuration is identical.
    let (_, yaml_2) = h.export("roundtrip", &export_dir_2);

    assert_eq!(yaml_1, yaml_2, "imported session differs from the original");

    // 6. Clean up.
    h.ok(&["kill", "roundtrip"]);

    assert!(h.sessions().is_empty());

    h.assert_daemon_healthy("end of roundtrip test");
}
/// True if a user systemd instance is reachable. CI runners usually have none,
/// so tests that need real systemd skip themselves there.
fn user_systemd_available() -> bool {
    Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Polls `list` until `pred` holds for the line of session `name`, or panics after 5 seconds.
fn wait_for_list_line(h: &Harness, name: &str, what: &str, pred: impl Fn(&str) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let list = h.ok(&["list"]);
        let line = list
            .lines()
            .find(|line| line.starts_with(&format!("- {name} ")))
            .unwrap_or("")
            .to_owned();
        if pred(&line) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {name} to be {what}, last line: {line:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
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

    let yaml = |name: &str| {
        format!("name: {name}\nwindows:\n- command: sleep 1\n  working_directory: /tmp\n")
    };

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
fn importing_nonexistent_file_fails() {
    let h = Harness::new();

    let missing = h.dir.path().join("does-not-exist.yaml");

    let err = h.fails(&["import", missing.to_str().unwrap()]);

    assert!(err.contains("Failed to get absolute path"), "got: {err}");
}

// #[test]
// fn export_unknown_session_fails() {
//     let h = Harness::new();

//     let export_dir = h.dir.path().join("export");

//     let err = h.fails(&[
//         "export",
//         "does-not-exist",
//         "--dir",
//         export_dir.to_str().unwrap(),
//     ]);

//     assert!(err.contains("Session not found"), "got: {err}");
// }

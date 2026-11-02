use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

/// One isolated daemon per test: own socket, own logs, killed on drop.
struct Harness {
    dir: TempDir,
}

impl Harness {
    fn new() -> Self {
        Self { dir: TempDir::new().unwrap() }
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn raw(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_daemon_test"))
            .args(args)
            .env("DAEMON_TEST_SOCKET", self.socket())
            .env("DAEMON_TEST_LOG_DIR", self.dir.path())
            .output()
            .expect("Failed to execute command")
    }

    fn assert_daemon_healthy(&self, step: &str) {
        let err = fs::read_to_string(self.dir.path().join("daemon.err")).unwrap_or_default();
        assert!(!err.contains("panicked"), "daemon panicked after '{step}':\n{err}");
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
        assert!(!out.status.success(), "`{}` unexpectedly succeeded", args.join(" "));
        self.assert_daemon_healthy(&args.join(" "));
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn sessions(&self) -> Vec<String> {
        self.ok(&["list", "--names"]).lines().map(str::to_owned).collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.raw(&["kill-daemon"]);
    }
}

#[test]
fn lifecycle_run_list_kill() {
    let h = Harness::new();

    h.ok(&["run", "sleep 10", "--name", "alpha"]);
    assert_eq!(h.sessions(), vec!["alpha"]);

    h.ok(&["kill", "alpha"]);
    assert!(h.sessions().is_empty());
}

#[test]
fn run_without_name_gets_a_generated_name() {
    let h = Harness::new();

    h.ok(&["run", "sleep 10"]);
    assert_eq!(h.sessions().len(), 1);
}

#[test]
fn multiple_sessions_and_kill_all() {
    let h = Harness::new();

    h.ok(&["run", "sleep 10", "-n", "one"]);
    h.ok(&["run", "sleep 10", "-n", "two"]);
    let mut names = h.sessions();
    names.sort();
    assert_eq!(names, vec!["one", "two"]);

    h.ok(&["kill-all"]);
    assert!(h.sessions().is_empty());
}

#[test]
fn killing_unknown_session_fails() {
    let h = Harness::new();

    h.ok(&["run", "sleep 10", "-n", "real"]);
    let err = h.fails(&["kill", "does-not-exist"]);
    assert!(err.contains("Session not found"), "got: {err}");
    assert_eq!(h.sessions(), vec!["real"]); // untouched
}

#[test]
fn commands_fail_cleanly_when_daemon_is_down() {
    let h = Harness::new();

    let err = h.fails(&["list"]);
    assert!(err.contains("Daemon not running"), "got: {err}");
}

#[test]
fn kill_daemon_stops_it() {
    let h = Harness::new();

    h.ok(&["run", "sleep 10", "-n", "x"]);
    h.ok(&["kill-daemon"]);
    h.fails(&["list"]);
}

// Enable once the daemon enforces unique names:
// #[test]
// fn duplicate_session_name_is_rejected() {
//     let h = Harness::new();
//     h.ok(&["run", "sleep 10", "-n", "dup"]);
//     h.fails(&["run", "sleep 10", "-n", "dup"]);
// }
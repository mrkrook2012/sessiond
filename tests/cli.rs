use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// One isolated daemon per test: own socket, own logs, killed on drop.
struct Harness {
    dir: TempDir,
}

impl Harness {
    fn new() -> Self {
        Self {
            dir: TempDir::new().unwrap(),
        }
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

#[test]
fn export_and_restore_session_roundtrip() {
    let h = Harness::new();
    let export_dir_1 = h.dir.path().join("export1");
    let export_dir_2 = h.dir.path().join("export2");

    // 1. Named session running a command
    h.ok(&["run", "sleep 300", "--name", "roundtrip"]);
    assert_eq!(h.sessions(), vec!["roundtrip"]);

    // 2. Export it
    let (yaml_path, yaml_1) = h.export("roundtrip", &export_dir_1);
    assert!(
        yaml_1.contains("roundtrip"),
        "name missing from yaml:\n{yaml_1}"
    );
    assert!(
        yaml_1.contains("sleep 300"),
        "command missing from yaml:\n{yaml_1}"
    );

    // 3. Kill the original so the restored one is the only one
    h.ok(&["kill", "roundtrip"]);
    assert!(h.sessions().is_empty());

    // 4. Start the same session from the file
    h.ok(&["run", yaml_path.to_str().unwrap()]);
    assert_eq!(h.sessions(), vec!["roundtrip"]);

    // 5. Same info: re-exporting the restored session must give identical yaml
    let (_, yaml_2) = h.export("roundtrip", &export_dir_2);
    assert_eq!(yaml_1, yaml_2, "restored session differs from the original");

    // 6. Kill it; every `ok` call already checked daemon.err for panics
    h.ok(&["kill", "roundtrip"]);
    assert!(h.sessions().is_empty());
    h.assert_daemon_healthy("end of roundtrip test");
}

// Enable once the daemon enforces unique names:
// #[test]
// fn duplicate_session_name_is_rejected() {
//     let h = Harness::new();
//     h.ok(&["run", "sleep 10", "-n", "dup"]);
//     h.fails(&["run", "sleep 10", "-n", "dup"]);
// }

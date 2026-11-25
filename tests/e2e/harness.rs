//! Shared helpers: one isolated daemon per test, plus systemd utilities.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// One isolated daemon per test: own socket, own logs, killed on drop.
pub struct Harness {
    pub dir: TempDir,
    /// When set, unit files go to a temp dir instead of ~/.config/systemd/user.
    isolated_sysd: bool,
}

impl Harness {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("data")).unwrap();
        Self {
            dir,
            isolated_sysd: false,
        }
    }

    /// Like `new`, but unit files live in a temp dir. systemd won't see them,
    /// so only use this for tests that don't need systemctl to actually work.
    pub fn with_isolated_sysd() -> Self {
        let mut h = Self::new();
        fs::create_dir_all(h.sysd_dir()).unwrap();
        h.isolated_sysd = true;
        h
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    /// Where the daemon stores session YAML files.
    pub fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    pub fn sysd_dir(&self) -> PathBuf {
        self.dir.path().join("sysd")
    }

    pub fn raw(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sessiond"));
        cmd.args(args)
            .env("SESSIOND_SOCKET", self.socket())
            .env("SESSIOND_LOG_DIR", self.dir.path())
            .env("SESSIOND_DATA_DIR", self.data_dir());

        if self.isolated_sysd {
            cmd.env("SESSIOND_SYSD_DIR", self.sysd_dir());
        }

        cmd.output().expect("Failed to execute command")
    }

    pub fn assert_daemon_healthy(&self, step: &str) {
        let err = fs::read_to_string(self.dir.path().join("daemon.log")).unwrap_or_default();

        assert!(
            !err.contains("panicked"),
            "daemon panicked after '{step}':\n{err}"
        );
    }

    /// Runs the CLI, asserts success, returns stdout.
    pub fn ok(&self, args: &[&str]) -> String {
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
    pub fn fails(&self, args: &[&str]) -> String {
        let out = self.raw(args);

        assert!(
            !out.status.success(),
            "`{}` unexpectedly succeeded",
            args.join(" ")
        );

        self.assert_daemon_healthy(&args.join(" "));
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    pub fn sessions(&self) -> Vec<String> {
        self.ok(&["list", "--names"])
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Exports a session into `dir` and returns (path, file contents).
    pub fn export(&self, name: &str, dir: &Path) -> (PathBuf, String) {
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

        if let Ok(contents) = fs::read_to_string(self.dir.path().join("daemon.log")) {
            eprintln!("--- daemon.log ---\n{contents}");
        }

        let _ = self.raw(&["daemon", "stop"]);
    }
}

/// A stored sysd session YAML as the daemon writes it.
pub fn session_yaml(name: &str) -> String {
    format!("name: {name}\nwindows:\n- command: sleep 1\n  working_directory: /tmp\n")
}

/// True if a user systemd instance is reachable. CI runners usually have none,
/// so tests that need real systemd skip themselves there.
pub fn user_sysd_available() -> bool {
    Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Polls `list` until `pred` holds for the line of session `name`, or panics after 5 seconds.
pub fn wait_for_list_line(h: &Harness, name: &str, what: &str, pred: impl Fn(&str) -> bool) {
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

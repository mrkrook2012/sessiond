//! `Daemon` without the RPC server. Everything here stays out of the real systemd
//! unit directory: methods that write unit files are covered by `tests/e2e` instead.

use std::fs;
use std::path::PathBuf;

use daemon_test::backend::daemon::Daemon;
use daemon_test::backend::client_view::ClientView;
use daemon_test::backend::session::{RunId, SessionId, Sessions};
use daemon_test::backend::systemd::UnitName;
use daemon_test::backend::terminal::Terminals;
use daemon_test::backend::terminal::virtual_terminal::TerminalEvent;
use daemon_test::backend::window::Windows;
use daemon_test::backend::storage::session_files::SessionFiles;
use daemon_test::ipc::protocol::{SessionInfo, SysdWaitStatus};
use daemon_test::backend::guid::Guid;
use tempfile::TempDir;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::support::eventually;

struct Fixture {
    dir: TempDir,
    daemon: Daemon,
    events: UnboundedReceiver<TerminalEvent>,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let (events_tx, events) = unbounded_channel();
        let daemon = Daemon::new(
            ClientView::new(),
            Terminals::new(events_tx),
            Windows::new(),
            Sessions::new(),
            SessionFiles::with_dir(dir.path().join("data")).unwrap(),
        );
        Self { dir, daemon, events }
    }

    /// Writes a session YAML outside the data dir and returns its path.
    fn yaml(&self, name: &str, windows: &[(&str, bool)]) -> PathBuf {
        let mut yaml = format!("name: {name}\nwindows:\n");
        for (command, tracked) in windows {
            yaml += &format!("- command: {command}\n  working_directory: /tmp\n  tracked: {tracked}\n");
        }
        let path = self.dir.path().join(format!("{name}.yaml"));
        fs::write(&path, yaml).unwrap();
        path
    }

    fn session(&self, name: &str) -> Option<SessionInfo> {
        self.daemon
            .handle_list_sessions()
            .into_iter()
            .find(|s| s.name == name)
    }

    fn id(&self, name: &str) -> SessionId {
        self.session(name).expect("session exists").id
    }

    fn status(&self, name: &str, run: RunId) -> SysdWaitStatus {
        self.daemon.sysd_wait_status(name, run).0
    }
}

/// A unit name nobody has a unit file for, so lookups in the real unit dir find nothing.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{}-{:?}", std::process::id(), std::thread::current().id())
        .replace(['(', ')'], "")
        .to_lowercase()
}

// ------------------------------------------------------------------ sessions

#[test]
fn create_list_and_kill_ephemeral_sessions() {
    let mut f = Fixture::new();
    let after_create = f
        .daemon
        .handle_create_session("/tmp".into(), Some("true".into()), Some("eph".into())).unwrap();
    assert_eq!(after_create.len(), 1);

    let id = f.id("eph");
    assert!(f.daemon.handle_kill_session(id).is_empty());
    assert!(f.daemon.handle_list_sessions().is_empty());
}

#[test]
fn start_session_from_file_loads_every_window() {
    let mut f = Fixture::new();
    let path = f.yaml("multi", &[("sleep 300", false), ("sleep 301", true)]);

    f.daemon.handle_start_session_from_file(path).unwrap();

    let session = f.session("multi").unwrap();
    assert_eq!(session.windows_info.len(), 2);
    assert!(session.sysd_data.is_none(), "imports are ephemeral");
}

#[test]
fn start_session_from_bad_file_fails_without_creating_anything() {
    let mut f = Fixture::new();
    let missing = f.dir.path().join("missing.yaml");
    assert!(f.daemon.handle_start_session_from_file(missing).is_err());

    let garbage = f.dir.path().join("garbage.yaml");
    fs::write(&garbage, "name: [").unwrap();
    assert!(f.daemon.handle_start_session_from_file(garbage).is_err());

    assert!(f.daemon.handle_list_sessions().is_empty());
}

#[test]
fn export_to_file_writes_name_dot_yaml_into_the_directory() {
    let mut f = Fixture::new();
    f.daemon
        .handle_create_session("/tmp".into(), Some("sleep 300".into()), Some("exp".into())).unwrap();
    let id = f.id("exp");
    let out = f.dir.path().join("out");
    fs::create_dir(&out).unwrap();

    let path = eventually("command to be exported", || {
        let path = f.daemon.handle_export_session_to_file(id, out.clone()).unwrap();
        fs::read_to_string(&path).unwrap().contains("sleep 300").then_some(path)
    });
    assert_eq!(path, out.join("exp.yaml"));

    // And it imports again.
    f.daemon.handle_kill_session(id);
    f.daemon.handle_start_session_from_file(path).unwrap();
    assert!(f.session("exp").is_some());
}

#[test]
fn export_of_unknown_session_fails() {
    let f = Fixture::new();
    assert!(f
        .daemon
        .handle_export_session_to_file(SessionId(Guid::from_u128(9)), f.dir.path().to_path_buf())
        .is_err());
}

// ------------------------------------------------------------------ sysd runs

#[test]
fn sysd_session_from_file_runs_and_restarts_with_a_new_run() {
    let mut f = Fixture::new();
    let path = f.yaml("Svc", &[("sleep 300", true)]);

    let first = f.daemon.handle_start_sysd_session_from_file(path.clone()).unwrap();
    // Stored under the lowercase unit name.
    let session = f.session("svc").unwrap();
    assert!(session.sysd_data.unwrap().running);
    assert_eq!(f.status("svc", first), SysdWaitStatus::Running);

    let second = f.daemon.handle_start_sysd_session_from_file(path).unwrap();
    assert_ne!(first, second);
    assert_eq!(f.status("svc", first), SysdWaitStatus::Ended);
    assert_eq!(f.status("svc", second), SysdWaitStatus::Running);
    assert_eq!(f.daemon.handle_list_sessions().len(), 1);
}

#[test]
fn stopping_a_sysd_session_ends_its_run() {
    let mut f = Fixture::new();
    let run = f
        .daemon
        .handle_start_sysd_session_from_file(f.yaml("svc", &[("sleep 300", true)]))
        .unwrap();
    let id = f.id("svc");

    f.daemon.handle_stop_sysd_session(id).unwrap();
    assert_eq!(f.status("svc", run), SysdWaitStatus::Ended);
    assert!(!f.session("svc").unwrap().sysd_data.unwrap().running);

    assert!(f.daemon.handle_stop_sysd_session(id).is_err());
}

#[test]
fn unit_deactivation_ends_the_run() {
    let mut f = Fixture::new();
    let run = f
        .daemon
        .handle_start_sysd_session_from_file(f.yaml("svc", &[("sleep 300", true)]))
        .unwrap();

    f.daemon.handle_unit_deactivated(&UnitName::new("svc").unwrap());
    assert_eq!(f.status("svc", run), SysdWaitStatus::Ended);
}

#[test]
fn tracked_exit_event_is_reported_to_waiters() {
    let mut f = Fixture::new();
    let run = f
        .daemon
        .handle_start_sysd_session_from_file(f.yaml("svc", &[("(exit 4)", true)]))
        .unwrap();
    let (_, changes) = f.daemon.sysd_wait_status("svc", run);

    let event = eventually("tracked exit", || f.events.try_recv().ok());
    f.daemon.handle_terminal_event(event);

    assert!(changes.has_changed().unwrap(), "waiters were not woken");
    assert_eq!(f.status("svc", run), SysdWaitStatus::TrackedExited { code: 4 });
}

#[test]
fn state_changes_wake_waiters() {
    let mut f = Fixture::new();
    f.daemon
        .handle_create_session("/tmp".into(), Some("true".into()), Some("eph".into())).unwrap();
    let (_, changes) = f.daemon.sysd_wait_status("eph", RunId(Guid::from_u128(1)));
    assert!(!changes.has_changed().unwrap());

    let id = f.id("eph");
    f.daemon.handle_kill_session(id);
    assert!(changes.has_changed().unwrap());
}

// ------------------------------------------------------------------ sysd update

#[test]
fn update_saves_added_windows_to_the_file_the_service_started_from() {
    let mut f = Fixture::new();
    let path = f.yaml("Server", &[("sleep 300", true)]);
    f.daemon.handle_start_sysd_session_from_file(path.clone()).unwrap();
    let id = f.id("server");

    // Add a window and run a monitor in it, like btop next to a game server.
    let info = f.daemon.handle_create_window(id).unwrap();
    let monitor = info.windows_info[1].terminal_id;
    f.daemon.handle_write(monitor, b"sleep 302\r".to_vec()).unwrap();

    let yaml = eventually("the monitor to be saved", || {
        let written = f.daemon.handle_update_sysd_session(id).unwrap();
        assert_eq!(written, path, "must overwrite the file the unit starts from");
        let yaml = SessionFiles::with_dir(f.dir.path().to_path_buf())
            .unwrap()
            .load(&path)
            .unwrap();
        (yaml.windows.get(1).is_some_and(|w| w.command == "sleep 302")).then_some(yaml)
    });

    // The name inside the file is unchanged, so the unit still matches it.
    assert_eq!(yaml.name, "server");
    assert_eq!(yaml.windows.len(), 2);
    assert_eq!(yaml.windows[0].command, "sleep 300");
    assert!(yaml.windows[0].tracked, "the server stays tracked");
    // Quitting the monitor must not restart the service.
    assert!(!yaml.windows[1].tracked, "added windows are not tracked");

    // The running session was left alone.
    assert!(f.session("server").unwrap().sysd_data.unwrap().running);
    assert_eq!(f.session("server").unwrap().windows_info.len(), 2);

    // And the next start brings both windows back.
    f.daemon.handle_start_sysd_session_from_file(path).unwrap();
    assert_eq!(f.session("server").unwrap().windows_info.len(), 2);
}

#[test]
fn update_rejects_ephemeral_and_stopped_sessions_without_writing() {
    let mut f = Fixture::new();
    f.daemon
        .handle_create_session("/tmp".into(), Some("sleep 300".into()), Some("eph".into())).unwrap();
    assert!(f.daemon.handle_update_sysd_session(f.id("eph")).is_err());

    let path = f.yaml("svc", &[("sleep 300", true)]);
    let before = fs::read_to_string(&path).unwrap();
    f.daemon.handle_start_sysd_session_from_file(path.clone()).unwrap();
    let id = f.id("svc");
    f.daemon.handle_stop_sysd_session(id).unwrap();

    let error = f.daemon.handle_update_sysd_session(id).unwrap_err();
    assert!(error.to_string().contains("stopped"), "got: {error}");
    assert_eq!(fs::read_to_string(&path).unwrap(), before);

    assert!(f.daemon.handle_update_sysd_session(SessionId(Guid::from_u128(9))).is_err());
}

// ------------------------------------------------------------------ sysd delete

#[test]
fn delete_target_rejects_ephemeral_sessions() {
    let mut f = Fixture::new();
    f.daemon
        .handle_create_session("/tmp".into(), Some("true".into()), Some("eph".into())).unwrap();
    assert!(f.daemon.sysd_delete_target("eph").is_err());
}

#[test]
fn delete_target_accepts_unknown_names() {
    let f = Fixture::new();
    let name = unique("ghost");
    let (unit, has_unit_file) = f.daemon.sysd_delete_target(&name).unwrap();
    assert_eq!(unit.as_str(), name);
    assert!(!has_unit_file);
}

#[test]
fn deleting_a_session_with_no_files_left_still_removes_it() {
    let mut f = Fixture::new();
    let name = unique("svc");
    f.daemon
        .handle_start_sysd_session_from_file(f.yaml(&name, &[("sleep 300", true)]))
        .unwrap();

    let removed = f.daemon.handle_delete_sysd_session(&UnitName::new(&name).unwrap()).unwrap();
    assert!(removed.is_empty(), "nothing was stored in the data dir: {removed:?}");
    assert!(f.session(&name).is_none());
}

#[test]
fn deleting_removes_stored_yaml_files() {
    let mut f = Fixture::new();
    let name = unique("svc");
    let stored = f.dir.path().join("data").join(format!("{name}.yaml"));
    fs::write(&stored, format!("name: {name}\nwindows: []\n")).unwrap();

    let removed = f.daemon.handle_delete_sysd_session(&UnitName::new(&name).unwrap()).unwrap();
    assert_eq!(removed, vec![stored.clone()]);
    assert!(!stored.exists());
}

#[test]
fn deleting_something_that_does_not_exist_fails() {
    let mut f = Fixture::new();
    let err = f
        .daemon
        .handle_delete_sysd_session(&UnitName::new(&unique("ghost")).unwrap())
        .unwrap_err();
    assert!(err.to_string().contains("No service named"), "got: {err}");
}

#[test]
fn starting_a_service_file_does_not_clobber_an_ephemeral_session() {
    let mut f = Fixture::new();
    f.daemon
        .handle_create_session("/tmp".into(), Some("sleep 300".into()), Some("Clash".into()))
        .unwrap();
    let path = f.yaml("clash", &[("sleep 301", true)]);

    assert!(f.daemon.handle_start_sysd_session_from_file(path).is_err());
    let session = f.session("Clash").expect("ephemeral session survives");
    assert!(session.sysd_data.is_none());
    assert_eq!(session.windows_info.len(), 1);
}

#[test]
fn sessions_from_files_need_a_valid_free_name() {
    let mut f = Fixture::new();
    let bad = f.dir.path().join("bad.yaml");
    fs::write(&bad, "name: ../../escape\nwindows: []\n").unwrap();
    assert!(f.daemon.handle_start_session_from_file(bad.clone()).is_err());
    assert!(f.daemon.handle_start_sysd_session_from_file(bad).is_err());

    let path = f.yaml("twice", &[("sleep 300", false)]);
    f.daemon.handle_start_session_from_file(path.clone()).unwrap();
    assert!(f.daemon.handle_start_session_from_file(path).is_err());
    assert_eq!(f.daemon.handle_list_sessions().len(), 1);
}

#[test]
fn creating_a_service_validates_before_writing() {
    let mut f = Fixture::new();
    for (command, name) in [
        (None, Some("svc")),
        (Some("true"), None),
        (Some("true"), Some("has space")),
        (Some("true"), Some("../escape")),
    ] {
        let result = f.daemon.handle_create_sysd_session(
            "/tmp".into(),
            command.map(String::from),
            name.map(String::from),
        );
        assert!(result.is_err(), "accepted {command:?} {name:?}");
    }
    assert!(fs::read_dir(f.dir.path().join("data")).unwrap().next().is_none(), "wrote a file");
    assert!(f.daemon.handle_list_sessions().is_empty());
}

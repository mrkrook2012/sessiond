//! Ephemeral session lifecycle: run, list, attach, kill.

use crate::harness::*;

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

#[test]
fn list_output_formats() {
    let h = Harness::new();

    h.ok(&["run", "-n", "eph", "sleep", "300"]);
    assert_eq!(h.ok(&["list"]), "Active sessions:\n- eph\n");

    h.ok(&["kill", "eph"]);
    assert_eq!(h.ok(&["list"]), "No active sessions.\n");
    assert_eq!(h.ok(&["list", "--names"]), "");
}

#[test]
fn attaching_to_unknown_session_fails() {
    let h = Harness::new();
    h.ok(&["run", "-n", "real", "sleep", "300"]);

    let err = h.fails(&["attach", "nope"]);
    assert!(err.contains("Session not found"), "got: {err}");
}

#[test]
fn kill_needs_a_name_or_all() {
    let h = Harness::new();
    let err = h.fails(&["kill"]);
    assert!(err.contains("required"), "got: {err}");
}

#[test]
fn kill_all_without_sessions_succeeds() {
    let h = Harness::new();
    h.ok(&["run", "-n", "x", "sleep", "300"]);
    h.ok(&["kill", "x"]);
    h.ok(&["kill", "--all"]);
}

#[test]
fn run_keeps_arguments_with_spaces_intact() {
    let h = Harness::new();
    h.ok(&["run", "-n", "quoted", "sh", "-c", "sleep 300"]);

    let (_, yaml) = h.export("quoted", &h.dir.path().join("export"));
    let session: daemon_test::backend::yaml::models::YamlSession =
        serde_yaml::from_str(&yaml).unwrap();
    // Exported right after `run`: the daemon must not depend on the shell having caught up.
    assert_eq!(session.windows[0].command, "sh -c 'sleep 300'");
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(session.windows[0].working_directory, cwd.to_str().unwrap());
}

#[test]
fn unknown_session_suggests_similar_names() {
    let h = Harness::new();
    h.ok(&["run", "-n", "webserver", "sleep", "300"]);
    h.ok(&["run", "-n", "database", "sleep", "300"]);

    let err = h.fails(&["kill", "websrever"]);
    assert!(err.contains("Session not found: 'websrever'"), "got: {err}");
    assert!(err.contains("Did you mean 'webserver'?"), "got: {err}");

    let err = h.fails(&["attach", "zzz"]);
    assert!(err.contains("list` to see all sessions"), "got: {err}");

    let mut sessions = h.sessions();
    sessions.sort();
    assert_eq!(sessions, vec!["database", "webserver"]);
}

#[test]
fn session_names_are_matched_ignoring_case() {
    let h = Harness::new();
    h.ok(&["run", "-n", "webserver", "sleep", "300"]);

    let out = h.ok(&["kill", "WebServer"]);
    assert!(out.contains("Killed session: webserver"), "got: {out}");
    assert!(h.sessions().is_empty());
}

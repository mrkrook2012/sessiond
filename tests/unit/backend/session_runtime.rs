use std::time::Duration;

use daemon_test::backend::mini_user_manager::MiniUserManager;
use daemon_test::backend::session_manager::{RunId, SessionId, SessionManager, UnitName};
use daemon_test::backend::session_runtime::SessionRuntime;
use daemon_test::backend::terminal_manager::{TerminalId, TerminalManager};
use daemon_test::backend::virtual_terminal::TerminalEvent;
use daemon_test::backend::window_manager::WindowManager;
use daemon_test::backend::yaml::models::YamlWindow;
use daemon_test::ipc::protocol::{SessionInfo, SysdWaitStatus};
use daemon_test::backend::guid::Guid;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::support::{eventually, screen_text};

fn runtime() -> (SessionRuntime, UnboundedReceiver<TerminalEvent>) {
    let (events, receiver) = unbounded_channel();
    let runtime = SessionRuntime::new(
        MiniUserManager::new(),
        TerminalManager::new(events),
        WindowManager::new(),
        SessionManager::new(),
    );
    (runtime, receiver)
}

fn window(command: &str, tracked: bool) -> YamlWindow {
    YamlWindow {
        command: command.to_string(),
        working_directory: "/tmp".to_string(),
        tracked,
    }
}

fn next_event(receiver: &mut UnboundedReceiver<TerminalEvent>) -> TerminalEvent {
    eventually("terminal event", || receiver.try_recv().ok())
}

fn start_sysd(runtime: &mut SessionRuntime, windows: Vec<YamlWindow>) -> RunId {
    runtime
        .start_or_create_sysd_session(UnitName::new("svc").unwrap(), windows)
        .unwrap()
}

fn id_of(runtime: &SessionRuntime, name: &str) -> SessionId {
    runtime.session_id_by_name(name).expect("session exists")
}

fn info(runtime: &SessionRuntime, name: &str) -> SessionInfo {
    runtime.session_info(id_of(runtime, name)).unwrap()
}

fn first_terminal(runtime: &SessionRuntime, name: &str) -> TerminalId {
    info(runtime, name).windows_info[0].terminal_id
}

fn ephemeral(runtime: &mut SessionRuntime, name: &str, command: &str) -> SessionId {
    runtime.create_ephemeral_session("/tmp".into(), Some(command.into()), Some(name.into())).unwrap();
    id_of(runtime, name)
}

// ------------------------------------------------------------------ ephemeral sessions

#[test]
fn ephemeral_session_starts_with_one_window() {
    let (mut runtime, _events) = runtime();
    ephemeral(&mut runtime, "eph", "sleep 300");

    let all = runtime.all_session_info();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "eph");
    assert_eq!(all[0].windows_info.len(), 1);
    assert!(all[0].sysd_data.is_none());
    assert_eq!(runtime.session_name(all[0].id).unwrap(), "eph");
}

#[test]
fn ephemeral_sessions_without_a_name_get_free_default_names() {
    let (mut runtime, _events) = runtime();
    runtime.create_ephemeral_session("/tmp".into(), None, None).unwrap();
    assert_eq!(runtime.all_session_info()[0].name, "session-1");

    runtime.create_ephemeral_session("/tmp".into(), None, None).unwrap();
    let mut names: Vec<_> = runtime.all_session_info().into_iter().map(|s| s.name).collect();
    names.sort();
    assert_eq!(names, vec!["session-1", "session-2"]);
}

#[test]
fn windows_can_be_added_and_the_last_kill_ends_the_session() {
    let (mut runtime, _events) = runtime();
    let id = ephemeral(&mut runtime, "eph", "true");

    let info = runtime.create_window(id).unwrap();
    assert_eq!(info.windows_info.len(), 2);

    let first = info.windows_info[0].window_id;
    let second = info.windows_info[1].window_id;

    runtime.kill_window(id, first).unwrap();
    assert_eq!(runtime.session_info(id).unwrap().windows_info.len(), 1);

    runtime.kill_window(id, second).unwrap();
    assert!(runtime.session_info(id).is_none());
    assert!(runtime.all_session_info().is_empty());
}

#[test]
fn create_window_on_unknown_session_fails() {
    let (mut runtime, _events) = runtime();
    assert!(runtime.create_window(SessionId(Guid::from_u128(5))).is_err());
}

#[test]
fn kill_session_removes_it() {
    let (mut runtime, _events) = runtime();
    let keep = ephemeral(&mut runtime, "keep", "true");
    let gone = ephemeral(&mut runtime, "gone", "true");

    runtime.kill_session(gone);

    assert!(runtime.session_info(gone).is_none());
    assert!(runtime.session_info(keep).is_some());
    assert!(!runtime.session_name_exists("gone"));
}

#[test]
fn open_session_returns_its_info() {
    let (mut runtime, _events) = runtime();
    let id = ephemeral(&mut runtime, "eph", "true");
    let info = runtime.open_session(id).unwrap();
    assert_eq!(info.name, "eph");
    assert_eq!(info.windows_info.len(), 1);
}

#[test]
fn open_session_without_windows_fails() {
    let (mut runtime, _events) = runtime();
    runtime.add_stopped_sysd_session(UnitName::new("svc").unwrap(), false).unwrap();
    assert!(runtime.open_session(id_of(&runtime, "svc")).is_err());
    assert!(runtime.open_session(SessionId(Guid::from_u128(99))).is_err());
}

#[test]
fn screen_size_updates_reach_existing_terminals() {
    let (mut runtime, _events) = runtime();
    ephemeral(&mut runtime, "eph", "true");
    let terminal = first_terminal(&runtime, "eph");

    runtime.update_screen_size(30, 70);
    runtime.write(terminal, b"echo size_$(stty size | tr ' ' x)\r".to_vec()).unwrap();

    eventually("resized terminal", || {
        let data = runtime.get_terminal_data(terminal, 0, 0).unwrap().data;
        screen_text(&data, 30, 70).contains("size_30x70").then_some(())
    });
}

// ------------------------------------------------------------------ yaml

#[test]
fn to_yaml_captures_command_and_directory() {
    let (mut runtime, _events) = runtime();
    let id = ephemeral(&mut runtime, "eph", "sleep 300");

    let yaml = eventually("command to be reported", || {
        let yaml = runtime.to_yaml(id).unwrap();
        (yaml.windows[0].command == "sleep 300").then_some(yaml)
    });
    assert_eq!(yaml.name, "eph");
    assert_eq!(yaml.windows[0].working_directory, "/tmp");
    // A window that runs something is tracked once exported.
    assert!(yaml.windows[0].tracked);
}

#[test]
fn to_yaml_of_a_stopped_or_unknown_session_fails() {
    let (mut runtime, _events) = runtime();
    runtime.add_stopped_sysd_session(UnitName::new("svc").unwrap(), false).unwrap();
    assert!(runtime.to_yaml(id_of(&runtime, "svc")).is_err());
    assert!(runtime.to_yaml(SessionId(Guid::from_u128(99))).is_err());
}

#[test]
fn spawned_windows_run_in_their_working_directory() {
    let (mut runtime, _events) = runtime();
    let windows = runtime.spawn_windows(
        vec![
            YamlWindow { command: "sleep 300".into(), working_directory: "/".into(), tracked: false },
            window("sleep 301", false),
        ],
        false,
    ).unwrap();
    runtime.add_ephemeral_session("multi".into(), windows).unwrap();
    let id = id_of(&runtime, "multi");

    let yaml = eventually("both commands", || {
        let yaml = runtime.to_yaml(id).unwrap();
        yaml.windows.iter().all(|w| w.command.starts_with("sleep")).then_some(yaml)
    });
    let dirs: Vec<_> = yaml.windows.iter().map(|w| w.working_directory.as_str()).collect();
    assert_eq!(dirs, vec!["/", "/tmp"]);
}

// ------------------------------------------------------------------ sysd sessions

#[test]
fn sysd_unit_only_exists_for_sysd_sessions() {
    let (mut runtime, _events) = runtime();
    let eph = ephemeral(&mut runtime, "eph", "true");
    runtime.add_stopped_sysd_session(UnitName::new("Svc").unwrap(), false).unwrap();

    assert!(runtime.sysd_unit(eph).is_err());
    assert!(runtime.sysd_unit(SessionId(Guid::from_u128(99))).is_err());
    assert_eq!(runtime.sysd_unit(id_of(&runtime, "svc")).unwrap().to_string(), "svc");
    assert_eq!(runtime.sysd_units().len(), 1);
}

#[test]
fn enabled_state_shows_in_session_info() {
    let (mut runtime, _events) = runtime();
    let unit = UnitName::new("svc").unwrap();
    runtime.add_stopped_sysd_session(unit.clone(), false).unwrap();

    let data = info(&runtime, "svc").sysd_data.unwrap();
    assert!(!data.enabled && !data.running);

    runtime.set_sysd_enabled(&unit, true);
    assert!(info(&runtime, "svc").sysd_data.unwrap().enabled);

    // Unknown units are ignored.
    runtime.set_sysd_enabled(&UnitName::new("other").unwrap(), true);
}

#[test]
fn running_sysd_session_reports_running() {
    let (mut runtime, _events) = runtime();
    start_sysd(&mut runtime, vec![window("sleep 300", true)]);
    let info = info(&runtime, "svc");
    assert!(info.sysd_data.unwrap().running);
    assert_eq!(info.windows_info.len(), 1);
}

#[test]
fn stopping_tears_down_windows_and_a_second_stop_fails() {
    let (mut runtime, _events) = runtime();
    start_sysd(&mut runtime, vec![window("sleep 300", true), window("sleep 300", false)]);
    let id = id_of(&runtime, "svc");

    runtime.stop_sysd_session(id).unwrap();
    let info = runtime.session_info(id).unwrap();
    assert!(info.windows_info.is_empty());
    assert!(!info.sysd_data.unwrap().running);

    assert!(runtime.stop_sysd_session(id).is_err());
}

#[test]
fn stopping_an_ephemeral_session_fails() {
    let (mut runtime, _events) = runtime();
    let id = ephemeral(&mut runtime, "eph", "true");
    assert!(runtime.stop_sysd_session(id).is_err());
    assert!(runtime.session_info(id).is_some());
}

#[test]
fn unit_deactivation_stops_a_running_session() {
    let (mut runtime, _events) = runtime();
    let run = start_sysd(&mut runtime, vec![window("sleep 300", true)]);
    let unit = UnitName::new("svc").unwrap();

    runtime.handle_unit_deactivated(&unit);
    assert_eq!(runtime.sysd_wait_status("svc", run), SysdWaitStatus::Ended);
    assert!(!info(&runtime, "svc").sysd_data.unwrap().running);

    // Already stopped, or unknown: nothing happens.
    runtime.handle_unit_deactivated(&unit);
    runtime.handle_unit_deactivated(&UnitName::new("other").unwrap());
}

#[test]
fn restarting_a_running_session_replaces_its_windows() {
    let (mut runtime, _events) = runtime();
    let first = start_sysd(&mut runtime, vec![window("sleep 300", true)]);
    // No stop in between (ExecStop did not run).
    let second = start_sysd(&mut runtime, vec![window("sleep 300", true), window("sleep 300", true)]);

    assert_eq!(runtime.sysd_wait_status("svc", first), SysdWaitStatus::Ended);
    assert_eq!(runtime.sysd_wait_status("svc", second), SysdWaitStatus::Running);
    assert_eq!(info(&runtime, "svc").windows_info.len(), 2);
    assert_eq!(runtime.all_session_info().len(), 1);
}

#[test]
fn restarting_gives_a_new_run_and_ends_the_old_one() {
    let (mut runtime, _events) = runtime();
    let first = start_sysd(&mut runtime, vec![window("sleep 300", true)]);

    runtime.stop_sysd_session(id_of(&runtime, "svc")).unwrap();
    assert_eq!(runtime.sysd_wait_status("svc", first), SysdWaitStatus::Ended);

    let second = start_sysd(&mut runtime, vec![window("sleep 300", true)]);
    assert_ne!(first, second);
    assert_eq!(runtime.sysd_wait_status("svc", first), SysdWaitStatus::Ended);
    assert_eq!(runtime.sysd_wait_status("svc", second), SysdWaitStatus::Running);
}

#[test]
fn wait_status_of_unknown_or_ephemeral_session_is_ended() {
    let (mut runtime, _events) = runtime();
    ephemeral(&mut runtime, "eph", "true");
    assert_eq!(runtime.sysd_wait_status("nope", RunId(Guid::from_u128(1))), SysdWaitStatus::Ended);
    assert_eq!(runtime.sysd_wait_status("eph", RunId(Guid::from_u128(1))), SysdWaitStatus::Ended);
}

// ------------------------------------------------------------------ tracked commands

#[test]
fn tracked_command_exit_ends_the_run() {
    let (mut runtime, mut events) = runtime();
    let run = start_sysd(&mut runtime, vec![window("sleep 0.2; (exit 3)", true)]);

    assert_eq!(runtime.sysd_wait_status("svc", run), SysdWaitStatus::Running);

    let TerminalEvent::TrackedCommandExited { terminal_id, code } = next_event(&mut events);
    assert_eq!(code, 3);
    assert!(runtime.handle_tracked_exit(terminal_id, code));

    assert_eq!(
        runtime.sysd_wait_status("svc", run),
        SysdWaitStatus::TrackedExited { code: 3 }
    );
}

#[test]
fn untracked_windows_do_not_report() {
    let (mut runtime, mut events) = runtime();
    let run = start_sysd(
        &mut runtime,
        vec![window("true", false), window("sleep 0.3", true)],
    );

    // Only the tracked window's exit arrives.
    let TerminalEvent::TrackedCommandExited { terminal_id, code } = next_event(&mut events);
    assert_eq!(code, 0);
    assert!(runtime.handle_tracked_exit(terminal_id, code));
    assert_eq!(
        runtime.sysd_wait_status("svc", run),
        SysdWaitStatus::TrackedExited { code: 0 }
    );
}

#[test]
fn manual_stop_does_not_end_the_run() {
    let (mut runtime, mut events) = runtime();
    let run = start_sysd(&mut runtime, vec![window("sleep 300", true)]);
    let terminal_id = first_terminal(&runtime, "svc");

    // Give bash a moment to start `sleep`, then press Ctrl+C like a user would.
    std::thread::sleep(Duration::from_millis(300));
    runtime.write(terminal_id, vec![0x03]).unwrap();

    let TerminalEvent::TrackedCommandExited { terminal_id, code } = next_event(&mut events);
    assert_eq!(code, 130, "Ctrl+C should end the command with SIGINT");
    assert!(!runtime.handle_tracked_exit(terminal_id, code));

    assert_eq!(runtime.sysd_wait_status("svc", run), SysdWaitStatus::Running);
}

#[test]
fn tracked_exit_is_ignored_for_ephemeral_sessions_and_unknown_terminals() {
    let (mut runtime, _events) = runtime();
    // Marked tracked in the YAML, but loaded as an ephemeral session.
    let windows = runtime.spawn_windows(vec![window("sleep 300", true)], false).unwrap();
    runtime.add_ephemeral_session("eph".into(), windows).unwrap();
    let terminal = first_terminal(&runtime, "eph");

    assert!(!runtime.handle_tracked_exit(terminal, 1));
    assert!(!runtime.handle_tracked_exit(TerminalId(Guid::from_u128(999)), 1));
}

#[test]
fn tracked_exit_from_an_untracked_window_is_ignored() {
    let (mut runtime, _events) = runtime();
    let run = start_sysd(&mut runtime, vec![window("sleep 300", false)]);
    let terminal = first_terminal(&runtime, "svc");

    assert!(!runtime.handle_tracked_exit(terminal, 1));
    assert_eq!(runtime.sysd_wait_status("svc", run), SysdWaitStatus::Running);
}

// ------------------------------------------------------------------ names and stale ids

#[test]
fn starting_a_service_never_touches_an_ephemeral_session_with_that_name() {
    let (mut runtime, _events) = runtime();
    let id = ephemeral(&mut runtime, "SVC", "sleep 300");
    let terminal = first_terminal(&runtime, "SVC");

    let err = runtime
        .start_or_create_sysd_session(UnitName::new("svc").unwrap(), vec![window("sleep 301", true)])
        .unwrap_err();
    assert!(err.to_string().contains("ephemeral session 'SVC'"), "got: {err}");

    // The ephemeral session is intact, still usable, and nothing was spawned for the service.
    let all = runtime.all_session_info();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].windows_info.len(), 1);
    assert!(runtime.get_terminal_data(terminal, 0, 0).is_ok());
    assert!(runtime.to_yaml(id).is_ok());
}

#[test]
fn duplicate_names_are_rejected_before_anything_is_spawned() {
    let (mut runtime, _events) = runtime();
    ephemeral(&mut runtime, "eph", "sleep 300");

    assert!(runtime
        .create_ephemeral_session("/tmp".into(), None, Some("EPH".into()))
        .is_err());
    assert!(runtime.add_stopped_sysd_session(UnitName::new("eph").unwrap(), false).is_err());
    assert!(runtime
        .create_ephemeral_session("/tmp".into(), None, Some("../escape".into()))
        .is_err());
    assert_eq!(runtime.all_session_info().len(), 1);
}

#[test]
fn rejected_windows_are_torn_down() {
    let (mut runtime, _events) = runtime();
    ephemeral(&mut runtime, "eph", "sleep 300");

    let windows = runtime.spawn_windows(vec![window("sleep 301", false)], false).unwrap();
    assert!(runtime.add_ephemeral_session("eph".into(), windows.clone()).is_err());
    // The orphaned window was destroyed, not leaked.
    assert!(runtime.change_window(windows[0]).is_err());
}

#[test]
fn stale_ids_are_errors_not_panics() {
    let (mut runtime, _events) = runtime();
    let id = ephemeral(&mut runtime, "eph", "sleep 300");
    let window = info(&runtime, "eph").windows_info[0].clone();
    runtime.kill_session(id);

    assert!(runtime.get_terminal_data(window.terminal_id, 0, 0).is_err());
    assert!(runtime.write(window.terminal_id, b"x".to_vec()).is_err());
    assert!(runtime.change_window(window.window_id).is_err());
    assert!(runtime.kill_window(id, window.window_id).is_err());
    assert!(runtime.create_window(id).is_err());
    assert!(runtime.open_session(id).is_err());
}

#[test]
fn a_window_can_only_be_killed_through_its_own_session() {
    let (mut runtime, _events) = runtime();
    let a = ephemeral(&mut runtime, "a", "sleep 300");
    ephemeral(&mut runtime, "b", "sleep 300");
    let b_window = info(&runtime, "b").windows_info[0].window_id;

    assert!(runtime.kill_window(a, b_window).is_err());
    assert_eq!(info(&runtime, "b").windows_info.len(), 1);
    assert!(runtime.change_window(b_window).is_ok());
}

#[test]
fn creating_a_window_in_a_stopped_service_fails_without_leaking() {
    let (mut runtime, _events) = runtime();
    let id = runtime.add_stopped_sysd_session(UnitName::new("svc").unwrap(), false).unwrap();
    assert!(runtime.create_window(id).is_err());
    assert!(info(&runtime, "svc").windows_info.is_empty());
}

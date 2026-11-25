use daemon_test::backend::terminal_manager::{TerminalId, TerminalManager};
use daemon_test::backend::guid::Guid;
use tokio::sync::mpsc::unbounded_channel;

use crate::support::{eventually, screen_text};

fn manager() -> TerminalManager {
    let (events, _receiver) = unbounded_channel();
    TerminalManager::new(events)
}

fn text(manager: &TerminalManager, id: TerminalId) -> String {
    screen_text(&manager.get_data_from_terminal(id).unwrap().data, 100, 200)
}

#[test]
fn killed_terminal_ids_are_never_reused() {
    let mut manager = manager();
    let a = manager.create_terminal(24, 80).unwrap();
    let b = manager.create_terminal(24, 80).unwrap();
    assert_ne!(a, b);

    manager.kill_terminal(a);
    let c = manager.create_terminal(24, 80).unwrap();
    assert_ne!(c, a);
    assert_ne!(c, b);
    assert_eq!(manager.get_all_data().len(), 2);
}

#[test]
fn commands_and_data_go_to_the_right_terminal() {
    let mut manager = manager();
    let a = manager.create_terminal(24, 80).unwrap();
    let b = manager.create_terminal(24, 80).unwrap();

    manager.execute_command_on_terminal(a, "cd /tmp && echo in_$((1))_a".into()).unwrap();
    manager.execute_command_on_terminal(b, "echo in_$((2))_b".into()).unwrap();

    eventually("a output", || text(&manager, a).contains("in_1_a").then_some(()));
    eventually("b output", || text(&manager, b).contains("in_2_b").then_some(()));
    assert!(!text(&manager, a).contains("in_2_b"));
    assert_eq!(manager.get_data_from_terminal(b).unwrap().terminal_id, b);

    eventually("a command", || {
        manager.get_command_from_terminal(a).unwrap().contains("echo in_").then_some(())
    });
    assert_eq!(manager.get_working_dir_from_terminal(a).as_deref(), Some("/tmp"));
}

#[test]
fn sending_bytes_is_user_input() {
    let mut manager = manager();
    let id = manager.create_terminal(24, 80).unwrap();
    assert_eq!(manager.last_user_input(id), None);
    assert_eq!(manager.last_user_input(TerminalId(Guid::from_u128(99))), None);

    manager.send_bytes_to_terminal(id, b"echo typed_$((3*3))\r").unwrap();
    assert!(manager.last_user_input(id).is_some());
    eventually("typed command", || text(&manager, id).contains("typed_9").then_some(()));
}

#[test]
fn resize_all_terminals_resizes_every_pty() {
    let mut manager = manager();
    let a = manager.create_terminal(24, 80).unwrap();
    let b = manager.create_terminal(24, 80).unwrap();
    manager.resize_all_terminals(30, 70);

    for id in [a, b] {
        manager.execute_command_on_terminal(id, "echo size_$(stty size | tr ' ' x)".into()).unwrap();
        eventually("resize", || text(&manager, id).contains("size_30x70").then_some(()));
    }
}

#[test]
fn unknown_terminals_are_errors_not_panics() {
    let mut manager = manager();
    let unknown = TerminalId(Guid::from_u128(42));
    assert!(manager.get_data_from_terminal(unknown).is_none());
    assert!(manager.get_command_from_terminal(unknown).is_none());
    assert!(manager.send_bytes_to_terminal(unknown, b"x").is_err());
    assert!(manager.launch_on_terminal(unknown, "/tmp", None, false).is_err());
    assert!(!manager.kill_terminal(unknown));
}

#[test]
fn killing_a_terminal_twice_is_harmless() {
    let mut manager = manager();
    let id = manager.create_terminal(24, 80).unwrap();
    assert!(manager.kill_terminal(id));
    assert!(!manager.kill_terminal(id));
}

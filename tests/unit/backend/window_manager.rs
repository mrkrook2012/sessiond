use daemon_test::backend::terminal_manager::TerminalId;
use daemon_test::backend::window_manager::{WindowId, WindowManager};

#[test]
fn windows_map_to_their_terminals_both_ways() {
    let mut manager = WindowManager::new();
    let a = manager.create_window(TerminalId(10));
    let b = manager.create_window(TerminalId(20));

    assert_eq!(manager.get_terminal_id_from_window_id(&a), TerminalId(10));
    assert_eq!(manager.get_terminal_id_from_window_id(&b), TerminalId(20));
    assert_eq!(manager.get_window_id_from_terminal(TerminalId(20)), Some(b));
    assert_eq!(manager.get_window_id_from_terminal(TerminalId(30)), None);
}

#[test]
fn window_info_carries_both_ids() {
    let mut manager = WindowManager::new();
    let id = manager.create_window(TerminalId(5));
    let info = manager.get_window_info_by_id(&id);
    assert_eq!(info.window_id, id);
    assert_eq!(info.terminal_id, TerminalId(5));
}

#[test]
fn tracking_flag_is_stored_per_window() {
    let mut manager = WindowManager::new();
    let plain = manager.create_window(TerminalId(1));
    let tracked = manager.create_window_with_tracking(TerminalId(2), true);

    assert!(!manager.get_window_by_id(&plain).tracked);
    assert!(manager.get_window_by_id(&tracked).tracked);

    manager.get_window_by_id_mut(&plain).tracked = true;
    assert!(manager.get_window_by_id(&plain).tracked);
}

#[test]
fn killed_window_ids_are_reused() {
    let mut manager = WindowManager::new();
    let first = manager.create_window(TerminalId(1));
    manager.create_window(TerminalId(2));
    manager.kill_window(first);

    assert_eq!(manager.get_window_id_from_terminal(TerminalId(1)), None);
    assert_eq!(manager.create_window(TerminalId(3)), first);
    assert_eq!(manager.create_window(TerminalId(4)), WindowId(3));
}

#[test]
#[should_panic]
fn killing_an_unknown_window_panics() {
    WindowManager::new().kill_window(WindowId(1));
}

use daemon_test::backend::terminal_manager::TerminalId;
use daemon_test::backend::window_manager::{WindowId, WindowManager};
use daemon_test::backend::guid::Guid;

#[test]
fn windows_map_to_their_terminals_both_ways() {
    let mut manager = WindowManager::new();
    let a = manager.create_window(TerminalId(Guid::from_u128(10)));
    let b = manager.create_window(TerminalId(Guid::from_u128(20)));

    assert_eq!(manager.get_terminal_id_from_window_id(&a), Some(TerminalId(Guid::from_u128(10))));
    assert_eq!(manager.get_terminal_id_from_window_id(&b), Some(TerminalId(Guid::from_u128(20))));
    assert_eq!(manager.get_window_id_from_terminal(TerminalId(Guid::from_u128(20))), Some(b));
    assert_eq!(manager.get_window_id_from_terminal(TerminalId(Guid::from_u128(30))), None);
}

#[test]
fn window_info_carries_both_ids() {
    let mut manager = WindowManager::new();
    let id = manager.create_window(TerminalId(Guid::from_u128(5)));
    let info = manager.get_window_info_by_id(&id).unwrap();
    assert_eq!(info.window_id, id);
    assert_eq!(info.terminal_id, TerminalId(Guid::from_u128(5)));
}

#[test]
fn tracking_flag_is_stored_per_window() {
    let mut manager = WindowManager::new();
    let plain = manager.create_window(TerminalId(Guid::from_u128(1)));
    let tracked = manager.create_window_with_tracking(TerminalId(Guid::from_u128(2)), true);

    assert!(!manager.get_window_by_id(&plain).unwrap().tracked);
    assert!(manager.get_window_by_id(&tracked).unwrap().tracked);

    manager.get_window_by_id_mut(&plain).unwrap().tracked = true;
    assert!(manager.get_window_by_id(&plain).unwrap().tracked);
}

#[test]
fn killed_window_ids_are_never_reused() {
    let mut manager = WindowManager::new();
    let first = manager.create_window(TerminalId(Guid::from_u128(1)));
    let second = manager.create_window(TerminalId(Guid::from_u128(2)));
    manager.kill_window(first);

    assert_eq!(manager.get_window_id_from_terminal(TerminalId(Guid::from_u128(1))), None);
    let third = manager.create_window(TerminalId(Guid::from_u128(3)));
    assert_ne!(third, first);
    assert_ne!(third, second);
}

#[test]
fn unknown_windows_are_none_not_panics() {
    let mut manager = WindowManager::new();
    let unknown = WindowId(Guid::from_u128(1));
    assert!(manager.kill_window(unknown).is_none());
    assert!(manager.get_window_by_id(&unknown).is_none());
    assert!(manager.get_terminal_id_from_window_id(&unknown).is_none());
    assert!(manager.get_window_info_by_id(&unknown).is_none());
}

use daemon_test::backend::terminal::TerminalId;
use daemon_test::backend::window::{WindowId, Windows};
use daemon_test::backend::guid::Guid;

#[test]
fn windows_map_to_their_terminals_both_ways() {
    let mut windows = Windows::new();
    let a = windows.create(TerminalId(Guid::from_u128(10)));
    let b = windows.create(TerminalId(Guid::from_u128(20)));

    assert_eq!(windows.terminal_of(&a), Some(TerminalId(Guid::from_u128(10))));
    assert_eq!(windows.terminal_of(&b), Some(TerminalId(Guid::from_u128(20))));
    assert_eq!(windows.id_by_terminal(TerminalId(Guid::from_u128(20))), Some(b));
    assert_eq!(windows.id_by_terminal(TerminalId(Guid::from_u128(30))), None);
}

#[test]
fn window_info_carries_both_ids() {
    let mut windows = Windows::new();
    let id = windows.create(TerminalId(Guid::from_u128(5)));
    let info = windows.info(&id).unwrap();
    assert_eq!(info.window_id, id);
    assert_eq!(info.terminal_id, TerminalId(Guid::from_u128(5)));
}

#[test]
fn tracking_flag_is_stored_per_window() {
    let mut windows = Windows::new();
    let plain = windows.create(TerminalId(Guid::from_u128(1)));
    let tracked = windows.create_with_tracking(TerminalId(Guid::from_u128(2)), true);

    assert!(!windows.get(&plain).unwrap().tracked);
    assert!(windows.get(&tracked).unwrap().tracked);

    windows.get_mut(&plain).unwrap().tracked = true;
    assert!(windows.get(&plain).unwrap().tracked);
}

#[test]
fn killed_window_ids_are_never_reused() {
    let mut windows = Windows::new();
    let first = windows.create(TerminalId(Guid::from_u128(1)));
    let second = windows.create(TerminalId(Guid::from_u128(2)));
    windows.remove(first);

    assert_eq!(windows.id_by_terminal(TerminalId(Guid::from_u128(1))), None);
    let third = windows.create(TerminalId(Guid::from_u128(3)));
    assert_ne!(third, first);
    assert_ne!(third, second);
}

#[test]
fn unknown_windows_are_none_not_panics() {
    let mut windows = Windows::new();
    let unknown = WindowId(Guid::from_u128(1));
    assert!(windows.remove(unknown).is_none());
    assert!(windows.get(&unknown).is_none());
    assert!(windows.terminal_of(&unknown).is_none());
    assert!(windows.info(&unknown).is_none());
}

use daemon_test::backend::mini_user_manager::MiniUserManager;
use daemon_test::backend::session_manager::SessionId;
use daemon_test::backend::terminal_manager::TerminalId;

#[test]
fn defaults() {
    let manager = MiniUserManager::new();
    assert_eq!(manager.get_dimensions(), (100, 100));
    assert_eq!(manager.get_current_id(), SessionId(1));
    assert!(manager.get_all_terminals().is_empty());
}

#[test]
fn dimensions_are_set_by_add_user_and_update() {
    let mut manager = MiniUserManager::new();
    manager.add_user(24, 80);
    assert_eq!(manager.get_dimensions(), (24, 80));
    manager.update_dimensions(50, 120);
    assert_eq!(manager.get_dimensions(), (50, 120));
}

#[test]
fn visible_terminals_can_be_added_removed_and_cleared() {
    let mut manager = MiniUserManager::new();
    manager.add_terminal(TerminalId(1));
    manager.add_terminal(TerminalId(2));
    manager.add_terminal(TerminalId(3));

    manager.remove_terminal(TerminalId(2));
    // Removing one that is not visible is a no-op.
    manager.remove_terminal(TerminalId(9));
    assert_eq!(manager.get_all_terminals(), vec![TerminalId(1), TerminalId(3)]);

    manager.clear_all();
    assert!(manager.get_all_terminals().is_empty());
}

#[test]
fn current_session_can_be_changed() {
    let mut manager = MiniUserManager::new();
    manager.set_current_id(SessionId(4));
    assert_eq!(manager.get_current_id(), SessionId(4));
}

//! `UserManager` is the multi-user replacement for `MiniUserManager`; not wired in yet.

use daemon_test::backend::terminal_manager::TerminalId;
use daemon_test::backend::user_manager::{UserId, UserManager, UserManagerError};
use daemon_test::backend::guid::Guid;

fn user(n: u64) -> UserId {
    UserId(n)
}

#[test]
fn users_can_be_added_once_and_removed() {
    let mut manager = UserManager::new();
    manager.add_user(user(1)).unwrap();
    assert!(manager.has_user(user(1)));
    assert!(matches!(
        manager.add_user(user(1)),
        Err(UserManagerError::UserAlreadyExists(_))
    ));

    manager.remove_user(user(1)).unwrap();
    assert!(!manager.has_user(user(1)));
    assert!(matches!(
        manager.remove_user(user(1)),
        Err(UserManagerError::UserNotFound(_))
    ));
}

#[test]
fn users_lists_in_insertion_order() {
    let mut manager = UserManager::new();
    manager.add_user(user(2)).unwrap();
    manager.add_user(user(1)).unwrap();
    assert_eq!(manager.users(), &[user(2), user(1)]);
}

#[test]
fn every_per_user_operation_requires_a_known_user() {
    fn missing<T>(result: Result<T, UserManagerError>) -> bool {
        matches!(result, Err(UserManagerError::UserNotFound(_)))
    }
    let mut manager = UserManager::new();

    assert!(missing(manager.add_visible_terminal(user(1), TerminalId(Guid::from_u128(1)))));
    assert!(missing(manager.remove_visible_terminal(user(1), TerminalId(Guid::from_u128(1)))));
    assert!(missing(manager.get_visible_terminals(user(1))));
    assert!(missing(manager.set_screen_size(user(1), 80, 24)));
    assert!(missing(manager.get_screen_size(user(1))));
    assert!(missing(manager.terminal_visible_to_user(user(1), TerminalId(Guid::from_u128(1)))));
}

#[test]
fn visible_terminals_per_user() {
    let mut manager = UserManager::new();
    manager.add_user(user(1)).unwrap();
    manager.add_user(user(2)).unwrap();

    assert!(manager.get_visible_terminals(user(1)).unwrap().is_empty());

    manager.add_visible_terminal(user(1), TerminalId(Guid::from_u128(10))).unwrap();
    manager.add_visible_terminal(user(1), TerminalId(Guid::from_u128(11))).unwrap();
    assert!(matches!(
        manager.add_visible_terminal(user(1), TerminalId(Guid::from_u128(10))),
        Err(UserManagerError::TerminalAlreadyVisible(_))
    ));

    assert_eq!(
        manager.get_visible_terminals(user(1)).unwrap(),
        &[TerminalId(Guid::from_u128(10)), TerminalId(Guid::from_u128(11))]
    );
    assert!(manager.terminal_visible_to_user(user(1), TerminalId(Guid::from_u128(10))).unwrap());
    assert!(!manager.terminal_visible_to_user(user(2), TerminalId(Guid::from_u128(10))).unwrap());

    manager.remove_visible_terminal(user(1), TerminalId(Guid::from_u128(10))).unwrap();
    assert!(matches!(
        manager.remove_visible_terminal(user(1), TerminalId(Guid::from_u128(10))),
        Err(UserManagerError::TerminalNotVisible(_))
    ));
    assert_eq!(manager.get_visible_terminals(user(1)).unwrap(), &[TerminalId(Guid::from_u128(11))]);
}

#[test]
fn removing_a_terminal_from_a_user_without_any_is_an_error() {
    let mut manager = UserManager::new();
    manager.add_user(user(1)).unwrap();
    assert!(matches!(
        manager.remove_visible_terminal(user(1), TerminalId(Guid::from_u128(1))),
        Err(UserManagerError::TerminalNotVisible(_))
    ));
}

#[test]
fn screen_size_is_unset_until_set() {
    let mut manager = UserManager::new();
    manager.add_user(user(1)).unwrap();
    assert_eq!(manager.get_screen_size(user(1)).unwrap(), None);
    manager.set_screen_size(user(1), 120, 40).unwrap();
    assert_eq!(manager.get_screen_size(user(1)).unwrap(), Some((120, 40)));
}

#[test]
fn removing_a_user_forgets_their_state() {
    let mut manager = UserManager::new();
    manager.add_user(user(1)).unwrap();
    manager.add_visible_terminal(user(1), TerminalId(Guid::from_u128(1))).unwrap();
    manager.set_screen_size(user(1), 80, 24).unwrap();

    manager.remove_user(user(1)).unwrap();
    manager.add_user(user(1)).unwrap();

    assert!(manager.get_visible_terminals(user(1)).unwrap().is_empty());
    assert_eq!(manager.get_screen_size(user(1)).unwrap(), None);
}

#[test]
fn clear_terminals_clears_every_user() {
    let mut manager = UserManager::new();
    manager.add_user(user(1)).unwrap();
    manager.add_user(user(2)).unwrap();
    manager.add_visible_terminal(user(1), TerminalId(Guid::from_u128(1))).unwrap();
    manager.add_visible_terminal(user(2), TerminalId(Guid::from_u128(2))).unwrap();

    manager.clear_terminals();

    assert!(manager.get_visible_terminals(user(1)).unwrap().is_empty());
    assert!(manager.get_visible_terminals(user(2)).unwrap().is_empty());
}

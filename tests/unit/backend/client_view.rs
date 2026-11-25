use daemon_test::backend::client_view::ClientView;
use daemon_test::backend::session::SessionId;
use daemon_test::backend::terminal::TerminalId;
use daemon_test::backend::guid::Guid;

#[test]
fn defaults() {
    let view = ClientView::new();
    assert_eq!(view.dimensions(), (100, 100));
    assert_eq!(view.current_session(), SessionId(Guid::NIL));
    assert!(view.visible_terminals().is_empty());
}

#[test]
fn dimensions_can_be_set() {
    let mut view = ClientView::new();
    view.set_dimensions(24, 80);
    assert_eq!(view.dimensions(), (24, 80));
    view.set_dimensions(50, 120);
    assert_eq!(view.dimensions(), (50, 120));
}

#[test]
fn visible_terminals_can_be_shown_hidden_and_cleared() {
    let mut view = ClientView::new();
    view.show_terminal(TerminalId(Guid::from_u128(1)));
    view.show_terminal(TerminalId(Guid::from_u128(2)));
    view.show_terminal(TerminalId(Guid::from_u128(3)));

    view.hide_terminal(TerminalId(Guid::from_u128(2)));
    // Hiding one that is not visible is a no-op.
    view.hide_terminal(TerminalId(Guid::from_u128(9)));
    assert_eq!(view.visible_terminals(), vec![TerminalId(Guid::from_u128(1)), TerminalId(Guid::from_u128(3))]);

    view.hide_all();
    assert!(view.visible_terminals().is_empty());
}

#[test]
fn current_session_can_be_changed() {
    let mut view = ClientView::new();
    view.set_current_session(SessionId(Guid::from_u128(4)));
    assert_eq!(view.current_session(), SessionId(Guid::from_u128(4)));
}

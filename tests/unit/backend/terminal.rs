use daemon_test::backend::terminal::{TerminalId, Terminals};
use daemon_test::backend::guid::Guid;
use tokio::sync::mpsc::unbounded_channel;

use crate::support::{eventually, screen_text};

fn terminals() -> Terminals {
    let (events, _receiver) = unbounded_channel();
    Terminals::new(events)
}

fn text(terminals: &Terminals, id: TerminalId) -> String {
    screen_text(&terminals.screen(id).unwrap().data, 100, 200)
}

#[test]
fn killed_terminal_ids_are_never_reused() {
    let mut terminals = terminals();
    let a = terminals.create(24, 80).unwrap();
    let b = terminals.create(24, 80).unwrap();
    assert_ne!(a, b);

    terminals.kill(a);
    let c = terminals.create(24, 80).unwrap();
    assert_ne!(c, a);
    assert_ne!(c, b);
    assert_eq!(terminals.all_screens().len(), 2);
}

#[test]
fn commands_and_data_go_to_the_right_terminal() {
    let mut terminals = terminals();
    let a = terminals.create(24, 80).unwrap();
    let b = terminals.create(24, 80).unwrap();

    terminals.execute(a, "cd /tmp && echo in_$((1))_a".into()).unwrap();
    terminals.execute(b, "echo in_$((2))_b".into()).unwrap();

    eventually("a output", || text(&terminals, a).contains("in_1_a").then_some(()));
    eventually("b output", || text(&terminals, b).contains("in_2_b").then_some(()));
    assert!(!text(&terminals, a).contains("in_2_b"));
    assert_eq!(terminals.screen(b).unwrap().terminal_id, b);

    eventually("a command", || {
        terminals.current_command(a).unwrap().contains("echo in_").then_some(())
    });
    assert_eq!(terminals.working_dir(a).as_deref(), Some("/tmp"));
}

#[test]
fn sending_bytes_is_user_input() {
    let mut terminals = terminals();
    let id = terminals.create(24, 80).unwrap();
    assert_eq!(terminals.last_user_input(id), None);
    assert_eq!(terminals.last_user_input(TerminalId(Guid::from_u128(99))), None);

    terminals.send_input(id, b"echo typed_$((3*3))\r").unwrap();
    assert!(terminals.last_user_input(id).is_some());
    eventually("typed command", || text(&terminals, id).contains("typed_9").then_some(()));
}

#[test]
fn resize_all_terminals_resizes_every_pty() {
    let mut terminals = terminals();
    let a = terminals.create(24, 80).unwrap();
    let b = terminals.create(24, 80).unwrap();
    terminals.resize_all(30, 70);

    for id in [a, b] {
        terminals.execute(id, "echo size_$(stty size | tr ' ' x)".into()).unwrap();
        eventually("resize", || text(&terminals, id).contains("size_30x70").then_some(()));
    }
}

#[test]
fn unknown_terminals_are_errors_not_panics() {
    let mut terminals = terminals();
    let unknown = TerminalId(Guid::from_u128(42));
    assert!(terminals.screen(unknown).is_none());
    assert!(terminals.current_command(unknown).is_none());
    assert!(terminals.send_input(unknown, b"x").is_err());
    assert!(terminals.launch(unknown, "/tmp", None, false).is_err());
    assert!(!terminals.kill(unknown));
}

#[test]
fn killing_a_terminal_twice_is_harmless() {
    let mut terminals = terminals();
    let id = terminals.create(24, 80).unwrap();
    assert!(terminals.kill(id));
    assert!(!terminals.kill(id));
}

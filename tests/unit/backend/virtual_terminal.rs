use std::process::Command;
use std::time::{Duration, Instant};

use daemon_test::backend::terminal::virtual_terminal::{TERMINATE_GRACE, TerminalEvent, VirtualTerminal};
use daemon_test::ids::{Guid, TerminalId};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::support::{eventually, screen_text};

fn terminal(rows: u16, cols: u16) -> (VirtualTerminal, UnboundedReceiver<TerminalEvent>) {
    let (events, receiver) = unbounded_channel();
    (VirtualTerminal::new(TerminalId(Guid::from_u128(1)), rows, cols, events).unwrap(), receiver)
}

fn text(terminal: &VirtualTerminal) -> String {
    screen_text(&terminal.get_screen(), 100, 200)
}

fn process_running(pattern: &str) -> bool {
    Command::new("pgrep")
        .args(["-f", pattern])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Regression: the shell used to survive while a command ran in the foreground,
/// because only the foreground group was signalled. Waiting on it then hung.
#[test]
fn dropping_a_terminal_kills_the_shell_and_its_command() {
    let (mut terminal, _events) = terminal(24, 80);

    // Odd duration so pgrep cannot match anything else on the machine.
    let marker = format!("sleep 31{}", std::process::id());
    terminal.execute_command(marker.clone());

    eventually("command to start", || process_running(&marker).then_some(()));

    let started = Instant::now();
    drop(terminal);
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "drop must not block the daemon"
    );

    let deadline = Instant::now() + TERMINATE_GRACE + Duration::from_secs(2);
    while process_running(&marker) {
        assert!(Instant::now() < deadline, "command survived terminal drop");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn command_output_appears_on_the_screen() {
    let (mut terminal, _events) = terminal(24, 80);
    // The arithmetic proves the command ran, rather than just being echoed.
    terminal.execute_command("echo marker_$((6*7))".into());
    eventually("output on screen", || text(&terminal).contains("marker_42").then_some(()));
}

#[test]
fn running_command_and_its_directory_are_reported() {
    let (mut terminal, _events) = terminal(24, 80);
    assert_eq!(terminal.get_current_command(), "");

    terminal.execute_command("cd /tmp".into());
    terminal.execute_command("sleep 300".into());

    eventually("command to be reported", || {
        (terminal.get_current_command() == "sleep 300").then_some(())
    });
    assert_eq!(terminal.get_current_pwd(), "/tmp");
}

/// Regression: right after a launch, the command and directory used to be whatever the
/// shell last reported, because it had not read the launch yet. Exporting a session just
/// after `run` then saved `""`, `clear` or the `cd` instead of the command.
#[test]
fn launch_is_reported_before_the_shell_gets_to_it() {
    let (mut terminal, _events) = terminal(24, 80);
    // Keep the shell busy so it cannot read the launch for a while.
    terminal.execute_command("sleep 1".into());
    eventually("the shell to be busy", || {
        (terminal.get_current_command() == "sleep 1").then_some(())
    });

    terminal.launch("/tmp", Some("sleep 300".into()), false);
    assert_eq!(terminal.get_current_command(), "sleep 300");
    assert_eq!(terminal.get_current_pwd(), "/tmp");

    // Still true while the shell is busy, and once it has started the command.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(terminal.get_current_command(), "sleep 300");
    eventually("the launched command to start", || {
        text(&terminal).contains("sleep 300").then_some(())
    });
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(terminal.get_current_command(), "sleep 300");
    assert_eq!(terminal.get_current_pwd(), "/tmp");
}

#[test]
fn after_a_launch_the_shell_reports_take_over() {
    let (mut terminal, _events) = terminal(24, 80);
    terminal.launch("/tmp", Some("true".into()), false);
    eventually("the launched command to finish", || {
        text(&terminal).contains("true").then_some(())
    });

    terminal.execute_command("cd / && sleep 301".into());
    eventually("the next command to be reported", || {
        (terminal.get_current_command() == "sleep 301").then_some(())
    });
    assert_eq!(terminal.get_current_pwd(), "/");
}

#[test]
fn launch_without_a_command_reports_the_directory() {
    let (mut terminal, _events) = terminal(24, 80);
    terminal.launch("/tmp", None, false);
    assert_eq!(terminal.get_current_command(), "");
    assert_eq!(terminal.get_current_pwd(), "/tmp");

    terminal.execute_command("sleep 302".into());
    eventually("the typed command to be reported", || {
        (terminal.get_current_command() == "sleep 302").then_some(())
    });
    assert_eq!(terminal.get_current_pwd(), "/tmp");
}

#[test]
fn tracked_launch_reports_its_exit_code() {
    let (mut terminal, mut events) = terminal(24, 80);
    terminal.launch("/tmp", Some("(exit 7)".into()), true);

    let TerminalEvent::TrackedCommandExited { code, .. } =
        eventually("tracked exit", || events.try_recv().ok());
    assert_eq!(code, 7);
}

#[test]
fn tracked_command_reports_its_exit_code() {
    let (mut terminal, mut events) = terminal(24, 80);
    terminal.run_tracked("(exit 5)".into());

    let TerminalEvent::TrackedCommandExited { terminal_id, code } =
        eventually("tracked exit", || events.try_recv().ok());
    assert_eq!(terminal_id, TerminalId(Guid::from_u128(1)));
    assert_eq!(code, 5);
}

#[test]
fn untracked_commands_report_nothing_and_tracking_is_one_shot() {
    let (mut terminal, mut events) = terminal(24, 80);
    terminal.execute_command("(exit 1)".into());
    terminal.run_tracked("(exit 2)".into());
    terminal.execute_command("(exit 3)".into());
    terminal.execute_command("echo done_$((1+1))".into());

    let TerminalEvent::TrackedCommandExited { code, .. } =
        eventually("tracked exit", || events.try_recv().ok());
    assert_eq!(code, 2);

    // Once the last command has run, nothing else was reported.
    eventually("last command", || text(&terminal).contains("done_2").then_some(()));
    std::thread::sleep(Duration::from_millis(100));
    assert!(events.try_recv().is_err(), "an untracked command was reported");
}

#[test]
fn only_user_input_counts_as_user_input() {
    let (mut terminal, _events) = terminal(24, 80);
    terminal.execute_command("true".into());
    assert_eq!(terminal.last_user_input(), None);

    let before = Instant::now();
    terminal.process_input(b"x");
    assert!(terminal.last_user_input().is_some_and(|at| at >= before));
}

#[test]
fn ratio_and_global_size_set_the_pty_size() {
    let (mut terminal, _events) = terminal(24, 80);
    assert_eq!(terminal.get_ratio(), (1.0, 1.0));

    terminal.set_ratio(0.5, 0.5);
    assert_eq!(terminal.get_ratio(), (0.5, 0.5));
    terminal.execute_command("echo size_$(stty size | tr ' ' x)".into());
    eventually("halved size", || text(&terminal).contains("size_12x40").then_some(()));

    terminal.set_global_size(40, 100);
    terminal.recalculate_size();
    terminal.execute_command("clear; echo size_$(stty size | tr ' ' x)".into());
    eventually("resized", || text(&terminal).contains("size_20x50").then_some(()));
}

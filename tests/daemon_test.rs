use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;

fn run_cli(args: Vec<&str>) -> std::process::Output {
    Command::new("./target/debug/daemon_test")
        .args(args)
        .output()
        .expect("Failed to execute command")
}

fn check_daemon_err(step: &str) {
    let err_content = fs::read_to_string("daemon.err").unwrap_or_default();
    assert!(
        !err_content.contains("Panic"),
        "daemon.err is not empty after step '{}': {}",
        step,
        err_content
    );
}

#[test]
fn test_daemon_start_kill() {
    // 1. Cleanup any existing daemon processes
    let _ = fs::remove_file("/tmp/daemon_test.sock");

    // 2. Kill any running daemon process if it exists
    let kill_output = run_cli(vec!["kill-daemon"]);
    println!("Kill daemon output: {:?}", kill_output);

    // Give the system time to clean up
    thread::sleep(Duration::from_millis(100));

    // 3. Start a new daemon with the start-daemon subcommand
    let start_output = run_cli(vec!["start-daemon"]);
    assert!(
        start_output.status.success(),
        "Start daemon command failed: {:?}",
        String::from_utf8_lossy(&start_output.stderr)
    );
    check_daemon_err("after start daemon");

    // 4. Verify daemon is running by checking for socket file
    let has_socket = fs::metadata("/tmp/daemon_test.sock").is_ok();
    assert!(
        has_socket,
        "Daemon socket file not found after starting daemon"
    );

    // 5. Kill the daemon process
    let kill_output = run_cli(vec!["kill-daemon"]);
    assert!(
        kill_output.status.success(),
        "Kill daemon command failed: {:?}",
        String::from_utf8_lossy(&kill_output.stderr)
    );
    check_daemon_err("after kill daemon");

    // 6. Verify the daemon is no longer running by checking socket file is gone
    let has_socket_after = fs::metadata("/tmp/daemon_test.sock").is_ok();
    assert!(
        !has_socket_after,
        "Daemon socket file still exists after killing daemon"
    );

    println!("Daemon start and kill test passed successfully!");
}

use std::fs;
use std::process::Command;

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
fn test_cli_lifecycle() {
    // 1. Cleanup
    let _ = fs::remove_file("/tmp/daemon_test.sock");
    let _ = fs::remove_file("/tmp/penar2.sock");

    // 2. Run a session
    let output_run = run_cli(vec!["run", "sleep 10"]);
    assert!(
        output_run.status.success(),
        "Run command failed: {:?}",
        String::from_utf8_lossy(&output_run.stderr)
    );
    check_daemon_err("after run");

    let stdout_run = String::from_utf8_lossy(&output_run.stdout);
    // Output format: "Starting session: session-Henk (running: sleep 10)"
    let name = stdout_run
        .lines()
        .find(|line| line.contains("Starting session:"))
        .and_then(|line| {
            line.split(':')
                .nth(1)?
                .split('(')
                .next()?
                .trim()
                .to_string()
                .into()
        })
        .expect("Could not find session name in run output");

    println!("Found session name: {}", name);

    // 3. List sessions
    let output_list = run_cli(vec!["list"]);
    assert!(
        output_list.status.success(),
        "List command failed: {:?}",
        String::from_utf8_lossy(&output_list.stderr)
    );
    check_daemon_err("after list");

    let stdout_list = String::from_utf8_lossy(&output_list.stdout);
    assert!(
        stdout_list.contains(&name),
        "Session {} not found in list output",
        name
    );

    // 4. Kill the session
    let output_kill = run_cli(vec!["kill", &name]);
    assert!(
        output_kill.status.success(),
        "Kill command failed: {:?}",
        String::from_utf8_lossy(&output_kill.stderr)
    );
    check_daemon_err("after kill");

    // 5. List sessions again to verify it's gone
    let output_list_after = run_cli(vec!["list"]);
    assert!(
        output_list_after.status.success(),
        "List command (after kill) failed: {:?}",
        String::from_utf8_lossy(&output_list_after.stderr)
    );
    check_daemon_err("after list again");

    let stdout_list_after = String::from_utf8_lossy(&output_list_after.stdout);
    assert!(
        !stdout_list_after.contains(&name),
        "Session {} still found in list output after kill",
        name
    );

    // Cleanup sockets again just in case
    let _ = fs::remove_file("/tmp/daemon_test.sock");
    let _ = fs::remove_file("/tmp/penar2.sock");
}

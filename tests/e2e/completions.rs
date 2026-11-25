//! fish completions, static and dynamic.

use crate::harness::*;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Runs fish's completion for `line` (as if TAB was pressed) with our script loaded.
fn fish_complete(h: &Harness, script: &Path, line: &str) -> Vec<String> {
    let bin_dir = Path::new(env!("CARGO_BIN_EXE_sessiond")).parent().unwrap();
    let path = format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap());

    let out = Command::new("fish")
        .args(["--no-config", "-c"])
        .arg(format!(
            "source {}; complete -C {}",
            script.display(),
            shell_quote(line)
        ))
        .env("PATH", path)
        .env("SESSIOND_SOCKET", h.socket())
        .env("SESSIOND_DATA_DIR", h.data_dir())
        .output()
        .expect("failed to run fish");

    assert!(
        out.status.success(),
        "fish failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Each line is `candidate<TAB>description`.
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| line.split('\t').next().unwrap().to_owned())
        .collect()
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "\\'"))
}

#[test]
fn fish_completions() {
    if Command::new("fish").arg("--version").output().is_err() {
        eprintln!("skipping: fish not installed");
        return;
    }
    let h = Harness::with_isolated_sysd();

    let script = h.dir.path().join("sessiond.fish");
    fs::write(&script, h.ok(&["completions", "fish"])).unwrap();

    // Static part: subcommands, without the hidden internal ones.
    let top = fish_complete(&h, &script, "sessiond ");
    assert!(top.contains(&"service".to_string()), "got: {top:?}");
    assert!(!top.iter().any(|c| c.starts_with("__internal")), "got: {top:?}");

    // No daemon: no names, and completing must not start one.
    assert!(fish_complete(&h, &script, "sessiond attach ").is_empty());
    h.fails(&["daemon", "status"]);

    // One ephemeral session and one (stopped) service.
    fs::write(
        h.data_dir().join("svc.yaml"),
        "name: svc\nwindows:\n- command: sleep 1\n  working_directory: /tmp\n",
    )
    .unwrap();
    fs::write(h.sysd_dir().join("svc.service"), "[Unit]\n").unwrap();
    h.ok(&["run", "-n", "eph", "sleep", "300"]);

    let mut attach = fish_complete(&h, &script, "sessiond attach ");
    attach.sort();
    assert_eq!(attach, vec!["eph", "svc"]);

    assert_eq!(
        fish_complete(&h, &script, "sessiond service start "),
        vec!["svc"]
    );

    // Only one name: nothing (not even files) after it.
    assert!(fish_complete(&h, &script, "sessiond attach eph ").is_empty());

    // `run` completes the wrapped command.
    assert!(fish_complete(&h, &script, "sessiond run -n x ech").contains(&"echo".to_string()));
}

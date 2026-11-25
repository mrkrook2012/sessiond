//! Exporting sessions to YAML and importing them again.

use std::fs;

use crate::harness::*;

#[test]
fn export_and_import_session_roundtrip() {
    let h = Harness::new();

    let export_dir_1 = h.dir.path().join("export1");
    let export_dir_2 = h.dir.path().join("export2");

    // 1. Start a named ephemeral session.
    h.ok(&["run", "--name", "roundtrip", "sleep", "300"]);

    assert_eq!(h.sessions(), vec!["roundtrip"]);

    // 2. Export it to disk.
    let (yaml_path, yaml_1) = h.export("roundtrip", &export_dir_1);

    assert!(
        yaml_1.contains("roundtrip"),
        "name missing from yaml:\n{yaml_1}"
    );

    assert!(
        yaml_1.contains("sleep 300"),
        "command missing from yaml:\n{yaml_1}"
    );

    assert!(
        yaml_1.contains("tracked: true"),
        "exported windows should be tracked by default:\n{yaml_1}"
    );

    assert!(yaml_path.is_file());

    // 3. Kill the original session.
    h.ok(&["kill", "roundtrip"]);

    assert!(h.sessions().is_empty());

    // 4. Import the exported YAML.
    h.ok(&["import", yaml_path.to_str().unwrap()]);

    assert_eq!(h.sessions(), vec!["roundtrip"]);

    // 5. Export again and verify the configuration is identical.
    let (_, yaml_2) = h.export("roundtrip", &export_dir_2);

    assert_eq!(yaml_1, yaml_2, "imported session differs from the original");

    // 6. Clean up.
    h.ok(&["kill", "roundtrip"]);

    assert!(h.sessions().is_empty());

    h.assert_daemon_healthy("end of roundtrip test");
}
#[test]
fn importing_nonexistent_file_fails() {
    let h = Harness::new();

    let missing = h.dir.path().join("does-not-exist.yaml");

    let err = h.fails(&["import", missing.to_str().unwrap()]);

    assert!(err.contains("Failed to get absolute path"), "got: {err}");
}

#[test]
fn export_unknown_session_fails() {
    let h = Harness::new();
    h.ok(&["run", "-n", "real", "sleep", "300"]);

    let export_dir = h.dir.path().join("export");
    fs::create_dir(&export_dir).unwrap();

    let err = h.fails(&["export", "does-not-exist", "--dir", export_dir.to_str().unwrap()]);
    assert!(err.contains("Session not found"), "got: {err}");
}

#[test]
fn export_to_a_bad_directory_fails() {
    let h = Harness::new();
    h.ok(&["run", "-n", "s", "sleep", "300"]);

    let missing = h.dir.path().join("missing");
    let err = h.fails(&["export", "s", "--dir", missing.to_str().unwrap()]);
    assert!(err.contains("Invalid export directory"), "got: {err}");

    let file = h.dir.path().join("a-file");
    fs::write(&file, "").unwrap();
    let err = h.fails(&["export", "s", "--dir", file.to_str().unwrap()]);
    assert!(err.contains("not a directory"), "got: {err}");
}

#[test]
fn importing_a_directory_fails() {
    let h = Harness::new();
    let err = h.fails(&["import", h.dir.path().to_str().unwrap()]);
    assert!(err.contains("not a file"), "got: {err}");
}

#[test]
fn importing_invalid_yaml_fails_and_creates_nothing() {
    let h = Harness::new();
    let file = h.dir.path().join("broken.yaml");
    fs::write(&file, "this: is: not: valid").unwrap();

    let err = h.fails(&["import", file.to_str().unwrap()]);
    // The whole chain reaches the user: which file, and what the parser said about it.
    assert!(err.contains("Invalid session file") && err.contains("broken.yaml"), "got: {err}");
    assert!(err.contains("line 1"), "parser detail missing, got: {err}");
    assert!(h.sessions().is_empty());
}

#[test]
fn importing_a_multi_window_session() {
    let h = Harness::new();
    let file = h.dir.path().join("multi.yaml");
    fs::write(
        &file,
        "name: multi\nwindows:\n- command: sleep 300\n  working_directory: /tmp\n- command: sleep 301\n  working_directory: /\n",
    )
    .unwrap();

    h.ok(&["import", file.to_str().unwrap()]);
    assert_eq!(h.sessions(), vec!["multi"]);

    let (_, yaml) = h.export("multi", &h.dir.path().join("export"));
    let session: sessiond::backend::storage::session_files::SessionConfig =
        serde_yaml::from_str(&yaml).unwrap();
    let dirs: Vec<_> = session.windows.iter().map(|w| w.working_directory.as_str()).collect();
    assert_eq!(dirs, vec!["/tmp", "/"]);
}

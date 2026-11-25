use std::fs;

use sessiond::backend::storage::session_files::{SessionConfig, WindowConfig};
use sessiond::backend::storage::session_files::SessionFiles;
use tempfile::TempDir;

fn files() -> (TempDir, SessionFiles) {
    let dir = TempDir::new().unwrap();
    let files = SessionFiles::with_dir(dir.path().join("sessions")).unwrap();
    (dir, files)
}

fn session(name: &str, tracked: bool) -> SessionConfig {
    SessionConfig {
        name: name.into(),
        windows: vec![WindowConfig {
            command: "sleep 300".into(),
            working_directory: "/tmp".into(),
            tracked,
        }],
    }
}

#[test]
fn with_dir_creates_the_directory() {
    let (dir, files) = files();
    assert!(dir.path().join("sessions").is_dir());
    assert_eq!(files.dir(), &dir.path().join("sessions"));
    assert_eq!(
        files.path_for("foo"),
        dir.path().join("sessions/foo.yaml")
    );
}

#[test]
fn save_and_load_round_trip() {
    let (_dir, files) = files();
    let path = files.save(&session("Alpha", true)).unwrap();
    assert_eq!(path, files.path_for("Alpha"));

    let loaded = files.load(&path).unwrap();
    assert_eq!(loaded.name, "Alpha");
    assert_eq!(loaded.windows.len(), 1);
    assert_eq!(loaded.windows[0].command, "sleep 300");
    assert_eq!(loaded.windows[0].working_directory, "/tmp");
    assert!(loaded.windows[0].tracked);
}

#[test]
fn untracked_is_left_out_of_the_yaml_and_is_the_default() {
    let (_dir, files) = files();
    let yaml = files.serialize(&session("a", false)).unwrap();
    assert!(!yaml.contains("tracked"), "got:\n{yaml}");

    let yaml = files.serialize(&session("a", true)).unwrap();
    assert!(yaml.contains("tracked: true"), "got:\n{yaml}");

    let parsed = files
        .deserialize("name: a\nwindows:\n- command: x\n  working_directory: /\n")
        .unwrap();
    assert!(!parsed.windows[0].tracked);
}

#[test]
fn invalid_yaml_is_an_error() {
    let (_dir, files) = files();
    assert!(files.deserialize("this: is: not: valid").is_err());
    assert!(files.deserialize("name: missing-windows\n").is_err());

    let path = files.dir().join("broken.yaml");
    fs::write(&path, "[[[").unwrap();
    assert!(files.load(&path).is_err());
    assert!(files.load(&files.path_for("nope")).is_err());
}

#[test]
fn list_session_files_is_sorted_and_only_yaml_files() {
    let (_dir, files) = files();
    let root = files.dir().clone();
    fs::write(root.join("b.yaml"), "").unwrap();
    fs::write(root.join("a.yaml"), "").unwrap();
    fs::write(root.join("notes.txt"), "").unwrap();
    fs::write(root.join("c.yml"), "").unwrap();
    fs::create_dir(root.join("dir.yaml")).unwrap();

    assert_eq!(
        files.list().unwrap(),
        vec![root.join("a.yaml"), root.join("b.yaml")]
    );
}

#[test]
fn find_session_files_matches_the_name_inside_the_file_ignoring_case() {
    let (_dir, files) = files();
    let root = files.dir().clone();
    files.save(&session("MixedCase", false)).unwrap();
    files.save(&session("mixedcase", false)).unwrap();
    // File name does not matter when the file parses...
    files
        .save_to(&session("MIXEDCASE", false), &root.join("renamed.yaml"))
        .unwrap();
    // ...and the file stem is used when it does not.
    fs::write(root.join("mixedCASE.yaml"), "garbage: [").unwrap();
    files.save(&session("other", false)).unwrap();

    let mut found = files.find_for_unit("mixedcase").unwrap();
    found.sort();
    let mut expected = vec![
        root.join("MixedCase.yaml"),
        root.join("mixedCASE.yaml"),
        root.join("mixedcase.yaml"),
        root.join("renamed.yaml"),
    ];
    expected.sort();
    assert_eq!(found, expected);

    assert!(files.find_for_unit("absent").unwrap().is_empty());
}

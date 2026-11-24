use std::fs;

use daemon_test::backend::yaml::models::{YamlSession, YamlWindow};
use daemon_test::backend::yaml::yaml_manager::YamlManager;
use tempfile::TempDir;

fn manager() -> (TempDir, YamlManager) {
    let dir = TempDir::new().unwrap();
    let manager = YamlManager::with_dir(dir.path().join("sessions")).unwrap();
    (dir, manager)
}

fn session(name: &str, tracked: bool) -> YamlSession {
    YamlSession {
        name: name.into(),
        windows: vec![YamlWindow {
            command: "sleep 300".into(),
            working_directory: "/tmp".into(),
            tracked,
        }],
    }
}

#[test]
fn with_dir_creates_the_directory() {
    let (dir, manager) = manager();
    assert!(dir.path().join("sessions").is_dir());
    assert_eq!(manager.get_storage_dir(), &dir.path().join("sessions"));
    assert_eq!(
        manager.get_session_path("foo"),
        dir.path().join("sessions/foo.yaml")
    );
}

#[test]
fn save_and_load_round_trip() {
    let (_dir, manager) = manager();
    let path = manager.save_session(&session("Alpha", true)).unwrap();
    assert_eq!(path, manager.get_session_path("Alpha"));

    let loaded = manager.load_session(&path).unwrap();
    assert_eq!(loaded.name, "Alpha");
    assert_eq!(loaded.windows.len(), 1);
    assert_eq!(loaded.windows[0].command, "sleep 300");
    assert_eq!(loaded.windows[0].working_directory, "/tmp");
    assert!(loaded.windows[0].tracked);
}

#[test]
fn untracked_is_left_out_of_the_yaml_and_is_the_default() {
    let (_dir, manager) = manager();
    let yaml = manager.serialize_session(&session("a", false)).unwrap();
    assert!(!yaml.contains("tracked"), "got:\n{yaml}");

    let yaml = manager.serialize_session(&session("a", true)).unwrap();
    assert!(yaml.contains("tracked: true"), "got:\n{yaml}");

    let parsed = manager
        .deserialize_session("name: a\nwindows:\n- command: x\n  working_directory: /\n")
        .unwrap();
    assert!(!parsed.windows[0].tracked);
}

#[test]
fn invalid_yaml_is_an_error() {
    let (_dir, manager) = manager();
    assert!(manager.deserialize_session("this: is: not: valid").is_err());
    assert!(manager.deserialize_session("name: missing-windows\n").is_err());

    let path = manager.get_storage_dir().join("broken.yaml");
    fs::write(&path, "[[[").unwrap();
    assert!(manager.load_session(&path).is_err());
    assert!(manager.load_session(&manager.get_session_path("nope")).is_err());
}

#[test]
fn list_session_files_is_sorted_and_only_yaml_files() {
    let (_dir, manager) = manager();
    let root = manager.get_storage_dir().clone();
    fs::write(root.join("b.yaml"), "").unwrap();
    fs::write(root.join("a.yaml"), "").unwrap();
    fs::write(root.join("notes.txt"), "").unwrap();
    fs::write(root.join("c.yml"), "").unwrap();
    fs::create_dir(root.join("dir.yaml")).unwrap();

    assert_eq!(
        manager.list_session_files().unwrap(),
        vec![root.join("a.yaml"), root.join("b.yaml")]
    );
}

#[test]
fn find_session_files_matches_the_name_inside_the_file_ignoring_case() {
    let (_dir, manager) = manager();
    let root = manager.get_storage_dir().clone();
    manager.save_session(&session("MixedCase", false)).unwrap();
    manager.save_session(&session("mixedcase", false)).unwrap();
    // File name does not matter when the file parses...
    manager
        .save_session_to(&session("MIXEDCASE", false), &root.join("renamed.yaml"))
        .unwrap();
    // ...and the file stem is used when it does not.
    fs::write(root.join("mixedCASE.yaml"), "garbage: [").unwrap();
    manager.save_session(&session("other", false)).unwrap();

    let mut found = manager.find_session_files("mixedcase").unwrap();
    found.sort();
    let mut expected = vec![
        root.join("MixedCase.yaml"),
        root.join("mixedCASE.yaml"),
        root.join("mixedcase.yaml"),
        root.join("renamed.yaml"),
    ];
    expected.sort();
    assert_eq!(found, expected);

    assert!(manager.find_session_files("absent").unwrap().is_empty());
}

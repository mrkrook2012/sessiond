use std::fs;
use std::path::Path;

use daemon_test::backend::systemd::UnitName;
use daemon_test::backend::systemd::unit_file::{self, RestartPolicy};
use tempfile::TempDir;

/// Writes a unit for `name` into a temp dir and returns (dir, unit file contents).
fn write_unit(name: &str) -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    unit_file::write_to(
        dir.path(),
        Path::new("/data/My Session.yaml"),
        "alice",
        &UnitName::new(name).unwrap(),
    )
    .unwrap();

    let file = dir.path().join(format!("{}.service", name.to_lowercase()));
    let contents = fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("unit file {} missing: {e}", file.display()));
    (dir, contents)
}

/// The value of `key=` in the unit, ignoring the indentation the template carries.
fn value<'a>(unit: &'a str, key: &str) -> Option<&'a str> {
    unit.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .map(str::trim)
}

#[test]
fn unit_file_is_named_after_the_lowercased_unit() {
    let (dir, _) = write_unit("MySvc");
    assert!(dir.path().join("mysvc.service").is_file());
}

#[test]
fn unit_runs_the_internal_commands_of_this_binary() {
    let (dir, unit) = write_unit("svc");
    let exe = std::env::current_exe().unwrap();
    let exe = exe.to_str().unwrap();

    assert_eq!(
        value(&unit, "ExecStart"),
        Some(format!("{exe} __internal-systemd-start \"/data/My Session.yaml\"").as_str())
    );
    assert_eq!(
        value(&unit, "ExecStop"),
        Some(format!("{exe} __internal-systemd-stop \"svc\"").as_str())
    );
    assert_eq!(value(&unit, "User"), Some("alice"));
    assert_eq!(value(&unit, "WorkingDirectory"), dir.path().to_str());
}

#[test]
fn unit_restarts_on_failure_and_only_kills_its_main_process() {
    let (_dir, unit) = write_unit("svc");
    assert_eq!(value(&unit, "Type"), Some("simple"));
    assert_eq!(value(&unit, "Restart"), Some("on-failure"));
    assert_eq!(value(&unit, "KillMode"), Some("process"));
    assert_eq!(value(&unit, "WantedBy"), Some("default.target"));
    for section in ["[Unit]", "[Service]", "[Install]"] {
        assert!(unit.lines().any(|l| l.trim() == section), "missing {section}:\n{unit}");
    }
}

#[test]
fn restart_policies_use_systemd_spelling() {
    let cases = [
        (RestartPolicy::No, "no"),
        (RestartPolicy::OnSuccess, "on-success"),
        (RestartPolicy::OnFailure, "on-failure"),
        (RestartPolicy::OnAbnormal, "on-abnormal"),
        (RestartPolicy::OnWatchdog, "on-watchdog"),
        (RestartPolicy::OnAbort, "on-abort"),
        (RestartPolicy::Always, "always"),
    ];
    for (policy, expected) in cases {
        assert_eq!(policy.as_str(), expected);
    }
}

#[test]
fn unit_file_path_is_inside_unit_dir() {
    let path = unit_file::path(&UnitName::new("svc").unwrap()).unwrap();
    assert_eq!(path, unit_file::dir().unwrap().join("svc.service"));
}

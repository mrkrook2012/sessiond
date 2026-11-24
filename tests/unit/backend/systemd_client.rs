use daemon_test::backend::session_manager::UnitName;
use daemon_test::backend::systemd_client::{unescape_bus_label, unit_from_object_path};

const PREFIX: &str = "/org/freedesktop/systemd1/unit/";

#[test]
fn unescapes_unit_paths() {
    let unit = unit_from_object_path(&format!("{PREFIX}my_2dsession_2eservice"));
    assert_eq!(unit, Some(UnitName::new("my-session").unwrap()));
}

#[test]
fn unit_from_path_is_lowercased() {
    let unit = unit_from_object_path(&format!("{PREFIX}MySvc_2eservice"));
    assert_eq!(unit.map(|u| u.to_string()), Some("mysvc".into()));
}

#[test]
fn ignores_non_service_units_and_other_paths() {
    assert_eq!(unit_from_object_path(&format!("{PREFIX}default_2etarget")), None);
    assert_eq!(unit_from_object_path("/org/freedesktop/systemd1/job/12"), None);
    assert_eq!(unit_from_object_path(&format!("{PREFIX}broken_2")), None);
}

#[test]
fn unescape_bus_label_cases() {
    assert_eq!(unescape_bus_label("plain"), Some("plain".into()));
    assert_eq!(unescape_bus_label(""), Some("".into()));
    assert_eq!(unescape_bus_label("a_2db_5fc"), Some("a-b_c".into()));
    assert_eq!(unescape_bus_label("_40home"), Some("@home".into()));
    // Multi-byte UTF-8, escaped byte by byte.
    assert_eq!(unescape_bus_label("caf_c3_a9"), Some("café".into()));
}

#[test]
fn unescape_bus_label_rejects_malformed_input() {
    assert_eq!(unescape_bus_label("trailing_"), None);
    assert_eq!(unescape_bus_label("short_2"), None);
    assert_eq!(unescape_bus_label("bad_zz"), None);
    // Valid escapes that do not form UTF-8.
    assert_eq!(unescape_bus_label("_ff"), None);
}

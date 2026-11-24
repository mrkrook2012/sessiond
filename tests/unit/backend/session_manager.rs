use daemon_test::backend::session_manager::{
    SessionId, SessionKind, SessionManager, SessionState, UnitName,
};
use daemon_test::backend::window_manager::WindowId;
use daemon_test::backend::guid::Guid;
use daemon_test::ipc::protocol::SessionLookup;

fn unit(name: &str) -> UnitName {
    UnitName::new(name).unwrap()
}

fn sysd_running(manager: &SessionManager, id: SessionId) -> bool {
    match &manager.get_session_by_id(id).unwrap().kind {
        SessionKind::SysdBased(sysd) => sysd.is_running(),
        SessionKind::Ephemoral { .. } => panic!("not a sysd session"),
    }
}

// ------------------------------------------------------------------ UnitName

#[test]
fn unit_names_are_lowercased() {
    assert_eq!(unit("MyService").to_string(), "myservice");
    assert_eq!(unit("MyService"), unit("myservice"));
}

// ------------------------------------------------------------------ ids

#[test]
fn ids_are_unique() {
    let mut manager = SessionManager::new();
    let a = manager.create_ephem_session(vec![], "a".into());
    let b = manager.create_ephem_session(vec![], "b".into());
    assert_ne!(a, b);
}

#[test]
fn killed_session_ids_are_never_reused() {
    let mut manager = SessionManager::new();
    let killed: Vec<SessionId> = ["a", "b", "c"]
        .into_iter()
        .map(|name| manager.create_ephem_session(vec![], name.into()))
        .collect();
    for &id in &killed {
        manager.kill_session(id);
    }

    for name in ["d", "e", "f"] {
        let id = manager.create_ephem_session(vec![], name.into());
        assert!(!killed.contains(&id), "{id:?} was handed out again");
    }
}

#[test]
fn every_start_gets_a_new_run_id() {
    let mut manager = SessionManager::new();
    let first = manager.create_running_sysd_session(unit("svc"), vec![]);
    let id = manager.get_id_by_name("svc").unwrap();
    manager.stop_sysd_session(id).unwrap();
    let second = manager.start_sysd_session(id, vec![]).unwrap();
    assert_ne!(first, second);
}

#[test]
fn killing_a_session_twice_does_not_hand_out_its_id_twice() {
    let mut manager = SessionManager::new();
    let id = manager.create_ephem_session(vec![], "a".into());
    manager.kill_session(id);
    manager.kill_session(id);

    let first = manager.create_ephem_session(vec![], "b".into());
    let second = manager.create_ephem_session(vec![], "c".into());
    assert_ne!(first, second);
    assert_eq!(manager.list_sessions().len(), 2);
}

// ------------------------------------------------------------------ lookup

#[test]
fn lookup_by_name_is_case_insensitive_but_existence_check_is_not() {
    let mut manager = SessionManager::new();
    let id = manager.create_ephem_session(vec![], "Alpha".into());

    assert_eq!(manager.get_id_by_name("alpha"), Some(id));
    assert_eq!(manager.get_id_by_name("ALPHA"), Some(id));
    assert_eq!(manager.get_id_by_name("beta"), None);

    assert!(manager.does_session_name_exist("Alpha"));
    assert!(!manager.does_session_name_exist("alpha"));
}

#[test]
fn session_is_found_by_its_window() {
    let mut manager = SessionManager::new();
    let a = manager.create_ephem_session(vec![WindowId(Guid::from_u128(1)), WindowId(Guid::from_u128(2))], "a".into());
    let b = manager.create_ephem_session(vec![WindowId(Guid::from_u128(3))], "b".into());

    assert_eq!(manager.get_session_id_by_window(WindowId(Guid::from_u128(2))), Some(a));
    assert_eq!(manager.get_session_id_by_window(WindowId(Guid::from_u128(3))), Some(b));
    assert_eq!(manager.get_session_id_by_window(WindowId(Guid::from_u128(9))), None);
}

#[test]
fn does_session_exist_follows_create_and_kill() {
    let mut manager = SessionManager::new();
    let id = manager.create_ephem_session(vec![], "a".into());
    assert!(manager.does_session_exist(id));
    manager.kill_session(id);
    assert!(!manager.does_session_exist(id));
    assert!(manager.get_session_by_id(id).is_none());
}

// ------------------------------------------------------------------ windows

#[test]
fn windows_can_be_added_and_removed() {
    let mut manager = SessionManager::new();
    let id = manager.create_ephem_session(vec![WindowId(Guid::from_u128(1))], "a".into());

    manager.add_window_to_session(id, WindowId(Guid::from_u128(2))).unwrap();
    assert_eq!(
        manager.get_all_windows_from_session(id),
        Some(&vec![WindowId(Guid::from_u128(1)), WindowId(Guid::from_u128(2))])
    );

    manager.remove_window_from_session(id, WindowId(Guid::from_u128(1))).unwrap();
    assert_eq!(manager.get_all_windows_from_session(id), Some(&vec![WindowId(Guid::from_u128(2))]));
}

#[test]
fn window_operations_on_unknown_session_fail() {
    let mut manager = SessionManager::new();
    assert!(manager.add_window_to_session(SessionId(Guid::from_u128(7)), WindowId(Guid::from_u128(1))).is_err());
    assert!(manager.remove_window_from_session(SessionId(Guid::from_u128(7)), WindowId(Guid::from_u128(1))).is_err());
    assert_eq!(manager.get_all_windows_from_session(SessionId(Guid::from_u128(7))), None);
}

#[test]
fn stopped_sysd_session_has_no_windows() {
    let mut manager = SessionManager::new();
    let id = manager.create_stopped_sysd_session(unit("svc"), false);

    assert_eq!(manager.get_all_windows_from_session(id), None);
    assert!(manager.add_window_to_session(id, WindowId(Guid::from_u128(1))).is_err());
}

// ------------------------------------------------------------------ sysd sessions

#[test]
fn sysd_session_start_stop_cycle() {
    let mut manager = SessionManager::new();
    let id = manager.create_stopped_sysd_session(unit("svc"), false);
    assert!(!sysd_running(&manager, id));

    let run = manager.start_sysd_session(id, vec![WindowId(Guid::from_u128(1))]).unwrap();
    assert!(sysd_running(&manager, id));
    assert_eq!(manager.get_all_windows_from_session(id), Some(&vec![WindowId(Guid::from_u128(1))]));

    manager.stop_sysd_session(id).unwrap();
    assert!(!sysd_running(&manager, id));

    // Every start is a new run.
    let next = manager.start_sysd_session(id, vec![]).unwrap();
    assert_ne!(run, next);
}

#[test]
fn run_ids_are_unique_across_sessions() {
    let mut manager = SessionManager::new();
    let a = manager.create_running_sysd_session(unit("a"), vec![]);
    let b = manager.create_running_sysd_session(unit("b"), vec![]);
    assert_ne!(a, b);
}

#[test]
fn sysd_operations_reject_ephemeral_and_unknown_sessions() {
    let mut manager = SessionManager::new();
    let eph = manager.create_ephem_session(vec![], "eph".into());

    assert!(manager.start_sysd_session(eph, vec![]).is_err());
    assert!(manager.stop_sysd_session(eph).is_err());
    assert!(manager.start_sysd_session(SessionId(Guid::from_u128(99)), vec![]).is_err());
    assert!(manager.stop_sysd_session(SessionId(Guid::from_u128(99))).is_err());
}

#[test]
fn sysd_units_lists_only_sysd_sessions() {
    let mut manager = SessionManager::new();
    manager.create_ephem_session(vec![], "eph".into());
    manager.create_stopped_sysd_session(unit("stopped"), false);
    manager.create_running_sysd_session(unit("running"), vec![]);

    let mut units: Vec<String> = manager.sysd_units().into_iter().map(UnitName::into_string).collect();
    units.sort();
    assert_eq!(units, vec!["running", "stopped"]);
}

#[test]
fn sysd_session_found_by_unit_and_enabled_flag_is_reported() {
    let mut manager = SessionManager::new();
    let id = manager.create_stopped_sysd_session(unit("svc"), false);

    let (found, sysd) = manager.get_sysd_session_by_unit_mut(&unit("SVC")).unwrap();
    assert_eq!(found, id);
    assert!(!sysd.get_sysd_data().enabled);

    sysd.set_enabled(true);
    let data = sysd.get_sysd_data();
    assert!(data.enabled);
    assert!(!data.running);

    assert!(manager.get_sysd_session_by_unit_mut(&unit("other")).is_none());
}

#[test]
fn only_the_first_tracked_exit_of_a_run_is_kept() {
    let mut manager = SessionManager::new();
    manager.create_running_sysd_session(unit("svc"), vec![]);

    let (_, sysd) = manager.get_sysd_session_by_unit_mut(&unit("svc")).unwrap();
    sysd.record_tracked_exit(3);
    sysd.record_tracked_exit(7);

    match sysd.state() {
        SessionState::Running { tracked_exit, .. } => assert_eq!(*tracked_exit, Some(3)),
        SessionState::Stopped => panic!("session should be running"),
    }
}

#[test]
fn tracked_exit_is_ignored_while_stopped_and_cleared_by_a_new_run() {
    let mut manager = SessionManager::new();
    let id = manager.create_stopped_sysd_session(unit("svc"), false);

    let (_, sysd) = manager.get_sysd_session_by_unit_mut(&unit("svc")).unwrap();
    sysd.record_tracked_exit(1);
    assert!(matches!(sysd.state(), SessionState::Stopped));

    manager.start_sysd_session(id, vec![]).unwrap();
    let (_, sysd) = manager.get_sysd_session_by_unit_mut(&unit("svc")).unwrap();
    assert!(matches!(
        sysd.state(),
        SessionState::Running { tracked_exit: None, .. }
    ));
}

// ------------------------------------------------------------------ lookup by name

fn lookup_id(manager: &SessionManager, name: &str) -> Option<SessionId> {
    match manager.lookup_by_name(name) {
        SessionLookup::Found { id, .. } => Some(id),
        SessionLookup::NotFound { .. } => None,
    }
}

#[test]
fn lookup_finds_exact_names() {
    let mut manager = SessionManager::new();
    let id = manager.create_ephem_session(vec![], "webserver".into());
    manager.create_ephem_session(vec![], "web".into());

    assert_eq!(
        manager.lookup_by_name("webserver"),
        SessionLookup::Found { id, name: "webserver".into() }
    );
}

#[test]
fn lookup_accepts_a_unique_match_in_another_case() {
    let mut manager = SessionManager::new();
    let id = manager.create_ephem_session(vec![], "WebServer".into());

    assert_eq!(
        manager.lookup_by_name("webserver"),
        SessionLookup::Found { id, name: "WebServer".into() }
    );
}

#[test]
fn lookup_prefers_the_exact_case_and_rejects_ambiguous_case() {
    let mut manager = SessionManager::new();
    let upper = manager.create_ephem_session(vec![], "Web".into());
    let lower = manager.create_ephem_session(vec![], "web".into());

    assert_eq!(lookup_id(&manager, "Web"), Some(upper));
    assert_eq!(lookup_id(&manager, "web"), Some(lower));

    // "WEB" could be either: not found, and both are suggested.
    let SessionLookup::NotFound { mut similar } = manager.lookup_by_name("WEB") else {
        panic!("an ambiguous name must not resolve");
    };
    similar.sort();
    assert_eq!(similar, vec!["Web", "web"]);
}

#[test]
fn lookup_suggests_similar_names() {
    let mut manager = SessionManager::new();
    manager.create_ephem_session(vec![], "webserver".into());
    manager.create_stopped_sysd_session(unit("database"), false);

    assert_eq!(
        manager.lookup_by_name("websrever"),
        SessionLookup::NotFound { similar: vec!["webserver".into()] }
    );
    assert_eq!(
        manager.lookup_by_name("databse"),
        SessionLookup::NotFound { similar: vec!["database".into()] }
    );
    assert_eq!(
        manager.lookup_by_name("zzz"),
        SessionLookup::NotFound { similar: vec![] }
    );
}

#[test]
fn lookup_does_not_find_killed_sessions() {
    let mut manager = SessionManager::new();
    let id = manager.create_ephem_session(vec![], "gone".into());
    manager.kill_session(id);

    assert_eq!(lookup_id(&manager, "gone"), None);
}

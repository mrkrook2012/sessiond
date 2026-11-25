use daemon_test::backend::session::{
    SessionKind, Sessions, SessionState,
};
use daemon_test::backend::systemd::UnitName;
use daemon_test::ids::{Guid, SessionId, WindowId};
use daemon_test::ipc::protocol::SessionLookup;

fn unit(name: &str) -> UnitName {
    UnitName::new(name).unwrap()
}

fn sysd_running(sessions: &Sessions, id: SessionId) -> bool {
    match &sessions.get(id).unwrap().kind {
        SessionKind::SysdBased(sysd) => sysd.is_running(),
        SessionKind::Ephemeral { .. } => panic!("not a sysd session"),
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
    let mut sessions = Sessions::new();
    let a = sessions.create_ephemeral(vec![], "a".into()).unwrap();
    let b = sessions.create_ephemeral(vec![], "b".into()).unwrap();
    assert_ne!(a, b);
}

#[test]
fn killed_session_ids_are_never_reused() {
    let mut sessions = Sessions::new();
    let killed: Vec<SessionId> = ["a", "b", "c"]
        .into_iter()
        .map(|name| sessions.create_ephemeral(vec![], name.into()).unwrap())
        .collect();
    for &id in &killed {
        sessions.remove(id);
    }

    for name in ["d", "e", "f"] {
        let id = sessions.create_ephemeral(vec![], name.into()).unwrap();
        assert!(!killed.contains(&id), "{id:?} was handed out again");
    }
}

#[test]
fn every_start_gets_a_new_run_id() {
    let mut sessions = Sessions::new();
    let first = sessions.create_running_sysd(unit("svc"), vec![]).unwrap();
    let id = sessions.id_by_name("svc").unwrap();
    sessions.stop_sysd(id).unwrap();
    let second = sessions.start_sysd(id, vec![]).unwrap();
    assert_ne!(first, second);
}

#[test]
fn killing_a_session_twice_does_not_hand_out_its_id_twice() {
    let mut sessions = Sessions::new();
    let id = sessions.create_ephemeral(vec![], "a".into()).unwrap();
    sessions.remove(id);
    sessions.remove(id);

    let first = sessions.create_ephemeral(vec![], "b".into()).unwrap();
    let second = sessions.create_ephemeral(vec![], "c".into()).unwrap();
    assert_ne!(first, second);
    assert_eq!(sessions.list().len(), 2);
}

// ------------------------------------------------------------------ lookup

#[test]
fn lookup_and_existence_checks_ignore_case() {
    let mut sessions = Sessions::new();
    let id = sessions.create_ephemeral(vec![], "Alpha".into()).unwrap();

    assert_eq!(sessions.id_by_name("alpha"), Some(id));
    assert_eq!(sessions.id_by_name("ALPHA"), Some(id));
    assert_eq!(sessions.id_by_name("beta"), None);

    assert!(sessions.name_exists("Alpha"));
    assert!(sessions.name_exists("alpha"));
    assert!(!sessions.name_exists("beta"));
}

#[test]
fn session_is_found_by_its_window() {
    let mut sessions = Sessions::new();
    let a = sessions.create_ephemeral(vec![WindowId(Guid::from_u128(1)), WindowId(Guid::from_u128(2))], "a".into()).unwrap();
    let b = sessions.create_ephemeral(vec![WindowId(Guid::from_u128(3))], "b".into()).unwrap();

    assert_eq!(sessions.id_by_window(WindowId(Guid::from_u128(2))), Some(a));
    assert_eq!(sessions.id_by_window(WindowId(Guid::from_u128(3))), Some(b));
    assert_eq!(sessions.id_by_window(WindowId(Guid::from_u128(9))), None);
}

#[test]
fn does_session_exist_follows_create_and_kill() {
    let mut sessions = Sessions::new();
    let id = sessions.create_ephemeral(vec![], "a".into()).unwrap();
    assert!(sessions.contains(id));
    sessions.remove(id);
    assert!(!sessions.contains(id));
    assert!(sessions.get(id).is_none());
}

// ------------------------------------------------------------------ windows

#[test]
fn windows_can_be_added_and_removed() {
    let mut sessions = Sessions::new();
    let id = sessions.create_ephemeral(vec![WindowId(Guid::from_u128(1))], "a".into()).unwrap();

    sessions.add_window(id, WindowId(Guid::from_u128(2))).unwrap();
    assert_eq!(
        sessions.windows(id),
        Some(&vec![WindowId(Guid::from_u128(1)), WindowId(Guid::from_u128(2))])
    );

    sessions.remove_window(id, WindowId(Guid::from_u128(1))).unwrap();
    assert_eq!(sessions.windows(id), Some(&vec![WindowId(Guid::from_u128(2))]));
}

#[test]
fn window_operations_on_unknown_session_fail() {
    let mut sessions = Sessions::new();
    assert!(sessions.add_window(SessionId(Guid::from_u128(7)), WindowId(Guid::from_u128(1))).is_err());
    assert!(sessions.remove_window(SessionId(Guid::from_u128(7)), WindowId(Guid::from_u128(1))).is_err());
    assert_eq!(sessions.windows(SessionId(Guid::from_u128(7))), None);
}

#[test]
fn stopped_sysd_session_has_no_windows() {
    let mut sessions = Sessions::new();
    let id = sessions.create_stopped_sysd(unit("svc"), false).unwrap();

    assert_eq!(sessions.windows(id), None);
    assert!(sessions.add_window(id, WindowId(Guid::from_u128(1))).is_err());
}

// ------------------------------------------------------------------ sysd sessions

#[test]
fn sysd_session_start_stop_cycle() {
    let mut sessions = Sessions::new();
    let id = sessions.create_stopped_sysd(unit("svc"), false).unwrap();
    assert!(!sysd_running(&sessions, id));

    let run = sessions.start_sysd(id, vec![WindowId(Guid::from_u128(1))]).unwrap();
    assert!(sysd_running(&sessions, id));
    assert_eq!(sessions.windows(id), Some(&vec![WindowId(Guid::from_u128(1))]));

    sessions.stop_sysd(id).unwrap();
    assert!(!sysd_running(&sessions, id));

    // Every start is a new run.
    let next = sessions.start_sysd(id, vec![]).unwrap();
    assert_ne!(run, next);
}

#[test]
fn run_ids_are_unique_across_sessions() {
    let mut sessions = Sessions::new();
    let a = sessions.create_running_sysd(unit("a"), vec![]).unwrap();
    let b = sessions.create_running_sysd(unit("b"), vec![]).unwrap();
    assert_ne!(a, b);
}

#[test]
fn sysd_operations_reject_ephemeral_and_unknown_sessions() {
    let mut sessions = Sessions::new();
    let eph = sessions.create_ephemeral(vec![], "eph".into()).unwrap();

    assert!(sessions.start_sysd(eph, vec![]).is_err());
    assert!(sessions.stop_sysd(eph).is_err());
    assert!(sessions.start_sysd(SessionId(Guid::from_u128(99)), vec![]).is_err());
    assert!(sessions.stop_sysd(SessionId(Guid::from_u128(99))).is_err());
}

#[test]
fn sysd_units_lists_only_sysd_sessions() {
    let mut sessions = Sessions::new();
    sessions.create_ephemeral(vec![], "eph".into()).unwrap();
    sessions.create_stopped_sysd(unit("stopped"), false).unwrap();
    sessions.create_running_sysd(unit("running"), vec![]).unwrap();

    let mut units: Vec<String> = sessions.sysd_units().into_iter().map(UnitName::into_string).collect();
    units.sort();
    assert_eq!(units, vec!["running", "stopped"]);
}

#[test]
fn sysd_session_found_by_unit_and_enabled_flag_is_reported() {
    let mut sessions = Sessions::new();
    let id = sessions.create_stopped_sysd(unit("svc"), false).unwrap();

    let (found, sysd) = sessions.sysd_by_unit_mut(&unit("SVC")).unwrap();
    assert_eq!(found, id);
    assert!(!sysd.sysd_data().enabled);

    sysd.set_enabled(true);
    let data = sysd.sysd_data();
    assert!(data.enabled);
    assert!(!data.running);

    assert!(sessions.sysd_by_unit_mut(&unit("other")).is_none());
}

#[test]
fn only_the_first_tracked_exit_of_a_run_is_kept() {
    let mut sessions = Sessions::new();
    sessions.create_running_sysd(unit("svc"), vec![]).unwrap();

    let (_, sysd) = sessions.sysd_by_unit_mut(&unit("svc")).unwrap();
    sysd.record_tracked_exit(3);
    sysd.record_tracked_exit(7);

    match sysd.state() {
        SessionState::Running { tracked_exit, .. } => assert_eq!(*tracked_exit, Some(3)),
        SessionState::Stopped => panic!("session should be running"),
    }
}

#[test]
fn tracked_exit_is_ignored_while_stopped_and_cleared_by_a_new_run() {
    let mut sessions = Sessions::new();
    let id = sessions.create_stopped_sysd(unit("svc"), false).unwrap();

    let (_, sysd) = sessions.sysd_by_unit_mut(&unit("svc")).unwrap();
    sysd.record_tracked_exit(1);
    assert!(matches!(sysd.state(), SessionState::Stopped));

    sessions.start_sysd(id, vec![]).unwrap();
    let (_, sysd) = sessions.sysd_by_unit_mut(&unit("svc")).unwrap();
    assert!(matches!(
        sysd.state(),
        SessionState::Running { tracked_exit: None, .. }
    ));
}

// ------------------------------------------------------------------ lookup by name

fn lookup_id(sessions: &Sessions, name: &str) -> Option<SessionId> {
    match sessions.lookup_by_name(name) {
        SessionLookup::Found { id, .. } => Some(id),
        SessionLookup::NotFound { .. } => None,
    }
}

#[test]
fn lookup_finds_exact_names() {
    let mut sessions = Sessions::new();
    let id = sessions.create_ephemeral(vec![], "webserver".into()).unwrap();
    sessions.create_ephemeral(vec![], "web".into()).unwrap();

    assert_eq!(
        sessions.lookup_by_name("webserver"),
        SessionLookup::Found { id, name: "webserver".into() }
    );
}

#[test]
fn lookup_accepts_a_unique_match_in_another_case() {
    let mut sessions = Sessions::new();
    let id = sessions.create_ephemeral(vec![], "WebServer".into()).unwrap();

    assert_eq!(
        sessions.lookup_by_name("webserver"),
        SessionLookup::Found { id, name: "WebServer".into() }
    );
}

#[test]
fn names_are_unique_ignoring_case_across_kinds() {
    let mut sessions = Sessions::new();
    sessions.create_ephemeral(vec![], "Web".into()).unwrap();

    let err = sessions.create_ephemeral(vec![], "web".into()).unwrap_err();
    assert!(err.to_string().contains("already exists"), "got: {err}");
    assert!(sessions.create_stopped_sysd(unit("WEB"), false).is_err());
    assert!(sessions.create_running_sysd(unit("web"), vec![]).is_err());
    assert_eq!(sessions.list().len(), 1);
}

#[test]
fn invalid_names_are_rejected() {
    let mut sessions = Sessions::new();
    for bad in ["", "../etc", "a/b", "has space", ".hidden", "-flag", "tab\t", "ünïcode", &"x".repeat(101)] {
        assert!(sessions.create_ephemeral(vec![], bad.into()).is_err(), "accepted {bad:?}");
        assert!(UnitName::new(bad).is_err(), "UnitName accepted {bad:?}");
    }
    for good in ["web", "Web-Server_2", "v1.2", "0day"] {
        sessions.create_ephemeral(vec![], good.into()).unwrap();
    }
}

#[test]
fn default_names_skip_taken_ones() {
    let mut sessions = Sessions::new();
    assert_eq!(sessions.free_default_name(), "session-1");
    sessions.create_ephemeral(vec![], "Session-1".into()).unwrap();
    assert_eq!(sessions.free_default_name(), "session-2");
}

#[test]
fn lookup_suggests_similar_names() {
    let mut sessions = Sessions::new();
    sessions.create_ephemeral(vec![], "webserver".into()).unwrap();
    sessions.create_stopped_sysd(unit("database"), false).unwrap();

    assert_eq!(
        sessions.lookup_by_name("websrever"),
        SessionLookup::NotFound { similar: vec!["webserver".into()] }
    );
    assert_eq!(
        sessions.lookup_by_name("databse"),
        SessionLookup::NotFound { similar: vec!["database".into()] }
    );
    assert_eq!(
        sessions.lookup_by_name("zzz"),
        SessionLookup::NotFound { similar: vec![] }
    );
}

#[test]
fn lookup_does_not_find_killed_sessions() {
    let mut sessions = Sessions::new();
    let id = sessions.create_ephemeral(vec![], "gone".into()).unwrap();
    sessions.remove(id);

    assert_eq!(lookup_id(&sessions, "gone"), None);
}

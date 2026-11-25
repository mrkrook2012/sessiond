use bincode_next::{Decode, Encode};
use serde::Serialize;
use std::collections::HashMap;

use color_eyre::eyre::{Ok, OptionExt, Result, bail, eyre};

use crate::{
    backend::{
        guid::{Guid, GuidManager},
        window_manager::WindowId,
    },
    backend::name_match::similar_names,
    ipc::protocol::{SessionLookup, SysdData},
};

#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    Hash,
    PartialEq,
    Ord,
    PartialOrd,
    Encode,
    Decode,
    Serialize,
    serde::Deserialize,
)]
pub struct SessionId(pub Guid);

pub struct SessionManager {
    /// Source of both `SessionId`s and `RunId`s; every start of a sysd session gets a fresh run.
    guids: GuidManager,
    sessions: HashMap<SessionId, Session>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            guids: GuidManager::new(),
            sessions: HashMap::new(),
        }
    }



    pub fn remove_window_from_session(&mut self, id: SessionId, window_id: WindowId) -> Result<()> {
        let session = self
            .get_session_by_id_mut(id)
            .ok_or(eyre!("Session not found"))?;
        let windows = session
            .get_windows_mut()
            .ok_or_eyre(eyre!("Session did not have windows"))?;

        windows.retain(|&id| id != window_id);
        Ok(())
    }



    pub fn get_all_windows_from_session(&self, id: SessionId) -> Option<&Vec<WindowId>> {
        let session = self.get_session_by_id(id);
        match session {
            Some(session) => return session.get_windows(),
            None => None,
        }
    }

    pub fn does_session_exist(&self, id: SessionId) -> bool {
        self.sessions.contains_key(&id)
    }

    /// Errors unless `name` is a valid session name that no session uses yet, in any case.
    /// Names are unique ignoring case because a sysd session's unit name is its name in
    /// lowercase, and two sessions must never end up with the same unit or YAML file.
    pub fn check_new_name(&self, name: &str) -> Result<()> {
        validate_session_name(name)?;
        if let Some(existing) = self
            .sessions
            .values()
            .find(|session| session.name.eq_ignore_ascii_case(name))
        {
            bail!("A session named '{}' already exists", existing.name);
        }
        Ok(())
    }

    /// The first of `session-1`, `session-2`, ... that no session uses yet.
    pub fn free_default_name(&self) -> String {
        (1..)
            .map(|n| format!("session-{n}"))
            .find(|name| !self.does_session_name_exist(name))
            .expect("some session-N name is free")
    }

    pub fn create_ephem_session(&mut self, windows: Vec<WindowId>, name: String) -> Result<SessionId> {
        self.check_new_name(&name)?;
        let id = self.generate_id();
        let session = Session {
            id,
            name,
            kind: SessionKind::Ephemoral { windows },
        };
        self.sessions.insert(id, session);
        Ok(id)
    }

    pub fn create_running_sysd_session(&mut self, name: UnitName, windows: Vec<WindowId>) -> Result<RunId> {
        self.check_new_name(name.as_str())?;
        let id = self.generate_id();
        let run = self.next_run();
        let session = Session {
            id,
            name: name.as_str().to_owned(),
            kind: SessionKind::SysdBased(SysdSession {
                name,
                state: SessionState::running(windows, run),
                enabled: false,
            }),
        };
        self.sessions.insert(id, session);
        Ok(run)
    }

    pub fn create_stopped_sysd_session(&mut self, name: UnitName, enabled: bool) -> Result<SessionId> {
        self.check_new_name(name.as_str())?;
        let id = self.generate_id();
        let session = Session {
            id,
            name: name.as_str().to_owned(),
            kind: SessionKind::SysdBased(SysdSession {
                name,
                state: SessionState::Stopped,
                enabled,
            }),
        };
        self.sessions.insert(id, session);
        Ok(id)
    }

    /// Unit names of every sysd session, running or stopped.
    pub fn sysd_units(&self) -> Vec<UnitName> {
        self.sessions
            .values()
            .filter_map(|session| match &session.kind {
                SessionKind::SysdBased(sysd) => Some(sysd.unit().clone()),
                SessionKind::Ephemoral { .. } => None,
            })
            .collect()
    }

    /// Finds the sysd session backed by this unit.
    pub fn get_sysd_session_by_unit_mut(&mut self, unit: &UnitName) -> Option<(SessionId, &mut SysdSession)> {
        self.sessions.values_mut().find_map(|session| match &mut session.kind {
            SessionKind::SysdBased(sysd) if sysd.unit() == unit => Some((session.id, sysd)),
            _ => None,
        })
    }

    /// The session that owns this window.
    pub fn get_session_id_by_window(&self, window_id: WindowId) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|session| {
                session
                    .get_windows()
                    .is_some_and(|windows| windows.contains(&window_id))
            })
            .map(|session| session.id)
    }

    /// True if a session has this name, ignoring case (names are unique ignoring case).
    pub fn does_session_name_exist(&self, name: &str) -> bool {
        self.sessions.values().any(|session| session.name.eq_ignore_ascii_case(name))
    }

    /// Resolves a name the user typed, ignoring case (names are unique ignoring case).
    /// If there is no such session, returns the sessions with similar names, so the user
    /// can be told what they probably meant.
    pub fn lookup_by_name(&self, name: &str) -> SessionLookup {
        if let Some(session) = self
            .sessions
            .values()
            .find(|session| session.name.eq_ignore_ascii_case(name))
        {
            return SessionLookup::Found {
                id: session.id,
                name: session.name.clone(),
            };
        }

        let names = self.sessions.values().map(|session| session.name.as_str());
        SessionLookup::NotFound {
            similar: similar_names(name, names).into_iter().map(str::to_owned).collect(),
        }
    }

    pub fn get_id_by_name(&self, name: &str) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|session| session.name.eq_ignore_ascii_case(name))
            .map(|session| session.id)
    }

    /// Marks a stopped sysd session as running with these windows. The caller must have
    /// torn down any previous windows.
    pub fn start_sysd_session(&mut self, id: SessionId, windows: Vec<WindowId>) -> Result<RunId> {
        let run = self.next_run();
        let session = self
            .get_session_by_id_mut(id)
            .ok_or_eyre("Session not found")?;

        match &mut session.kind {
            SessionKind::Ephemoral { .. } => {
                bail!("Tried to start a non sysd session")
            }
            SessionKind::SysdBased(sysd_session) => {
                sysd_session.state = SessionState::running(windows, run);
            }
        }

        Ok(run)
    }

    fn next_run(&mut self) -> RunId {
        RunId(self.guids.generate())
    }

    pub fn stop_sysd_session(&mut self, id: SessionId) -> Result<()> {
        let session = self
            .get_session_by_id_mut(id)
            .ok_or_eyre("Session not found")?;

        match &mut session.kind {
            SessionKind::Ephemoral { .. } => {
                bail!("Tried to stop a non sysd session")
            }
            SessionKind::SysdBased(sysd_session) => {
                sysd_session.state = SessionState::Stopped;
            }
        }

        Ok(())
    }
    //ect
    // Also make start session, which will take in the window id's

    // TODO pub fn append_to_session maybe in IPC API
    pub fn add_window_to_session(&mut self, id: SessionId, window_id: WindowId) -> Result<()> {
        let session = self
            .get_session_by_id_mut(id)
            .ok_or(eyre!("Session not found"))?;
        let windows = session
            .get_windows_mut()
            .ok_or_eyre(eyre!("Session did not have windows"))?;

        windows.push(window_id);
        Ok(())
    }

    pub fn get_session_by_id(&self, id: SessionId) -> Option<&Session> {
        self.sessions.get(&id)
    }

    pub fn get_session_by_id_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.get_mut(&id)
    }
    pub fn list_sessions(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn kill_session(&mut self, id: SessionId) {
        self.sessions.remove(&id);
    }

    fn generate_id(&mut self) -> SessionId {
        SessionId(self.guids.generate())
    }
}

pub struct Session {
    pub id: SessionId,
    pub name: String,
    pub kind: SessionKind,
}

impl Session {
    pub fn get_windows(&self) -> Option<&Vec<WindowId>> {
        match &self.kind {
            SessionKind::Ephemoral { windows } => Some(windows),
            SessionKind::SysdBased(session) => match &session.state {
                SessionState::Running { windows, .. } => Some(windows),
                SessionState::Stopped => None,
            },
        }
    }

    pub fn get_windows_mut(&mut self) -> Option<&mut Vec<WindowId>> {
        match &mut self.kind {
            SessionKind::Ephemoral { windows } => Some(windows),
            SessionKind::SysdBased(session) => match &mut session.state {
                SessionState::Running { windows, .. } => Some(windows),
                SessionState::Stopped => None,
            },
        }
    }
}
pub enum SessionKind {
    Ephemoral { windows: Vec<WindowId> },
    SysdBased(SysdSession),
}
pub struct SysdSession {
    name: UnitName,
    state: SessionState,
    enabled: bool,
}

impl SessionState {
    fn running(windows: Vec<WindowId>, run: RunId) -> Self {
        SessionState::Running {
            windows,
            run,
            tracked_exit: None,
        }
    }
}

impl SysdSession {
    pub fn unit(&self) -> &UnitName {
        &self.name
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, SessionState::Running { .. })
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Records that a tracked command of the current run exited with `code`.
    /// Only the first exit of a run counts.
    pub fn record_tracked_exit(&mut self, code: i32) {
        if let SessionState::Running { tracked_exit, .. } = &mut self.state {
            tracked_exit.get_or_insert(code);
        }
    }

    pub fn get_sysd_data(&self) -> SysdData {
        SysdData {
            enabled: self.enabled,
            running: self.is_running(),
        }
    }
}

/// Identifies one start of a sysd session, so a waiter from a previous run
/// never mistakes the next run's state for its own.
#[derive(
    Debug, Clone, Copy, Eq, PartialEq, Hash, Encode, Decode, Serialize, serde::Deserialize,
)]
pub struct RunId(pub Guid);

pub enum SessionState {
    Running {
        windows: Vec<WindowId>,
        run: RunId,
        /// Exit code of the first tracked command that exited during this run.
        tracked_exit: Option<i32>,
    },
    Stopped, // TODO Handle possible exit codes later
}

/// Longest session name accepted. Well under systemd's 255-byte unit name limit.
pub const MAX_SESSION_NAME_LEN: usize = 100;

/// Errors unless `name` can be used as a session name.
///
/// Session names become file names (`<name>.yaml`, `<name>.service`) and systemd unit
/// names, and are passed around on command lines, so only a conservative set is allowed:
/// ASCII letters, digits, `-`, `_` and `.`, not starting with `.` or `-`. That rules out
/// path separators, `..`, whitespace and anything a shell or unit file would interpret.
pub fn validate_session_name(name: &str) -> Result<()> {
    let Some(first) = name.chars().next() else {
        bail!("Session name must not be empty");
    };
    if name.len() > MAX_SESSION_NAME_LEN {
        bail!("Session name '{name}' is too long (at most {MAX_SESSION_NAME_LEN} characters)");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        bail!(
            "Session name '{name}' contains {bad:?}: use only letters, digits, '-', '_' and '.'"
        );
    }
    if matches!(first, '.' | '-') {
        bail!("Session name '{name}' must not start with '{first}'");
    }
    Ok(())
}

/// A sysd session's systemd unit name, without `.service`: a valid session name
/// (see `validate_session_name`), lowercased.
///
/// This only guarantees the name is safe to use in file names and unit files. Whether
/// systemd accepts it, and whether a unit by that name already exists, only systemd can
/// say: see `SystemdClient::ensure_unit_name_free`, which is asked before a service is made.
#[derive(Debug, Clone, Eq, Hash, PartialEq, Encode, Decode)]
pub struct UnitName(String);

impl UnitName {
    pub fn new(name: &str) -> Result<Self> {
        validate_session_name(name)?;
        Ok(UnitName(name.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for UnitName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

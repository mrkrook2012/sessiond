use std::collections::HashMap;

use color_eyre::eyre::{Ok, OptionExt, Result, bail, eyre};

use crate::{
    ids::{GuidGenerator, RunId, SessionId, WindowId},
    backend::name_match::similar_names,
    backend::systemd::UnitName,
    ipc::protocol::{SessionLookup, SysdData},
};

pub struct Sessions {
    /// Source of both `SessionId`s and `RunId`s; every start of a sysd session gets a fresh run.
    guids: GuidGenerator,
    sessions: HashMap<SessionId, Session>,
}

impl Sessions {
    pub fn new() -> Self {
        Self {
            guids: GuidGenerator::new(),
            sessions: HashMap::new(),
        }
    }

    pub fn remove_window(&mut self, id: SessionId, window_id: WindowId) -> Result<()> {
        let session = self
            .get_mut(id)
            .ok_or(eyre!("Session not found"))?;
        let windows = session
            .windows_mut()
            .ok_or_eyre(eyre!("Session did not have windows"))?;

        windows.retain(|&id| id != window_id);
        Ok(())
    }

    pub fn windows(&self, id: SessionId) -> Option<&Vec<WindowId>> {
        let session = self.get(id);
        match session {
            Some(session) => return session.windows(),
            None => None,
        }
    }

    pub fn contains(&self, id: SessionId) -> bool {
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
            .find(|name| !self.name_exists(name))
            .expect("some session-N name is free")
    }

    pub fn create_ephemeral(&mut self, windows: Vec<WindowId>, name: String) -> Result<SessionId> {
        self.check_new_name(&name)?;
        let id = self.generate_id();
        let session = Session {
            id,
            name,
            kind: SessionKind::Ephemeral { windows },
        };
        self.sessions.insert(id, session);
        Ok(id)
    }

    pub fn create_running_sysd(&mut self, name: UnitName, windows: Vec<WindowId>) -> Result<RunId> {
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

    pub fn create_stopped_sysd(&mut self, name: UnitName, enabled: bool) -> Result<SessionId> {
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
                SessionKind::Ephemeral { .. } => None,
            })
            .collect()
    }

    /// Finds the sysd session backed by this unit.
    pub fn sysd_by_unit_mut(&mut self, unit: &UnitName) -> Option<(SessionId, &mut SysdSession)> {
        self.sessions.values_mut().find_map(|session| match &mut session.kind {
            SessionKind::SysdBased(sysd) if sysd.unit() == unit => Some((session.id, sysd)),
            _ => None,
        })
    }

    /// The session that owns this window.
    pub fn id_by_window(&self, window_id: WindowId) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|session| {
                session
                    .windows()
                    .is_some_and(|windows| windows.contains(&window_id))
            })
            .map(|session| session.id)
    }

    /// True if a session has this name, ignoring case (names are unique ignoring case).
    pub fn name_exists(&self, name: &str) -> bool {
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

    pub fn id_by_name(&self, name: &str) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|session| session.name.eq_ignore_ascii_case(name))
            .map(|session| session.id)
    }

    /// Marks a stopped sysd session as running with these windows. The caller must have
    /// torn down any previous windows.
    pub fn start_sysd(&mut self, id: SessionId, windows: Vec<WindowId>) -> Result<RunId> {
        let run = self.next_run();
        let session = self
            .get_mut(id)
            .ok_or_eyre("Session not found")?;

        match &mut session.kind {
            SessionKind::Ephemeral { .. } => {
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

    pub fn stop_sysd(&mut self, id: SessionId) -> Result<()> {
        let session = self
            .get_mut(id)
            .ok_or_eyre("Session not found")?;

        match &mut session.kind {
            SessionKind::Ephemeral { .. } => {
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
    pub fn add_window(&mut self, id: SessionId, window_id: WindowId) -> Result<()> {
        let session = self
            .get_mut(id)
            .ok_or(eyre!("Session not found"))?;
        let windows = session
            .windows_mut()
            .ok_or_eyre(eyre!("Session did not have windows"))?;

        windows.push(window_id);
        Ok(())
    }

    pub fn get(&self, id: SessionId) -> Option<&Session> {
        self.sessions.get(&id)
    }

    pub fn get_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.get_mut(&id)
    }
    pub fn list(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn remove(&mut self, id: SessionId) {
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
    pub fn windows(&self) -> Option<&Vec<WindowId>> {
        match &self.kind {
            SessionKind::Ephemeral { windows } => Some(windows),
            SessionKind::SysdBased(session) => match &session.state {
                SessionState::Running { windows, .. } => Some(windows),
                SessionState::Stopped => None,
            },
        }
    }

    pub fn windows_mut(&mut self) -> Option<&mut Vec<WindowId>> {
        match &mut self.kind {
            SessionKind::Ephemeral { windows } => Some(windows),
            SessionKind::SysdBased(session) => match &mut session.state {
                SessionState::Running { windows, .. } => Some(windows),
                SessionState::Stopped => None,
            },
        }
    }
}
pub enum SessionKind {
    Ephemeral { windows: Vec<WindowId> },
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

    pub fn sysd_data(&self) -> SysdData {
        SysdData {
            enabled: self.enabled,
            running: self.is_running(),
        }
    }
}

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

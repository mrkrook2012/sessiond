use bincode_next::{Decode, Encode};
use serde::Serialize;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    path::{Path, PathBuf},
    process::Command,
    vec,
};

use color_eyre::eyre::{Ok, OptionExt, Result, bail, eyre};
use mk_utils::AssignOps;

use crate::{
    backend::{
        window_manager::WindowId,
        yaml::{self, models::YamlSession},
    },
    ipc::protocol::SysdData,
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
    AssignOps,
    Encode,
    Decode,
    Serialize,
    serde::Deserialize,
)]
pub struct SessionId(pub u64);

pub struct SessionManager {
    last_id: SessionId,
    free_ids: BinaryHeap<Reverse<SessionId>>,
    sessions: HashMap<SessionId, Session>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            last_id: SessionId(0),
            free_ids: BinaryHeap::new(),
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

    pub fn create_ephem_session(&mut self, window_id: Vec<WindowId>, name: String) -> SessionId {
        let id = self.generate_id();
        let session = Session {
            id,
            name: name,
            kind: SessionKind::Ephemoral { windows: window_id },
        };
        self.sessions.insert(id, session);
        id
    }

    pub fn create_running_sysd_session(
        &mut self,
        name: UnitName,
        windows: Vec<WindowId>,
    ) -> SessionId {
        let id = self.generate_id();
        let session = Session {
            id,
            name: name.to_string(),
            kind: SessionKind::SysdBased(SysdSession {
                name,
                state: SessionState::Running { windows },
                enabled: false,
            }),
        };
        self.sessions.insert(id, session);
        id
    }

    pub fn create_stopped_sysd_session(&mut self, name: UnitName) -> SessionId {
        let id = self.generate_id();
        let session = Session {
            id,
            name: name.to_string(),
            kind: SessionKind::SysdBased(SysdSession {
                name,
                state: SessionState::Stopped,
                enabled: false,
            }),
        };
        self.sessions.insert(id, session);
        id
    }

    pub fn does_session_name_exist(&self, name: &str) -> bool {
        self.sessions.values().any(|session| session.name == name)
    }

    pub fn get_id_by_name(&self, name: &str) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|session| session.name.eq_ignore_ascii_case(name))
            .map(|session| session.id)
    }

    // Only used on dormant sessions
    pub fn start_sysd_session(&mut self, id: SessionId, windows: Vec<WindowId>) -> Result<()> {
        let session = self
            .get_session_by_id_mut(id)
            .ok_or_eyre("Session not found")?;

        match &mut session.kind {
            SessionKind::Ephemoral { .. } => {
                bail!("Tried to start a non sysd session")
            }
            SessionKind::SysdBased(sysd_session) => {
                sysd_session.state = SessionState::Running { windows };
            }
        }

        Ok(())
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
        self.free_ids.push(Reverse(id));
        self.sessions.remove(&id);
    }

    fn generate_id(&mut self) -> SessionId {
        if let Some(Reverse(id)) = self.free_ids.pop() {
            id
        } else {
            self.last_id += SessionId(1);
            self.last_id
        }
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
                SessionState::Running { windows } => Some(windows),
                SessionState::Stopped => None,
            },
        }
    }

    pub fn get_windows_mut(&mut self) -> Option<&mut Vec<WindowId>> {
        match &mut self.kind {
            SessionKind::Ephemoral { windows } => Some(windows),
            SessionKind::SysdBased(session) => match &mut session.state {
                SessionState::Running { windows } => Some(windows),
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

impl SysdSession {
    // TODO improve this by using the actual commands
    // This code wont change the state, will be done temp by start-sysd command
    pub fn start(&mut self) {
        Command::new("systemctl")
            .arg("--user")
            .arg("start")
            .arg(format!("{}.service", self.name.to_string()))
            .output()
            .unwrap();
    }

    pub fn stop(&mut self) {
        Command::new("systemctl")
            .arg("--user")
            .arg("start")
            .arg(format!("{}.service", self.name.to_string()))
            .output()
            .unwrap();
    }
    pub fn restart(&mut self) {
        Command::new("systemctl")
            .arg("--user")
            .arg("restart")
            .arg(format!("{}.service", self.name.to_string()))
            .output()
            .unwrap();
    }

    pub fn enable(&mut self) {
        Command::new("systemctl")
            .arg("--user")
            .arg("enable")
            .arg(format!("{}.service", self.name.to_string()))
            .output()
            .unwrap();
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        Command::new("systemctl")
            .arg("--user")
            .arg("disable")
            .arg(format!("{}", self.name.to_string()))
            .output()
            .unwrap();
        self.enabled = false;
    }
    pub fn get_sysd_data(&self) -> SysdData {
        SysdData {
            enabled: self.enabled,
            running: matches!(self.state, SessionState::Running { windows: _ }),
        }
    }
}

pub enum SessionState {
    Running { windows: Vec<WindowId> },
    Stopped, // TODO Handle possible exit codes later
}

// Unit name struct that has to be a valid unit name
#[derive(Debug, Clone, Eq, Hash, PartialEq, Encode, Decode)]
pub struct UnitName(String);

impl UnitName {
    pub fn new(name: &str) -> Result<Self> {
        // TODO add actual verification here.
        Ok(UnitName(name.to_string().to_ascii_lowercase()))
    }

    pub fn to_string(&self) -> String {
        self.0.clone()
    }
}

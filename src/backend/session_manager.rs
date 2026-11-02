use bincode_next::{Decode, Encode};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    path::{Path, PathBuf},
    vec,
};

use color_eyre::eyre::{Ok, OptionExt, Result, eyre};
use mk_utils::AssignOps;

use crate::backend::{
    window_manager::WindowId,
    yaml::{self, models::YamlSession},
};

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd, AssignOps, Encode, Decode)]
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

    pub fn remove_window_from_session(
        &mut self,
        id: SessionId,
        window_id: WindowId,
    ) -> Result<()> {
        let session = self
            .get_session_by_id_mut(id)
            .ok_or(eyre!("Session not found"))?;
        let windows = session
            .get_windows_mut()
            .ok_or_eyre(eyre!("Session did not have windows"))?;

        windows.retain(|&id| id != window_id);
        Ok(())
    }

    pub fn get_name_by_id(&self, _id: SessionId) -> String {
        "VERY_RANDOM_name".to_string() // TODO add this when needed
    }

    pub fn get_all_windows_from_session(&self, id: SessionId) -> Option<&Vec<WindowId>> {
        let session = self.get_session_by_id(id);
        match session {
            Some(session) => return session.get_windows(),
            None => todo!(),
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
            kind: SessionKind::Ephemoral {
                windows: window_id,
            },
        };
        self.sessions.insert(id, session);
        id
    }

    // TODO pub fn append_to_session maybe in IPC API
    pub fn add_window_to_session(
        &mut self,
        id: SessionId,
        window_id: WindowId,
    ) -> Result<()> {
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

    fn get_session_by_id_mut(&mut self, id: SessionId) -> Option<&mut Session> {
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
    pub fn start(&mut self) {
        // ...
    }

    pub fn stop(&mut self) {
        // ...
    }

    pub fn enable(&mut self) {
        // ...
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
        Ok(UnitName(name.to_string()))
    }

    pub fn to_string(&self) -> String {
        self.0.clone()
    }
}

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    vec,
};
use bincode_next::{Decode, Encode};

use mk_utils::AssignOps;

use crate::backend::window_manager::WindowId;

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

    pub fn get_window_ids_by_session_id(&self, session_id: SessionId) -> Vec<WindowId> {
        let session = self.sessions.get(&session_id).unwrap(); //TODO
        session.windows.clone()
    }

    pub fn remove_window_from_session(&mut self, window_id: WindowId) {
        let session_id = self.get_session_id_from_window(window_id).unwrap(); // TODO
        self.sessions
            .get_mut(&session_id)
            .unwrap()
            .windows
            .retain(|&id| id != window_id)
    }

    pub fn get_session_name_by_id(&self, _session_id: SessionId) -> String {
        "VERY_RANDOM_SESSION_NAME".to_string() // TODO add this when needed
    }
    pub fn get_all_windows_from_session(&self, session_id: SessionId) -> Vec<WindowId> {
        self.get_session_by_id(session_id).windows.clone()
    }

    pub fn is_session_empty(&self, session_id: SessionId) -> bool {
        self.get_all_windows_from_session(session_id).is_empty()
    }

    pub fn does_session_exist(&self, session_id: SessionId) -> bool {
        self.sessions.contains_key(&session_id)
    }

    pub fn create_session(&mut self, window_id: WindowId, session_name: String) -> SessionId {
        let id = self.generate_id();
        self.sessions.insert(
            id,
            Session {
                session_id: id,
                session_name,
                windows: vec![window_id],
            },
        );
        id
    }

    pub fn create_empty_session(&mut self, session_name: String) -> SessionId {
        let id = self.generate_id();
        self.sessions.insert(
            id,
            Session {
                session_id: id,
                session_name,

                windows: vec![],
            },
        );
        id
    }
    pub fn get_session_id_from_window(&self, window_id: WindowId) -> Option<SessionId> {
        for (id, session) in &self.sessions {
            if session.windows.contains(&window_id) {
                return Some(*id);
            }
        }
        return None;
    }
    pub fn add_window_to_session(&mut self, session_id: SessionId, window_id: WindowId) {
        let session = self.get_session_by_id_mut(session_id);
        session.windows.push(window_id);
    }

    pub fn get_session_by_id(&self, session_id: SessionId) -> &Session {
        self.sessions.get(&session_id).unwrap()
    }

    fn get_session_by_id_mut(&mut self, session_id: SessionId) -> &mut Session {
        self.sessions.get_mut(&session_id).unwrap()
    }
    pub fn list_sessions(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn kill_session(&mut self, session_id: SessionId) {
        self.free_ids.push(Reverse(session_id));
        self.sessions.remove(&session_id).unwrap();
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
    pub session_id: SessionId,
    pub session_name: String,
    pub windows: Vec<WindowId>, 
}

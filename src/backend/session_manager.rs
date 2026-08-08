use crate::ipc::protocol::SessionInfo;
use std::{collections::HashMap, vec};

pub struct SessionManager {
    last_id: u64,
    sessions: HashMap<u64, Session>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            last_id: 0,
            sessions: HashMap::new(),
        }
    }

    pub fn get_window_ids_by_session_id(&self, session_id: u64) -> Vec<u64> {
        let session = self.sessions.get(&session_id).unwrap(); //TODO
        session.windows.clone()
    }
    pub fn create_session(&mut self, window_id: u64, session_name: String) -> u64 {
        self.last_id += 1;

        self.sessions.insert(
            self.last_id.clone(),
            Session {
                session_id: self.last_id,
                session_name,
                windows: vec![window_id],
            },
        );
        self.last_id
    }

    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions
            .iter()
            .map(|(id, session)| SessionInfo::new(*id, session.session_name.clone()))
            .collect()
    }

    pub fn kill_session(&mut self, session_id: u64) {
        self.sessions.remove(&session_id).unwrap();
    }
}

pub struct Session {
    session_id: u64,
    session_name: String,
    windows: Vec<u64>, // window_id
}

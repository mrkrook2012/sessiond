use crate::{
    backend::window_manager::WindowManager,
    ipc::protocol::{SessionInfo, WindowInfo},
};
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

    pub fn remove_window_from_session(&mut self, window_id: u64) {
        let session_id = self.get_session_id_from_window(window_id);
        self.sessions
            .get_mut(&session_id)
            .unwrap()
            .windows
            .retain(|&id| id != window_id)
    }

    pub fn get_session_name_by_id(&self, session_id: u64) -> String {
        "Penar".to_string()
    }
    pub fn get_all_windows_from_session(&self, session_id: u64) -> Vec<u64> {
        self.get_session_by_id(session_id).windows.clone()
    }

    pub fn does_session_exist(&self, session_id: u64) -> bool {
        self.sessions.contains_key(&session_id)
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
    pub fn get_session_id_from_window(&self, window_id: u64) -> u64 {
        for (id, session) in &self.sessions {
            if session.windows.contains(&window_id) {
                return *id;
            }
        }
        return 0;
    }
    pub fn add_window_to_session(&mut self, session_id: u64, window_id: u64) {
        let session = self.get_session_by_id_mut(session_id);
        session.windows.push(window_id);
    }

    pub fn get_session_by_id(&self, session_id: u64) -> &Session {
        self.sessions.get(&session_id).unwrap()
    }

    fn get_session_by_id_mut(&mut self, session_id: u64) -> &mut Session {
        self.sessions.get_mut(&session_id).unwrap()
    }
    pub fn list_sessions(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn kill_session(&mut self, session_id: u64) {
        self.sessions.remove(&session_id).unwrap();
    }
}

pub struct Session {
    pub session_id: u64,
    pub session_name: String,
    pub windows: Vec<u64>, // window_id
}

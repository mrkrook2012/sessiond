use std::collections::HashMap;

use crate::{
    backend::{
        window::{self, Window},
        window_manager,
    },
    ipc::protocol::WindowInfo,
};

pub struct WindowManager {
    last_id: u64,
    windows: HashMap<u64, Window>, //window_id
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            last_id: 0,
            windows: HashMap::new(),
        }
    }

    pub fn create_window(&mut self, terminal_id: u64) -> u64 {
        self.last_id += 1;
        let window = Window::new(terminal_id, self.last_id);
        self.windows.insert(self.last_id.clone(), window);
        self.last_id
    }

    pub fn kill_window(&mut self, window_id: u64) {
        self.windows.remove(&window_id).unwrap();
    }

    pub fn get_window_id_from_terminal(&self, terminal_id: u64) -> u64 {
        for (id, window) in &self.windows {
            if window.terminal_id == terminal_id {
                return *id;
            }
        }
        0
    }

    pub fn get_terminal_id_from_window_id(&self, window_id: u64) -> u64 {
        self.windows.get(&window_id).unwrap().terminal_id
    }

    pub fn get_window_info_by_id(&self, window_id: u64) -> WindowInfo {
        let window = self.get_window_by_id(window_id);
        window.get_window_info()
    }

    pub fn get_window_by_id(&self, window_id: u64) -> &Window {
        self.windows.get(&window_id).unwrap()
    }
    pub fn get_window_by_id_mut(&mut self, window_id: u64) -> &mut Window {
        self.windows.get_mut(&window_id).unwrap()
    }
}

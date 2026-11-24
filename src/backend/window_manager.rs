use std::collections::HashMap;

use bincode_next::{Decode, Encode};
use serde::Serialize;

use crate::{
    backend::{
        guid::{Guid, GuidManager},
        terminal_manager::TerminalId,
        window::Window,
    },
    ipc::protocol::WindowInfo,
};

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd, Encode, Decode, Serialize, serde::Deserialize)]
pub struct WindowId(pub Guid);

pub struct WindowManager {
    guids: GuidManager,
    windows: HashMap<WindowId, Window>,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            guids: GuidManager::new(),
            windows: HashMap::new(),
        }
    }

    pub fn create_window(&mut self, terminal_id: TerminalId) -> WindowId {
        self.create_window_with_tracking(terminal_id, false)
    }

    pub fn create_window_with_tracking(&mut self, terminal_id: TerminalId, tracked: bool) -> WindowId {
        let id = WindowId(self.guids.generate());
        let window = Window::new(terminal_id, id, tracked);
        self.windows.insert(id, window);
        id
    }

    pub fn kill_window(&mut self, window_id: WindowId) {
        self.windows.remove(&window_id).unwrap();
    }

    pub fn get_window_id_from_terminal(&self, terminal_id: TerminalId) -> Option<WindowId> {
        for (id, window) in &self.windows {
            if window.terminal_id == terminal_id {
                return Some(*id);
            }
        }
        None
    }

    pub fn get_terminal_id_from_window_id(&self, window_id: &WindowId) -> TerminalId {
        self.windows.get(&window_id).unwrap().terminal_id
    }

    pub fn get_window_info_by_id(&self, window_id: &WindowId) -> WindowInfo {
        let window = self.get_window_by_id(window_id);
        window.get_window_info()
    }

    pub fn get_window_by_id(&self, window_id: &WindowId) -> &Window {
        self.windows.get(&window_id).unwrap()
    }
    pub fn get_window_by_id_mut(&mut self, window_id: &WindowId) -> &mut Window {
        self.windows.get_mut(&window_id).unwrap()
    }
}

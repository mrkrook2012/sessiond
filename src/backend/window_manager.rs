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

    /// Removes a window and returns it, or `None` if there is no such window.
    pub fn kill_window(&mut self, window_id: WindowId) -> Option<Window> {
        self.windows.remove(&window_id)
    }

    pub fn get_window_id_from_terminal(&self, terminal_id: TerminalId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, window)| window.terminal_id == terminal_id)
            .map(|(id, _)| *id)
    }

    pub fn get_terminal_id_from_window_id(&self, window_id: &WindowId) -> Option<TerminalId> {
        self.windows.get(window_id).map(|window| window.terminal_id)
    }

    pub fn get_window_info_by_id(&self, window_id: &WindowId) -> Option<WindowInfo> {
        self.get_window_by_id(window_id).map(Window::get_window_info)
    }

    pub fn get_window_by_id(&self, window_id: &WindowId) -> Option<&Window> {
        self.windows.get(window_id)
    }

    pub fn get_window_by_id_mut(&mut self, window_id: &WindowId) -> Option<&mut Window> {
        self.windows.get_mut(window_id)
    }
}

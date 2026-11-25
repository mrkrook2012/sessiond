use std::collections::HashMap;

use bincode_next::{Decode, Encode};
use serde::Serialize;

use crate::{
    backend::{
        guid::{Guid, GuidManager},
        terminal::TerminalId,
    },
    ipc::protocol::WindowInfo,
};

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd, Encode, Decode, Serialize, serde::Deserialize)]
pub struct WindowId(pub Guid);

pub struct Windows {
    guids: GuidManager,
    windows: HashMap<WindowId, Window>,
}

impl Windows {
    pub fn new() -> Self {
        Self {
            guids: GuidManager::new(),
            windows: HashMap::new(),
        }
    }

    pub fn create(&mut self, terminal_id: TerminalId) -> WindowId {
        self.create_with_tracking(terminal_id, false)
    }

    pub fn create_with_tracking(&mut self, terminal_id: TerminalId, tracked: bool) -> WindowId {
        let id = WindowId(self.guids.generate());
        let window = Window::new(terminal_id, id, tracked);
        self.windows.insert(id, window);
        id
    }

    /// Removes a window and returns it, or `None` if there is no such window.
    pub fn remove(&mut self, window_id: WindowId) -> Option<Window> {
        self.windows.remove(&window_id)
    }

    pub fn id_by_terminal(&self, terminal_id: TerminalId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, window)| window.terminal_id == terminal_id)
            .map(|(id, _)| *id)
    }

    pub fn terminal_of(&self, window_id: &WindowId) -> Option<TerminalId> {
        self.windows.get(window_id).map(|window| window.terminal_id)
    }

    pub fn info(&self, window_id: &WindowId) -> Option<WindowInfo> {
        self.get(window_id).map(Window::info)
    }

    pub fn get(&self, window_id: &WindowId) -> Option<&Window> {
        self.windows.get(window_id)
    }

    pub fn get_mut(&mut self, window_id: &WindowId) -> Option<&mut Window> {
        self.windows.get_mut(window_id)
    }
}

pub struct Window {
    pub terminal_id: TerminalId,
    window_id: WindowId,
    /// If set, the command this window was started with is watched: when it exits on its
    /// own, the systemd session it belongs to is restarted.
    pub tracked: bool,
}

impl Window {
    pub fn new(terminal_id: TerminalId, window_id: WindowId, tracked: bool) -> Self {
        Self {
            terminal_id,
            window_id,
            tracked,
        }
    }

    pub fn info(&self) -> WindowInfo {
        WindowInfo::new(self.window_id, self.terminal_id)
    }
}

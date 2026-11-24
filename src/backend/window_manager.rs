use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
};

use bincode_next::{Decode, Encode};
use mk_utils::AssignOps;
use serde::Serialize;

use crate::{
    backend::{terminal_manager::TerminalId, window::Window},
    ipc::protocol::WindowInfo,
};

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd, AssignOps, Encode, Decode, Serialize, serde::Deserialize)]
pub struct WindowId(u64);

pub struct WindowManager {
    last_id: WindowId,
    free_ids: BinaryHeap<Reverse<WindowId>>,
    windows: HashMap<WindowId, Window>, //window_id
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            last_id: WindowId(0),
            free_ids: BinaryHeap::new(),
            windows: HashMap::new(),
        }
    }

    pub fn create_window(&mut self, terminal_id: TerminalId) -> WindowId {
        self.create_window_with_tracking(terminal_id, false)
    }

    pub fn create_window_with_tracking(&mut self, terminal_id: TerminalId, tracked: bool) -> WindowId {
        let id = self.generate_id();
        let window = Window::new(terminal_id, id, tracked);
        self.windows.insert(id, window);
        id
    }

    pub fn kill_window(&mut self, window_id: WindowId) {
        self.free_ids.push(Reverse(window_id));
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

    fn generate_id(&mut self) -> WindowId {
        if let Some(Reverse(id)) = self.free_ids.pop() {
            id
        } else {
            self.last_id += WindowId(1);
            self.last_id
        }
    }
}

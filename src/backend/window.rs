use crate::{backend::{terminal_manager::TerminalId, window_manager::WindowId}, ipc::protocol::WindowInfo};

pub struct Window {
    pub terminal_id: TerminalId, // terminal_id
    window_id: WindowId,
}

impl Window {
    pub fn new(terminal_id: TerminalId, window_id: WindowId) -> Self {
        Self {
            terminal_id,
            window_id,
        }
    }

    pub fn get_window_info(&self) -> WindowInfo {
        WindowInfo::new(self.window_id, self.terminal_id)
    }
}

use crate::ipc::protocol::WindowInfo;

pub struct Window {
    pub terminal_id: u64, // terminal_id
    window_id: u64,
}

impl Window {
    pub fn new(terminal_id: u64, window_id: u64) -> Self {
        Self {
            terminal_id,
            window_id,
        }
    }

    pub fn get_window_info(&self) -> WindowInfo {
        WindowInfo::new(self.window_id, self.terminal_id)
    }
}

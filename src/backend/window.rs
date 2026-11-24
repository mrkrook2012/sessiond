use crate::{
    backend::{terminal_manager::TerminalId, window_manager::WindowId},
    ipc::protocol::WindowInfo,
};

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

    pub fn get_window_info(&self) -> WindowInfo {
        WindowInfo::new(self.window_id, self.terminal_id)
    }
}

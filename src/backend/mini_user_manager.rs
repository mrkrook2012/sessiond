use crate::backend::{session_manager::SessionId, terminal_manager::TerminalId};

pub struct MiniUserManager {
    rows: u16,
    cols: u16,
    current_id: SessionId,
    visible_terminals: Vec<TerminalId>, 
}

impl MiniUserManager {
    pub fn new() -> Self {
        Self {
            rows: 100,
            cols: 100,
            current_id: SessionId(1),
            visible_terminals: Vec::new(),
        }
        
    }

    pub fn get_dimensions(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    pub fn update_dimensions(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
    }
    pub fn add_user(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
    }
    pub fn add_terminal(&mut self, terminal_id: TerminalId) {
        self.visible_terminals.push(terminal_id);
    }

    pub fn remove_terminal(&mut self, terminal_id: TerminalId) {
        if let Some(pos) = self
            .visible_terminals
            .iter()
            .position(|&x| x == terminal_id)
        {
            self.visible_terminals.remove(pos);
        }
    }
    pub fn get_current_id(&self) -> SessionId {
        self.current_id
    }

    pub fn set_current_id(&mut self, id: SessionId) {
        self.current_id = id
    }

    pub fn clear_all(&mut self) {
        self.visible_terminals.clear()
    }

    pub fn get_all_terminals(&self) -> Vec<TerminalId> {
        self.visible_terminals.clone()
    }
}

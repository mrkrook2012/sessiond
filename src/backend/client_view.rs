use crate::backend::{guid::Guid, session::SessionId, terminal::TerminalId};

/// What the attached client is looking at: its screen size, the session it has open
/// and the terminals currently visible to it.
pub struct ClientView {
    rows: u16,
    cols: u16,
    current_session: SessionId,
    visible_terminals: Vec<TerminalId>,
}

impl ClientView {
    pub fn new() -> Self {
        Self {
            rows: 100,
            cols: 100,
            // Placeholder until a session is opened; never a real session.
            current_session: SessionId(Guid::NIL),
            visible_terminals: Vec::new(),
        }
    }

    pub fn dimensions(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    pub fn set_dimensions(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
    }

    pub fn show_terminal(&mut self, terminal_id: TerminalId) {
        self.visible_terminals.push(terminal_id);
    }

    pub fn hide_terminal(&mut self, terminal_id: TerminalId) {
        if let Some(pos) = self
            .visible_terminals
            .iter()
            .position(|&x| x == terminal_id)
        {
            self.visible_terminals.remove(pos);
        }
    }

    pub fn current_session(&self) -> SessionId {
        self.current_session
    }

    pub fn set_current_session(&mut self, id: SessionId) {
        self.current_session = id
    }

    pub fn hide_all(&mut self) {
        self.visible_terminals.clear()
    }

    pub fn visible_terminals(&self) -> &[TerminalId] {
        &self.visible_terminals
    }
}

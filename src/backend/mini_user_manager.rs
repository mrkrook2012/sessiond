pub struct MiniUserManager {
    rows: u16,
    cols: u16,
    current_session: u64,
    visible_terminals: Vec<u64>, //Terminal Id
}

impl MiniUserManager {
    pub fn new() -> Self {
        Self {
            rows: 0,
            cols: 0,
            current_session: 0,
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
    pub fn add_terminal(&mut self, terminal_id: u64) {
        self.visible_terminals.push(terminal_id);
    }

    pub fn remove_terminal(&mut self, terminal_id: u64) {
        if let Some(pos) = self
            .visible_terminals
            .iter()
            .position(|&x| x == terminal_id)
        {
            self.visible_terminals.remove(pos);
        }
    }

    pub fn get_all_terminals(&self) -> Vec<u64> {
        self.visible_terminals.clone()
    }
}

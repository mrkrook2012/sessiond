use crate::{backend::tree::Node, ipc::protocol::WindowInfo};

pub struct Window {
    pub terminals: Vec<u64>, // terminal_id
    window_id: u64,
    root: Node,
}

impl Window {
    pub fn new(terminal_id: u64, window_id: u64) -> Self {
        let mut terminals = Vec::new();
        terminals.push(terminal_id);
        Self {
            terminals,
            window_id,
            root: Node::Leaf(terminal_id),
        }
    }

    pub fn get_window_layout(&self) -> Node {
        self.root.clone()
    }

    pub fn get_window_info(&self) -> WindowInfo {
        WindowInfo::new(self.window_id, "Penar".to_string(), self.root.clone())
    }
}

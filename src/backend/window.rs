use crate::{backend::tree::Node, ipc::protocol::WindowInfo};

pub struct Window {
    pub terminals: Vec<u64>, // terminal_id
    window_id: u64,
    name: String,
    root: Node,
}

impl Window {
    pub fn new(terminal_id: u64, window_id: u64, name: String) -> Self {
        let mut terminals = Vec::new();
        terminals.push(terminal_id);
        Self {
            terminals,
            window_id,
            root: Node::Leaf(terminal_id),
            name,
        }
    }

    pub fn get_window_layout(&self) -> Node {
        self.root.clone()
    }

    pub fn get_window_layout_ref_mut(&mut self) -> &mut Node {
        &mut self.root
    }

    pub fn get_window_info(&self) -> WindowInfo {
        WindowInfo::new(self.window_id, self.name.clone(), self.root.clone())
    }

    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

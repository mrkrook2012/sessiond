use crate::backend::tree::Node;

pub struct Window {
    terminals: Vec<u64>,
    root: Node,
}

impl Window {
    pub fn new(terminal_id: u64) -> Self {
        let mut terminal_vec = Vec::new();
        terminal_vec.push(terminal_id);
        Self {
            terminals: terminal_vec,
            root: Node::Leaf(terminal_id),
        }
    }
}

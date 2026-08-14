// Original will always move left.

use bincode_next::{Decode, Encode};

#[derive(Debug, Encode, Decode, Clone)]
pub enum Node {
    Leaf(u64),
    Split {
        direction: SplitDirection,
        children: Vec<(Node, f32)>,
    },
}

impl Node {
    pub fn terminal_ids(&self) -> Vec<u64> {
        match self {
            Node::Leaf(id) => vec![*id],
            Node::Split { children, .. } => children
                .iter()
                .flat_map(|(child, _)| child.terminal_ids())
                .collect(),
        }
    }
}

#[derive(Decode, Encode, Debug, Clone)]
pub enum SplitDirection {
    Horizontal, //Side by side panels
    Vertical,
}

// Original will always move left.

use bincode_next::{Decode, Encode};

use crate::backend::{terminal_manager::TerminalManager, tree::SplitDirection::Horizontal};

#[derive(Debug, Encode, Decode, Clone)]
pub enum Node {
    Leaf(u64), // Leaf/terminal Id
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

    pub fn add_terminal(
        &mut self,
        terminal_manager: &mut TerminalManager,
        leaf_id: u64,
        new_terminal_id: u64,
        direction: SplitDirection,
    ) -> bool {
        match self {
            Node::Leaf(id) if *id == leaf_id => {
                let terminal = terminal_manager.get_terminal_from_id_mut(leaf_id);

                let (mut height, mut width) = terminal.get_ratio();

                match direction {
                    SplitDirection::Horizontal => {
                        width *= 0.5;
                    }
                    SplitDirection::Vertical => {
                        height *= 0.5;
                    }
                }

                terminal.set_ratio(height, width);

                let new_terminal = terminal_manager.get_terminal_from_id_mut(new_terminal_id);

                new_terminal.set_ratio(height, width);

                *self = Node::Split {
                    direction,
                    children: vec![
                        (Node::Leaf(leaf_id), 0.5),
                        (Node::Leaf(new_terminal_id), 0.5),
                    ],
                };

                true
            }

            Node::Leaf(_) => false,

            Node::Split { children, .. } => children.iter_mut().any(|(child, _)| {
                child.add_terminal(
                    terminal_manager,
                    leaf_id,
                    new_terminal_id,
                    direction.clone(),
                )
            }),
        }
    }
    pub fn remove_terminal(
        &mut self,
        terminal_manager: &mut TerminalManager,
        terminal_id: u64,
    ) -> bool {
        match self {
            Node::Leaf(_) => false,

            Node::Split {
                direction,
                children,
            } => {
                // Is the terminal a direct child of this split?
                if let Some(index) = children
                    .iter()
                    .position(|(child, _)| matches!(child, Node::Leaf(id) if *id == terminal_id))
                {
                    children.remove(index);

                    // The split has one child left, so collapse it.
                    if children.len() == 1 {
                        let (child, _) = children.remove(0);

                        if let Node::Leaf(id) = &child {
                            let terminal = terminal_manager.get_terminal_from_id_mut(*id);

                            let (mut height, mut width) = terminal.get_ratio();

                            match direction {
                                SplitDirection::Horizontal => {
                                    width *= 2.0;
                                }
                                SplitDirection::Vertical => {
                                    height *= 2.0;
                                }
                            }

                            terminal.set_ratio(height, width);
                        }

                        *self = child;
                    }

                    return true;
                }

                // Otherwise, recursively search the children.
                for (child, _) in children.iter_mut() {
                    if child.remove_terminal(terminal_manager, terminal_id) {
                        return true;
                    }
                }

                false
            }
        }
    }
    // pub fn split_leaf(&mut self, terminal_manager: &mut TerminalManager, leaf_id: u64, direction: SplitDirection) {
    //     let terminal = terminal_manager.
    //     *self = Node::Split { direction: Horizontal, children: Vec::new() }
    // }
}

#[derive(Decode, Encode, Debug, Clone)]
pub enum SplitDirection {
    Horizontal, //Side by side panels
    Vertical,
}

// Original will always move left.

use bincode_next::{Decode, Encode};

pub enum Node {
    Leaf(u64),
    Split {
        direction: SplitDirection,
        children: Vec<(Node, f32)>,
    },
}

#[derive(Decode, Encode, Debug)]
pub enum SplitDirection {
    Horizontal, //Side by side panels
    Vertical,
}

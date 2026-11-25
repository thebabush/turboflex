//! Target for c08: `IntoIterator for Bag` and the iterator type, in a
//! module separate from `Bag` itself.

use crate::collection::Bag;

pub struct BagIter {
    items: Vec<u32>,
    pos: usize,
}

impl Iterator for BagIter {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        let v = self.items.get(self.pos).copied();
        self.pos += 1;
        v
    }
}

impl IntoIterator for Bag {
    type Item = u32;
    type IntoIter = BagIter;

    fn into_iter(self) -> BagIter {
        BagIter { items: self.items, pos: 0 }
    }
}

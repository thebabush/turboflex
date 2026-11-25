//! Target for c08: operator impls (`Add`, `Index`) for `vector::Vec2`.

use crate::vector::Vec2;
use std::ops::{Add, Index};

impl Add for Vec2 {
    type Output = Vec2;

    fn add(self, o: Vec2) -> Vec2 {
        Vec2 { x: self.x + o.x, y: self.y + o.y }
    }
}

impl Index<usize> for Vec2 {
    type Output = f32;

    fn index(&self, i: usize) -> &f32 {
        if i == 0 { &self.x } else { &self.y }
    }
}

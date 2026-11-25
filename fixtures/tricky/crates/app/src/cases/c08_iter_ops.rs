//! c08: `for` over a user type whose `IntoIterator`/`Iterator` impls are in
//! `app::iteration`; `+` and `[]` whose `Add`/`Index` impls are in
//! `app::vector_ops`. All target calls are implicit.

use crate::collection::Bag;
use crate::vector::Vec2;

pub fn sum_bag(b: Bag) -> u32 {
    let mut total = 0;
    for x in b {
        total += x;
    }
    total
}

pub fn add(a: Vec2, b: Vec2) -> Vec2 {
    a + b
}

pub fn first(v: &Vec2) -> f32 {
    v[0]
}

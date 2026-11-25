//! Target for c02/c23/c24: concrete type with inherent methods.

#[derive(Clone, Copy)]
pub struct Circle {
    pub r: f32,
}

impl Circle {
    pub fn new(r: f32) -> Self {
        Circle { r }
    }

    pub fn area(&self) -> f32 {
        self.r * self.r
    }
}

/// Free fn taking `&Circle`; c02 calls it with `&MyBox<Circle>` (deref coercion).
pub fn measure(c: &Circle) -> f32 {
    c.r
}

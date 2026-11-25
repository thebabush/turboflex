//! c02: inherent method call on a concrete type, and the same call reached
//! through autoderef via `Box`, `Arc`, and a custom `Deref` type
//! (`app::wrappers::MyBox`), plus a deref *coercion* at a fn argument.

use crate::shapes::Circle;
use crate::wrappers::MyBox;
use std::sync::Arc;

pub fn concrete() -> f32 {
    Circle::new(1.0).area()
}

#[allow(clippy::boxed_local)]
pub fn via_box(c: Box<Circle>) -> f32 {
    c.area()
}

pub fn via_arc(c: Arc<Circle>) -> f32 {
    c.area()
}

pub fn via_custom_deref(c: MyBox<Circle>) -> f32 {
    c.area()
}

pub fn deref_coercion(c: &MyBox<Circle>) -> f32 {
    crate::shapes::measure(c)
}

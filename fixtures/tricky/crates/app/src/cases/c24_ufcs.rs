//! c24: fully qualified / UFCS calls — `<Type as Trait>::method`,
//! `Trait::method(&x)`, and `Type::method(&x)` for an inherent method.

use crate::animals::Dog;
use crate::shapes::Circle;

pub fn qualified() -> String {
    <Dog as core_api::Greeter>::greet(&Dog)
}

pub fn trait_path() -> String {
    core_api::Greeter::greet(&Dog)
}

pub fn inherent_path(c: &Circle) -> f32 {
    Circle::area(c)
}

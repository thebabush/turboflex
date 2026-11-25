//! Target for c03/c20/c24: concrete type with a cross-crate trait impl
//! (`core_api::Greeter`) living next to the type.

pub struct Dog;

impl core_api::Greeter for Dog {
    fn greet(&self) -> String {
        String::from("woof")
    }
}

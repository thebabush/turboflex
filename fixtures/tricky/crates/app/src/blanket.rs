//! Target for c20: trait with a blanket impl over a cross-crate bound.

use core_api::Greeter;

pub trait Shout {
    fn shout(&self) -> String;
}

impl<T: Greeter> Shout for T {
    fn shout(&self) -> String {
        self.greet().to_uppercase()
    }
}

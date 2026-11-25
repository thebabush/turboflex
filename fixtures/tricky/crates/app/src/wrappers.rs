//! Target for c02: custom smart pointer with a user `Deref` impl.

use std::ops::Deref;

pub struct MyBox<T>(pub T);

impl<T> Deref for MyBox<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

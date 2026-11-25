//! Target for c04: "module B" — defines the trait.

pub trait Describe {
    fn describe(&self) -> String;
}

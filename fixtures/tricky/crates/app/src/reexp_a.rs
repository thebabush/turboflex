//! Target for c12: the definition site of `Foo`.

pub struct Foo;

impl Foo {
    pub fn make() -> Foo {
        Foo
    }
}

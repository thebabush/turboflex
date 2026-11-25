//! c12: `Foo` is defined in `app::reexp_a` but named here only through the
//! `pub use` facade `app::reexp_b`.

pub fn run() -> crate::reexp_b::Foo {
    crate::reexp_b::Foo::make()
}

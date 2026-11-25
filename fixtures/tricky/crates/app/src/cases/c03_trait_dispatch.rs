//! c03: trait method calls resolved different ways — generic bound, `dyn`,
//! concrete receiver with a known impl, associated type + where-clause, and
//! `impl Trait` in argument and return position.

use crate::animals::Dog;
use core_api::{Greeter, Storage};

pub fn generic_bound<T: Greeter>(t: &T) -> String {
    t.greet()
}

pub fn dyn_dispatch(g: &dyn Greeter) -> String {
    g.greet()
}

pub fn concrete_receiver() -> String {
    Dog.greet()
}

pub fn assoc_type_where<S>(s: &mut S)
where
    S: Storage<Item = Dog>,
{
    s.put(Dog);
}

pub fn impl_arg(g: impl Greeter) -> String {
    g.greet()
}

pub fn impl_return() -> impl Greeter {
    Dog
}

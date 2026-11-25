//! c04: calls trait methods whose impls live in neither the trait's module
//! (`app::traits_b` / crate `core_api`) nor the type's module
//! (`app::model_a`), but in a third module (`app::impls_c`).

use crate::model_a::Widget;
use crate::traits_b::Describe;
use core_api::Greeter;

pub fn run() -> String {
    let w = Widget { id: 3 };
    w.describe() + &w.greet()
}

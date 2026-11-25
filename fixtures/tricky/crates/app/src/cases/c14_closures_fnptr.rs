//! c14: a closure that captures a local and calls `app::callbacks::notify`;
//! and `app::math::triple` reified as a fn pointer and passed to
//! `app::callbacks::apply` (which calls it indirectly).

pub fn with_closure() -> u32 {
    let k = 2;
    let c = |x: u32| crate::callbacks::notify(x + k);
    c(1)
}

pub fn with_fn_pointer() -> u32 {
    let f: fn(u32) -> u32 = crate::math::triple;
    crate::callbacks::apply(f, 3)
}

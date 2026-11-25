//! Target for c14: fn called from a closure, and a higher-order fn taking a
//! fn pointer.

pub fn notify(x: u32) -> u32 {
    x
}

pub fn apply(f: fn(u32) -> u32, x: u32) -> u32 {
    f(x)
}

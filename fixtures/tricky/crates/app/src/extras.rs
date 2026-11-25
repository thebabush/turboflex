//! Target for c16: only called from a `#[cfg(feature = "extra")]` fn.
//! The module itself is NOT gated.

pub fn bonus() -> u32 {
    99
}

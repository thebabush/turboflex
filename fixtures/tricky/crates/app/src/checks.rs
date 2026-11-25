//! Target for c16: only called from a `#[cfg(test)]` module.

pub fn validate(x: u32) -> bool {
    x > 0
}

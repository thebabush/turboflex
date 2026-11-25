//! Cross-crate `#[macro_export]` macro whose expansion calls back into this crate.

pub fn double_it(x: u32) -> u32 {
    x * 2
}

/// Expands to a call to `macro_kit::double_it` at the invocation site.
#[macro_export]
macro_rules! exported_double {
    ($e:expr) => {
        $crate::double_it($e)
    };
}

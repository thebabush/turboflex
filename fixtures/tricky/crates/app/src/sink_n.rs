//! Target for c05: "module N" — the fn called by the expansion of
//! `macros_m::record_via_n!`.

pub fn record(x: u32) -> u32 {
    x + 1
}

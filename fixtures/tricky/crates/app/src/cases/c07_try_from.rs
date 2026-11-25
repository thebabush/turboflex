//! c07: `?` converts `ErrA` (`app::errors_a`) into `ErrB` (`app::errors_b`)
//! via `impl From<ErrA> for ErrB` located in a third module
//! (`app::error_conv`). The `from` call exists only after desugaring.

use crate::errors_b::ErrB;

pub fn run(ok: bool) -> Result<u32, ErrB> {
    let v = crate::errors_a::fallible(ok)?;
    Ok(v)
}

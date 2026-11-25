//! Target for c07: source error type and a fallible fn returning it.

pub struct ErrA;

pub fn fallible(ok: bool) -> Result<u32, ErrA> {
    if ok { Ok(1) } else { Err(ErrA) }
}

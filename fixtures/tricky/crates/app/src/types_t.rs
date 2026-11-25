//! Target for c11: types and an associated const used only in type
//! positions (plus one const read).

pub struct Inner(pub u32);

pub struct Elem(pub u32);

pub struct Limits;

impl Limits {
    pub const MAX: u32 = 100;
}

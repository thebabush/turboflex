//! c11: type-only dependencies — struct field types, a `Vec<_>` generic arg,
//! a type alias from another module (`app::aliases::Id` = `types_t::Inner`),
//! fn signature types, a body-only local type, and an associated const.

use crate::types_t::{Elem, Inner, Limits};

pub struct Holder {
    pub id: crate::aliases::Id,
    pub items: Vec<Elem>,
}

pub fn sig_only(_x: Inner, e: Elem) -> Elem {
    e
}

pub fn body_only() -> usize {
    let v: Vec<Elem> = Vec::new();
    v.len()
}

pub fn assoc_const() -> u32 {
    Limits::MAX
}

//! Target for c13: consts and statics (one with interior mutability).

use std::sync::atomic::AtomicU32;

pub const LIMIT: u32 = 10;

pub static GREETING: &str = "hi";

pub static COUNTER: AtomicU32 = AtomicU32::new(0);

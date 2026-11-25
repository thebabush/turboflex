//! c13: reading a const and a static from `app::config`, and writing to a
//! static with interior mutability (`AtomicU32`) through `&`.

use std::sync::atomic::Ordering;

pub fn read() -> u32 {
    crate::config::LIMIT + crate::config::GREETING.len() as u32
}

pub fn bump() {
    crate::config::COUNTER.fetch_add(1, Ordering::Relaxed);
}

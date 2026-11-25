//! c25: building values of another module's / crate's ADTs — struct literal,
//! tuple-struct constructor, unit/tuple/struct-like enum variants, a variant
//! constructor passed as a fn value, and cross-crate struct and variant.

use crate::payload::{Job, Meters, Point};

pub fn struct_literal() -> i32 {
    let p = Point { x: 1, y: 2 };
    p.x + p.y
}

pub fn tuple_struct() -> u32 {
    let m = Meters(3);
    m.0
}

pub fn variants() -> Vec<Job> {
    vec![Job::Idle, Job::Resize(4), Job::Move { dx: 1, dy: -1 }]
}

pub fn ctor_as_fn(sizes: Vec<u32>) -> Vec<Job> {
    sizes.into_iter().map(Job::Resize).collect()
}

pub fn cross_crate() -> u32 {
    let v = core_api::Version { major: 1 };
    let l = core_api::Level::High(2);
    v.major + matches!(l, core_api::Level::Low) as u32
}

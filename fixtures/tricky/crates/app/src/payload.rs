//! Target for c25/c26: ADTs built (struct literal, tuple struct, every
//! variant shape) and branched on from another module.

pub struct Point {
    pub x: i32,
    pub y: i32,
}

pub struct Meters(pub u32);

/// The derived `PartialEq` matches on `(self, other)`: a `matches_on` edge
/// from the derived `eq` to `Job` itself (derive provenance; same module,
/// so not listed in `expected_edges.tsv`).
#[derive(PartialEq)]
pub enum Job {
    Idle,
    Resize(u32),
    Move { dx: i32, dy: i32 },
}

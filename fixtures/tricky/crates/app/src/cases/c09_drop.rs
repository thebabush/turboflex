//! c09: user `Drop` impl (`app::resources`) triggered only implicitly at
//! scope end; no explicit `drop` call.

pub fn run() -> u32 {
    let g = crate::resources::Guard::new();
    g.id
}

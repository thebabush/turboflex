//! Cross-crate dependency target: traits and a free fn used by `app`.

/// Called directly from `app::cases::c01_direct_call`.
pub fn core_helper() -> u32 {
    7
}

/// Plain object-safe trait, used via generic bound, `dyn`, concrete impl,
/// blanket impl and UFCS in `app`.
pub trait Greeter {
    fn greet(&self) -> String;
}

/// Trait with an associated type, used with a where-clause in `app`.
pub trait Storage {
    type Item;
    fn put(&mut self, item: Self::Item);
}

/// Struct built cross-crate by `app::cases::c25_constructs`.
pub struct Version {
    pub major: u32,
}

/// Enum built and matched on cross-crate by `app::cases::c25_constructs` / `c26_matches`.
pub enum Level {
    Low,
    High(u8),
}

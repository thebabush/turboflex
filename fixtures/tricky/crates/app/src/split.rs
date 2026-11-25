//! Target for c22: type definition plus its first inherent impl block.

pub struct Thing {
    pub n: u32,
}

impl Thing {
    pub fn new() -> Self {
        Thing { n: 1 }
    }
}

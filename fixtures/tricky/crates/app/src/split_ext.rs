//! Target for c22: a *second* inherent impl block for `split::Thing`,
//! in a different module than the type.

impl crate::split::Thing {
    pub fn extra(&self) -> u32 {
        self.n + 1
    }
}

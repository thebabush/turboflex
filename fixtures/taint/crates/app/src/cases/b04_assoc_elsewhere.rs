//! b04 (sanitizer bypass): an inherent associated constructor on `SafeCmd`, written in another
//! module than the type (inherent impls may live anywhere in the crate).

use crate::safe_cmd::SafeCmd;

impl SafeCmd {
    pub fn from_trusted(raw: &str) -> Self {
        SafeCmd(raw.to_owned())
    }
}

/// Only calls the two unchecked constructors: no construction here.
pub fn trusted_pair(a: &str, b: String) -> (SafeCmd, SafeCmd) {
    (SafeCmd::from_trusted(a), SafeCmd::new_unchecked(b))
}

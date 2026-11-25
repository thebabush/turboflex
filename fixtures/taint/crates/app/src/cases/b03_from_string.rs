//! b03 (sanitizer bypass): a `From<String>` impl written outside the sanitizer, so
//! `String::into()` makes a `SafeCmd`.

use crate::safe_cmd::SafeCmd;

impl From<String> for SafeCmd {
    fn from(raw: String) -> Self {
        SafeCmd(raw)
    }
}

/// Only calls the impl: the construction is in `from` above, not here.
pub fn convert(raw: String) -> SafeCmd {
    raw.into()
}

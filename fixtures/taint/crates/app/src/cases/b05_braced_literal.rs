//! b05 (sanitizer bypass): a braced literal of the tuple struct (`SafeCmd { 0: .. }`) and
//! `SafeCmd::default()` called outside the sanitizer.

use crate::safe_cmd::SafeCmd;

#[expect(clippy::init_numbered_fields, reason = "the braced form is the case")]
pub fn braced(raw: &str) -> SafeCmd {
    SafeCmd { 0: raw.to_owned() }
}

/// Only calls the derived `Default`: the construction is in the derive (b06), not here.
pub fn empty() -> SafeCmd {
    SafeCmd::default()
}

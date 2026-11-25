//! b09 (sanitizer bypass, NOT caught): appending to the `pub(crate)` field of an existing
//! `SafeCmd`. Nothing is constructed, so the constructs-only view has no edge to flag.

use crate::safe_cmd::SafeCmd;

pub fn append_arg(cmd: &mut SafeCmd, arg: &str) {
    cmd.0.push(' ');
    cmd.0.push_str(arg);
}

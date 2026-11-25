//! b01 (sanitizer bypass): a tuple-struct literal outside the sanitizer.

use crate::exec;
use crate::safe_cmd::SafeCmd;

pub fn run_raw(input: &str) -> bool {
    exec::run(SafeCmd(input.to_owned()))
}

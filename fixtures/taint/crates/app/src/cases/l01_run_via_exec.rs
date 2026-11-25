//! l01 (legit, both views): sanitize user input, then hand the `SafeCmd` to the process sink.

use crate::{exec, sanitizer};

pub fn run_user_command(input: &str) -> bool {
    sanitizer::sanitize(input).is_some_and(exec::run)
}

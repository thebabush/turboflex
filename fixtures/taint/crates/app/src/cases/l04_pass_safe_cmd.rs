//! l04 (legit, sanitizer view): using a `SafeCmd` without building one: the checked associated
//! constructor, a clone, reading it, passing it on, naming it in a signature.

use crate::exec;
use crate::safe_cmd::SafeCmd;

pub fn run_twice(raw: &str) -> bool {
    let Some(cmd) = SafeCmd::parse(raw) else {
        return false;
    };
    let again = cmd.clone();
    exec::run(cmd) && !again.as_str().is_empty() && exec::run(again)
}

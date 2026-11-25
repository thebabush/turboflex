//! b02 (sanitizer bypass): the tuple-struct constructor used as a fn value (`.map(SafeCmd)`).

use crate::safe_cmd::SafeCmd;

pub fn wrap_all(inputs: Vec<String>) -> Vec<SafeCmd> {
    inputs.into_iter().map(SafeCmd).collect()
}

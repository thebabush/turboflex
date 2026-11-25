//! Target for c21: called only from a default trait method body.

pub fn frame(s: &str) -> String {
    format!("[{s}]")
}

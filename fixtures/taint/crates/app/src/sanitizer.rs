//! The sanitizer: the only component that may build a `SafeCmd`.

use crate::safe_cmd::SafeCmd;

/// The policy: a short allow-list of programs, no shell metacharacters.
pub fn is_safe(raw: &str) -> bool {
    let program = raw.split_whitespace().next().unwrap_or_default();
    matches!(program, "echo" | "true" | "ls")
        && !raw.contains([';', '|', '&', '$', '`', '>', '<'])
}

/// Legit: the free-fn constructor, a tuple-struct literal inside the sanitizer.
pub fn sanitize(raw: &str) -> Option<SafeCmd> {
    is_safe(raw).then(|| SafeCmd(raw.to_owned()))
}

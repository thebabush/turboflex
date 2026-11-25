//! Fixture crate for the two taint views (`capability.kdl`, `sanitizer.kdl`): every module under
//! `cases` is one legit use or one violation; the other modules are the sinks, the sanitizer and
//! the sanitized type. See the README at the workspace root.

pub mod cases;

pub mod exec;
pub mod safe_cmd;
pub mod sanitizer;
pub mod storage;

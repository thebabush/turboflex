//! The sanitized type, `SafeCmd`, and everything written next to it. Holds three bypass cases
//! that can only live here: the derives on the type (`Default`, `Deserialize`) and an inherent
//! constructor (`new_unchecked`). `Debug` and `Clone` are legit derives; `parse` is the legit
//! associated constructor.

use crate::sanitizer;

/// A command line the sanitizer accepted. The field is `pub(crate)`, so the type system lets any
/// module of the crate build one: `sanitizer.kdl` is what says only the sanitizer may.
#[derive(Debug, Clone)] // legit: Debug builds nothing, Clone copies an already sanitized value
#[derive(Default)] // bypass b06: `SafeCmd::default()` is an unchecked (empty) command
#[derive(serde::Deserialize)] // bypass b07: any JSON string becomes a SafeCmd
pub struct SafeCmd(pub(crate) String);

impl SafeCmd {
    /// Legit: the checked associated constructor, delegating to the sanitizer's policy.
    pub fn parse(raw: &str) -> Option<Self> {
        sanitizer::is_safe(raw).then(|| SafeCmd(raw.to_owned()))
    }

    /// Bypass b08: an associated constructor next to the type that skips the check.
    pub fn new_unchecked(raw: String) -> Self {
        SafeCmd(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

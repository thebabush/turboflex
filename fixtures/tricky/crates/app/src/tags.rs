//! Target for c06: field type whose Debug/Clone/PartialEq impls are
//! hand-written (serde impls derived).

use std::fmt;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Tag(pub String);

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tag({})", self.0)
    }
}

impl Clone for Tag {
    fn clone(&self) -> Self {
        Tag(self.0.clone())
    }
}

impl PartialEq for Tag {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

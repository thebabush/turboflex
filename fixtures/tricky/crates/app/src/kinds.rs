//! Target for c06: field type whose impls are all *derived*.

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Small,
    Large,
}

//! Target for c18: fns referenced only from `#[serde(...)]` attributes.

use serde::Serializer;

pub fn ser_upper<S: Serializer>(v: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&v.to_uppercase())
}

/// Used via `#[serde(with = "crate::serde_helpers::hex_u32")]`.
pub mod hex_u32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u32, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{v:x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
        let s = String::deserialize(d)?;
        u32::from_str_radix(&s, 16).map_err(serde::de::Error::custom)
    }
}

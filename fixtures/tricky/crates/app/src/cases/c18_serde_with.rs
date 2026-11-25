//! c18: serde derive whose generated code calls fns in `app::serde_helpers`
//! named only in string attributes (`serialize_with`, `with`).

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Payload {
    #[serde(serialize_with = "crate::serde_helpers::ser_upper")]
    pub name: String,
    #[serde(with = "crate::serde_helpers::hex_u32")]
    pub code: u32,
}

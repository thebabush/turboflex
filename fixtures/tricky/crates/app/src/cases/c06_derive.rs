//! c06: derived impls on a struct whose fields come from other modules. The
//! derived Debug/Clone/PartialEq/Serialize/Deserialize impls call the field
//! types' corresponding impls (`app::kinds` all derived, `app::tags` partly
//! hand-written).

use crate::kinds::Kind;
use crate::tags::Tag;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub kind: Kind,
    pub tag: Tag,
}

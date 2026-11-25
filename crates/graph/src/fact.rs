//! The fact records: entities, edges and the header of the compile unit they come from.

use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::kind::{CompileMode, EdgeKind, EntityKind, Origin, Provenance, TargetKind};

/// Defines a transparent string newtype: serialized as the bare string.
macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_newtype!(
    /// Canonical, crate-qualified path of an item: `app::util::helper`,
    /// `<app::animals::Dog as core_api::Greeter>::greet`, `app::shapes::Circle::area`.
    ///
    /// A path identifies exactly one item across every unit of an analysis, so an extractor must
    /// make paths unique where Rust names are not: impl blocks are named by the module they are
    /// written in (`app::split_ext::{impl app::split::Thing}`, two blocks for one type in two
    /// modules stay distinct), and targets that share a crate name (a package's lib and bin) must
    /// not produce the same path for different items. [`Graph::merge`](crate::Graph::merge)
    /// rejects two different items under one path.
    ItemPath
);

string_newtype!(
    /// Crate-qualified path of a module: `app::cases::c01_direct_call`, or just `app` for a root.
    ModulePath
);

string_newtype!(
    /// A crate name as rustc sees it (`-` already mapped to `_`).
    CrateName
);

/// A source location: the file as rustc prints it, and a 1-based line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub file: String,
    pub line: NonZeroU32,
}

/// A named item that edges start or end at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub path: ItemPath,
    pub kind: EntityKind,
    /// The module the item is written in (for impl items: where the impl block is written). A
    /// module's own module is itself.
    pub module: ModulePath,
    pub crate_name: CrateName,
    pub origin: Origin,
    /// Definition site; `None` when rustc has no real span for it.
    pub span: Option<Span>,
}

/// One dependency of `src` on `dst`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    /// The depending item; its module is its [`Entity::module`].
    pub src: ItemPath,
    /// The item depended on; its module is its [`Entity::module`].
    pub dst: ItemPath,
    pub kind: EdgeKind,
    pub provenance: Provenance,
    /// Where the dependency is written (the macro call site for expanded code); `None` when rustc
    /// has no real span for it.
    pub span: Option<Span>,
    /// Free-form extractor notes for humans, e.g. `QuestionMark;nested-impl via ...`. Empty when
    /// there is nothing to say. Never parse it; promote anything an analysis needs to a field.
    pub detail: String,
}

/// What a compile unit is: one rustc invocation of one Cargo target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitHeader {
    pub crate_name: CrateName,
    pub target: TargetKind,
    pub mode: CompileMode,
    /// Cargo features enabled for the build, in the order the extractor saw them.
    pub features: Vec<String>,
}

/// The facts of one compile unit: one fact file.
///
/// Every edge endpoint has an entity record in the same unit (enforced when reading), and no
/// path has two entity records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub header: UnitHeader,
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
}

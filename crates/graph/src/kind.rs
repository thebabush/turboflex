//! The closed vocabularies of the schema. On the wire every variant is its `snake_case` name;
//! a value this build does not know is a read error, never a fallback.

use serde::{Deserialize, Serialize};

/// What a dependency edge means. Each edge has exactly one kind; an extractor that sees several
/// relationships between the same two items emits one edge per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// A written call (free fn, method, UFCS), including an fn item reified into a fn pointer.
    Calls,
    /// A type named in a struct/enum/union field.
    UsesTypeField,
    /// A type named in a signature: fn parameters/return, type alias target, const/static type.
    UsesTypeSig,
    /// A type used inside a body (types of locals and temporaries).
    UsesTypeBody,
    /// A type appearing as a generic argument of another type, trait reference or bound.
    GenericArg,
    /// An impl block to the trait it implements.
    ImplOfTrait,
    /// An impl block to its self type.
    ImplForType,
    /// A trait named in a bound or where-clause.
    Bound,
    /// A read of a `const` or `static`.
    ReadsGlobal,
    /// A write to a `static` (`static mut`, or interior mutability, where a write is assumed).
    WritesGlobal,
    /// An invocation of a macro (to the macro definition).
    MacroUse,
    /// `?` converting an error through a user `From` impl.
    ImplicitFrom,
    /// Drop glue running a user `Drop` impl at scope end.
    ImplicitDrop,
    /// Autoderef or deref coercion through a user `Deref`/`DerefMut` impl.
    ImplicitDeref,
    /// An operator (arithmetic, indexing, comparison) resolved to a user impl.
    ImplicitOp,
    /// A `for` loop driving a user `IntoIterator`/`Iterator` impl.
    ImplicitIter,
    /// Formatting machinery (`format!`, `{}`, derived `Debug`) reaching a user `fmt` impl.
    ImplicitFmt,
    /// A value of a struct, enum or union built (literal, variant, tuple constructor, or a
    /// constructor used as a fn value), to the ADT; `detail` is the enum variant.
    Constructs,
    /// A branch on an enum's discriminant (`match`, `if let`, `let else`, `matches!`), to the enum.
    MatchesOn,
}

impl EdgeKind {
    /// The coarse group a model rule can name instead of individual kinds.
    #[must_use]
    pub fn group(self) -> EdgeGroup {
        match self {
            Self::UsesTypeField
            | Self::UsesTypeSig
            | Self::UsesTypeBody
            | Self::GenericArg
            | Self::Bound => EdgeGroup::Types,
            Self::Calls
            | Self::ImplicitFrom
            | Self::ImplicitDrop
            | Self::ImplicitDeref
            | Self::ImplicitOp
            | Self::ImplicitIter
            | Self::ImplicitFmt => EdgeGroup::Calls,
            Self::ImplOfTrait | Self::ImplForType => EdgeGroup::Impls,
            Self::MacroUse => EdgeGroup::Macros,
            Self::ReadsGlobal | Self::WritesGlobal => EdgeGroup::Globals,
            Self::Constructs => EdgeGroup::Constructs,
            Self::MatchesOn => EdgeGroup::Matches,
        }
    }

    /// Whether the edge only depends on a type (field, signature, body, generic argument, bound).
    #[must_use]
    pub fn is_type_use(self) -> bool {
        self.group() == EdgeGroup::Types
    }

    /// Whether the edge runs code of the destination, written or implicit.
    #[must_use]
    pub fn is_call(self) -> bool {
        self.group() == EdgeGroup::Calls
    }
}

/// Coarse grouping of [`EdgeKind`]s, as named by the architecture model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeGroup {
    /// Type-level dependencies: `uses_type_*`, `generic_arg`, `bound`.
    Types,
    /// Code execution: `calls` and every `implicit_*` kind.
    Calls,
    /// `impl_of_trait`, `impl_for_type`.
    Impls,
    /// `macro_use`.
    Macros,
    /// `reads_global`, `writes_global`.
    Globals,
    /// `constructs`.
    Constructs,
    /// `matches_on`.
    Matches,
}

/// Where in the source an edge comes from, judged by the span's expansion chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Written by hand.
    Source,
    /// Inside a bang or attribute macro expansion.
    MacroExpanded,
    /// Produced by a compiler desugaring (`?`, `for`, `.await`, ...).
    Desugared,
    /// Inside a `#[derive]` expansion.
    Derive,
}

/// What an [`Entity`](crate::Entity) is.
///
/// The set is the named items an edge can start or end at, after the extractor folds anonymous
/// bodies (closures, coroutines, inline consts, items local to fn bodies) into the nearest named
/// item. Associated items are split by where they live because the model may treat a trait's
/// declaration and its impls differently. Items of an `extern` block map to [`Self::Function`] /
/// [`Self::Static`]. Variants, fields and `use` items are not entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// A module, including a crate root.
    Module,
    /// A free function.
    Function,
    /// An associated function of an impl block (inherent or trait impl).
    Method,
    /// An associated function declared in a trait (with or without a default body).
    TraitMethod,
    Struct,
    Enum,
    Union,
    Trait,
    /// An impl block (`impl Type` or `impl Trait for Type`).
    Impl,
    /// A `type` alias outside traits and impls.
    TypeAlias,
    /// An associated type, in a trait or an impl.
    AssocType,
    /// A `const` item outside traits and impls.
    Const,
    /// An associated const, in a trait or an impl.
    AssocConst,
    Static,
    /// A `macro_rules!` or procedural macro.
    Macro,
}

/// Whether an entity belongs to the analysed workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Defined in a crate of the analysed workspace.
    Workspace,
    /// Defined in a dependency, including `std`/`core`/`alloc`.
    External,
}

/// The Cargo target a compile unit was built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Lib,
    Bin,
    /// An integration test (`tests/*.rs`).
    Test,
    Example,
    Bench,
    ProcMacro,
    BuildScript,
}

/// Whether a compile unit was built with `cfg(test)` (rustc `--test`).
///
/// Separate from [`TargetKind`]: a lib or bin target's unit tests are a `Test`-mode build of that
/// target, and an integration test with `harness = false` is a `Normal`-mode build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompileMode {
    Normal,
    Test,
}

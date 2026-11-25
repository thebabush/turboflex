//! Renders reflexion results as text for humans and as JSON for machines.
//!
//! A [`Report`] is a plain-data summary of one check: the fact base and, for each view (one
//! model file), its headline counts and the findings worth reading. The service fills it from
//! the engine's results; this crate only renders it, with [`Report::write_text`] or
//! [`Report::write_json`]. The engine's types are not serializable and reading them takes calls
//! the architecture does not give this crate, so the summary has its own types here, and the
//! JSON shape is this crate's, not the engine's internals.
//!
//! Kinds, groups and provenances are rendered by their wire names (`uses_type_sig`, `calls`,
//! `source`), in text as in JSON, the same names the model language and fact files use.
//!
//! Like the other core crates, this one is pure: it writes to a caller-provided `Write`.

mod summary;
mod text;

pub use summary::{
    Absence, Conflict, Divergence, EdgeSample, Facts, GroupCount, Headline, ModelLocation, Report,
    SkipReason, UnmappedModule, View,
};

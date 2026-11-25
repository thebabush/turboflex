//! Fact schema shared by extractors and the analysis core: entities, edges, kinds, provenance.
//!
//! # The contract
//!
//! Facts are the only interface between extraction and analysis:
//!
//! - **Writers** are extractors, chiefly the rustc driver (its own process, on a pinned nightly).
//!   Each rustc invocation of a Cargo target is one compile unit and produces one fact file: a
//!   [`Unit`] written with [`Unit::write_jsonl`].
//! - **Readers** are the analysis side (model mapping, reflexion engine, reports). They read fact
//!   files with [`Unit::read_jsonl`] and merge them with [`Graph::merge`], which deduplicates
//!   entities and edges across units and marks edges seen only in `cfg(test)` builds as
//!   [`Scope::TestOnly`].
//!
//! Because both sides link this crate, it builds on stable and on the driver's nightly, and it is
//! pure: no filesystem, process, environment or network access. I/O goes through caller-provided
//! `BufRead`/`Write` values.
//!
//! # Format and versioning
//!
//! A fact file is JSON Lines: a header record carrying [`SCHEMA_VERSION`], then one record per
//! entity, then one per edge (see [`Unit::write_jsonl`]). Readers are strict: a different schema
//! version, an unknown field, or an unknown enum value is an error, never a silent fallback. So
//! **any** change to the records (new field, new kind, renamed value) bumps [`SCHEMA_VERSION`],
//! and writer and reader must be built from the same schema.

mod fact;
mod graph;
mod jsonl;
mod kind;

pub use fact::{CrateName, Edge, Entity, ItemPath, ModulePath, Span, Unit, UnitHeader};
pub use graph::{Graph, GraphEdge, MergeError, Scope};
pub use jsonl::{ReadError, SCHEMA_VERSION};
pub use kind::{CompileMode, EdgeGroup, EdgeKind, EntityKind, Origin, Provenance, TargetKind};

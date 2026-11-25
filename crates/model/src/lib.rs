//! Architecture model language and the mapping of extracted entities to its components.
//!
//! A model says what the architecture should be: the components, which code belongs to each, and
//! which dependencies between them are allowed. [`Model::parse`] reads one from KDL text
//! ([`Model::parse_with`] if it includes other files);
//! [`Mapping::new`] places every entity of a fact [`Graph`](turboflex_graph::Graph) in a
//! component. Comparing the mapped edges with the rules (convergences, divergences, absences) is
//! the reflexion engine's job, not this crate's.
//!
//! Like `turboflex-graph`, the crate is pure: no filesystem, process, environment or network
//! access. Callers read the model file and pass its text, and load included files through a
//! closure (see [Includes](#includes)).
//!
//! # Example
//!
//! ```
//! # use turboflex_model::{Mapping, Model};
//! # let source = r##"
//! component "model" {
//!     doc "Typed vocabulary"
//!     modules "app::model" "app::model::**"
//! }
//!
//! component "ledger" {
//!     modules "app::db" "app::db::**"
//! }
//!
//! // A parent groups components; its own patterns are its shared part.
//! component "worker" {
//!     modules "app::worker" "app::worker::util"
//!     component "runtime" {
//!         modules "app::worker::v2" "app::worker::v2::**"
//!     }
//!     component "acquire" {
//!         modules "app::ingest" "app::ingest::**"
//!         // One item of another component's module, taking its private helpers along.
//!         items "app::worker::v2::fetch_source" follow=#true
//!     }
//! }
//!
//! component "report" {
//!     modules "app::report"
//! }
//!
//! external "postgres" {
//!     crates "sqlx" "sqlx_*"
//! }
//!
//! external "io" {
//!     modules "std::fs" "std::fs::**" "std::process" "std::process::**"
//! }
//!
//! // Every dependency is a rule: a new component may depend on nothing until one says so.
//! allow "worker.runtime" -> "worker.acquire" "ledger"
//! allow "worker.acquire" -> "ledger" "io"
//! allow "worker" -> "model"
//! allow "ledger" -> "postgres" "model"
//! allow "worker" -> "postgres" kinds="types"
//! allow "report" -> "ledger" kinds="types,calls"
//! allow "report" -> "model"
//!
//! // Not a code dependency: declared, never checked.
//! link "report" -> "worker" via="process"
//!
//! policy {
//!     test-edges "exclude"
//!     // Hand-written decisions only: derived impls' constructs/matches are not.
//!     provenance "source" "macro_expanded" "desugared"
//! }
//! # "##;
//! # let model = Model::parse("example.kdl", source).unwrap_or_else(|e| panic!("{e}"));
//! # let _ = Mapping::new(&model, &turboflex_graph::Graph::default());
//! ```
//!
//! # Grammar
//!
//! The source is a [KDL v2](https://kdl.dev) document. Anything not listed here is an error with
//! its location: unknown nodes, properties or values, a property given twice (KDL would let the
//! last one win), missing or extra arguments, children on a node that takes none, and type
//! annotations. All arguments are strings; bare identifiers (`allow a -> b`) are KDL strings
//! too. One node per line, or separate nodes with `;`.
//!
//! Top level:
//!
//! - `component "NAME" { BODY }`: a workspace component. The block is optional. `NAME` is
//!   `[A-Za-z][A-Za-z0-9_-]*`. Its body may contain:
//!   - `doc "TEXT"`: at most once.
//!   - `modules "PATTERN"...`: module patterns (below).
//!   - `items "PATH"... [follow=#true]`: item patterns, with optional helper-follow.
//!   - `component "NAME" { BODY }`: a nested component, id `PARENT.NAME`, at any depth.
//! - `external "NAME" { BODY }`: an external component (dependencies, std included). Top level
//!   only. Its body may contain `doc`, `modules`, `items` (no `follow`) and
//!   `crates "NAME"...`, where a crate name is as rustc sees it (`sqlx_core`, not
//!   `sqlx-core`) and may end in `*` to match a prefix (`rustc_*`).
//! - `include "PATH"`: the declarations of another model file, as if written here (see
//!   [Includes](#includes)). Exactly one string, no properties, no children.
//! - `allow "FROM" -> "TO"... [kinds="GROUP,..."]`: allowed dependencies (below).
//! - `link "FROM" -> "TO"... via="process"`: a relationship that is not a code dependency, such
//!   as running another component as a subprocess. Recorded, never checked. Workspace
//!   components on both ends.
//! - `policy { ... }`: analysis knobs. At most once, each child at most once:
//!   - `test-edges "exclude"|"check"`: whether edges seen only in `cfg(test)` builds are
//!     checked (default `exclude`).
//!   - `edges "GROUP"...`: the edge groups the model considers at all (default: every group).
//!     Group names as for `kinds` (below).
//!   - `provenance "PROVENANCE"...`: the edge provenances the model considers at all (default:
//!     every one): `source`, `macro_expanded`, `desugared`, `derive`. Leaving out `derive` drops
//!     the edges of derived impls (`Clone`, `PartialEq`, `Deserialize`, ...), which construct and
//!     match on types without anyone having written that code.
//!   - `targets "ID"...`: the components whose incoming dependencies the model considers at all
//!     (default: every component). An edge is considered when its target is, or is nested in,
//!     a listed component. Workspace and external components may both be listed.
//!
//!   `edges`, `provenance` and `targets` take one or more names, each at most once. Together
//!   they make the model a *view*: several models can look at one fact base, each at the
//!   relations it cares about (say, only who constructs a payload type and who matches on a
//!   format enum). The engine ignores every other edge: it is not checked, does not satisfy a
//!   rule and is not counted against unmapped entities. A rule whose `kinds` lists only groups
//!   `edges` leaves out could never match, and is an error. A rule whose target is neither
//!   within a listed component nor an ancestor of one is out of the view's scope: it is ignored
//!   (never an absence) and is not an error, so a view can include rule files shared with other
//!   views.
//!
//! Component ids are global and dotted (`worker.runtime`); externals share the id space. Every
//! id named by `allow`, `link` or `targets` must be declared, anywhere in the
//! model: in the same file or in any file of its includes.
//!
//! The `->` is a plain KDL argument: KDL v2 accepts `->` as a bare identifier string (`<`, `>`
//! are identifier characters in v2), so `allow "a" -> "b"` needs no quoting. `"->"` works too.
//!
//! # Includes
//!
//! Several models (views) over one codebase can share their component, external, rule and link
//! declarations through `include "PATH"`. `PATH` is relative to the directory of the file that
//! contains the `include`; the crate does no I/O, so [`Model::parse_with`] hands it to the
//! caller's loader together with the including file's name. An included file has the same
//! grammar but may only contain `component`, `external`, `allow`, `link` and `include`:
//! `policy` makes the root file a view, and is an error anywhere else.
//!
//! - The model is the composition of all its files, and its checks span all of them: an id or
//!   a pattern declared in two files, or a `(FROM, TO)` rule pair repeated in another file, is
//!   the same error as within one file, and references may name ids declared in any file.
//! - Declaration order (as in [`Model::components`]) is depth-first: an included file's
//!   declarations take the place of its `include` node.
//! - A file included more than once (say, by two files the root includes) is read once; later
//!   includes of it add nothing. Files are the same when the loader gives them the same
//!   [`Source::name`].
//! - An include cycle is an error naming the chain (`a.kdl -> b.kdl -> a.kdl`), at the `include`
//!   that closes it. A file the loader cannot provide is an error at its `include`.
//! - Every [`Location`] names its file ([`Model::source_name_of`]); [`ModelError`] reports and
//!   renders each problem against the file it is in.
//!
//! ```
//! # use std::collections::HashMap;
//! # use turboflex_model::{Model, Source};
//! let files = HashMap::from([(
//!     "shared/components.kdl",
//!     "component \"core\"\ncomponent \"app\"\nallow \"app\" -> \"core\"",
//! )]);
//! let view = "include \"shared/components.kdl\"\npolicy {\n    test-edges \"check\"\n}";
//! let model = Model::parse_with("view.kdl", view, |_including, path| {
//!     let text = files.get(path).ok_or_else(|| format!("no file `{path}`"))?;
//!     Ok(Source {
//!         name: path.to_owned(),
//!         text: (*text).to_owned(),
//!     })
//! })
//! .unwrap_or_else(|e| panic!("{e}"));
//! assert_eq!(model.rules().len(), 1);
//! assert_eq!(
//!     model.source_name_of(model.rules()[0].location),
//!     "shared/components.kdl"
//! );
//! ```
//!
//! # Patterns and specificity
//!
//! - **Module pattern** `a::b`: entities whose [`Entity::module`] is exactly `a::b`.
//!   `a::b::**`: entities whose module is strictly below `a::b` (not `a::b` itself; write both
//!   to get both). `**` may only be the last segment; there are no other wildcards.
//! - **Item pattern** `a::b::f`: the one entity whose [`Entity::path`] is exactly this.
//! - **Crate pattern** (externals only) `sqlx` / `rustc_*`: entities by [`Entity::crate_name`].
//!
//! Workspace components only map [`Origin::Workspace`] entities and external components only map
//! [`Origin::External`] ones. A workspace entity no pattern matches is *unmapped* (always
//! reported); an external entity no external component matches is out of scope.
//!
//! When several patterns match, the most specific wins: an item pattern beats any module
//! pattern, which beats any crate pattern; among module patterns the one with more path segments
//! wins, and on equal segments exact beats `::**` (so: the exact module, else the nearest
//! ancestor's `::**`); among crate patterns exact beats prefix, then the longer prefix wins.
//!
//! The same pattern declared twice, in one component or two, is an error: every pattern has a
//! single owner. With that rule two different patterns can never tie, so a pattern-level
//! conflict cannot happen. A [`Placement::Conflict`] can only come from helper-follow.
//!
//! # Helper-follow
//!
//! `items "PATH" follow=#true` moves an item to another component together with the private
//! helpers that exist only for it. For each such item (the *anchor*), owned by component `C`, in
//! module `M`:
//!
//! 1. *Candidates* are the workspace entities of module `M` that a module pattern maps to a
//!    component other than `C`. Entities mapped by an item pattern never follow.
//! 2. The moving set starts as the candidates reachable from the anchors of `C` in `M`, through
//!    candidates only, along any edge forwards (any kind, any scope), plus from a type to the
//!    impl blocks for it (`impl_for_type` backwards), so a moved type keeps its impls.
//! 3. A candidate is dropped if some entity outside the moving set and the anchors, mapped to
//!    the candidate's own (original) component, has an edge to it. References from other
//!    components do not hold it back. Dropping can disconnect others, so steps 2 and 3 repeat
//!    until nothing changes. What is left moves to `C`
//!    ([`Reason::Follows`] names the anchor).
//! 4. An entity that would follow anchors of two different components is a
//!    [`Placement::Conflict`] and gets no owner.
//!
//! Associated items are not tied to their type: a method follows only if it is reachable (say,
//! called by the anchor) and not used by its original component.
//!
//! # Rules
//!
//! Rules are an allow-list, read by the reflexion engine. An edge from an entity of workspace
//! component `S` to an entity of component `T` (`S` ≠ `T`, both mapped) is allowed when:
//!
//! - one of `S` and `T` is nested in the other (a parent's shared part and its descendants
//!   depend on each other freely; siblings do not), or
//! - a rule `allow F -> G` exists with `S` nested in (or equal to) `F` and `T` nested in (or
//!   equal to) `G`, and, if the rule has `kinds`, the edge kind's
//!   [`EdgeGroup`](turboflex_graph::EdgeGroup) is listed. Group names: `types`, `calls`,
//!   `impls`, `macros`, `globals`, `constructs`, `matches`.
//!
//! Externals are rule targets like any component (and never sources). Edges to out-of-scope
//! externals are not checked. The same `(FROM, TO)` pair in two rules is an error.
//!
//! Nothing else is allowed: no node grants a dependency to every component. The former
//! `foundation "ID"...` did, including to components added later, so permissions nobody wrote
//! down were inherited; it is now an error, wherever it appears, asking for explicit `allow`
//! rules instead (or `policy { targets ... }` for a view about a few components).
//!
//! [`Entity::module`]: turboflex_graph::Entity::module
//! [`Entity::path`]: turboflex_graph::Entity::path
//! [`Entity::crate_name`]: turboflex_graph::Entity::crate_name
//! [`Origin::Workspace`]: turboflex_graph::Origin::Workspace
//! [`Origin::External`]: turboflex_graph::Origin::External

mod error;
mod mapping;
mod model;
mod parse;

pub use error::{Location, ModelError, Problem, ProblemKind, SourceId};
pub use mapping::{Assignment, Mapping, Placement, Reason};
pub use model::{
    Component, ComponentId, ComponentKind, CratePattern, Follow, Link, LinkVia, Model,
    ModulePattern, ModuleScope, Pattern, PatternId, Policy, Rule, Selector, Source, TestEdges,
};

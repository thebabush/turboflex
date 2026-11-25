//! Reflexion engine: compares the dependencies extracted from code with an architecture model.
//!
//! The inputs are a [`Model`](turboflex_model::Model) (components and allowed dependencies), a
//! fact [`Graph`](turboflex_graph::Graph) and the [`Mapping`](turboflex_model::Mapping) of the
//! graph's entities to the model's components. [`Reflexion::compute`] lifts every graph edge to
//! the components of its ends and checks it against the model, after Murphy and Notkin's
//! software reflexion models. The model language and the rules' meaning are specified in the
//! [`turboflex_model`] crate docs; this page only says how the engine applies them.
//!
//! Like the other core crates, this one is pure: no filesystem, process, environment or network
//! access.
//!
//! # Lifting
//!
//! Each [`GraphEdge`](turboflex_graph::GraphEdge) becomes a [`LiftedEdge`] from the component
//! owning its source to the component owning its destination, unless it is skipped
//! ([`Skip`], counted in [`Reflexion::skipped`]), for the first of these reasons that applies:
//!
//! - edges outside the model's view: an [`EdgeGroup`](turboflex_graph::EdgeGroup) not in the
//!   policy's `edges`, or else a [`Provenance`](turboflex_graph::Provenance) not in its
//!   `provenance`. For the model these edges do not exist, so they come before everything else:
//!   the other counts, including the test-only one and the unmapped and conflict edge counts,
//!   are about edges the model considers. The provenance is that of the edge's representative
//!   occurrence ([`GraphEdge::edge`](turboflex_graph::GraphEdge::edge));
//! - [`Scope::TestOnly`](turboflex_graph::Scope::TestOnly) edges, unless the model's policy is
//!   `test-edges "check"`;
//! - edges into components outside the policy's `targets` ([`Skip::NotTargeted`]): the
//!   destination is owned by a component that is not, and is not within, a target, or is a
//!   conflict none of whose claimants is. Like the filters above, these edges are outside the
//!   model's view and are not counted against the unmapped source or conflict they may have.
//!   An unmapped or out-of-scope destination may belong to a target, so its edge goes on to the
//!   next reason;
//! - edges with an end that has no owner: an unmapped workspace entity, a helper-follow
//!   conflict, or an external entity no external component claims (out of scope). The source is
//!   looked at first, so an edge is counted once, under its source's reason if both ends lack an
//!   owner;
//! - edges whose two ends have the same owner.
//!
//! # Verdicts
//!
//! Every lifted edge from `A` to `B` gets a [`Verdict`]: allowed, with the first applicable
//! [`Allowance`] in this order, or divergent.
//!
//! 1. [`Allowance::Nested`]: `A` is within `B` or `B` is within `A`
//!    ([`ComponentId::is_within`](turboflex_model::ComponentId::is_within)). Siblings are not
//!    allowed implicitly.
//! 2. [`Allowance::Rule`]: the first rule `allow F -> G` (declaration order) with `A` within `F`,
//!    `B` within `G`, and the edge kind's [`EdgeGroup`](turboflex_graph::EdgeGroup) in the rule's
//!    `kinds` (or no `kinds`).
//!
//! The verdict of one edge depends only on `(A, B, group)`, so within a pair each group is
//! wholly allowed or wholly divergent.
//!
//! # Findings
//!
//! - A **convergence** is a component pair whose lifted edges are all allowed; a **divergence**
//!   is a pair with at least one divergent edge. A divergence may still have allowed groups (a
//!   `kinds="types"` rule allows the pair's `types` edges but not its `calls` edges):
//!   [`Dependency::groups`] tells them apart. See [`Reflexion::convergences`] and
//!   [`Reflexion::divergences`].
//! - An **absence** is a rule that no lifted edge matches (as in step 2 above, whatever the
//!   allowance the edge actually got): [`Reflexion::absences`]. Skipped edges, filtered ones
//!   included, never match. A rule whose target is neither within one of the policy's
//!   `targets` nor an ancestor of one is out of the view's scope and never an absence; a rule
//!   to an ancestor of a target is matched only by the edges into the target.
//! - **Unmapped** workspace entities and helper-follow **conflicts** come straight from the
//!   mapping, each with the number of edges skipped because of it: [`Reflexion::unmapped`],
//!   [`Reflexion::conflicts`].
//!
//! Each pair keeps all of its edges ([`Dependency::edges`], in `(src, dst, kind)` order) with
//! their verdicts, and counts them by kind, provenance, group and allowance.
//!
//! [`Reflexion::explain`] answers "why is `A -> B` what it is": the model's ruling for a direct
//! `A -> B` edge of each group, and every lifted edge from within `A` to within `B`.
//!
//! # Roll-up
//!
//! [`Reflexion::rolled_up`] views the result with every component replaced by its ancestor at a
//! given depth (depth 1: the top-level components), so `worker.acquire -> ledger` and
//! `worker.runtime -> ledger` become one `worker -> ledger` pair. Edges keep the verdicts they
//! got between their real components, which makes the rolled-up verdicts consistent with the
//! detailed ones: a rolled-up pair is a divergence exactly when one of its edges is. Rules are
//! not re-evaluated between rolled-up components, since `worker -> ledger` need not be a rule for
//! all of its edges to be allowed. Edges between components that roll up into the same one are
//! kept apart ([`RolledUp::internal`]) rather than dropped, so a divergence between two siblings
//! stays visible. Absences, unmapped entities and conflicts do not depend on the level and stay
//! on [`Reflexion`].

mod dependency;
mod error;
mod reflexion;
mod rollup;

pub use dependency::{Allowance, Dependency, GroupCounts, LiftedEdge, Relation, RuleId, Verdict};
pub use error::EngineError;
pub use reflexion::{Conflict, Explanation, Reflexion, Skip, Unowned};
pub use rollup::RolledUp;

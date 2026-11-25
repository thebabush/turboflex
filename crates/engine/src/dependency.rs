//! Lifted edges, their verdicts, and the component pairs they make up.

use std::collections::{BTreeMap, BTreeSet};

use turboflex_graph::{EdgeGroup, EdgeKind, GraphEdge, ItemPath, Provenance};
use turboflex_model::ComponentId;

/// Index of a rule in [`Model::rules`](turboflex_model::Model::rules) (and in
/// [`Reflexion::rules`](crate::Reflexion::rules)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(pub(crate) usize);

impl RuleId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0
    }
}

/// Why an edge between two components is allowed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Allowance {
    /// One end is nested in the other: a parent's own code and its descendants.
    Nested,
    /// This rule allows it.
    Rule(RuleId),
}

/// The model's verdict on one lifted edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verdict {
    Allowed(Allowance),
    Divergent,
}

impl Verdict {
    #[must_use]
    pub fn is_divergent(&self) -> bool {
        match self {
            Self::Allowed(_) => false,
            Self::Divergent => true,
        }
    }
}

/// A graph edge lifted to the components owning its ends, with its verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiftedEdge {
    /// The owner of the edge's source.
    pub from: ComponentId,
    /// The owner of the edge's destination.
    pub to: ComponentId,
    pub edge: GraphEdge,
    /// Judged between [`Self::from`] and [`Self::to`], whatever level the edge is viewed at.
    pub verdict: Verdict,
}

impl LiftedEdge {
    #[must_use]
    pub fn group(&self) -> EdgeGroup {
        self.edge.edge.kind.group()
    }
}

/// What a component pair is, as a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relation {
    /// Every edge of the pair is allowed.
    Convergence,
    /// At least one edge of the pair is divergent.
    Divergence,
}

/// How many edges of one group a pair has, by verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GroupCounts {
    pub allowed: usize,
    pub divergent: usize,
}

/// The lifted edges from one component to another, the evidence for a convergence or a
/// divergence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub(crate) from: ComponentId,
    pub(crate) to: ComponentId,
    /// Ordered by the graph edge's `(src, dst, kind)`.
    pub(crate) edges: Vec<LiftedEdge>,
}

impl Dependency {
    /// Builds a dependency, putting `edges` in `(src, dst, kind)` order.
    pub(crate) fn new(from: ComponentId, to: ComponentId, mut edges: Vec<LiftedEdge>) -> Self {
        edges.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        Self { from, to, edges }
    }

    #[must_use]
    pub fn from(&self) -> &ComponentId {
        &self.from
    }

    #[must_use]
    pub fn to(&self) -> &ComponentId {
        &self.to
    }

    /// Every edge, ordered by the graph edge's `(src, dst, kind)`.
    #[must_use]
    pub fn edges(&self) -> &[LiftedEdge] {
        &self.edges
    }

    pub fn divergent_edges(&self) -> impl Iterator<Item = &LiftedEdge> {
        self.edges.iter().filter(|e| e.verdict.is_divergent())
    }

    #[must_use]
    pub fn relation(&self) -> Relation {
        if self.edges.iter().any(|e| e.verdict.is_divergent()) {
            Relation::Divergence
        } else {
            Relation::Convergence
        }
    }

    /// Edge counts per group present in the pair, split by verdict.
    #[must_use]
    pub fn groups(&self) -> BTreeMap<EdgeGroup, GroupCounts> {
        let mut groups: BTreeMap<EdgeGroup, GroupCounts> = BTreeMap::new();
        for edge in &self.edges {
            let counts = groups.entry(edge.group()).or_default();
            match edge.verdict {
                Verdict::Allowed(_) => counts.allowed += 1,
                Verdict::Divergent => counts.divergent += 1,
            }
        }
        groups
    }

    /// The groups with at least one divergent edge.
    #[must_use]
    pub fn divergent_groups(&self) -> BTreeSet<EdgeGroup> {
        self.divergent_edges().map(LiftedEdge::group).collect()
    }

    #[must_use]
    pub fn by_kind(&self) -> BTreeMap<EdgeKind, usize> {
        count(self.edges.iter().map(|e| e.edge.edge.kind))
    }

    #[must_use]
    pub fn by_provenance(&self) -> BTreeMap<Provenance, usize> {
        count(self.edges.iter().map(|e| e.edge.edge.provenance))
    }

    /// How many edges each allowance allowed.
    #[must_use]
    pub fn allowances(&self) -> BTreeMap<&Allowance, usize> {
        count(self.edges.iter().filter_map(|e| match &e.verdict {
            Verdict::Allowed(allowance) => Some(allowance),
            Verdict::Divergent => None,
        }))
    }
}

fn sort_key(edge: &LiftedEdge) -> (&ItemPath, &ItemPath, EdgeKind) {
    (
        &edge.edge.edge.src,
        &edge.edge.edge.dst,
        edge.edge.edge.kind,
    )
}

fn count<T: Ord>(items: impl Iterator<Item = T>) -> BTreeMap<T, usize> {
    let mut counts = BTreeMap::new();
    for item in items {
        *counts.entry(item).or_default() += 1;
    }
    counts
}

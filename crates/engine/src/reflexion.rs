//! Computing the reflexion of a graph under a model, and querying it.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

use turboflex_graph::{EdgeGroup, Graph, GraphEdge, ItemPath, Scope};
use turboflex_model::{
    Assignment, ComponentId, Mapping, Model, Placement, Policy, Rule, TestEdges,
};

use crate::dependency::{Allowance, Dependency, LiftedEdge, Relation, RuleId, Verdict};
use crate::error::EngineError;
use crate::rollup::RolledUp;

/// Every [`EdgeGroup`], in order.
const GROUPS: [EdgeGroup; 7] = [
    EdgeGroup::Types,
    EdgeGroup::Calls,
    EdgeGroup::Impls,
    EdgeGroup::Macros,
    EdgeGroup::Globals,
    EdgeGroup::Constructs,
    EdgeGroup::Matches,
];

// Fails to compile when a group is added to `EdgeGroup`, as a reminder to extend `GROUPS`; the
// position check below fails if one is listed out of place.
const _: () = {
    const fn position(group: EdgeGroup) -> usize {
        match group {
            EdgeGroup::Types => 0,
            EdgeGroup::Calls => 1,
            EdgeGroup::Impls => 2,
            EdgeGroup::Macros => 3,
            EdgeGroup::Globals => 4,
            EdgeGroup::Constructs => 5,
            EdgeGroup::Matches => 6,
        }
    }
    let mut i = 0;
    while i < GROUPS.len() {
        assert!(position(GROUPS[i]) == i, "GROUPS out of order");
        i += 1;
    }
};

/// Why a graph edge was not lifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Skip {
    /// Its [`EdgeGroup`] is not in the policy's `edges`.
    GroupFiltered,
    /// Its [`Provenance`](turboflex_graph::Provenance) is not in the policy's `provenance`.
    ProvenanceFiltered,
    /// Seen only in `cfg(test)` builds, and the policy excludes those.
    TestOnly,
    /// An end is a workspace entity no pattern matches.
    Unmapped,
    /// An end is claimed by several components (helper-follow).
    Conflict,
    /// An end is an external entity no external component claims, or not an entity of the
    /// graph at all.
    OutOfScope,
    /// Its target end belongs only to components outside the policy's `targets`.
    NotTargeted,
    /// Both ends belong to the same component.
    SameComponent,
}

/// An entity without an owner, with the number of edges (after the policy's filters and test-edge
/// setting) that have it at one end and so were not checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unowned {
    pub path: ItemPath,
    pub edges: usize,
}

/// An entity several components claim, with every claim and the number of edges (after the
/// policy's filters and test-edge setting) that have it at one end and so were not checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: ItemPath,
    pub claims: Vec<Assignment>,
    pub edges: usize,
}

/// The answer to "why is `from -> to` what it is"; see [`Reflexion::explain`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub from: ComponentId,
    pub to: ComponentId,
    /// The model's verdict on an edge of each group going directly from `from` to `to`.
    pub rulings: BTreeMap<EdgeGroup, Verdict>,
    /// Every lifted edge from `from` or a component within it to `to` or a component within it,
    /// each with the verdict it got between its own components.
    pub dependency: Dependency,
}

/// The comparison of a graph with a model: see the crate docs for the semantics.
#[derive(Debug, Clone)]
pub struct Reflexion {
    components: BTreeSet<ComponentId>,
    rules: Vec<Rule>,
    dependencies: BTreeMap<(ComponentId, ComponentId), Dependency>,
    absences: Vec<RuleId>,
    unmapped: Vec<Unowned>,
    conflicts: Vec<Conflict>,
    skipped: BTreeMap<Skip, usize>,
}

impl Reflexion {
    /// Lifts every edge of `graph` to the components `mapping` (built from `model` and `graph`)
    /// gives its ends, and checks it against `model`.
    #[must_use]
    pub fn compute(model: &Model, graph: &Graph, mapping: &Mapping) -> Self {
        let mut reflexion = Self {
            components: model.components().iter().map(|c| c.id.clone()).collect(),
            rules: model.rules().to_vec(),
            dependencies: BTreeMap::new(),
            absences: Vec::new(),
            unmapped: Vec::new(),
            conflicts: Vec::new(),
            skipped: BTreeMap::new(),
        };
        let mut unowned_edges: BTreeMap<&ItemPath, usize> = BTreeMap::new();
        let mut supported = vec![false; reflexion.rules.len()];
        let mut lifted: BTreeMap<(ComponentId, ComponentId), Vec<LiftedEdge>> = BTreeMap::new();
        let policy = model.policy();
        for edge in graph.edges() {
            if let Some(skip) = outside_view(policy, mapping, edge) {
                *reflexion.skipped.entry(skip).or_default() += 1;
                continue;
            }
            let group = edge.edge.kind.group();
            let (src, dst) = (&edge.edge.src, &edge.edge.dst);
            let owners = (owner(mapping, src), owner(mapping, dst));
            for (path, owner) in [(src, &owners.0), (dst, &owners.1)] {
                if let Err(Skip::Unmapped | Skip::Conflict) = owner {
                    *unowned_edges.entry(path).or_default() += 1;
                }
            }
            let (from, to) = match owners {
                (Ok(from), Ok(to)) => (from, to),
                (Err(skip), _) | (Ok(_), Err(skip)) => {
                    *reflexion.skipped.entry(skip).or_default() += 1;
                    continue;
                }
            };
            if from == to {
                *reflexion.skipped.entry(Skip::SameComponent).or_default() += 1;
                continue;
            }
            for (i, rule) in reflexion.rules.iter().enumerate() {
                if rule_matches(rule, from, to, group) {
                    supported[i] = true;
                }
            }
            lifted
                .entry((from.clone(), to.clone()))
                .or_default()
                .push(LiftedEdge {
                    from: from.clone(),
                    to: to.clone(),
                    edge: GraphEdge::clone(edge),
                    verdict: reflexion.ruling(from, to, group),
                });
        }
        reflexion.dependencies = lifted
            .into_iter()
            .map(|((from, to), edges)| {
                let dependency = Dependency::new(from.clone(), to.clone(), edges);
                ((from, to), dependency)
            })
            .collect();
        reflexion.absences = (0..supported.len())
            .filter(|&i| !supported[i] && policy.considers_rule_target(&reflexion.rules[i].to))
            .map(RuleId)
            .collect();
        let edges_of = |path: &ItemPath| unowned_edges.get(path).copied().unwrap_or(0);
        reflexion.unmapped = mapping
            .unmapped()
            .map(|path| Unowned {
                path: path.clone(),
                edges: edges_of(path),
            })
            .collect();
        reflexion.conflicts = mapping
            .conflicts()
            .map(|(path, claims)| Conflict {
                path: path.clone(),
                claims: claims.to_vec(),
                edges: edges_of(path),
            })
            .collect();
        reflexion
    }

    /// The model's verdict on an edge of `group` from component `from` to component `to`.
    fn ruling(&self, from: &ComponentId, to: &ComponentId, group: EdgeGroup) -> Verdict {
        if from.is_within(to) || to.is_within(from) {
            return Verdict::Allowed(Allowance::Nested);
        }
        self.rules
            .iter()
            .position(|rule| rule_matches(rule, from, to, group))
            .map_or(Verdict::Divergent, |i| {
                Verdict::Allowed(Allowance::Rule(RuleId(i)))
            })
    }

    /// The model's rules, in declaration order; [`RuleId`]s index this.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    #[must_use]
    pub fn rule(&self, id: RuleId) -> &Rule {
        &self.rules[id.0]
    }

    /// Every component pair with lifted edges, ordered by `(from, to)`.
    pub fn dependencies(&self) -> impl Iterator<Item = &Dependency> {
        self.dependencies.values()
    }

    /// The lifted edges from exactly `from` to exactly `to`, if any.
    #[must_use]
    pub fn dependency(&self, from: &ComponentId, to: &ComponentId) -> Option<&Dependency> {
        self.dependencies.get(&(from.clone(), to.clone()))
    }

    /// Pairs whose edges are all allowed, ordered by `(from, to)`.
    pub fn convergences(&self) -> impl Iterator<Item = &Dependency> {
        self.with_relation(Relation::Convergence)
    }

    /// Pairs with at least one divergent edge, ordered by `(from, to)`.
    pub fn divergences(&self) -> impl Iterator<Item = &Dependency> {
        self.with_relation(Relation::Divergence)
    }

    fn with_relation(&self, relation: Relation) -> impl Iterator<Item = &Dependency> {
        self.dependencies()
            .filter(move |dependency| dependency.relation() == relation)
    }

    /// Rules no lifted edge matches, in declaration order; rules out of the view's scope (see
    /// [`Policy::considers_rule_target`]) are never absences.
    pub fn absences(&self) -> impl Iterator<Item = (RuleId, &Rule)> {
        self.absences.iter().map(|&id| (id, self.rule(id)))
    }

    /// Workspace entities no pattern matches, ordered by path.
    #[must_use]
    pub fn unmapped(&self) -> &[Unowned] {
        &self.unmapped
    }

    /// Entities claimed by several components, ordered by path.
    #[must_use]
    pub fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
    }

    /// How many graph edges were not lifted, by reason; reasons with no edge are absent.
    #[must_use]
    pub fn skipped(&self) -> &BTreeMap<Skip, usize> {
        &self.skipped
    }

    /// Why `from -> to` is what it is: the model's ruling for a direct edge of each group, and
    /// every lifted edge from within `from` to within `to` (for two components without children,
    /// exactly [`Self::dependency`]).
    pub fn explain(
        &self,
        from: &ComponentId,
        to: &ComponentId,
    ) -> Result<Explanation, EngineError> {
        for id in [from, to] {
            if !self.components.contains(id) {
                return Err(EngineError::UnknownComponent(id.clone()));
            }
        }
        let edges = self
            .dependencies()
            .filter(|d| d.from.is_within(from) && d.to.is_within(to))
            .flat_map(|d| d.edges.iter().cloned())
            .collect();
        Ok(Explanation {
            from: from.clone(),
            to: to.clone(),
            rulings: GROUPS
                .iter()
                .map(|&group| (group, self.ruling(from, to, group)))
                .collect(),
            dependency: Dependency::new(from.clone(), to.clone(), edges),
        })
    }

    /// The pairs with every component replaced by its ancestor at `depth` (1: top level);
    /// components at `depth` or above stay as they are. See the crate docs.
    #[must_use]
    pub fn rolled_up(&self, depth: NonZeroUsize) -> RolledUp {
        RolledUp::new(depth, self.dependencies())
    }
}

/// The owner of `path`, or why it has none.
fn owner<'m>(mapping: &'m Mapping, path: &ItemPath) -> Result<&'m ComponentId, Skip> {
    match mapping.placement(path) {
        Some(Placement::Owned(assignment)) => Ok(&assignment.component),
        Some(Placement::Unmapped) => Err(Skip::Unmapped),
        Some(Placement::Conflict(_)) => Err(Skip::Conflict),
        None => Err(Skip::OutOfScope),
    }
}

/// Why the policy leaves `edge` out of the model's view, if it does: its group, its provenance,
/// its test-only scope or its target, checked in that order.
fn outside_view(policy: &Policy, mapping: &Mapping, edge: &GraphEdge) -> Option<Skip> {
    if !policy.considers_group(edge.edge.kind.group()) {
        return Some(Skip::GroupFiltered);
    }
    if !policy.considers_provenance(edge.edge.provenance) {
        return Some(Skip::ProvenanceFiltered);
    }
    let checked = match (edge.scope, policy.test_edges) {
        (Scope::Normal, TestEdges::Exclude | TestEdges::Check)
        | (Scope::TestOnly, TestEdges::Check) => true,
        (Scope::TestOnly, TestEdges::Exclude) => false,
    };
    if !checked {
        return Some(Skip::TestOnly);
    }
    if !targeted(policy, mapping, &edge.edge.dst) {
        return Some(Skip::NotTargeted);
    }
    None
}

/// Whether the policy's `targets` consider an edge into `dst`: its owner is a target or within
/// one, or, for a conflict, at least one claimant is. An unmapped or out-of-scope `dst` is
/// considered, as it may belong to a target; its edge is skipped for that reason instead.
fn targeted(policy: &Policy, mapping: &Mapping, dst: &ItemPath) -> bool {
    match mapping.placement(dst) {
        Some(Placement::Owned(assignment)) => policy.considers_target(&assignment.component),
        Some(Placement::Conflict(claims)) => claims
            .iter()
            .any(|claim| policy.considers_target(&claim.component)),
        Some(Placement::Unmapped) | None => true,
    }
}

fn rule_matches(rule: &Rule, from: &ComponentId, to: &ComponentId, group: EdgeGroup) -> bool {
    from.is_within(&rule.from)
        && to.is_within(&rule.to)
        && rule
            .kinds
            .as_ref()
            .is_none_or(|kinds| kinds.contains(&group))
}

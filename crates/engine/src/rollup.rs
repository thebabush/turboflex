//! The reflexion viewed with components collapsed to their ancestors at some depth.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use turboflex_model::ComponentId;

use crate::dependency::{Dependency, LiftedEdge, Relation};

/// Component pairs with every component replaced by its ancestor at [`Self::depth`]. Edges keep
/// the verdicts of their real components; see the crate docs.
#[derive(Debug, Clone)]
pub struct RolledUp {
    depth: NonZeroUsize,
    /// Includes pairs whose two ends rolled up into the same component.
    dependencies: BTreeMap<(ComponentId, ComponentId), Dependency>,
}

impl RolledUp {
    pub(crate) fn new<'r>(
        depth: NonZeroUsize,
        dependencies: impl Iterator<Item = &'r Dependency>,
    ) -> Self {
        let mut edges: BTreeMap<(ComponentId, ComponentId), Vec<LiftedEdge>> = BTreeMap::new();
        for dependency in dependencies {
            let key = (
                ancestor_at(&dependency.from, depth),
                ancestor_at(&dependency.to, depth),
            );
            edges
                .entry(key)
                .or_default()
                .extend(dependency.edges.iter().cloned());
        }
        let dependencies = edges
            .into_iter()
            .map(|((from, to), edges)| {
                let dependency = Dependency::new(from.clone(), to.clone(), edges);
                ((from, to), dependency)
            })
            .collect();
        Self {
            depth,
            dependencies,
        }
    }

    #[must_use]
    pub fn depth(&self) -> NonZeroUsize {
        self.depth
    }

    /// Pairs of two different rolled-up components, ordered by `(from, to)`.
    pub fn dependencies(&self) -> impl Iterator<Item = &Dependency> {
        self.dependencies.values().filter(|d| d.from != d.to)
    }

    /// The rolled-up pair `from -> to`; `from == to` gives the edges inside one rolled-up
    /// component.
    #[must_use]
    pub fn dependency(&self, from: &ComponentId, to: &ComponentId) -> Option<&Dependency> {
        self.dependencies.get(&(from.clone(), to.clone()))
    }

    /// Pairs of different components whose edges are all allowed, ordered by `(from, to)`.
    pub fn convergences(&self) -> impl Iterator<Item = &Dependency> {
        self.with_relation(Relation::Convergence)
    }

    /// Pairs of different components with at least one divergent edge, ordered by `(from, to)`.
    pub fn divergences(&self) -> impl Iterator<Item = &Dependency> {
        self.with_relation(Relation::Divergence)
    }

    fn with_relation(&self, relation: Relation) -> impl Iterator<Item = &Dependency> {
        self.dependencies()
            .filter(move |dependency| dependency.relation() == relation)
    }

    /// For each rolled-up component with edges between its nested components, those edges as a
    /// `c -> c` dependency, ordered by component.
    pub fn internal(&self) -> impl Iterator<Item = &Dependency> {
        self.dependencies.values().filter(|d| d.from == d.to)
    }
}

/// `id`'s ancestor at `depth` (`worker.acquire.fetch` at depth 1 is `worker`), or `id` itself
/// when it is at `depth` or above. Nesting is the dotted id (see [`ComponentId`]), so the
/// truncated id is always a declared ancestor.
fn ancestor_at(id: &ComponentId, depth: NonZeroUsize) -> ComponentId {
    match id.as_str().match_indices('.').nth(depth.get() - 1) {
        Some((end, _)) => ComponentId::new(&id.as_str()[..end]),
        None => id.clone(),
    }
}

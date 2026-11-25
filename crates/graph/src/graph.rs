//! The merged view of many compile units that the analysis side works on.

use std::collections::HashMap;
use std::collections::btree_map::{BTreeMap, Entry};

use crate::fact::{Edge, Entity, ItemPath, Unit, UnitHeader};
use crate::kind::{CompileMode, EdgeKind};

/// Whether a merged edge exists outside `cfg(test)` builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Seen in at least one [`CompileMode::Normal`] unit.
    Normal,
    /// Seen only in [`CompileMode::Test`] units: `#[cfg(test)]` code or test targets.
    TestOnly,
}

/// A deduplicated edge of a [`Graph`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    /// A representative occurrence: the first one from a [`Scope::Normal`] unit if any, else the
    /// first one overall. Its provenance, span and detail are that occurrence's.
    pub edge: Edge,
    pub scope: Scope,
}

/// Why units could not be merged.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("units disagree on the kind, module, crate or origin of `{path}`")]
    ConflictingEntity { path: ItemPath },
}

/// Facts of many compile units merged into one graph.
///
/// - Entities are deduplicated by path. Units must agree on everything but the span; the first
///   known span is kept.
/// - Edges are deduplicated by `(src, dst, kind)`. Kind is part of the key so a rule
///   about one [`EdgeGroup`](crate::EdgeGroup) is never hidden by another kind on the same pair;
///   provenance, span and detail are not, since the same dependency compiled in two targets (lib
///   and its `--test` build) differs only there.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    headers: Vec<UnitHeader>,
    entities: BTreeMap<ItemPath, Entity>,
    /// Sorted by the dedupe key.
    edges: Vec<GraphEdge>,
    by_src: HashMap<ItemPath, Vec<usize>>,
    by_dst: HashMap<ItemPath, Vec<usize>>,
}

type EdgeKey = (ItemPath, ItemPath, EdgeKind);

impl Graph {
    pub fn merge(units: impl IntoIterator<Item = Unit>) -> Result<Self, MergeError> {
        let mut headers = Vec::new();
        let mut entities = BTreeMap::new();
        let mut merged: BTreeMap<EdgeKey, GraphEdge> = BTreeMap::new();
        for Unit {
            header,
            entities: unit_entities,
            edges,
        } in units
        {
            let scope = match header.mode {
                CompileMode::Normal => Scope::Normal,
                CompileMode::Test => Scope::TestOnly,
            };
            for entity in unit_entities {
                match entities.entry(entity.path.clone()) {
                    Entry::Vacant(slot) => {
                        slot.insert(entity);
                    }
                    Entry::Occupied(mut slot) => merge_entity(slot.get_mut(), entity)?,
                }
            }
            for edge in edges {
                let key = (edge.src.clone(), edge.dst.clone(), edge.kind);
                match merged.entry(key) {
                    Entry::Vacant(slot) => {
                        slot.insert(GraphEdge { edge, scope });
                    }
                    Entry::Occupied(mut slot) => {
                        if slot.get().scope == Scope::TestOnly && scope == Scope::Normal {
                            slot.insert(GraphEdge { edge, scope });
                        }
                    }
                }
            }
            headers.push(header);
        }
        let edges: Vec<GraphEdge> = merged.into_values().collect();
        let mut by_src: HashMap<ItemPath, Vec<usize>> = HashMap::new();
        let mut by_dst: HashMap<ItemPath, Vec<usize>> = HashMap::new();
        for (i, GraphEdge { edge, .. }) in edges.iter().enumerate() {
            by_src.entry(edge.src.clone()).or_default().push(i);
            by_dst.entry(edge.dst.clone()).or_default().push(i);
        }
        Ok(Self {
            headers,
            entities,
            edges,
            by_src,
            by_dst,
        })
    }

    /// Headers of the merged units, in merge order.
    #[must_use]
    pub fn headers(&self) -> &[UnitHeader] {
        &self.headers
    }

    /// All entities, ordered by path.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    #[must_use]
    pub fn entity(&self, path: &ItemPath) -> Option<&Entity> {
        self.entities.get(path)
    }

    /// All edges, ordered by `(src, dst, kind)`.
    #[must_use]
    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    /// Edges starting at `src`, in [`Graph::edges`] order.
    pub fn edges_from(&self, src: &ItemPath) -> impl Iterator<Item = &GraphEdge> {
        self.indexed(&self.by_src, src)
    }

    /// Edges ending at `dst`, in [`Graph::edges`] order.
    pub fn edges_to(&self, dst: &ItemPath) -> impl Iterator<Item = &GraphEdge> {
        self.indexed(&self.by_dst, dst)
    }

    fn indexed<'a>(
        &'a self,
        index: &'a HashMap<ItemPath, Vec<usize>>,
        path: &ItemPath,
    ) -> impl Iterator<Item = &'a GraphEdge> {
        index
            .get(path)
            .into_iter()
            .flatten()
            .map(|&i| &self.edges[i])
    }
}

/// Folds a later record for the same path into `kept`.
fn merge_entity(kept: &mut Entity, other: Entity) -> Result<(), MergeError> {
    let Entity {
        path,
        kind,
        module,
        crate_name,
        origin,
        span,
    } = other;
    if kind != kept.kind
        || module != kept.module
        || crate_name != kept.crate_name
        || origin != kept.origin
    {
        return Err(MergeError::ConflictingEntity { path });
    }
    if kept.span.is_none() {
        kept.span = span;
    }
    Ok(())
}

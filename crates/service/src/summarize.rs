//! Turning a reflexion into the report's plain-data summary.

use std::collections::BTreeMap;

use turboflex_engine::{Dependency, Reflexion, Skip};
use turboflex_graph::{Graph, ModulePath};
use turboflex_model::Model;
use turboflex_report::{
    Absence, Conflict, Divergence, EdgeSample, GroupCount, Headline, ModelLocation, SkipReason,
    UnmappedModule, View,
};

use crate::Options;

/// Summarizes the reflexion of `model` (parsed with its file name as source name) over `graph`.
#[must_use]
pub fn summarize(model: &Model, graph: &Graph, reflexion: &Reflexion, options: Options) -> View {
    let divergences: Vec<Divergence> = reflexion
        .divergences()
        .map(|dependency| divergence(dependency, options.samples))
        .collect();
    let absences: Vec<Absence> = reflexion
        .absences()
        .map(|(_, rule)| Absence {
            from: rule.from.to_string(),
            to: rule.to.to_string(),
            kinds: rule
                .kinds
                .as_ref()
                .map(|kinds| kinds.iter().copied().collect()),
            location: ModelLocation {
                file: model.source_name_of(rule.location).to_owned(),
                line: rule.location.line,
                column: rule.location.column,
            },
        })
        .collect();
    let mut unmapped: BTreeMap<ModulePath, UnmappedModule> = BTreeMap::new();
    for unowned in reflexion.unmapped() {
        // The mapping only reports entities of the graph, so the lookup cannot fail; an empty
        // module keeps such an entity counted rather than lost.
        let module = graph
            .entity(&unowned.path)
            .map_or_else(|| ModulePath::new(""), |entity| entity.module.clone());
        let entry = unmapped
            .entry(module.clone())
            .or_insert_with(|| UnmappedModule {
                module,
                entities: 0,
                edges: 0,
            });
        entry.entities += 1;
        entry.edges += unowned.edges;
    }
    let conflicts: Vec<Conflict> = reflexion
        .conflicts()
        .iter()
        .map(|conflict| Conflict {
            path: conflict.path.clone(),
            claims: conflict
                .claims
                .iter()
                .map(|claim| claim.component.to_string())
                .collect(),
            edges: conflict.edges,
        })
        .collect();
    View {
        model: model.source_name().to_owned(),
        headline: Headline {
            convergences: reflexion.convergences().count(),
            divergences: divergences.len(),
            absences: absences.len(),
            unmapped: reflexion.unmapped().len(),
            conflicts: conflicts.len(),
        },
        skipped: skipped(reflexion),
        divergences,
        absences,
        unmapped: unmapped.into_values().collect(),
        conflicts,
    }
}

fn divergence(dependency: &Dependency, samples: usize) -> Divergence {
    let groups: BTreeMap<_, _> = dependency
        .groups()
        .into_iter()
        .map(|(group, counts)| {
            let counts = GroupCount {
                allowed: counts.allowed,
                divergent: counts.divergent,
            };
            (group, counts)
        })
        .collect();
    Divergence {
        from: dependency.from().to_string(),
        to: dependency.to().to_string(),
        edges: dependency.edges().len(),
        divergent: groups.values().map(|counts| counts.divergent).sum(),
        groups,
        samples: dependency
            .divergent_edges()
            .take(samples)
            .map(|lifted| {
                let edge = &lifted.edge.edge;
                EdgeSample {
                    src: edge.src.clone(),
                    dst: edge.dst.clone(),
                    kind: edge.kind,
                    provenance: edge.provenance,
                    span: edge.span.clone(),
                }
            })
            .collect(),
    }
}

/// The engine's skip counts, with every reason present: the engine omits those with no edge.
fn skipped(reflexion: &Reflexion) -> BTreeMap<SkipReason, usize> {
    let mut skipped: BTreeMap<SkipReason, usize> = SkipReason::ALL
        .into_iter()
        .map(|reason| (reason, 0))
        .collect();
    for (&skip, &count) in reflexion.skipped() {
        *skipped.entry(skip_reason(skip)).or_default() += count;
    }
    skipped
}

fn skip_reason(skip: Skip) -> SkipReason {
    match skip {
        Skip::GroupFiltered => SkipReason::GroupFiltered,
        Skip::ProvenanceFiltered => SkipReason::ProvenanceFiltered,
        Skip::TestOnly => SkipReason::TestOnly,
        Skip::Unmapped => SkipReason::Unmapped,
        Skip::Conflict => SkipReason::Conflict,
        Skip::OutOfScope => SkipReason::OutOfScope,
        Skip::NotTargeted => SkipReason::NotTargeted,
        Skip::SameComponent => SkipReason::SameComponent,
    }
}

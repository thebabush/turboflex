//! Reflexion on small hand-built models and graphs: verdicts, findings, roll-up, explain.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

use turboflex_engine::{
    Allowance, Dependency, EngineError, Explanation, GroupCounts, Reflexion, Relation, Skip,
    Verdict,
};
use turboflex_graph::{
    CompileMode, CrateName, Edge, EdgeGroup, EdgeKind, Entity, EntityKind, Graph, ItemPath,
    ModulePath, Origin, Provenance, TargetKind, Unit, UnitHeader,
};
use turboflex_model::{ComponentId, Mapping, Model};

fn entity(path: &str, module: &str, origin: Origin) -> Entity {
    let crate_name = module
        .split("::")
        .next()
        .expect("split yields at least one piece");
    Entity {
        path: ItemPath::new(path),
        kind: EntityKind::Function,
        module: ModulePath::new(module),
        crate_name: CrateName::new(crate_name),
        origin,
        span: None,
    }
}

/// An entity for `path`, its module being everything before the last `::`; `sqlx`, `serde` and
/// `std` paths are external.
fn entity_for(path: &str) -> Entity {
    let (module, _) = path.rsplit_once("::").expect("paths have a module");
    let external = ["sqlx", "serde", "std"]
        .iter()
        .any(|c| path.starts_with(&format!("{c}::")));
    let origin = if external {
        Origin::External
    } else {
        Origin::Workspace
    };
    entity(path, module, origin)
}

fn edge(src: &str, dst: &str, kind: EdgeKind) -> Edge {
    Edge {
        src: ItemPath::new(src),
        dst: ItemPath::new(dst),
        kind,
        provenance: Provenance::Source,
        span: None,
        detail: String::new(),
    }
}

fn derived(edge: Edge) -> Edge {
    Edge {
        provenance: Provenance::Derive,
        ..edge
    }
}

fn unit(mode: CompileMode, edges: Vec<Edge>) -> Unit {
    let mut paths = BTreeSet::new();
    for e in &edges {
        paths.insert(e.src.as_str().to_owned());
        paths.insert(e.dst.as_str().to_owned());
    }
    Unit {
        header: UnitHeader {
            crate_name: CrateName::new("app"),
            target: TargetKind::Lib,
            mode,
            features: Vec::new(),
        },
        entities: paths.iter().map(|p| entity_for(p)).collect(),
        edges,
    }
}

/// A graph of `normal` edges and edges seen only in `cfg(test)` builds; entities are derived
/// from the edge ends.
fn graph(normal: Vec<Edge>, test_only: Vec<Edge>) -> Graph {
    Graph::merge([
        unit(CompileMode::Normal, normal),
        unit(CompileMode::Test, test_only),
    ])
    .expect("derived entities agree")
}

fn model(text: &str) -> Model {
    Model::parse("test.kdl", text).unwrap_or_else(|e| panic!("{e}"))
}

fn id(s: &str) -> ComponentId {
    ComponentId::new(s)
}

fn reflexion(model: &Model, graph: &Graph) -> Reflexion {
    Reflexion::compute(model, graph, &Mapping::new(model, graph))
}

fn pairs<'r>(deps: impl Iterator<Item = &'r Dependency>) -> Vec<(&'r str, &'r str)> {
    deps.map(|d| (d.from().as_str(), d.to().as_str())).collect()
}

fn dep<'r>(reflexion: &'r Reflexion, from: &str, to: &str) -> &'r Dependency {
    reflexion
        .dependency(&id(from), &id(to))
        .unwrap_or_else(|| panic!("no dependency {from} -> {to}"))
}

/// A verdict as text: `divergent`, `nested` or the rule `F->G`.
fn describe(reflexion: &Reflexion, verdict: &Verdict) -> String {
    match verdict {
        Verdict::Divergent => "divergent".to_owned(),
        Verdict::Allowed(Allowance::Nested) => "nested".to_owned(),
        Verdict::Allowed(Allowance::Rule(rule)) => {
            let rule = reflexion.rule(*rule);
            format!("{}->{}", rule.from, rule.to)
        }
    }
}

/// What allowed a dependency's allowed edges.
fn rule_allowances(reflexion: &Reflexion, dependency: &Dependency) -> Vec<String> {
    dependency
        .allowances()
        .into_keys()
        .map(|allowance| describe(reflexion, &Verdict::Allowed(allowance.clone())))
        .collect()
}

/// The rulings of an explanation as text, in group order.
fn rulings(reflexion: &Reflexion, explanation: &Explanation) -> Vec<String> {
    explanation
        .rulings
        .iter()
        .map(|(group, verdict)| format!("{group:?}: {}", describe(reflexion, verdict)))
        .collect()
}

const MODEL: &str = r#"
component "model" { modules "app::model" }
component "ledger" { modules "app::db" }
component "worker" {
    modules "app::worker"
    component "runtime" { modules "app::worker::v2" }
    component "acquire" {
        modules "app::ingest"
        component "fetch" { modules "app::ingest::fetch" }
    }
    component "analyze" { modules "app::analyze" }
}
component "report" { modules "app::report" }
component "cli" { modules "app::cli" }
external "postgres" { crates "sqlx" }

allow "worker" -> "postgres" kinds="types"
allow "worker.runtime" -> "worker.acquire"
allow "report" -> "ledger" kinds="types"
allow "cli" -> "report"
allow "report" -> "model"
"#;

fn main_graph() -> Graph {
    graph(
        vec![
            // Rule source inherited: `worker` -> postgres covers worker.acquire.fetch.
            edge(
                "app::ingest::fetch::get",
                "sqlx::Pool",
                EdgeKind::UsesTypeSig,
            ),
            // ... but only for types.
            edge("app::ingest::run", "sqlx::query", EdgeKind::Calls),
            // Parent target: `worker.runtime -> worker.acquire` covers worker.acquire.fetch.
            edge(
                "app::worker::v2::tick",
                "app::ingest::fetch::get",
                EdgeKind::Calls,
            ),
            // Parent and child, both ways.
            edge(
                "app::worker::v2::tick",
                "app::worker::shared",
                EdgeKind::Calls,
            ),
            edge("app::worker::shared", "app::analyze::scan", EdgeKind::Calls),
            // Siblings.
            edge("app::analyze::scan", "app::ingest::run", EdgeKind::Calls),
            // A rule with no `kinds`: every group.
            edge(
                "app::report::render",
                "app::model::Job",
                EdgeKind::UsesTypeBody,
            ),
            // Allowed types and divergent calls on one pair.
            edge("app::report::render", "app::db::Row", EdgeKind::UsesTypeSig),
            edge("app::report::render", "app::db::get", EdgeKind::Calls),
            edge("app::report::render", "app::db::put", EdgeKind::Calls),
            // Skipped: unmapped source, out-of-scope destination, one component.
            edge("app::stray::f", "app::db::get", EdgeKind::Calls),
            edge("app::db::get", "serde::Serialize", EdgeKind::Bound),
            edge("app::db::get", "app::db::put", EdgeKind::Calls),
        ],
        vec![edge("app::cli::main", "app::db::get", EdgeKind::Calls)],
    )
}

#[test]
fn verdicts_and_relations() {
    let model = model(MODEL);
    let reflexion = reflexion(&model, &main_graph());

    assert_eq!(
        pairs(reflexion.convergences()),
        [
            ("report", "model"),
            ("worker", "worker.analyze"),
            ("worker.acquire.fetch", "postgres"),
            ("worker.runtime", "worker"),
            ("worker.runtime", "worker.acquire.fetch"),
        ]
    );
    assert_eq!(
        pairs(reflexion.divergences()),
        [
            ("report", "ledger"),
            ("worker.acquire", "postgres"),
            ("worker.analyze", "worker.acquire"),
        ]
    );

    let fetch = dep(&reflexion, "worker.acquire.fetch", "postgres");
    assert_eq!(rule_allowances(&reflexion, fetch), ["worker->postgres"]);
    let runtime = dep(&reflexion, "worker.runtime", "worker.acquire.fetch");
    assert_eq!(
        rule_allowances(&reflexion, runtime),
        ["worker.runtime->worker.acquire"]
    );
    for (from, to) in [("worker.runtime", "worker"), ("worker", "worker.analyze")] {
        let nested = dep(&reflexion, from, to);
        assert_eq!(rule_allowances(&reflexion, nested), ["nested"], "{from}");
    }
    let model_rule = dep(&reflexion, "report", "model");
    assert_eq!(rule_allowances(&reflexion, model_rule), ["report->model"]);

    // Allowed types, divergent calls: a divergence, for the calls only.
    let report = dep(&reflexion, "report", "ledger");
    assert_eq!(report.relation(), Relation::Divergence);
    assert_eq!(
        report.groups(),
        BTreeMap::from([
            (
                EdgeGroup::Types,
                GroupCounts {
                    allowed: 1,
                    divergent: 0
                }
            ),
            (
                EdgeGroup::Calls,
                GroupCounts {
                    allowed: 0,
                    divergent: 2
                }
            ),
        ])
    );
    assert_eq!(
        report.divergent_groups(),
        BTreeSet::from([EdgeGroup::Calls])
    );
    assert_eq!(rule_allowances(&reflexion, report), ["report->ledger"]);
    let divergent: Vec<&str> = report
        .divergent_edges()
        .map(|e| e.edge.edge.dst.as_str())
        .collect();
    assert_eq!(divergent, ["app::db::get", "app::db::put"]);
    assert_eq!(
        report.by_kind(),
        BTreeMap::from([(EdgeKind::Calls, 2), (EdgeKind::UsesTypeSig, 1)])
    );
    assert_eq!(
        report.by_provenance(),
        BTreeMap::from([(Provenance::Source, 3)])
    );
    // Edges in (src, dst, kind) order.
    let order: Vec<&str> = report
        .edges()
        .iter()
        .map(|e| e.edge.edge.dst.as_str())
        .collect();
    assert_eq!(order, ["app::db::Row", "app::db::get", "app::db::put"]);

    let analyze = dep(&reflexion, "worker.analyze", "worker.acquire");
    assert!(
        analyze
            .edges()
            .iter()
            .all(|e| e.verdict == Verdict::Divergent)
    );
}

#[test]
fn absences_unmapped_and_skipped() {
    let model = model(MODEL);
    let reflexion = reflexion(&model, &main_graph());
    let absences: Vec<String> = reflexion
        .absences()
        .map(|(_, rule)| format!("{}->{}", rule.from, rule.to))
        .collect();
    assert_eq!(absences, ["cli->report"]);

    let unmapped: Vec<(&str, usize)> = reflexion
        .unmapped()
        .iter()
        .map(|u| (u.path.as_str(), u.edges))
        .collect();
    assert_eq!(unmapped, [("app::stray::f", 1)]);
    assert_eq!(reflexion.conflicts(), []);
    assert_eq!(
        reflexion.skipped(),
        &BTreeMap::from([
            (Skip::TestOnly, 1),
            (Skip::Unmapped, 1),
            (Skip::OutOfScope, 1),
            (Skip::SameComponent, 1),
        ])
    );
}

#[test]
fn test_edges_policy() {
    let excluded = model(MODEL);
    let reflexion_excluded = reflexion(&excluded, &main_graph());
    assert!(
        reflexion_excluded
            .dependency(&id("cli"), &id("ledger"))
            .is_none()
    );

    let checked = model(&format!("{MODEL}\npolicy {{ test-edges \"check\" }}\n"));
    let reflexion_checked = reflexion(&checked, &main_graph());
    let cli = dep(&reflexion_checked, "cli", "ledger");
    assert_eq!(cli.relation(), Relation::Divergence);
    assert!(!reflexion_checked.skipped().contains_key(&Skip::TestOnly));
}

#[test]
fn an_edge_of_the_wrong_kind_does_not_support_a_rule() {
    let model = model(
        r#"
component "a" { modules "app::a" }
component "b" { modules "app::b" }
allow "a" -> "b" kinds="types"
"#,
    );
    let calls = graph(
        vec![edge("app::a::f", "app::b::g", EdgeKind::Calls)],
        vec![],
    );
    let reflexion_calls = reflexion(&model, &calls);
    assert_eq!(reflexion_calls.absences().count(), 1);
    assert_eq!(pairs(reflexion_calls.divergences()), [("a", "b")]);

    let types = graph(
        vec![edge("app::a::f", "app::b::T", EdgeKind::GenericArg)],
        vec![],
    );
    assert_eq!(reflexion(&model, &types).absences().count(), 0);
}

const VIEW: &str = r#"
component "a" { modules "app::a" }
component "b" { modules "app::b" }
component "c" { modules "app::c" }
allow "a" -> "b" kinds="matches"
allow "a" -> "c" kinds="constructs"
"#;

#[test]
fn edges_outside_the_group_filter_are_not_lifted() {
    let model = model(&format!(
        "{VIEW}\npolicy {{ edges \"constructs\" \"matches\" }}\n"
    ));
    let graph = graph(
        vec![
            edge("app::a::f", "app::b::Format", EdgeKind::MatchesOn),
            edge("app::a::f", "app::b::g", EdgeKind::Calls),
            edge("app::a::f", "app::b::T", EdgeKind::UsesTypeSig),
            edge("app::b::g", "app::c::Payload", EdgeKind::Constructs),
            edge("app::a::f", "app::c::Payload", EdgeKind::Constructs),
        ],
        vec![],
    );
    let reflexion = reflexion(&model, &graph);
    // Without the filter, a -> b would diverge on its calls and types.
    assert_eq!(pairs(reflexion.convergences()), [("a", "b"), ("a", "c")]);
    assert_eq!(pairs(reflexion.divergences()), [("b", "c")]);
    assert_eq!(
        dep(&reflexion, "a", "b").by_kind(),
        BTreeMap::from([(EdgeKind::MatchesOn, 1)])
    );
    assert_eq!(reflexion.absences().count(), 0);
    assert_eq!(
        reflexion.skipped(),
        &BTreeMap::from([(Skip::GroupFiltered, 2)])
    );
    let explanation = reflexion
        .explain(&id("a"), &id("b"))
        .expect("known components");
    assert_eq!(explanation.dependency.edges().len(), 1);
}

#[test]
fn edges_outside_the_provenance_filter_are_not_lifted() {
    let graph = graph(
        vec![
            edge("app::a::f", "app::b::Format", EdgeKind::MatchesOn),
            // A derived impl in `a` building `c`'s type: not a hand-written decision.
            derived(edge(
                "app::a::clone",
                "app::c::Payload",
                EdgeKind::Constructs,
            )),
            derived(edge("app::b::eq", "app::c::Payload", EdgeKind::MatchesOn)),
        ],
        vec![],
    );

    let unfiltered = reflexion(&model(VIEW), &graph);
    assert_eq!(unfiltered.absences().count(), 0);
    assert_eq!(pairs(unfiltered.divergences()), [("b", "c")]);
    assert_eq!(
        dep(&unfiltered, "a", "c").by_provenance(),
        BTreeMap::from([(Provenance::Derive, 1)])
    );

    let model = model(&format!(
        "{VIEW}\npolicy {{ provenance \"source\" \"macro_expanded\" \"desugared\" }}\n"
    ));
    let filtered = reflexion(&model, &graph);
    assert_eq!(pairs(filtered.convergences()), [("a", "b")]);
    assert_eq!(filtered.divergences().count(), 0);
    // The derived edge was the only support of `a -> c`.
    let absences: Vec<String> = filtered
        .absences()
        .map(|(_, rule)| format!("{}->{}", rule.from, rule.to))
        .collect();
    assert_eq!(absences, ["a->c"]);
    assert_eq!(
        filtered.skipped(),
        &BTreeMap::from([(Skip::ProvenanceFiltered, 2)])
    );
}

#[test]
fn filtered_edges_come_before_every_other_skip() {
    let model = model(&format!(
        "{VIEW}\npolicy {{\n    edges \"matches\" \"constructs\"\n    provenance \"source\"\n}}\n"
    ));
    let graph = graph(
        vec![
            // Group and provenance both filtered: counted once, under the group.
            derived(edge("app::a::f", "app::b::g", EdgeKind::Calls)),
            // Would be unmapped and out of scope.
            edge("app::stray::f", "app::b::g", EdgeKind::Calls),
            derived(edge("app::stray::f", "app::b::T", EdgeKind::MatchesOn)),
            edge("app::a::f", "serde::Serialize", EdgeKind::Bound),
            // Kept: counted as unmapped.
            edge("app::stray::f", "app::b::U", EdgeKind::MatchesOn),
        ],
        // Would be test-only.
        vec![
            edge("app::a::t", "app::b::g", EdgeKind::Calls),
            edge("app::a::t", "app::c::P", EdgeKind::Constructs),
        ],
    );
    let reflexion = reflexion(&model, &graph);
    assert_eq!(
        reflexion.skipped(),
        &BTreeMap::from([
            (Skip::GroupFiltered, 4),
            (Skip::ProvenanceFiltered, 1),
            (Skip::TestOnly, 1),
            (Skip::Unmapped, 1),
        ])
    );
    let unmapped: Vec<(&str, usize)> = reflexion
        .unmapped()
        .iter()
        .map(|u| (u.path.as_str(), u.edges))
        .collect();
    assert_eq!(unmapped, [("app::stray::f", 1)]);
    assert_eq!(reflexion.dependencies().count(), 0);
}

#[test]
fn conflicts_are_reported_and_not_checked() {
    let model = model(
        r#"
component "analyze" { modules "app::m" }
component "a" { items "app::m::first" follow=#true }
component "b" { items "app::m::second" follow=#true }
component "c" { modules "app::c" }
"#,
    );
    let graph = graph(
        vec![
            edge("app::m::first", "app::m::helper", EdgeKind::Calls),
            edge("app::m::second", "app::m::helper", EdgeKind::Calls),
            edge("app::c::f", "app::m::helper", EdgeKind::Calls),
        ],
        vec![],
    );
    let reflexion = reflexion(&model, &graph);
    let [conflict] = reflexion.conflicts() else {
        panic!("{:?}", reflexion.conflicts())
    };
    assert_eq!(conflict.path.as_str(), "app::m::helper");
    let claimants: Vec<&str> = conflict
        .claims
        .iter()
        .map(|c| c.component.as_str())
        .collect();
    assert_eq!(claimants, ["a", "b"]);
    assert_eq!(conflict.edges, 3);
    assert_eq!(reflexion.dependencies().count(), 0);
    assert_eq!(reflexion.skipped(), &BTreeMap::from([(Skip::Conflict, 3)]));
}

const TARGETS: &str = r#"
component "a" { modules "app::a" }
component "b" { modules "app::b" }
component "c" { modules "app::c" }
allow "a" -> "c"
allow "b" -> "a"
"#;

fn unmapped_edges(reflexion: &Reflexion) -> Vec<(&str, usize)> {
    reflexion
        .unmapped()
        .iter()
        .map(|u| (u.path.as_str(), u.edges))
        .collect()
}

#[test]
fn edges_into_components_outside_the_targets_are_not_lifted() {
    let graph = graph(
        vec![
            edge("app::a::f", "app::c::g", EdgeKind::Calls),
            edge("app::b::f", "app::c::g", EdgeKind::Calls),
            // Into `b`, not a target.
            edge("app::a::f", "app::b::g", EdgeKind::Calls),
            // Into `b` from an unmapped entity: not counted against it.
            edge("app::stray::f", "app::b::g", EdgeKind::Calls),
            // Into `c` from an unmapped entity: counted.
            edge("app::stray::f", "app::c::h", EdgeKind::Calls),
            // An unmapped or out-of-scope target may belong to a target: skipped as such.
            edge("app::a::f", "app::stray::x", EdgeKind::Calls),
            edge("app::a::f", "serde::Serialize", EdgeKind::Bound),
            edge("app::c::f", "app::c::g", EdgeKind::Calls),
        ],
        vec![],
    );

    let all = reflexion(&model(TARGETS), &graph);
    assert_eq!(pairs(all.convergences()), [("a", "c")]);
    assert_eq!(pairs(all.divergences()), [("a", "b"), ("b", "c")]);
    assert!(!all.skipped().contains_key(&Skip::NotTargeted));
    assert_eq!(
        unmapped_edges(&all),
        [("app::stray::f", 2), ("app::stray::x", 1)]
    );

    let model = model(&format!("{TARGETS}\npolicy {{ targets \"c\" }}\n"));
    let targeted = reflexion(&model, &graph);
    assert_eq!(pairs(targeted.convergences()), [("a", "c")]);
    assert_eq!(pairs(targeted.divergences()), [("b", "c")]);
    // `b -> a` is out of the view's scope: not an absence.
    assert_eq!(targeted.absences().count(), 0);
    assert_eq!(
        targeted.skipped(),
        &BTreeMap::from([
            (Skip::Unmapped, 2),
            (Skip::OutOfScope, 1),
            (Skip::NotTargeted, 2),
            (Skip::SameComponent, 1),
        ])
    );
    assert_eq!(
        unmapped_edges(&targeted),
        [("app::stray::f", 1), ("app::stray::x", 1)]
    );
}

#[test]
fn a_conflict_is_targeted_when_one_claimant_is() {
    let components = r#"
component "analyze" { modules "app::m" }
component "a" { items "app::m::first" follow=#true }
component "b" { items "app::m::second" follow=#true }
component "c" { modules "app::c" }
"#;
    let graph = graph(
        vec![
            edge("app::m::first", "app::m::helper", EdgeKind::Calls),
            edge("app::m::second", "app::m::helper", EdgeKind::Calls),
            edge("app::c::f", "app::m::helper", EdgeKind::Calls),
        ],
        vec![],
    );

    let claimed = model(&format!("{components}\npolicy {{ targets \"b\" }}\n"));
    let claimed = reflexion(&claimed, &graph);
    assert_eq!(claimed.skipped(), &BTreeMap::from([(Skip::Conflict, 3)]));
    let [conflict] = claimed.conflicts() else {
        panic!("{:?}", claimed.conflicts())
    };
    assert_eq!(conflict.edges, 3);

    let unclaimed = model(&format!("{components}\npolicy {{ targets \"c\" }}\n"));
    let unclaimed = reflexion(&unclaimed, &graph);
    assert_eq!(
        unclaimed.skipped(),
        &BTreeMap::from([(Skip::NotTargeted, 3)])
    );
    let [conflict] = unclaimed.conflicts() else {
        panic!("{:?}", unclaimed.conflicts())
    };
    assert_eq!(conflict.edges, 0);
}

const NESTED: &str = r#"
component "a" { modules "app::a" }
component "c" {
    modules "app::c"
    component "inner" { modules "app::c::inner" }
    component "other" { modules "app::c::other" }
}
allow "a" -> "c"
"#;

#[test]
fn a_rule_to_an_ancestor_of_a_target_counts_only_targeted_edges() {
    let model = model(&format!("{NESTED}\npolicy {{ targets \"c.inner\" }}\n"));
    let other = graph(
        vec![edge("app::a::f", "app::c::other::g", EdgeKind::Calls)],
        vec![],
    );
    let untargeted = reflexion(&model, &other);
    assert_eq!(untargeted.dependencies().count(), 0);
    let absences: Vec<String> = untargeted
        .absences()
        .map(|(_, rule)| format!("{}->{}", rule.from, rule.to))
        .collect();
    assert_eq!(absences, ["a->c"]);

    let both = graph(
        vec![
            edge("app::a::f", "app::c::other::g", EdgeKind::Calls),
            edge("app::a::f", "app::c::inner::g", EdgeKind::Calls),
        ],
        vec![],
    );
    let targeted = reflexion(&model, &both);
    assert_eq!(pairs(targeted.convergences()), [("a", "c.inner")]);
    assert_eq!(targeted.absences().count(), 0);
    assert_eq!(
        targeted.skipped(),
        &BTreeMap::from([(Skip::NotTargeted, 1)])
    );
}

#[test]
fn components_nested_in_a_target_are_targeted() {
    let model = model(&format!("{NESTED}\npolicy {{ targets \"c\" }}\n"));
    let graph = graph(
        vec![
            edge("app::a::f", "app::c::inner::g", EdgeKind::Calls),
            edge("app::c::f", "app::a::g", EdgeKind::Calls),
        ],
        vec![],
    );
    let reflexion = reflexion(&model, &graph);
    assert_eq!(pairs(reflexion.convergences()), [("a", "c.inner")]);
    assert_eq!(
        reflexion.skipped(),
        &BTreeMap::from([(Skip::NotTargeted, 1)])
    );
}

#[test]
fn roll_up_keeps_edge_verdicts() {
    let model = model(MODEL);
    let reflexion = reflexion(&model, &main_graph());
    let top = reflexion.rolled_up(NonZeroUsize::MIN);
    assert_eq!(top.depth(), NonZeroUsize::MIN);
    assert_eq!(pairs(top.convergences()), [("report", "model")]);
    // worker.acquire.fetch -> postgres (allowed) and worker.acquire -> postgres (divergent calls)
    // roll into one worker -> postgres divergence.
    assert_eq!(
        pairs(top.divergences()),
        [("report", "ledger"), ("worker", "postgres")]
    );
    let worker = top
        .dependency(&id("worker"), &id("postgres"))
        .expect("rolled-up pair");
    assert_eq!(worker.edges().len(), 2);
    assert_eq!(
        worker.divergent_groups(),
        BTreeSet::from([EdgeGroup::Calls])
    );
    // The sibling divergence inside `worker` is kept, not dropped.
    let internal = pairs(top.internal());
    assert_eq!(internal, [("worker", "worker")]);
    let inside = top
        .dependency(&id("worker"), &id("worker"))
        .expect("internal edges");
    assert_eq!(inside.relation(), Relation::Divergence);
    assert_eq!(inside.edges().len(), 4);

    // Depth 2 collapses only worker.acquire.fetch.
    let two = reflexion.rolled_up(NonZeroUsize::new(2).expect("non-zero"));
    assert_eq!(
        pairs(two.convergences()),
        [
            ("report", "model"),
            ("worker", "worker.analyze"),
            ("worker.runtime", "worker"),
            ("worker.runtime", "worker.acquire"),
        ]
    );
    assert_eq!(
        pairs(two.divergences()),
        [
            ("report", "ledger"),
            ("worker.acquire", "postgres"),
            ("worker.analyze", "worker.acquire"),
        ]
    );
    assert_eq!(two.internal().count(), 0);
}

#[test]
fn explain_gives_rulings_and_subtree_edges() {
    let model = model(MODEL);
    let reflexion = reflexion(&model, &main_graph());

    let report = reflexion
        .explain(&id("report"), &id("ledger"))
        .expect("known components");
    assert_eq!(
        rulings(&reflexion, &report),
        [
            "Types: report->ledger",
            "Calls: divergent",
            "Impls: divergent",
            "Macros: divergent",
            "Globals: divergent",
            "Constructs: divergent",
            "Matches: divergent",
        ]
    );
    assert_eq!(&report.dependency, dep(&reflexion, "report", "ledger"));

    // A parent gathers its subtree's edges, each with its own verdict.
    let worker = reflexion
        .explain(&id("worker"), &id("postgres"))
        .expect("known components");
    assert_eq!(
        describe(&reflexion, &worker.rulings[&EdgeGroup::Types]),
        "worker->postgres"
    );
    assert_eq!(worker.rulings[&EdgeGroup::Calls], Verdict::Divergent);
    let froms: Vec<(&str, bool)> = worker
        .dependency
        .edges()
        .iter()
        .map(|e| (e.from.as_str(), e.verdict.is_divergent()))
        .collect();
    assert_eq!(
        froms,
        [("worker.acquire.fetch", false), ("worker.acquire", true)]
    );

    // No edges: still a ruling.
    let cli = reflexion
        .explain(&id("cli"), &id("report"))
        .expect("known components");
    assert_eq!(cli.dependency.edges(), []);
    assert_eq!(
        describe(&reflexion, &cli.rulings[&EdgeGroup::Calls]),
        "cli->report"
    );
    let nested = reflexion
        .explain(&id("worker.acquire.fetch"), &id("worker"))
        .expect("known components");
    assert_eq!(
        nested.rulings[&EdgeGroup::Calls],
        Verdict::Allowed(Allowance::Nested)
    );

    assert_eq!(
        reflexion.explain(&id("nope"), &id("ledger")),
        Err(EngineError::UnknownComponent(id("nope")))
    );
}

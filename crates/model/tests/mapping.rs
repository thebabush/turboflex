//! Mapping entities to components: specificity, externals, unused patterns, helper-follow.

use turboflex_graph::{
    CompileMode, CrateName, Edge, EdgeKind, Entity, EntityKind, Graph, ItemPath, ModulePath,
    Origin, Provenance, TargetKind, Unit, UnitHeader,
};
use turboflex_model::{ComponentId, Mapping, Model, Placement, Reason, Selector};

fn entity(path: &str, kind: EntityKind, module: &str, origin: Origin) -> Entity {
    let crate_name = module
        .split("::")
        .next()
        .expect("split yields at least one piece");
    Entity {
        path: ItemPath::new(path),
        kind,
        module: ModulePath::new(module),
        crate_name: CrateName::new(crate_name),
        origin,
        span: None,
    }
}

/// A workspace function `module::name`.
fn func(module: &str, name: &str) -> Entity {
    entity(
        &format!("{module}::{name}"),
        EntityKind::Function,
        module,
        Origin::Workspace,
    )
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

fn graph(entities: Vec<Entity>, edges: Vec<Edge>) -> Graph {
    let unit = Unit {
        header: UnitHeader {
            crate_name: CrateName::new("app"),
            target: TargetKind::Lib,
            mode: CompileMode::Normal,
            features: Vec::new(),
        },
        entities,
        edges,
    };
    Graph::merge([unit]).expect("one unit always merges")
}

fn model(text: &str) -> Model {
    Model::parse("test.kdl", text).unwrap_or_else(|e| panic!("{e}"))
}

fn owner<'m>(mapping: &'m Mapping, path: &str) -> Option<&'m str> {
    mapping
        .component_of(&ItemPath::new(path))
        .map(ComponentId::as_str)
}

/// The text of the pattern that placed `path`.
fn why(model: &Model, mapping: &Mapping, path: &str) -> String {
    let assignment = mapping
        .assignment(&ItemPath::new(path))
        .expect("the entity has an owner");
    match &assignment.reason {
        Reason::Pattern(id) => model.pattern(*id).selector.to_string(),
        Reason::Follows { anchor, .. } => format!("follows {anchor}"),
    }
}

#[test]
fn most_specific_pattern_wins() {
    let model = model(
        r#"
component "root" { modules "app" "app::**" }
component "deep" { modules "app::a::**" }
component "exact" { modules "app::a::b" }
component "item" { items "app::a::b::special" }
"#,
    );
    let graph = graph(
        vec![
            func("app", "main"),
            func("app::x", "f"),
            func("app::a", "f"),
            func("app::a::b", "f"),
            func("app::a::b", "special"),
            func("app::a::b::c", "f"),
        ],
        vec![],
    );
    let mapping = Mapping::new(&model, &graph);
    let cases = [
        ("app::main", "root", "app"),
        ("app::x::f", "root", "app::**"),
        // `app::a::**` does not include `app::a` itself.
        ("app::a::f", "root", "app::**"),
        ("app::a::b::f", "exact", "app::a::b"),
        ("app::a::b::special", "item", "app::a::b::special"),
        ("app::a::b::c::f", "deep", "app::a::**"),
    ];
    for (path, component, pattern) in cases {
        assert_eq!(owner(&mapping, path), Some(component), "{path}");
        assert_eq!(why(&model, &mapping, path), pattern, "{path}");
    }
    assert_eq!(mapping.unmapped().count(), 0);
    assert_eq!(mapping.conflicts().count(), 0);
    assert_eq!(mapping.unused_patterns(), []);
}

#[test]
fn unmapped_workspace_and_out_of_scope_externals() {
    let model = model(
        r#"
component "app" { modules "app::core" }
external "postgres" { crates "sqlx" "sqlx_*" }
external "io" {
    modules "std::fs" "std::fs::**"
    items "std::env::var"
}
external "everything-std" { crates "std" }
component "unused" { modules "app::gone"; items "app::core::nope" }
"#,
    );
    let ext =
        |path: &str, module: &str| entity(path, EntityKind::Function, module, Origin::External);
    let mut core_pool = ext("sqlx_core::pool::Pool", "sqlx_core::pool");
    core_pool.kind = EntityKind::Struct;
    let graph = graph(
        vec![
            func("app::core", "run"),
            func("app::other", "lonely"),
            ext("sqlx::query", "sqlx"),
            core_pool,
            ext("std::fs::read", "std::fs"),
            ext("std::fs::unix::chown", "std::fs::unix"),
            ext("std::env::var", "std::env"),
            ext("std::env::args", "std::env"),
            ext("serde::Serialize", "serde"),
            // A workspace module that looks like an external crate is still workspace code.
            func("sqlx_shim", "f"),
        ],
        vec![],
    );
    let mapping = Mapping::new(&model, &graph);
    assert_eq!(owner(&mapping, "app::core::run"), Some("app"));
    assert_eq!(owner(&mapping, "sqlx::query"), Some("postgres"));
    assert_eq!(why(&model, &mapping, "sqlx::query"), "sqlx");
    assert_eq!(owner(&mapping, "sqlx_core::pool::Pool"), Some("postgres"));
    assert_eq!(why(&model, &mapping, "sqlx_core::pool::Pool"), "sqlx_*");
    // A module pattern beats a crate pattern.
    assert_eq!(owner(&mapping, "std::fs::read"), Some("io"));
    assert_eq!(owner(&mapping, "std::fs::unix::chown"), Some("io"));
    assert_eq!(owner(&mapping, "std::env::var"), Some("io"));
    assert_eq!(owner(&mapping, "std::env::args"), Some("everything-std"));
    // No external claims serde: out of scope, which is not "unmapped".
    assert_eq!(mapping.placement(&ItemPath::new("serde::Serialize")), None);
    let unmapped: Vec<&str> = mapping.unmapped().map(ItemPath::as_str).collect();
    assert_eq!(unmapped, ["app::other::lonely", "sqlx_shim::f"]);
    assert_eq!(
        mapping.placement(&ItemPath::new("app::other::lonely")),
        Some(&Placement::Unmapped)
    );
    let unused: Vec<String> = mapping
        .unused_patterns()
        .iter()
        .map(|id| model.pattern(*id).selector.to_string())
        .collect();
    assert_eq!(unused, ["app::gone", "app::core::nope"]);
}

/// Module `app::m` belongs to `analyze`; `artifacts` takes `app::m::anchor` with its helpers.
fn follow_model(follow: bool) -> Model {
    model(&format!(
        r#"
component "analyze" {{ modules "app::m" }}
component "artifacts" {{ items "app::m::anchor" follow=#{follow} }}
component "other" {{ modules "app::other"; items "app::m::pinned" }}
"#
    ))
}

fn follow_graph() -> Graph {
    let m = "app::m";
    let ty = |name: &str| {
        entity(
            &format!("{m}::{name}"),
            EntityKind::Struct,
            m,
            Origin::Workspace,
        )
    };
    let mut impl_block = func(m, "{impl app::m::Only}");
    impl_block.kind = EntityKind::Impl;
    graph(
        vec![
            func(m, "anchor"),
            func(m, "stays"),
            ty("Only"),
            impl_block,
            ty("Deep"),
            ty("Shared"),
            ty("CycleA"),
            ty("CycleB"),
            ty("UsedElsewhere"),
            ty("Unrelated"),
            ty("pinned"),
            ty("BehindShared"),
            func("app::other", "user"),
        ],
        vec![
            // Only the anchor uses `Only`, and `Only` uses `Deep`: both follow, with the impl.
            edge("app::m::anchor", "app::m::Only", EdgeKind::UsesTypeBody),
            edge("app::m::Only", "app::m::Deep", EdgeKind::UsesTypeField),
            edge(
                "app::m::{impl app::m::Only}",
                "app::m::Only",
                EdgeKind::ImplForType,
            ),
            // `stays` (still in analyze) also uses `Shared`: it stays, and so does what only it
            // reaches.
            edge("app::m::anchor", "app::m::Shared", EdgeKind::UsesTypeSig),
            edge("app::m::stays", "app::m::Shared", EdgeKind::UsesTypeSig),
            edge(
                "app::m::Shared",
                "app::m::BehindShared",
                EdgeKind::UsesTypeField,
            ),
            // A cycle reachable only from the anchor follows as a whole.
            edge("app::m::anchor", "app::m::CycleA", EdgeKind::Calls),
            edge("app::m::CycleA", "app::m::CycleB", EdgeKind::Calls),
            edge("app::m::CycleB", "app::m::CycleA", EdgeKind::Calls),
            // References from other components do not hold a helper back.
            edge("app::m::anchor", "app::m::UsedElsewhere", EdgeKind::Calls),
            edge("app::other::user", "app::m::UsedElsewhere", EdgeKind::Calls),
            // Item patterns are never overridden by following.
            edge("app::m::anchor", "app::m::pinned", EdgeKind::Calls),
        ],
    )
}

#[test]
fn helpers_follow_their_item() {
    let model = follow_model(true);
    let mapping = Mapping::new(&model, &follow_graph());
    let expected = [
        ("app::m::anchor", "artifacts"),
        ("app::m::Only", "artifacts"),
        ("app::m::{impl app::m::Only}", "artifacts"),
        ("app::m::Deep", "artifacts"),
        ("app::m::CycleA", "artifacts"),
        ("app::m::CycleB", "artifacts"),
        ("app::m::UsedElsewhere", "artifacts"),
        ("app::m::Shared", "analyze"),
        ("app::m::BehindShared", "analyze"),
        ("app::m::stays", "analyze"),
        ("app::m::Unrelated", "analyze"),
        ("app::m::pinned", "other"),
    ];
    for (path, component) in expected {
        assert_eq!(owner(&mapping, path), Some(component), "{path}");
    }
    assert_eq!(why(&model, &mapping, "app::m::anchor"), "app::m::anchor");
    let Reason::Follows { anchor, pattern } = &mapping
        .assignment(&ItemPath::new("app::m::Deep"))
        .unwrap()
        .reason
    else {
        panic!("Deep should follow")
    };
    assert_eq!(anchor.as_str(), "app::m::anchor");
    assert!(matches!(
        model.pattern(*pattern).selector,
        Selector::Item(_)
    ));
}

#[test]
fn without_follow_only_the_item_moves() {
    let mapping = Mapping::new(&follow_model(false), &follow_graph());
    assert_eq!(owner(&mapping, "app::m::anchor"), Some("artifacts"));
    for helper in ["app::m::Only", "app::m::Deep", "app::m::CycleA"] {
        assert_eq!(owner(&mapping, helper), Some("analyze"), "{helper}");
    }
}

#[test]
fn helpers_claimed_by_two_components_conflict() {
    let model = model(
        r#"
component "analyze" { modules "app::m" }
component "a" { items "app::m::first" follow=#true }
component "b" { items "app::m::second" follow=#true }
"#,
    );
    let graph = graph(
        vec![
            func("app::m", "first"),
            func("app::m", "second"),
            func("app::m", "helper"),
        ],
        vec![
            edge("app::m::first", "app::m::helper", EdgeKind::Calls),
            edge("app::m::second", "app::m::helper", EdgeKind::Calls),
        ],
    );
    let mapping = Mapping::new(&model, &graph);
    let conflicts: Vec<_> = mapping.conflicts().collect();
    let [(path, claims)] = conflicts.as_slice() else {
        panic!("{conflicts:?}")
    };
    assert_eq!(path.as_str(), "app::m::helper");
    let claimants: Vec<&str> = claims.iter().map(|c| c.component.as_str()).collect();
    assert_eq!(claimants, ["a", "b"]);
    assert_eq!(owner(&mapping, "app::m::helper"), None);
}

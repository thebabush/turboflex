//! Round-trips, strict reading and merge semantics of the fact schema.

use std::num::NonZeroU32;

use turboflex_graph::{
    CompileMode, CrateName, Edge, EdgeGroup, EdgeKind, Entity, EntityKind, Graph, ItemPath,
    MergeError, ModulePath, Origin, Provenance, ReadError, SCHEMA_VERSION, Scope, Span, TargetKind,
    Unit, UnitHeader,
};

fn span(file: &str, line: u32) -> Span {
    Span {
        file: file.to_owned(),
        line: NonZeroU32::new(line).expect("lines are 1-based"),
    }
}

fn entity(path: &str, kind: EntityKind, module: &str) -> Entity {
    let crate_name = path
        .split("::")
        .next()
        .expect("split yields at least one piece");
    Entity {
        path: ItemPath::new(path),
        kind,
        module: ModulePath::new(module),
        crate_name: CrateName::new(crate_name),
        origin: Origin::Workspace,
        span: None,
    }
}

fn edge(src: &str, dst: &str, kind: EdgeKind, detail: &str) -> Edge {
    Edge {
        src: ItemPath::new(src),
        dst: ItemPath::new(dst),
        kind,
        provenance: Provenance::Source,
        span: Some(span("crates/app/src/lib.rs", 1)),
        detail: detail.to_owned(),
    }
}

fn header(mode: CompileMode) -> UnitHeader {
    UnitHeader {
        crate_name: CrateName::new("app"),
        target: TargetKind::Lib,
        mode,
        features: Vec::new(),
    }
}

/// A slice of fixtures/tricky: c01 (cross-module + cross-crate call), c07 (`?` through a user
/// `From` impl), c16 (`#[cfg(test)]`-only call).
fn tricky(mode: CompileMode) -> Unit {
    const C01: &str = "app::cases::c01_direct_call";
    const C07: &str = "app::cases::c07_try_from";
    const C16: &str = "app::cases::c16_cfg";
    let mut core_helper = entity("core_api::core_helper", EntityKind::Function, "core_api");
    core_helper.span = Some(span("crates/core_api/src/lib.rs", 3));
    let mut entities = vec![
        entity(&format!("{C01}::cross_module"), EntityKind::Function, C01),
        entity(&format!("{C01}::cross_crate"), EntityKind::Function, C01),
        entity("app::util::helper", EntityKind::Function, "app::util"),
        core_helper,
        entity(&format!("{C07}::run"), EntityKind::Function, C07),
        entity(
            "<app::errors_b::ErrB as core::convert::From<app::errors_a::ErrA>>::from",
            EntityKind::Method,
            "app::error_conv",
        ),
    ];
    let mut edges = vec![
        edge(
            &format!("{C01}::cross_module"),
            "app::util::helper",
            EdgeKind::Calls,
            "",
        ),
        edge(
            &format!("{C01}::cross_crate"),
            "core_api::core_helper",
            EdgeKind::Calls,
            "",
        ),
        Edge {
            provenance: Provenance::Desugared,
            ..edge(
                &format!("{C07}::run"),
                "<app::errors_b::ErrB as core::convert::From<app::errors_a::ErrA>>::from",
                EdgeKind::ImplicitFrom,
                "QuestionMark;nested-impl via core::ops::FromResidual",
            )
        },
    ];
    if mode == CompileMode::Test {
        entities.push(entity(&format!("{C16}::tested"), EntityKind::Function, C16));
        entities.push(entity(
            "app::checks::check",
            EntityKind::Function,
            "app::checks",
        ));
        edges.push(edge(
            &format!("{C16}::tested"),
            "app::checks::check",
            EdgeKind::Calls,
            "",
        ));
    }
    Unit {
        header: header(mode),
        entities,
        edges,
    }
}

fn to_jsonl(unit: &Unit) -> String {
    let mut out = Vec::new();
    unit.write_jsonl(&mut out).expect("writing to a Vec");
    String::from_utf8(out).expect("JSON is UTF-8")
}

fn read(text: &str) -> Result<Unit, ReadError> {
    Unit::read_jsonl(text.as_bytes())
}

#[test]
fn round_trip() {
    let mut unit = tricky(CompileMode::Test);
    unit.header.target = TargetKind::ProcMacro;
    unit.header.features = vec!["extra".to_owned(), "default".to_owned()];
    unit.entities[0].origin = Origin::External;
    let text = to_jsonl(&unit);
    assert_eq!(
        text.lines().count(),
        1 + unit.entities.len() + unit.edges.len()
    );
    assert_eq!(read(&text).unwrap(), unit);
}

#[test]
fn wire_format() {
    let unit = Unit {
        header: header(CompileMode::Normal),
        entities: vec![entity("app::a", EntityKind::TraitMethod, "app")],
        edges: Vec::new(),
    };
    assert_eq!(
        to_jsonl(&unit),
        format!(
            "{{\"record\":\"header\",\"schema_version\":{SCHEMA_VERSION},\"crate_name\":\"app\",\
             \"target\":\"lib\",\"mode\":\"normal\",\"features\":[]}}\n\
             {{\"record\":\"entity\",\"path\":\"app::a\",\"kind\":\"trait_method\",\
             \"module\":\"app\",\"crate_name\":\"app\",\"origin\":\"workspace\",\"span\":null}}\n"
        )
    );
}

#[test]
fn rejects_other_versions() {
    let text = to_jsonl(&tricky(CompileMode::Normal));
    let current = format!("\"schema_version\":{SCHEMA_VERSION}");
    for found in [SCHEMA_VERSION + 1, 0] {
        let other = text.replacen(&current, &format!("\"schema_version\":{found}"), 1);
        let err = read(&other).unwrap_err();
        assert!(
            matches!(err, ReadError::UnsupportedVersion { found: f } if f == found),
            "{err:?}"
        );
    }
}

#[test]
fn newer_version_is_reported_before_shape_errors() {
    let newer = format!(
        "{{\"record\":\"header\",\"schema_version\":{},\"something_new\":true}}\n",
        SCHEMA_VERSION + 1
    );
    let err = read(&newer).unwrap_err();
    assert!(
        matches!(err, ReadError::UnsupportedVersion { .. }),
        "{err:?}"
    );
}

#[test]
fn rejects_unknown_kind() {
    let text = to_jsonl(&tricky(CompileMode::Normal)).replacen(
        "\"kind\":\"implicit_from\"",
        "\"kind\":\"implicit_magic\"",
        1,
    );
    let err = read(&text).unwrap_err();
    let ReadError::Json { line, source } = &err else {
        panic!("{err:?}")
    };
    assert_eq!(*line, 10);
    assert!(
        source
            .to_string()
            .contains("unknown variant `implicit_magic`"),
        "{source}"
    );
}

#[test]
fn rejects_unknown_fields_and_records() {
    let text = to_jsonl(&tricky(CompileMode::Normal));
    for (from, to) in [
        ("\"detail\":\"\"", "\"detail\":\"\",\"weight\":3"),
        ("\"features\":[]", "\"features\":[],\"profile\":\"dev\""),
        (
            "\"origin\":\"workspace\"",
            "\"origin\":\"workspace\",\"extra\":1",
        ),
        ("{\"record\":\"edge\"", "{\"record\":\"annotation\""),
        ("\"provenance\":\"source\"", "\"provenance\":\"generated\""),
        ("\"target\":\"lib\"", "\"target\":\"cdylib\""),
    ] {
        let changed = text.replacen(from, to, 1);
        assert_ne!(changed, text, "{from}");
        let err = read(&changed).unwrap_err();
        assert!(matches!(err, ReadError::Json { .. }), "{from}: {err:?}");
    }
}

#[test]
fn rejects_malformed_units() {
    let text = to_jsonl(&tricky(CompileMode::Normal));
    let mut lines: Vec<&str> = text.lines().collect();

    assert!(matches!(read(""), Err(ReadError::MissingHeader)));
    let err = read(&lines[1..].join("\n")).unwrap_err();
    assert!(matches!(err, ReadError::MissingHeader), "{err:?}");

    let doubled = format!("{text}{}\n", lines[0]);
    let err = read(&doubled).unwrap_err();
    assert!(
        matches!(err, ReadError::DuplicateHeader { line: 11 }),
        "{err:?}"
    );

    let duplicate = format!("{text}{}\n", lines[1]);
    let err = read(&duplicate).unwrap_err();
    assert!(
        matches!(err, ReadError::DuplicateEntity { line: 11, .. }),
        "{err:?}"
    );

    // An edge before the entity record of its destination.
    lines.swap(3, 7);
    let err = read(&lines.join("\n")).unwrap_err();
    let ReadError::UndeclaredEndpoint { line, path } = &err else {
        panic!("{err:?}")
    };
    assert_eq!((*line, path.as_str()), (4, "app::util::helper"));
}

#[test]
fn merge_dedupes_and_marks_test_only() {
    let mut normal = tricky(CompileMode::Normal);
    // Same pair, another kind: kept as a separate edge.
    normal.edges.push(edge(
        "app::cases::c01_direct_call::cross_module",
        "app::util::helper",
        EdgeKind::UsesTypeSig,
        "",
    ));
    let mut test = tricky(CompileMode::Test);
    // Merged first, without a span for this entity: the later unit's span fills it in.
    test.entities[3].span = None;
    // The test build sees the shared edge with another span and detail; the normal one wins.
    for e in &mut test.edges {
        e.detail = "from test".to_owned();
    }

    let graph = Graph::merge([test, normal]).unwrap();
    assert_eq!(graph.headers().len(), 2);
    assert_eq!(graph.entities().count(), 8);
    assert_eq!(graph.edges().len(), 5);

    let helper = ItemPath::new("app::util::helper");
    let kinds: Vec<EdgeKind> = graph.edges_to(&helper).map(|e| e.edge.kind).collect();
    assert_eq!(kinds, [EdgeKind::Calls, EdgeKind::UsesTypeSig]);
    for e in graph.edges().iter().filter(|e| e.scope == Scope::Normal) {
        assert_ne!(e.edge.detail, "from test", "{e:?}");
    }

    let tested = ItemPath::new("app::cases::c16_cfg::tested");
    let test_only: Vec<&ItemPath> = graph
        .edges()
        .iter()
        .filter(|e| e.scope == Scope::TestOnly)
        .map(|e| &e.edge.src)
        .collect();
    assert_eq!(test_only, [&tested]);
    assert_eq!(graph.edges_from(&tested).count(), 1);
    assert_eq!(graph.edges_from(&helper).count(), 0);

    let core_helper = graph
        .entity(&ItemPath::new("core_api::core_helper"))
        .unwrap();
    assert_eq!(
        core_helper.span,
        Some(span("crates/core_api/src/lib.rs", 3))
    );
    assert_eq!(graph.entity(&ItemPath::new("app::nope")), None);
}

#[test]
fn merge_rejects_conflicting_entities() {
    let a = tricky(CompileMode::Normal);
    let mut b = tricky(CompileMode::Normal);
    b.entities[2].kind = EntityKind::Method;
    let err = Graph::merge([a, b]).unwrap_err();
    let MergeError::ConflictingEntity { path } = &err;
    assert_eq!(path.as_str(), "app::util::helper");
}

#[test]
fn groups() {
    assert!(EdgeKind::ImplicitDrop.is_call());
    assert!(EdgeKind::Bound.is_type_use());
    assert!(!EdgeKind::ImplForType.is_type_use());
    assert_eq!(EdgeKind::MacroUse.group(), EdgeGroup::Macros);
    assert_eq!(EdgeKind::WritesGlobal.group(), EdgeGroup::Globals);
    assert_eq!(EdgeKind::Constructs.group(), EdgeGroup::Constructs);
    assert_eq!(EdgeKind::MatchesOn.group(), EdgeGroup::Matches);
    for kind in [EdgeKind::Constructs, EdgeKind::MatchesOn] {
        assert!(!kind.is_type_use() && !kind.is_call(), "{kind:?}");
    }
    assert_eq!(
        serde_json::to_string(&EdgeKind::MatchesOn).unwrap(),
        "\"matches_on\""
    );
    assert_eq!(
        serde_json::to_string(&EdgeGroup::Types).unwrap(),
        "\"types\""
    );
}

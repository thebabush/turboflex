//! The model language: what parses, what is rejected, and where problems are reported.

use std::collections::BTreeSet;

use turboflex_graph::{CrateName, EdgeGroup, ItemPath, ModulePath, Provenance};
use turboflex_model::{
    ComponentId, ComponentKind, CratePattern, Follow, LinkVia, Model, ModulePattern, ModuleScope,
    ProblemKind, Selector, TestEdges,
};

const GOOD: &str = r#"
component "model" {
    modules "app::model" "app::model::**"
}

component "worker" {
    doc "queue workers"
    modules "app::worker::util"
    component "runtime" {
        modules "app::worker::v2::**"
        items "app::worker::v2::fetch" follow=#true
    }
    component "acquire" {
        modules "app::ingest"
        items "app::worker::v2::plain"
    }
}

component "report"

external "postgres" {
    crates "sqlx" "sqlx_*"
}

external "io" {
    modules "std::fs" "std::fs::**"
    items "std::env::var"
}

allow "worker.runtime" -> "worker.acquire" "postgres"
allow report "->" worker kinds="types, calls"
link "report" -> "worker" via="process"

policy {
    test-edges "check"
}
"#;

fn id(value: &str) -> ComponentId {
    ComponentId::new(value)
}

fn ids<'a>(components: impl Iterator<Item = &'a turboflex_model::Component>) -> Vec<String> {
    components.map(|c| c.id.to_string()).collect()
}

/// The problems of a source that must not parse, as `(line, column, kind)`.
fn problems(text: &str) -> Vec<(usize, usize, ProblemKind)> {
    let error = Model::parse("test.kdl", text).expect_err("the model must be rejected");
    error
        .problems()
        .iter()
        .map(|p| (p.location.line, p.location.column, p.kind.clone()))
        .collect()
}

/// The single problem of a source with exactly one.
fn problem(text: &str) -> (usize, usize, ProblemKind) {
    let mut all = problems(text);
    assert_eq!(all.len(), 1, "{all:#?}");
    all.remove(0)
}

#[test]
fn parses_components_and_patterns() {
    let model = Model::parse("good.kdl", GOOD).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(model.source_name(), "good.kdl");
    assert_eq!(
        ids(model.components().iter()),
        [
            "model",
            "worker",
            "worker.runtime",
            "worker.acquire",
            "report",
            "postgres",
            "io"
        ]
    );
    let worker = model.component(&id("worker")).unwrap();
    assert_eq!(worker.kind, ComponentKind::Workspace);
    assert_eq!(worker.doc.as_deref(), Some("queue workers"));
    assert_eq!(
        model.component(&id("io")).unwrap().kind,
        ComponentKind::External
    );
    assert_eq!(model.policy().test_edges, TestEdges::Check);

    let selectors: Vec<(String, Selector, Follow)> = model
        .patterns()
        .iter()
        .map(|p| (p.owner.to_string(), p.selector.clone(), p.follow))
        .collect();
    let module = |path: &str, scope| {
        Selector::Module(ModulePattern {
            module: ModulePath::new(path),
            scope,
        })
    };
    let item = |path: &str| Selector::Item(ItemPath::new(path));
    assert_eq!(
        selectors,
        [
            (
                "model".into(),
                module("app::model", ModuleScope::Exact),
                Follow::Off
            ),
            (
                "model".into(),
                module("app::model", ModuleScope::Descendants),
                Follow::Off
            ),
            (
                "worker".into(),
                module("app::worker::util", ModuleScope::Exact),
                Follow::Off
            ),
            (
                "worker.runtime".into(),
                module("app::worker::v2", ModuleScope::Descendants),
                Follow::Off
            ),
            (
                "worker.runtime".into(),
                item("app::worker::v2::fetch"),
                Follow::Helpers
            ),
            (
                "worker.acquire".into(),
                module("app::ingest", ModuleScope::Exact),
                Follow::Off
            ),
            (
                "worker.acquire".into(),
                item("app::worker::v2::plain"),
                Follow::Off
            ),
            (
                "postgres".into(),
                Selector::Crate(CratePattern::Exact(CrateName::new("sqlx"))),
                Follow::Off
            ),
            (
                "postgres".into(),
                Selector::Crate(CratePattern::Prefix("sqlx_".into())),
                Follow::Off
            ),
            (
                "io".into(),
                module("std::fs", ModuleScope::Exact),
                Follow::Off
            ),
            (
                "io".into(),
                module("std::fs", ModuleScope::Descendants),
                Follow::Off
            ),
            ("io".into(), item("std::env::var"), Follow::Off),
        ]
    );
    assert_eq!(model.patterns()[1].selector.to_string(), "app::model::**");
    assert_eq!(model.patterns()[8].selector.to_string(), "sqlx_*");
}

#[test]
fn parses_rules_and_links() {
    let model = Model::parse("good.kdl", GOOD).unwrap();
    let rules: Vec<(String, String, Option<BTreeSet<EdgeGroup>>)> = model
        .rules()
        .iter()
        .map(|r| (r.from.to_string(), r.to.to_string(), r.kinds.clone()))
        .collect();
    assert_eq!(
        rules,
        [
            ("worker.runtime".into(), "worker.acquire".into(), None),
            ("worker.runtime".into(), "postgres".into(), None),
            (
                "report".into(),
                "worker".into(),
                Some(BTreeSet::from([EdgeGroup::Types, EdgeGroup::Calls]))
            ),
        ]
    );
    // The location of a rule is its target argument.
    let rule = &model.rules()[1];
    assert_eq!((rule.location.line, rule.location.column), (30, 44));

    let [link] = model.links() else {
        panic!("one link expected")
    };
    assert_eq!((link.from.as_str(), link.to.as_str()), ("report", "worker"));
    assert_eq!(link.via, LinkVia::Process);
}

#[test]
fn parses_every_edge_group() {
    let text = "component \"a\"\ncomponent \"b\"\nallow \"a\" -> \"b\" \
                kinds=\"types,calls,impls,macros,globals,constructs,matches\"";
    let model = Model::parse("groups.kdl", text).unwrap();
    assert_eq!(
        model.rules()[0].kinds,
        Some(BTreeSet::from([
            EdgeGroup::Types,
            EdgeGroup::Calls,
            EdgeGroup::Impls,
            EdgeGroup::Macros,
            EdgeGroup::Globals,
            EdgeGroup::Constructs,
            EdgeGroup::Matches,
        ]))
    );
}

#[test]
fn nesting_queries() {
    let model = Model::parse("good.kdl", GOOD).unwrap();
    let runtime = id("worker.runtime");
    assert_eq!(model.parent(&runtime).unwrap().id, id("worker"));
    assert!(model.parent(&id("worker")).is_none());
    assert_eq!(
        ids(model.children(&id("worker"))),
        ["worker.runtime", "worker.acquire"]
    );
    assert_eq!(ids(model.ancestors(&runtime)), ["worker"]);
    assert_eq!(
        ids(model.descendants(&id("worker"))),
        ["worker.runtime", "worker.acquire"]
    );
    assert_eq!(
        ids(model.roots()),
        ["model", "worker", "report", "postgres", "io"]
    );
    assert_eq!(
        model.patterns_of(&id("worker")).count(),
        1,
        "a parent owns only its own patterns"
    );
    assert!(runtime.is_within(&id("worker")));
    assert!(runtime.is_within(&runtime));
    assert!(!id("workers").is_within(&id("worker")));
    assert!(!id("worker").is_within(&runtime));
}

#[test]
fn policy_defaults_to_excluding_test_edges() {
    let model = Model::parse("m.kdl", "component \"a\"").unwrap();
    assert_eq!(model.policy().test_edges, TestEdges::Exclude);
    assert_eq!(model.policy().edges, None);
    assert_eq!(model.policy().provenance, None);
    assert_eq!(model.policy().targets, None);
    assert!(model.policy().considers_group(EdgeGroup::Calls));
    assert!(model.policy().considers_provenance(Provenance::Derive));
    assert_eq!(model.rules(), []);
}

#[test]
fn policy_edge_filters() {
    let text = "component \"a\"\ncomponent \"b\"\n\
                allow \"a\" -> \"b\" kinds=\"calls,matches\"\n\
                allow \"b\" -> \"a\"\n\
                policy {\n    \
                    edges \"constructs\" \"matches\"\n    \
                    provenance \"source\" \"macro_expanded\" \"desugared\"\n\
                }";
    let model = Model::parse("view.kdl", text).unwrap_or_else(|e| panic!("{e}"));
    let policy = model.policy();
    assert_eq!(policy.test_edges, TestEdges::Exclude);
    assert_eq!(
        policy.edges,
        Some(BTreeSet::from([EdgeGroup::Constructs, EdgeGroup::Matches]))
    );
    assert_eq!(
        policy.provenance,
        Some(BTreeSet::from([
            Provenance::Source,
            Provenance::MacroExpanded,
            Provenance::Desugared,
        ]))
    );
    assert!(policy.considers_group(EdgeGroup::Matches));
    assert!(!policy.considers_group(EdgeGroup::Calls));
    assert!(policy.considers_provenance(Provenance::Desugared));
    assert!(!policy.considers_provenance(Provenance::Derive));

    let every = Model::parse(
        "every.kdl",
        "policy {\n    provenance \"source\" \"macro_expanded\" \"desugared\" \"derive\"\n\
         edges \"types\" \"calls\" \"impls\" \"macros\" \"globals\" \"constructs\" \"matches\"\n}",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        every.policy().provenance.as_ref().map(BTreeSet::len),
        Some(4)
    );
    assert_eq!(every.policy().edges.as_ref().map(BTreeSet::len), Some(7));
}

#[test]
fn policy_edge_filter_errors() {
    assert_eq!(
        problem("policy {\n    edges \"calls\" \"code\"\n}"),
        (
            2,
            19,
            ProblemKind::BadArguments {
                node: "edges",
                expected: "one or more distinct edge groups: types, calls, impls, macros, \
                           globals, constructs, matches"
            }
        )
    );
    let (line, column, kind) = problem("policy {\n    provenance \"derived\"\n}");
    assert_eq!((line, column), (2, 16));
    assert!(
        matches!(
            kind,
            ProblemKind::BadArguments {
                node: "provenance",
                ..
            }
        ),
        "{kind:?}"
    );
    // Wire names, not Rust ones.
    let (_, _, kind) = problem("policy {\n    provenance \"MacroExpanded\"\n}");
    assert!(
        matches!(
            kind,
            ProblemKind::BadArguments {
                node: "provenance",
                ..
            }
        ),
        "{kind:?}"
    );
    for node in ["edges", "provenance"] {
        let (line, column, kind) = problem(&format!("policy {{\n    {node}\n}}"));
        assert_eq!((line, column), (2, 5), "{node}");
        assert!(
            matches!(kind, ProblemKind::BadArguments { node: n, .. } if n == node),
            "{kind:?}"
        );
    }
    let error = Model::parse(
        "dup.kdl",
        "policy {\n    edges \"calls\" \"types\" \"calls\"\n}",
    )
    .unwrap_err();
    let [dup] = error.problems() else {
        panic!("{error}")
    };
    assert_eq!(
        dup.kind,
        ProblemKind::DuplicateValue {
            node: "edges",
            value: "calls".into()
        }
    );
    assert_eq!((dup.location.line, dup.location.column), (2, 27));
    assert_eq!(dup.previous.map(|p| (p.line, p.column)), Some((2, 11)));

    let kinds: Vec<_> = problems(
        "policy {\n    provenance \"source\"\n    provenance \"derive\"\n    \
         edges \"calls\"\n    edges \"types\"\n    test-edges \"check\"\n    \
         test-edges \"exclude\"\n}",
    )
    .into_iter()
    .map(|(line, _, kind)| (line, kind))
    .collect();
    assert_eq!(
        kinds,
        [
            (3, ProblemKind::DuplicateNode { node: "provenance" }),
            (5, ProblemKind::DuplicateNode { node: "edges" }),
            (7, ProblemKind::DuplicateNode { node: "test-edges" }),
        ]
    );
    assert_eq!(
        problem("policy {\n    edges \"calls\" only=#true\n}"),
        (2, 19, ProblemKind::UnexpectedProperty { node: "edges" })
    );
}

#[test]
fn policy_targets() {
    let text = GOOD.replace(
        "test-edges \"check\"",
        "targets \"model\" \"worker.runtime\" \"postgres\"",
    );
    let model = Model::parse("view.kdl", &text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        model.policy().targets,
        Some(BTreeSet::from([
            id("model"),
            id("postgres"),
            id("worker.runtime")
        ]))
    );
}

#[test]
fn policy_target_errors() {
    let text = "component \"a\" {\n    component \"b\"\n}\n\
                policy {\n    targets \"a\" \"nope\" \"b\"\n}";
    let unknown = |id: &str| ProblemKind::UnknownComponent { id: id.into() };
    // Nested components are named by their full id.
    assert_eq!(
        problems(text),
        [(5, 17, unknown("nope")), (5, 24, unknown("b"))]
    );

    let error = Model::parse(
        "dup.kdl",
        "component \"a\"\ncomponent \"b\"\npolicy {\n    targets \"a\" \"b\" \"a\"\n}",
    )
    .unwrap_err();
    let [dup] = error.problems() else {
        panic!("{error}")
    };
    assert_eq!(
        dup.kind,
        ProblemKind::DuplicateValue {
            node: "targets",
            value: "a".into()
        }
    );
    assert_eq!((dup.location.line, dup.location.column), (4, 21));
    assert_eq!(dup.previous.map(|p| (p.line, p.column)), Some((4, 13)));

    assert_eq!(
        problems("component \"a\"\npolicy {\n    targets \"a\"\n    targets \"a\"\n}"),
        [(4, 5, ProblemKind::DuplicateNode { node: "targets" })]
    );
    let (line, column, kind) = problem("policy {\n    targets\n}");
    assert_eq!((line, column), (2, 5));
    assert!(
        matches!(
            kind,
            ProblemKind::BadArguments {
                node: "targets",
                ..
            }
        ),
        "{kind:?}"
    );
    assert_eq!(
        problem("component \"a\"\npolicy {\n    targets \"a\" only=#true\n}"),
        (3, 17, ProblemKind::UnexpectedProperty { node: "targets" })
    );
    assert_eq!(
        problem("component \"a\"\npolicy {\n    targets \"a\" {\n        x\n    }\n}"),
        (3, 5, ProblemKind::UnexpectedChildren { node: "targets" })
    );
}

#[test]
fn policy_targets_cover_nested_components() {
    let mut policy = Model::parse("m.kdl", "component \"a\"")
        .unwrap()
        .policy()
        .clone();
    // No `targets`: everything is in scope.
    assert!(policy.considers_target(&id("anything")));
    assert!(policy.considers_rule_target(&id("anything")));

    policy.targets = Some(BTreeSet::from([id("worker.runtime"), id("postgres")]));
    assert!(policy.considers_target(&id("worker.runtime")));
    assert!(policy.considers_target(&id("worker.runtime.pool")));
    assert!(policy.considers_target(&id("postgres")));
    assert!(!policy.considers_target(&id("worker")));
    assert!(!policy.considers_target(&id("worker.acquire")));
    assert!(!policy.considers_target(&id("worker.runtimes")));
    assert!(!policy.considers_target(&id("report")));

    // A rule applies when its target and a listed one are nested either way.
    assert!(policy.considers_rule_target(&id("worker.runtime")));
    assert!(policy.considers_rule_target(&id("worker.runtime.pool")));
    assert!(policy.considers_rule_target(&id("worker")));
    assert!(policy.considers_rule_target(&id("postgres")));
    assert!(!policy.considers_rule_target(&id("worker.acquire")));
    assert!(!policy.considers_rule_target(&id("worker.runtimes")));
    assert!(!policy.considers_rule_target(&id("report")));
}

#[test]
fn rule_kinds_outside_the_edge_filter_are_errors() {
    let text = "component \"a\"\ncomponent \"b\"\n\
                allow \"a\" -> \"b\" kinds=\"calls,types\"\n\
                policy {\n    edges \"constructs\" \"matches\"\n}";
    assert_eq!(problem(text), (3, 18, ProblemKind::KindsOutsidePolicy));
    // One problem per rule node, whatever its number of targets.
    let text = "component \"a\"\ncomponent \"b\"\ncomponent \"c\"\n\
                allow \"a\" -> \"b\" \"c\" kinds=\"calls\"\n\
                policy {\n    edges \"matches\"\n}";
    assert_eq!(problem(text), (4, 22, ProblemKind::KindsOutsidePolicy));
    // Overlapping kinds and rules without kinds are fine.
    let text = "component \"a\"\ncomponent \"b\"\n\
                allow \"a\" -> \"b\" kinds=\"calls,matches\"\n\
                allow \"b\" -> \"a\"\n\
                policy {\n    edges \"matches\"\n}";
    assert!(Model::parse("ok.kdl", text).is_ok());
}

#[test]
fn syntax_errors_are_located() {
    let (line, column, kind) = problem("component \"a\" {\n    modules \"x\n}\n");
    assert_eq!((line, column), (2, 13));
    assert!(matches!(kind, ProblemKind::Syntax { .. }), "{kind:?}");
}

#[test]
fn unknown_nodes_are_errors_at_every_level() {
    let text = "components \"a\"\ncomponent \"b\" {\n    module \"x\"\n}\npolicy {\n    tests \"check\"\n}\nexternal \"e\" {\n    component \"f\"\n}\n";
    let found: Vec<_> = problems(text)
        .into_iter()
        .map(|(line, column, kind)| {
            let ProblemKind::UnknownNode { name, .. } = kind else {
                panic!("unexpected {kind:?}")
            };
            (line, column, name)
        })
        .collect();
    assert_eq!(
        found,
        [
            (1, 1, "components".to_owned()),
            (3, 5, "module".to_owned()),
            (6, 5, "tests".to_owned()),
            (9, 5, "component".to_owned()),
        ]
    );
}

#[test]
fn unknown_and_duplicate_properties() {
    assert_eq!(
        problem("component \"a\" {\n    items \"a::f\" folow=#true\n}"),
        (
            2,
            18,
            ProblemKind::UnknownProperty {
                node: "items",
                name: "folow".into(),
                expected: "follow".into()
            }
        )
    );
    assert_eq!(
        problem("component \"a\" color=\"red\""),
        (1, 15, ProblemKind::UnexpectedProperty { node: "component" })
    );
    // KDL lets the last duplicate win; the model refuses to drop one silently.
    assert_eq!(
        problem("component \"a\"\nallow \"a\" -> \"a\" kinds=\"types\" kinds=\"calls\""),
        (
            2,
            32,
            ProblemKind::DuplicateProperty {
                name: "kinds".into()
            }
        )
    );
    // Helpers follow only workspace items.
    assert_eq!(
        problem("external \"e\" {\n    items \"std::env::var\" follow=#true\n}"),
        (2, 27, ProblemKind::UnexpectedProperty { node: "items" })
    );
}

#[test]
fn bad_arguments_and_values() {
    assert_eq!(
        problem("component \"a\" \"b\""),
        (
            1,
            1,
            ProblemKind::BadArguments {
                node: "component",
                expected: "exactly one name"
            }
        )
    );
    assert_eq!(
        problem("component \"a\" {\n    modules 3\n}"),
        (2, 13, ProblemKind::NotAString { found: "3".into() })
    );
    let (line, column, kind) = problem("component \"a\"\nallow \"a\" \"a\"");
    assert_eq!((line, column), (2, 1));
    assert!(
        matches!(kind, ProblemKind::BadArguments { node: "allow", .. }),
        "{kind:?}"
    );
    let (line, column, kind) = problem("component \"a\"\nallow \"a\" -> kinds=\"types\"");
    assert_eq!((line, column), (2, 1));
    assert!(
        matches!(kind, ProblemKind::BadArguments { node: "allow", .. }),
        "{kind:?}"
    );
    let (line, column, kind) =
        problem("component \"a\"\nallow \"a\" -> \"a\" kinds=\"types,code\"");
    assert_eq!((line, column), (2, 18));
    assert!(
        matches!(kind, ProblemKind::BadPropertyValue { name: "kinds", .. }),
        "{kind:?}"
    );
    let (line, column, kind) = problem("component \"a\"\ncomponent \"b\"\nlink \"a\" -> \"b\"");
    assert_eq!((line, column), (3, 1));
    assert!(
        matches!(kind, ProblemKind::BadPropertyValue { name: "via", .. }),
        "{kind:?}"
    );
    let (line, column, kind) = problem("policy {\n    test-edges \"sometimes\"\n}");
    assert_eq!((line, column), (2, 5));
    assert!(
        matches!(
            kind,
            ProblemKind::BadArguments {
                node: "test-edges",
                ..
            }
        ),
        "{kind:?}"
    );
    assert_eq!(
        problem("(note)component \"a\""),
        (1, 2, ProblemKind::TypeAnnotation)
    );
    assert_eq!(
        problem("component \"a\" {\n    modules \"x\" {\n        y\n    }\n}"),
        (2, 5, ProblemKind::UnexpectedChildren { node: "modules" })
    );
}

#[test]
fn component_names_and_duplicates() {
    assert_eq!(
        problem("component \"a.b\""),
        (
            1,
            11,
            ProblemKind::InvalidComponentName { name: "a.b".into() }
        )
    );
    let error = Model::parse(
        "t.kdl",
        "component \"a\" {\n  component \"b\"\n}\ncomponent \"a\"",
    )
    .unwrap_err();
    let [dup] = error.problems() else {
        panic!("{error}")
    };
    assert_eq!(dup.kind, ProblemKind::DuplicateComponent { id: id("a") });
    assert_eq!((dup.location.line, dup.location.column), (4, 11));
    let previous = dup.previous.unwrap();
    assert_eq!((previous.line, previous.column), (1, 11));
    // The same name at another level is a different component.
    Model::parse("t.kdl", "component \"a\" {\n  component \"a\"\n}").unwrap();
    // Externals share the id space.
    assert_eq!(
        problem("component \"a\"\nexternal \"a\"").2,
        ProblemKind::DuplicateComponent { id: id("a") }
    );
}

#[test]
fn duplicate_patterns_need_a_single_owner() {
    let text = "component \"a\" {\n  modules \"x::y\"\n}\ncomponent \"b\" {\n  modules \"x::y\" \"x::y::**\" \"x::y::**\"\n}";
    let problems = problems(text);
    assert_eq!(
        problems,
        [
            (
                5,
                11,
                ProblemKind::DuplicatePattern {
                    pattern: "x::y".into(),
                    first: id("a"),
                    second: id("b"),
                }
            ),
            (
                5,
                29,
                ProblemKind::DuplicatePattern {
                    pattern: "x::y::**".into(),
                    first: id("b"),
                    second: id("b"),
                }
            ),
        ]
    );
}

#[test]
fn malformed_patterns() {
    let text = r#"component "a" {
    modules "a::**::b" "a::*" "a::" "" "a b" "**"
    items "a::b::**" " a::f"
}
external "e" {
    crates "sqlx-core" "*" "a::b"
}"#;
    let found: Vec<_> = problems(text)
        .into_iter()
        .map(|(line, column, kind)| {
            let ProblemKind::MalformedPattern { what, pattern, .. } = kind else {
                panic!("unexpected {kind:?}")
            };
            (line, column, what, pattern)
        })
        .collect();
    let expected = [
        (2, 13, "module", "a::**::b"),
        (2, 24, "module", "a::*"),
        (2, 31, "module", "a::"),
        (2, 37, "module", ""),
        (2, 40, "module", "a b"),
        (2, 46, "module", "**"),
        (3, 11, "item", "a::b::**"),
        (3, 22, "item", " a::f"),
        (6, 12, "crate", "sqlx-core"),
        (6, 24, "crate", "*"),
        (6, 28, "crate", "a::b"),
    ];
    let expected: Vec<_> = expected
        .into_iter()
        .map(|(line, column, what, pattern)| (line, column, what, pattern.to_owned()))
        .collect();
    assert_eq!(found, expected);
}

#[test]
fn references_must_name_declared_components() {
    let text = r#"component "a" {
    component "b"
}
external "e"
policy { targets "a" "x"; }
allow "a.b" -> "a.c" "e"
allow "e" -> "a"
allow "a" -> "b"
allow "a.b" -> "e" kinds="types"
link "a" -> "e" via="process"
"#;
    let unknown = |id: &str| ProblemKind::UnknownComponent { id: id.into() };
    assert_eq!(
        problems(text),
        [
            (5, 22, unknown("x")),
            (6, 16, unknown("a.c")),
            (
                7,
                7,
                ProblemKind::ExternalSource {
                    id: id("e"),
                    node: "rule"
                }
            ),
            // Nested components are named by their full id.
            (8, 14, unknown("b")),
            (
                9,
                16,
                ProblemKind::DuplicateRule {
                    from: id("a.b"),
                    to: id("e")
                }
            ),
            (
                10,
                13,
                ProblemKind::BadArguments {
                    node: "link",
                    expected: "workspace components on both ends"
                }
            ),
        ]
    );
}

#[test]
fn single_use_nodes() {
    let text = "component \"a\" {\n    doc \"x\"\n    doc \"y\"\n}\npolicy\npolicy\n";
    let kinds: Vec<_> = problems(text)
        .into_iter()
        .map(|(line, _, kind)| (line, kind))
        .collect();
    assert_eq!(
        kinds,
        [
            (3, ProblemKind::DuplicateNode { node: "doc" }),
            (6, ProblemKind::DuplicateNode { node: "policy" }),
        ]
    );
}

#[test]
fn foundation_is_rejected_with_its_replacement() {
    let text = "component \"a\"\ncomponent \"b\"\n  foundation \"a\"\nallow \"b\" -> \"a\"\n";
    assert_eq!(problem(text), (3, 3, ProblemKind::FoundationRemoved));
    let rendered = Model::parse("arch.kdl", text).unwrap_err().to_string();
    assert!(
        rendered.contains(
            "arch.kdl:3:3: `foundation` has been removed: write an `allow` rule for each \
             dependency it covered (`allow \"FROM\" -> \"TO\"...`), or, for a view about a few \
             components, use `policy { targets ... }`"
        ),
        "{rendered}"
    );
    // Whatever its arguments, and each time: the node itself is the problem.
    assert_eq!(
        problems("foundation\nfoundation \"nope\" x=1 { child; }"),
        [
            (1, 1, ProblemKind::FoundationRemoved),
            (2, 1, ProblemKind::FoundationRemoved),
        ]
    );
}

#[test]
fn arrow_may_be_bare_or_quoted() {
    let bare = Model::parse("t.kdl", "component \"a\"\ncomponent \"b\"\nallow a -> b").unwrap();
    let quoted = Model::parse(
        "t.kdl",
        "component \"a\"\ncomponent \"b\"\nallow \"a\" \"->\" \"b\"",
    )
    .unwrap();
    assert_eq!(bare.rules().len(), 1);
    assert_eq!(bare.rules()[0].to, quoted.rules()[0].to);
}

#[test]
fn every_problem_is_reported_and_rendered_with_its_location() {
    let text = "component \"a\" {\n    modules \"x::**::y\"\n}\nallow \"a\" -> \"nope\"\nbogus\n";
    let error = Model::parse("arch.kdl", text).unwrap_err();
    assert_eq!(error.problems().len(), 3);
    let rendered = error.to_string();
    assert!(
        rendered.starts_with("arch.kdl: invalid architecture model\n"),
        "{rendered}"
    );
    assert!(
        rendered.contains("arch.kdl:2:13: malformed module pattern `x::**::y`"),
        "{rendered}"
    );
    assert!(
        rendered.contains("arch.kdl:4:14: `nope` is not a declared component"),
        "{rendered}"
    );
    assert!(
        rendered.contains("arch.kdl:5:1: unknown node `bogus`"),
        "{rendered}"
    );
}

#[test]
fn the_repository_models_are_valid() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    for file in [
        "turboflex.kdl",
        "turboflex-construction.kdl",
        "fixtures/taint/capability.kdl",
        "fixtures/taint/sanitizer.kdl",
    ] {
        let text = std::fs::read_to_string(format!("{root}/{file}")).unwrap();
        if let Err(error) = Model::parse(file, &text) {
            panic!("{error}");
        }
    }
    let text = std::fs::read_to_string(format!("{root}/turboflex.kdl")).unwrap();
    let model = Model::parse("turboflex.kdl", &text).unwrap();
    let targets = |from: &str| -> Vec<String> {
        model
            .rules()
            .iter()
            .filter(|r| r.from.as_str() == from)
            .map(|r| r.to.to_string())
            .collect()
    };
    assert_eq!(targets("graph"), Vec::<String>::new());
    assert_eq!(targets("driver"), ["graph", "rustc", "io"]);
    assert_eq!(targets("report"), ["graph"]);
    assert_eq!(targets("cli"), ["service", "io"]);
    assert_eq!(model.links().len(), 2);
}

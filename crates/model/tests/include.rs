//! `include`: composing a model from several files through a caller-supplied loader.

use std::collections::HashMap;

use miette::Diagnostic;
use turboflex_model::{ComponentId, Model, ModelError, ProblemKind, Source};

/// An in-memory file tree. Paths are resolved like the service would: relative to the
/// including file's directory, `..` and `.` folded, so the name is canonical.
struct Files {
    files: HashMap<String, String>,
    /// Every `(including, path)` the loader was called with, in order.
    calls: Vec<(String, String)>,
}

impl Files {
    fn new(files: &[(&str, &str)]) -> Self {
        Self {
            files: files
                .iter()
                .map(|&(name, text)| (name.to_owned(), text.to_owned()))
                .collect(),
            calls: Vec::new(),
        }
    }

    fn load(&mut self, including: &str, path: &str) -> Result<Source, String> {
        self.calls.push((including.to_owned(), path.to_owned()));
        let mut segments: Vec<&str> = including.split('/').collect();
        segments.pop();
        for segment in path.split('/') {
            match segment {
                "." => {}
                ".." => {
                    segments.pop();
                }
                segment => segments.push(segment),
            }
        }
        let name = segments.join("/");
        let text = self
            .files
            .get(&name)
            .ok_or_else(|| format!("no such file `{name}`"))?;
        Ok(Source {
            name,
            text: text.clone(),
        })
    }

    /// Parses the file `root` of the tree.
    fn parse(&mut self, root: &str) -> Result<Model, ModelError> {
        let text = self.files[root].clone();
        Model::parse_with(root, &text, |including, path| self.load(including, path))
    }
}

fn parse(files: &[(&str, &str)], root: &str) -> Model {
    Files::new(files)
        .parse(root)
        .unwrap_or_else(|e| panic!("{e}"))
}

/// The problems of a tree that must not parse, as `(file, line, column, kind)`.
fn problems(files: &[(&str, &str)], root: &str) -> Vec<(String, usize, usize, ProblemKind)> {
    let error = Files::new(files)
        .parse(root)
        .expect_err("the model must be rejected");
    let labelled = labelled(&error);
    error
        .problems()
        .iter()
        .zip(labelled)
        .map(|(p, (name, _))| (name, p.location.line, p.location.column, p.kind.clone()))
        .collect()
}

/// What a `miette` handler renders for each problem of `error`, in order: the name of the file
/// and the text of the primary label.
fn labelled(error: &ModelError) -> Vec<(String, String)> {
    error
        .related()
        .into_iter()
        .flatten()
        .map(|diagnostic| {
            let source = diagnostic
                .source_code()
                .expect("every problem has its source");
            let label = diagnostic
                .labels()
                .into_iter()
                .flatten()
                .next()
                .expect("every problem has a label");
            let contents = source
                .read_span(label.inner(), 0, 0)
                .expect("the label is in the source");
            let name = contents.name().expect("the source is named").to_owned();
            (name, String::from_utf8_lossy(contents.data()).into_owned())
        })
        .collect()
}

fn ids(model: &Model) -> Vec<&str> {
    model.components().iter().map(|c| c.id.as_str()).collect()
}

fn s(value: &str) -> String {
    value.to_owned()
}

#[test]
fn an_included_file_declares_in_place_of_its_include() {
    let model = parse(
        &[
            (
                "views/view.kdl",
                "component \"first\"\ninclude \"../shared/base.kdl\"\ncomponent \"last\"\n\
                 allow \"last\" -> \"core\"\npolicy {\n    test-edges \"check\"\n}\n",
            ),
            (
                "shared/base.kdl",
                "component \"core\" {\n    modules \"app::core\"\n}\n\
                 external \"io\" {\n    crates \"tokio\"\n}\n\
                 allow \"core\" -> \"io\"\nlink \"core\" -> \"first\" via=\"process\"\n",
            ),
        ],
        "views/view.kdl",
    );
    assert_eq!(ids(&model), ["first", "core", "io", "last"]);
    let rules: Vec<(&str, &str)> = model
        .rules()
        .iter()
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect();
    assert_eq!(rules, [("core", "io"), ("last", "core")]);
    assert_eq!(model.links().len(), 1);
    assert_eq!(model.source_name(), "views/view.kdl");
    assert_eq!(model.source_names(), ["views/view.kdl", "shared/base.kdl"]);
    let core = model.component(&ComponentId::new("core")).unwrap();
    assert_eq!(model.source_name_of(core.location), "shared/base.kdl");
    assert_eq!((core.location.line, core.location.column), (1, 11));
    let last = model.component(&ComponentId::new("last")).unwrap();
    assert_eq!(model.source_name_of(last.location), "views/view.kdl");
    assert_eq!(
        model.source_name_of(model.patterns()[0].location),
        "shared/base.kdl"
    );
    assert_eq!(
        model.source_name_of(model.rules()[0].location),
        "shared/base.kdl"
    );
    assert_eq!(
        model.source_name_of(model.rules()[1].location),
        "views/view.kdl"
    );
    assert_eq!(
        model.source_name_of(model.links()[0].location),
        "shared/base.kdl"
    );
}

#[test]
fn includes_nest_and_paths_are_passed_through_as_written() {
    let mut files = Files::new(&[
        ("root.kdl", "include \"a/a.kdl\"\ncomponent \"root\""),
        ("a/a.kdl", "include \"./b/../b/b.kdl\"\ncomponent \"a\""),
        ("a/b/b.kdl", "component \"b\""),
    ]);
    let model = files.parse("root.kdl").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(ids(&model), ["b", "a", "root"]);
    assert_eq!(model.source_names(), ["root.kdl", "a/a.kdl", "a/b/b.kdl"]);
    assert_eq!(
        files.calls,
        [
            (s("root.kdl"), s("a/a.kdl")),
            (s("a/a.kdl"), s("./b/../b/b.kdl")),
        ]
    );
}

#[test]
fn a_file_included_twice_is_loaded_once() {
    let mut files = Files::new(&[
        (
            "root.kdl",
            "include \"left.kdl\"\ninclude \"right.kdl\"\ninclude \"./base.kdl\"",
        ),
        ("left.kdl", "include \"base.kdl\"\ncomponent \"left\""),
        ("right.kdl", "include \"base.kdl\"\ncomponent \"right\""),
        ("base.kdl", "component \"base\" {\n    modules \"app\"\n}"),
    ]);
    let model = files.parse("root.kdl").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(ids(&model), ["base", "left", "right"]);
    assert_eq!(model.patterns().len(), 1);
    assert_eq!(
        model.source_names(),
        ["root.kdl", "left.kdl", "base.kdl", "right.kdl"]
    );
    // The loader is still asked every time: only it knows which file a path names.
    assert_eq!(files.calls.len(), 5);
}

#[test]
fn include_cycles_are_errors_naming_the_chain() {
    let chain = |names: &[&str]| ProblemKind::IncludeCycle {
        chain: names.iter().map(|&name| s(name)).collect(),
    };
    assert_eq!(
        problems(
            &[
                ("root.kdl", "include \"a.kdl\""),
                ("a.kdl", "include \"b.kdl\"\ncomponent \"a\""),
                ("b.kdl", "component \"b\"\ninclude \"a.kdl\""),
            ],
            "root.kdl",
        ),
        [(s("b.kdl"), 2, 9, chain(&["a.kdl", "b.kdl", "a.kdl"]))]
    );
    // Through the root, and a file including itself.
    assert_eq!(
        problems(
            &[
                ("root.kdl", "include \"a.kdl\""),
                ("a.kdl", "include \"root.kdl\""),
            ],
            "root.kdl",
        ),
        [(s("a.kdl"), 1, 9, chain(&["root.kdl", "a.kdl", "root.kdl"]))]
    );
    assert_eq!(
        problems(&[("self.kdl", "include \"self.kdl\"")], "self.kdl"),
        [(s("self.kdl"), 1, 9, chain(&["self.kdl", "self.kdl"]))]
    );
}

#[test]
fn policy_belongs_to_the_root() {
    assert_eq!(
        problems(
            &[
                ("root.kdl", "include \"base.kdl\"\npolicy"),
                (
                    "base.kdl",
                    "component \"a\"\npolicy {\n    test-edges \"check\"\n}",
                ),
            ],
            "root.kdl",
        ),
        [(
            s("base.kdl"),
            2,
            1,
            ProblemKind::RootOnly { node: "policy" }
        )]
    );
}

#[test]
fn foundation_is_rejected_in_every_file() {
    let files = [
        ("root.kdl", "include \"base.kdl\"\nfoundation \"a\""),
        (
            "base.kdl",
            "component \"a\"\ncomponent \"b\"\nfoundation \"a\"",
        ),
    ];
    assert_eq!(
        problems(&files, "root.kdl"),
        [
            (s("root.kdl"), 2, 1, ProblemKind::FoundationRemoved),
            (s("base.kdl"), 3, 1, ProblemKind::FoundationRemoved),
        ]
    );
    let rendered = Files::new(&files)
        .parse("root.kdl")
        .unwrap_err()
        .to_string();
    assert!(
        rendered.contains("\nbase.kdl:3:1: `foundation` has been removed: write an `allow` rule"),
        "{rendered}"
    );
}

#[test]
fn include_is_top_level_only_and_takes_one_path() {
    let unknown = |expected| ProblemKind::UnknownNode {
        name: s("include"),
        expected,
    };
    let found = problems(
        &[
            (
                "root.kdl",
                "component \"a\" {\n    include \"base.kdl\"\n}\n\
                 external \"x\" {\n    include \"base.kdl\"\n}\n",
            ),
            ("base.kdl", "component \"b\""),
        ],
        "root.kdl",
    );
    assert_eq!(
        found,
        [
            (
                s("root.kdl"),
                2,
                5,
                unknown("doc, modules, items, component")
            ),
            (s("root.kdl"), 5, 5, unknown("doc, crates, modules, items")),
        ]
    );

    let bad = ProblemKind::BadArguments {
        node: "include",
        expected: "exactly one path",
    };
    let found = problems(
        &[
            (
                "root.kdl",
                "include\ninclude \"a.kdl\" \"b.kdl\"\ninclude \"a.kdl\" x=\"y\"\n\
             include \"a.kdl\" {\n    component \"c\"\n}\n",
            ),
            ("a.kdl", ""),
        ],
        "root.kdl",
    );
    assert_eq!(
        found,
        [
            (s("root.kdl"), 1, 1, bad.clone()),
            (s("root.kdl"), 2, 1, bad),
            (
                s("root.kdl"),
                3,
                17,
                ProblemKind::UnexpectedProperty { node: "include" }
            ),
            (
                s("root.kdl"),
                4,
                1,
                ProblemKind::UnexpectedChildren { node: "include" }
            ),
        ]
    );
}

#[test]
fn ids_and_patterns_are_global_across_files() {
    let found = problems(
        &[
            (
                "root.kdl",
                "component \"a\" {\n    modules \"app::a\"\n}\ninclude \"base.kdl\"\n",
            ),
            (
                "base.kdl",
                "component \"a\"\ncomponent \"b\" {\n    modules \"app::a\"\n}\n",
            ),
        ],
        "root.kdl",
    );
    assert_eq!(
        found,
        [
            (
                s("base.kdl"),
                1,
                11,
                ProblemKind::DuplicateComponent {
                    id: ComponentId::new("a")
                }
            ),
            (
                s("base.kdl"),
                3,
                13,
                ProblemKind::DuplicatePattern {
                    pattern: s("app::a"),
                    first: ComponentId::new("a"),
                    second: ComponentId::new("b"),
                }
            ),
        ]
    );
    // The earlier declaration is in the other file, and the message names it.
    let error = Files::new(&[
        ("root.kdl", "component \"a\"\ninclude \"base.kdl\""),
        ("base.kdl", "component \"a\""),
    ])
    .parse("root.kdl")
    .unwrap_err();
    let previous = error.problems()[0].previous.unwrap();
    assert_eq!((previous.line, previous.column), (1, 11));
    assert!(
        error.to_string().contains(
            "base.kdl:1:11: component `a` is declared twice (first declared at root.kdl:1:11)"
        ),
        "{error}"
    );
}

#[test]
fn duplicate_rules_across_files() {
    let found = problems(
        &[
            ("root.kdl", "include \"base.kdl\"\nallow \"a\" -> \"b\"\n"),
            (
                "base.kdl",
                "component \"a\"\ncomponent \"b\"\nallow \"a\" -> \"b\"\n",
            ),
        ],
        "root.kdl",
    );
    assert_eq!(
        found,
        [(
            s("root.kdl"),
            2,
            14,
            ProblemKind::DuplicateRule {
                from: ComponentId::new("a"),
                to: ComponentId::new("b"),
            }
        )]
    );
}

#[test]
fn references_may_name_components_of_any_file() {
    let model = parse(
        &[
            (
                "root.kdl",
                "component \"app\"\nallow \"app\" -> \"shared\" \"db\"\n\
                 link \"app\" -> \"tool\" via=\"process\"\ninclude \"base.kdl\"\n",
            ),
            (
                "base.kdl",
                "component \"shared\"\ncomponent \"tool\"\nexternal \"db\" {\n    crates \"sqlx\"\n}\n\
                 allow \"tool\" -> \"app\"\n",
            ),
        ],
        "root.kdl",
    );
    let rules: Vec<(&str, &str)> = model
        .rules()
        .iter()
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect();
    assert_eq!(rules, [("app", "shared"), ("app", "db"), ("tool", "app")]);
    assert_eq!(model.links()[0].to.as_str(), "tool");
}

#[test]
fn policy_targets_may_name_components_of_any_file() {
    let model = parse(
        &[
            (
                "root.kdl",
                "include \"base.kdl\"\npolicy {\n    targets \"core\" \"db\"\n}\n",
            ),
            (
                "base.kdl",
                "component \"core\"\nexternal \"db\" {\n    crates \"sqlx\"\n}\n",
            ),
        ],
        "root.kdl",
    );
    assert_eq!(
        model.policy().targets,
        Some([ComponentId::new("core"), ComponentId::new("db")].into())
    );
}

#[test]
fn problems_in_included_files_are_reported_and_rendered_in_their_file() {
    let base = "component \"b\" {\n    modules \"x::**::y\"\n}\nallow \"b\" -> \"nope\"\n";
    let error = Files::new(&[
        ("root.kdl", "include \"base.kdl\"\nbogus\n"),
        ("base.kdl", base),
    ])
    .parse("root.kdl")
    .unwrap_err();
    let rendered = error.to_string();
    assert_eq!(
        rendered.lines().collect::<Vec<_>>()[..3],
        [
            "root.kdl: invalid architecture model",
            "root.kdl:2:1: unknown node `bogus` here; expected one of: component, external, \
             allow, link, policy, include",
            "base.kdl:2:13: malformed module pattern `x::**::y`: `**` is only allowed as the \
             last segment, as `a::b::**`",
        ]
    );
    assert!(
        rendered.contains("base.kdl:4:14: `nope` is not a declared component"),
        "{rendered}"
    );

    // As a diagnostic, each problem carries its own file's text.
    let spans = labelled(&error);
    assert_eq!(
        spans,
        [
            (s("root.kdl"), s("bogus")),
            (s("base.kdl"), s("\"x::**::y\"")),
            (s("base.kdl"), s("\"nope\"")),
        ]
    );
}

#[test]
fn syntax_errors_in_included_files_are_located_there() {
    let found = problems(
        &[
            ("root.kdl", "include \"base.kdl\"\nallow \"a\" -> \"b\"\n"),
            ("base.kdl", "component \"a\" {\n"),
        ],
        "root.kdl",
    );
    // Only syntax errors: `a` and `b` would be reported as unknown, wrongly.
    assert_ne!(found.len(), 0);
    for (file, _, _, kind) in &found {
        assert_eq!(file, "base.kdl");
        assert!(matches!(kind, ProblemKind::Syntax { .. }), "{found:#?}");
    }
}

#[test]
fn include_without_a_loader_is_an_error() {
    let error = Model::parse("root.kdl", "component \"a\"\ninclude \"base.kdl\"\n").unwrap_err();
    let found: Vec<_> = error
        .problems()
        .iter()
        .map(|p| (p.location.line, p.location.column, p.kind.clone()))
        .collect();
    assert_eq!(found, [(2, 9, ProblemKind::IncludeWithoutLoader)]);
}

#[test]
fn a_failed_load_is_reported_at_its_include() {
    let found = problems(
        &[
            (
                "root.kdl",
                "component \"a\"\ninclude \"base.kdl\"\nallow \"a\" -> \"b\"\n",
            ),
            ("base.kdl", "include \"missing.kdl\"\ncomponent \"b\""),
        ],
        "root.kdl",
    );
    assert_eq!(
        found,
        [(
            s("base.kdl"),
            1,
            9,
            ProblemKind::IncludeFailed {
                path: s("missing.kdl"),
                message: s("no such file `missing.kdl`"),
            }
        )]
    );
}

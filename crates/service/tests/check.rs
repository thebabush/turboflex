//! Loading fact directories and models from disk, and summarizing each view.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use turboflex_graph::{
    CompileMode, CrateName, Edge, EdgeGroup, EdgeKind, Entity, EntityKind, ItemPath, ModulePath,
    Origin, Provenance, TargetKind, Unit, UnitHeader,
};
use turboflex_model::ProblemKind;
use turboflex_report::{GroupCount, SkipReason};
use turboflex_service::{Options, ServiceError, check, load_facts, load_model};

/// An entity for `path`, its module being everything before the last `::`; `sqlx` paths are
/// external.
fn entity(path: &str) -> Entity {
    let (module, _) = path.rsplit_once("::").expect("paths have a module");
    let crate_name = path.split("::").next().expect("non-empty");
    Entity {
        path: ItemPath::new(path),
        kind: EntityKind::Function,
        module: ModulePath::new(module),
        crate_name: CrateName::new(crate_name),
        origin: if crate_name == "sqlx" {
            Origin::External
        } else {
            Origin::Workspace
        },
        span: None,
    }
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

/// A unit of `edges`, its entities derived from the edge ends.
fn unit(mode: CompileMode, edges: Vec<Edge>) -> Unit {
    let mut paths: Vec<&str> = edges
        .iter()
        .flat_map(|e| [e.src.as_str(), e.dst.as_str()])
        .collect();
    paths.sort_unstable();
    paths.dedup();
    Unit {
        header: UnitHeader {
            crate_name: CrateName::new("app"),
            target: TargetKind::Lib,
            mode,
            features: Vec::new(),
        },
        entities: paths.into_iter().map(entity).collect(),
        edges,
    }
}

fn write_unit(dir: &Path, name: &str, unit: &Unit) {
    let file = File::create(dir.join(name)).expect("temp dir is writable");
    unit.write_jsonl(file).expect("unit written");
}

fn write_model(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).expect("temp dir is writable");
    path
}

const TARGET: &str = r#"
component "report" { modules "app::report" }
component "ledger" { modules "app::db" }
component "cli" { modules "app::cli" }
external "postgres" { crates "sqlx" }
allow "report" -> "ledger" kinds="types"
allow "ledger" -> "postgres"
allow "cli" -> "report"
"#;

/// Another view of the same facts: only calls, and report may call the ledger.
const CALLS_VIEW: &str = r#"
component "report" { modules "app::report" }
component "ledger" { modules "app::db" }
allow "report" -> "ledger"
policy { edges "calls" }
"#;

/// Two units (one a test build), and a stray file the loader must ignore.
fn facts() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    write_unit(
        dir.path(),
        "app-lib.jsonl",
        &unit(
            CompileMode::Normal,
            vec![
                edge("app::report::render", "app::db::Row", EdgeKind::UsesTypeSig),
                edge("app::report::render", "app::db::get", EdgeKind::Calls),
                edge("app::report::render", "app::db::put", EdgeKind::Calls),
                edge("app::report::render", "app::db::scan", EdgeKind::Calls),
                edge("app::db::get", "sqlx::query", EdgeKind::Calls),
                edge("app::stray::f", "app::db::get", EdgeKind::Calls),
                edge("app::stray::g", "app::db::put", EdgeKind::Calls),
            ],
        ),
    );
    write_unit(
        dir.path(),
        "app-test.jsonl",
        &unit(
            CompileMode::Test,
            vec![edge("app::db::tests::t", "app::db::get", EdgeKind::Calls)],
        ),
    );
    fs::write(dir.path().join("notes.txt"), "not a fact file").expect("temp dir is writable");
    dir
}

#[test]
fn every_view_runs_over_one_fact_base() {
    let facts = facts();
    let models = tempfile::tempdir().expect("temp dir");
    let target = write_model(models.path(), "target.kdl", TARGET);
    let calls = write_model(models.path(), "calls.kdl", CALLS_VIEW);

    let checked = check(
        facts.path(),
        &[target.clone(), calls.clone()],
        Options { samples: 2 },
    )
    .expect("check runs");
    let report = checked.report();
    assert_eq!(report.facts.units, 2);
    assert_eq!(report.facts.edges, 8);
    assert_eq!(checked.views().len(), 2);
    assert_eq!(checked.views()[0].path(), target);

    let [target_view, calls_view] = report.views.as_slice() else {
        panic!("two views: {:?}", report.views)
    };
    assert_eq!(target_view.model, target.display().to_string());
    assert_eq!(target_view.headline.convergences, 1);
    assert_eq!(target_view.headline.divergences, 1);
    assert_eq!(target_view.headline.absences, 1);
    // app::stray::{f,g}, and app::db::tests::t: `app::db` does not cover its submodules.
    assert_eq!(target_view.headline.unmapped, 3);
    assert_eq!(target_view.headline.conflicts, 0);
    assert_eq!(target_view.skipped[&SkipReason::TestOnly], 1);
    assert_eq!(target_view.skipped[&SkipReason::Unmapped], 2);
    // Reasons no edge has are listed too, at 0, in declaration order.
    assert!(target_view.skipped.keys().eq(&SkipReason::ALL));
    assert_eq!(target_view.skipped[&SkipReason::Conflict], 0);

    let [divergence] = target_view.divergences.as_slice() else {
        panic!("{:?}", target_view.divergences)
    };
    assert_eq!(
        (divergence.from.as_str(), divergence.to.as_str()),
        ("report", "ledger")
    );
    assert_eq!((divergence.edges, divergence.divergent), (4, 3));
    assert_eq!(
        divergence.groups[&EdgeGroup::Types],
        GroupCount {
            allowed: 1,
            divergent: 0
        }
    );
    // The first `samples` divergent edges, in (src, dst, kind) order.
    let samples: Vec<&str> = divergence.samples.iter().map(|s| s.dst.as_str()).collect();
    assert_eq!(samples, ["app::db::get", "app::db::put"]);

    let [absence] = target_view.absences.as_slice() else {
        panic!("{:?}", target_view.absences)
    };
    assert_eq!(
        (absence.from.as_str(), absence.to.as_str()),
        ("cli", "report")
    );
    assert_eq!(absence.location.file, target.display().to_string());
    assert_eq!(absence.location.line, 8);

    let [db_tests, stray] = target_view.unmapped.as_slice() else {
        panic!("{:?}", target_view.unmapped)
    };
    assert_eq!(db_tests.module.as_str(), "app::db::tests");
    // Its only edge is test-only: skipped as such, not counted against it.
    assert_eq!((db_tests.entities, db_tests.edges), (1, 0));
    assert_eq!(stray.module.as_str(), "app::stray");
    assert_eq!((stray.entities, stray.edges), (2, 2));

    // The calls-only view over the same graph: everything allowed, the type edge filtered.
    assert_eq!(calls_view.headline.divergences, 0);
    assert_eq!(calls_view.headline.convergences, 1);
    assert_eq!(calls_view.skipped[&SkipReason::GroupFiltered], 1);
    assert!(report.views[0].has_findings());
    assert!(!calls_view.has_findings());
    assert!(checked.has_findings());

    let mut text = Vec::new();
    checked.write_text(&mut text).expect("writing to a Vec");
    let text = String::from_utf8(text).expect("UTF-8");
    assert!(
        text.contains(&format!("== {}\n", calls.display())),
        "{text}"
    );
}

#[test]
fn a_bad_fact_file_is_named() {
    let facts = facts();
    let bad = facts.path().join("broken.jsonl");
    fs::write(&bad, "{\"record\":\"header\",\"schema_version\":1}\n").expect("writable");
    let error = load_facts(facts.path()).expect_err("an old schema version is rejected");
    let ServiceError::Facts { path, .. } = &error else {
        panic!("{error:?}")
    };
    assert_eq!(path, &bad);
    let message = error.to_string();
    assert!(message.starts_with(&bad.display().to_string()), "{message}");
    assert!(message.contains("schema version 1"), "{message}");
}

#[test]
fn a_directory_without_facts_is_an_error() {
    let empty = tempfile::tempdir().expect("temp dir");
    let error = load_facts(empty.path()).expect_err("no fact files");
    assert!(matches!(error, ServiceError::NoFacts { .. }), "{error:?}");
    let missing = empty.path().join("missing");
    let error = load_facts(&missing).expect_err("no directory");
    assert!(matches!(error, ServiceError::FactsDir { .. }), "{error:?}");
}

#[test]
fn model_problems_are_reported_with_their_locations() {
    let facts = facts();
    let models = tempfile::tempdir().expect("temp dir");
    let good = write_model(models.path(), "good.kdl", TARGET);
    let bad = write_model(
        models.path(),
        "bad.kdl",
        "component \"a\" { modules \"app::a\" }\nallow \"a\" -> \"nope\"\n",
    );
    let error = check(facts.path(), &[good, bad.clone()], Options::default())
        .expect_err("the second model is invalid");
    assert!(matches!(error, ServiceError::Model(_)), "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains(&format!(
            "{}:2:14: `nope` is not a declared component",
            bad.display()
        )),
        "{message}"
    );

    let missing = models.path().join("missing.kdl");
    let error = check(facts.path(), &[missing], Options::default()).expect_err("no such file");
    assert!(matches!(error, ServiceError::ModelFile { .. }), "{error:?}");
}

/// The kinds of the problems of an invalid model, in order.
fn model_problems(error: &ServiceError) -> Vec<ProblemKind> {
    let ServiceError::Model(error) = error else {
        panic!("{error:?}")
    };
    error.problems().iter().map(|p| p.kind.clone()).collect()
}

#[test]
fn included_files_are_resolved_from_the_including_file() {
    let facts = facts();
    let models = tempfile::tempdir().expect("temp dir");
    fs::create_dir(models.path().join("shared")).expect("temp dir is writable");
    let view = write_model(
        models.path(),
        "view.kdl",
        "include \"shared/components.kdl\"\n\
         allow \"report\" -> \"ledger\" kinds=\"types\"\n\
         allow \"ledger\" -> \"postgres\"\n",
    );
    write_model(
        models.path(),
        "shared/components.kdl",
        "component \"report\" { modules \"app::report\" }\n\
         component \"ledger\" { modules \"app::db\" }\n\
         component \"cli\" { modules \"app::cli\" }\n\
         external \"postgres\" { crates \"sqlx\" }\n\
         include \"../rules.kdl\"\n",
    );
    write_model(
        models.path(),
        "rules.kdl",
        "\nallow \"cli\" -> \"report\"\n",
    );

    let checked = check(
        facts.path(),
        std::slice::from_ref(&view),
        Options::default(),
    )
    .expect("check runs");
    let [summary] = checked.report().views.as_slice() else {
        panic!("one view: {:?}", checked.report().views)
    };
    assert_eq!(summary.model, view.display().to_string());
    assert_eq!(summary.headline.convergences, 1);
    let [absence] = summary.absences.as_slice() else {
        panic!("{:?}", summary.absences)
    };
    assert_eq!(
        (absence.from.as_str(), absence.to.as_str()),
        ("cli", "report")
    );
    // Named by the file the rule is in, `..` folded.
    let rules = models.path().join("rules.kdl").display().to_string();
    assert_eq!(absence.location.file, rules);
    assert_eq!((absence.location.line, absence.location.column), (2, 16));

    let mut text = Vec::new();
    checked.write_text(&mut text).expect("writing to a Vec");
    let text = String::from_utf8(text).expect("UTF-8");
    assert!(text.contains(&format!("== {}\n", view.display())), "{text}");
    assert!(
        text.contains(&format!("cli -> report  {rules}:2:16")),
        "{text}"
    );
}

#[test]
fn a_file_included_through_two_spellings_is_loaded_once() {
    let models = tempfile::tempdir().expect("temp dir");
    fs::create_dir(models.path().join("left")).expect("temp dir is writable");
    let view = write_model(
        models.path(),
        "view.kdl",
        "include \"left/left.kdl\"\ninclude \"right.kdl\"\n",
    );
    write_model(
        models.path(),
        "left/left.kdl",
        "include \"../common.kdl\"\n",
    );
    write_model(
        models.path(),
        "right.kdl",
        "include \"./left/../common.kdl\"\n",
    );
    write_model(models.path(), "common.kdl", TARGET);

    let model = load_model(&view).unwrap_or_else(|e| panic!("{e}"));
    let name = |file: &str| models.path().join(file).display().to_string();
    assert_eq!(
        model.source_names(),
        [
            name("view.kdl"),
            name("left/left.kdl"),
            name("common.kdl"),
            name("right.kdl")
        ]
    );
    assert_eq!(model.rules().len(), 3);
}

#[test]
fn an_include_cycle_through_the_root_is_reported_once() {
    let models = tempfile::tempdir().expect("temp dir");
    fs::create_dir(models.path().join("shared")).expect("temp dir is writable");
    write_model(
        models.path(),
        "view.kdl",
        "component \"a\" { modules \"app::a\" }\ninclude \"shared/loop.kdl\"\n",
    );
    write_model(
        models.path(),
        "shared/loop.kdl",
        "include \"../view.kdl\"\n",
    );

    // Another spelling of the root: its name is normalized like the included files', so the
    // cycle closes at the root instead of after reading it a second time.
    let root = models.path().join("shared/../view.kdl");
    let error = load_model(&root).expect_err("a cycle");
    let name = |file: &str| models.path().join(file).display().to_string();
    assert_eq!(
        model_problems(&error),
        [ProblemKind::IncludeCycle {
            chain: vec![name("view.kdl"), name("shared/loop.kdl"), name("view.kdl")]
        }]
    );
    let message = error.to_string();
    assert!(
        message.contains(&format!("{}:1:9: include cycle: ", name("shared/loop.kdl"))),
        "{message}"
    );
}

#[test]
fn a_missing_include_is_reported_at_its_include() {
    let models = tempfile::tempdir().expect("temp dir");
    let view = write_model(
        models.path(),
        "view.kdl",
        "include \"shared/missing.kdl\"\n",
    );

    let error = load_model(&view).expect_err("no such file");
    let problems = model_problems(&error);
    let [ProblemKind::IncludeFailed { path, .. }] = problems.as_slice() else {
        panic!("{error:?}")
    };
    assert_eq!(path, "shared/missing.kdl");
    let message = error.to_string();
    let missing = models.path().join("shared/missing.kdl");
    assert!(
        message.contains(&format!(
            "{}:1:9: cannot include `shared/missing.kdl`: {}: ",
            view.display(),
            missing.display()
        )),
        "{message}"
    );
}

//! Smoke tests of the binary: output and exit status.

use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Output};

use turboflex_graph::{
    CompileMode, CrateName, Edge, EdgeKind, Entity, EntityKind, ItemPath, ModulePath, Origin,
    Provenance, TargetKind, Unit, UnitHeader,
};

fn entity(path: &str, module: &str) -> Entity {
    Entity {
        path: ItemPath::new(path),
        kind: EntityKind::Function,
        module: ModulePath::new(module),
        crate_name: CrateName::new("app"),
        origin: Origin::Workspace,
        span: None,
    }
}

/// One unit with a single call `app::a::f -> app::b::g`.
fn write_facts(dir: &Path) {
    let unit = Unit {
        header: UnitHeader {
            crate_name: CrateName::new("app"),
            target: TargetKind::Lib,
            mode: CompileMode::Normal,
            features: Vec::new(),
        },
        entities: vec![entity("app::a::f", "app::a"), entity("app::b::g", "app::b")],
        edges: vec![Edge {
            src: ItemPath::new("app::a::f"),
            dst: ItemPath::new("app::b::g"),
            kind: EdgeKind::Calls,
            provenance: Provenance::Source,
            span: None,
            detail: String::new(),
        }],
    };
    let file = File::create(dir.join("app.jsonl")).expect("temp dir is writable");
    unit.write_jsonl(file).expect("unit written");
}

const COMPONENTS: &str =
    "component \"a\" { modules \"app::a\" }\ncomponent \"b\" { modules \"app::b\" }\n";

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_turboflex"))
        .args(args)
        .output()
        .expect("the binary runs")
}

#[test]
fn exit_status_says_whether_a_view_has_findings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let facts = dir.path().join("facts");
    fs::create_dir(&facts).expect("temp dir is writable");
    write_facts(&facts);
    let allowed = dir.path().join("allowed.kdl");
    fs::write(&allowed, format!("{COMPONENTS}allow \"a\" -> \"b\"\n")).expect("writable");
    let forbidden = dir.path().join("forbidden.kdl");
    fs::write(&forbidden, COMPONENTS).expect("writable");
    let (facts, allowed, forbidden) = (
        facts.to_str().expect("UTF-8 temp path"),
        allowed.to_str().expect("UTF-8 temp path"),
        forbidden.to_str().expect("UTF-8 temp path"),
    );

    let clean = run(&["check", "--facts", facts, allowed]);
    assert_eq!(clean.status.code(), Some(0), "{clean:?}");
    let stdout = String::from_utf8(clean.stdout).expect("UTF-8");
    assert!(
        stdout.starts_with("facts: 1 units, 2 entities, 1 edges\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("== {allowed}\nconvergences 1, divergences 0")),
        "{stdout}"
    );
    // Nothing is skipped, and every reason is still listed.
    assert!(
        stdout.contains(
            "\nskipped edges: group_filtered 0, provenance_filtered 0, test_only 0, unmapped 0, \
             conflict 0, out_of_scope 0, not_targeted 0, same_component 0\n"
        ),
        "{stdout}"
    );

    let both = run(&[
        "check", "--facts", facts, allowed, forbidden, "--format", "json",
    ]);
    assert_eq!(both.status.code(), Some(1), "{both:?}");
    let stdout = String::from_utf8(both.stdout).expect("UTF-8");
    assert!(
        stdout.starts_with('{') && stdout.contains("\"divergences\": 1"),
        "{stdout}"
    );

    let missing = run(&["check", "--facts", facts, "missing.kdl"]);
    assert_eq!(missing.status.code(), Some(2), "{missing:?}");
    let stderr = String::from_utf8(missing.stderr).expect("UTF-8");
    assert!(stderr.starts_with("error: missing.kdl: "), "{stderr}");

    let usage = run(&["check", "--facts", facts, allowed, "--format", "xml"]);
    assert_eq!(usage.status.code(), Some(2), "{usage:?}");
}

#[test]
fn locations_name_the_included_file_they_are_in() {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("facts")).expect("temp dir is writable");
    write_facts(&dir.path().join("facts"));
    fs::create_dir_all(dir.path().join("views/shared")).expect("temp dir is writable");
    fs::write(
        dir.path().join("views/view.kdl"),
        "include \"shared/common.kdl\"\nallow \"a\" -> \"b\"\n",
    )
    .expect("writable");
    fs::write(
        dir.path().join("views/shared/common.kdl"),
        format!("{COMPONENTS}component \"c\" {{ modules \"app::c\" }}\n\nallow \"c\" -> \"a\"\n"),
    )
    .expect("writable");
    fs::write(
        dir.path().join("views/broken.kdl"),
        "include \"nope.kdl\"\n",
    )
    .expect("writable");

    // Relative paths, from the temp dir: reports show them as typed.
    let run_here = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_turboflex"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .expect("the binary runs")
    };
    let ok = run_here(&["check", "--facts", "facts", "./views/view.kdl"]);
    assert_eq!(ok.status.code(), Some(0), "{ok:?}");
    let stdout = String::from_utf8(ok.stdout).expect("UTF-8");
    assert!(stdout.contains("== views/view.kdl\n"), "{stdout}");
    assert!(
        stdout.contains("absences:\n  c -> a  views/shared/common.kdl:5:14\n"),
        "{stdout}"
    );

    let broken = run_here(&["check", "--facts", "facts", "views/broken.kdl"]);
    assert_eq!(broken.status.code(), Some(2), "{broken:?}");
    assert_eq!(broken.stdout, b"");
    let stderr = String::from_utf8(broken.stderr).expect("UTF-8");
    assert!(
        stderr.starts_with(
            "error: views/broken.kdl: invalid architecture model\n\
             views/broken.kdl:1:9: cannot include `nope.kdl`: views/nope.kdl: "
        ),
        "{stderr}"
    );
}

#[test]
fn edges_outside_the_targets_are_counted_as_not_targeted() {
    let dir = tempfile::tempdir().expect("temp dir");
    let facts = dir.path().join("facts");
    fs::create_dir(&facts).expect("temp dir is writable");
    write_facts(&facts);
    let view = dir.path().join("view.kdl");
    fs::write(&view, format!("{COMPONENTS}policy {{ targets \"a\" }}\n")).expect("writable");
    let (facts, view) = (
        facts.to_str().expect("UTF-8 temp path"),
        view.to_str().expect("UTF-8 temp path"),
    );

    // The only edge goes into `b`, which the view does not target.
    let text = run(&["check", "--facts", facts, view]);
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    let stdout = String::from_utf8(text.stdout).expect("UTF-8");
    assert!(
        stdout.contains(
            "\nskipped edges: group_filtered 0, provenance_filtered 0, test_only 0, unmapped 0, \
             conflict 0, out_of_scope 0, not_targeted 1, same_component 0\n"
        ),
        "{stdout}"
    );

    let json = run(&["check", "--facts", facts, view, "--format", "json"]);
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    let stdout = String::from_utf8(json.stdout).expect("UTF-8");
    assert!(stdout.contains("\"not_targeted\": 1"), "{stdout}");
}

//! The taint fixture (`fixtures/taint`): its capability and sanitizer views over its real facts,
//! with the exact findings its README lists.
//!
//! The facts are generated, gitignored data (`runs/taint-facts`; the README says how to make
//! them), so this test is ignored by default:
//! `cargo test -p turboflex-service --test taint -- --ignored`.

use std::collections::BTreeMap;
use std::path::Path;

use turboflex_engine::{Dependency, LiftedEdge, Reflexion};
use turboflex_service::{Check, Options, View, check};

type Pairs = BTreeMap<(String, String), Vec<String>>;

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Each dependency's `(from, to)` and the `src -> dst` of the edges `pick` selects, sorted.
fn pairs<'a, I>(
    dependencies: impl Iterator<Item = &'a Dependency>,
    pick: fn(&'a Dependency) -> I,
) -> Pairs
where
    I: Iterator<Item = &'a LiftedEdge>,
{
    dependencies
        .map(|dependency| {
            let mut edges: Vec<String> = pick(dependency)
                .map(|e| format!("{} -> {}", e.edge.edge.src, e.edge.edge.dst))
                .collect();
            edges.sort();
            let key = (dependency.from().to_string(), dependency.to().to_string());
            (key, edges)
        })
        .collect()
}

fn divergences(reflexion: &Reflexion) -> Pairs {
    pairs(reflexion.divergences(), Dependency::divergent_edges)
}

fn convergences(reflexion: &Reflexion) -> Pairs {
    pairs(reflexion.convergences(), |d| d.edges().iter())
}

fn expected(entries: &[(&str, &str, &[&str])]) -> Pairs {
    entries
        .iter()
        .map(|(from, to, edges)| {
            let mut edges: Vec<String> = edges.iter().map(|&e| e.to_owned()).collect();
            edges.sort();
            (((*from).to_owned(), (*to).to_owned()), edges)
        })
        .collect()
}

/// Every divergent edge's source, across all pairs.
fn divergent_sources(reflexion: &Reflexion) -> Vec<String> {
    reflexion
        .divergences()
        .flat_map(Dependency::divergent_edges)
        .map(|e| e.edge.edge.src.to_string())
        .collect()
}

fn assert_complete(view: &View) {
    let reflexion = view.reflexion();
    assert_eq!(reflexion.unmapped().len(), 0, "{:?}", reflexion.unmapped());
    assert_eq!(
        reflexion.conflicts().len(),
        0,
        "{:?}",
        reflexion.conflicts()
    );
    assert_eq!(reflexion.absences().count(), 0);
}

/// The one view of `fixtures/taint/{model}` over `runs/taint-facts`.
fn check_view(model: &str) -> Check {
    let root = repo_root();
    let models = [root.join("fixtures/taint").join(model)];
    check(&root.join("runs/taint-facts"), &models, Options::default())
        .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
#[ignore = "needs the gitignored runs/taint-facts"]
fn capability_view() {
    let checked = check_view("capability.kdl");
    let [capability] = checked.views() else {
        panic!("one view")
    };

    // Capability: v01 and v02 use a sink from the rest of the workspace, v03 is the process sink
    // writing a file.
    assert_complete(capability);
    let reflexion = capability.reflexion();
    assert_eq!(
        divergences(reflexion),
        expected(&[
            (
                "exec",
                "fs-write",
                &["app::exec::run_logged -> std::fs::write"]
            ),
            (
                "rest",
                "fs-write",
                &["app::cases::v02_write_directly::dump -> std::fs::write"]
            ),
            (
                "rest",
                "process",
                &[
                    "app::cases::v01_spawn_directly::list_dir -> std::process::Command::arg",
                    "app::cases::v01_spawn_directly::list_dir -> std::process::Command::new",
                    "app::cases::v01_spawn_directly::list_dir -> std::process::Command::status",
                ]
            ),
        ])
    );
    let sinks = convergences(reflexion);
    assert_eq!(
        sinks[&("exec".to_owned(), "process".to_owned())],
        [
            "app::exec::run -> std::process::Command::args",
            "app::exec::run -> std::process::Command::new",
            "app::exec::run -> std::process::Command::status",
            "app::exec::run -> std::process::ExitStatus::success",
        ]
    );
    assert_eq!(
        sinks[&("storage".to_owned(), "fs-write".to_owned())],
        [
            "app::storage::append -> std::fs::OpenOptions::open",
            "app::storage::save -> std::fs::write",
            "app::storage::touch -> std::fs::File::create",
        ]
    );
    // l03 reads a file: `read_to_string` is in no sink.
    assert!(
        !divergent_sources(reflexion)
            .iter()
            .any(|src| src.starts_with("app::cases::l0")),
    );
}

#[test]
#[ignore = "needs the gitignored runs/taint-facts"]
fn sanitizer_view() {
    let checked = check_view("sanitizer.kdl");
    let [sanitizer] = checked.views() else {
        panic!("one view")
    };

    // Sanitizer: b01-b05 build a SafeCmd outside the type's module, b06-b08 inside it.
    assert_complete(sanitizer);
    let reflexion = sanitizer.reflexion();
    assert_eq!(
        divergences(reflexion),
        expected(&[
            (
                "rest",
                "safe-cmd",
                &[
                    "app::cases::b01_tuple_literal::run_raw -> app::safe_cmd::SafeCmd",
                    "app::cases::b02_ctor_as_fn::wrap_all -> app::safe_cmd::SafeCmd",
                    "<app::safe_cmd::SafeCmd as core::convert::From<alloc::string::String>>::from \
                     -> app::safe_cmd::SafeCmd",
                    "app::safe_cmd::SafeCmd::from_trusted -> app::safe_cmd::SafeCmd",
                    "app::cases::b05_braced_literal::braced -> app::safe_cmd::SafeCmd",
                ]
            ),
            (
                "safe-cmd-home",
                "safe-cmd",
                &[
                    "<app::safe_cmd::SafeCmd as core::default::Default>::default \
                     -> app::safe_cmd::SafeCmd",
                    "<app::safe_cmd::SafeCmd as serde_core::Deserialize>::deserialize \
                     -> app::safe_cmd::SafeCmd",
                    "app::safe_cmd::SafeCmd::new_unchecked -> app::safe_cmd::SafeCmd",
                ]
            ),
        ])
    );
    // The legit constructors: the sanitizer's fn, and the two it claims next to the type.
    assert_eq!(
        convergences(reflexion),
        expected(&[(
            "sanitizer",
            "safe-cmd",
            &[
                "<app::safe_cmd::SafeCmd as core::clone::Clone>::clone -> app::safe_cmd::SafeCmd",
                "app::safe_cmd::SafeCmd::parse -> app::safe_cmd::SafeCmd",
                "app::sanitizer::sanitize -> app::safe_cmd::SafeCmd",
            ]
        )])
    );
    let sources = divergent_sources(reflexion);
    // Legit uses (l01, l04, exec's destructuring) and mere callers of a bypass are not flagged,
    // and neither is b09: mutating the field constructs nothing (a known gap, see the README).
    for module in [
        "app::cases::l0",
        "app::exec::",
        "app::cases::b03_from_string::convert",
        "app::cases::b04_assoc_elsewhere::trusted_pair",
        "app::cases::b05_braced_literal::empty",
        "app::cases::b09_field_mutation::",
    ] {
        assert!(
            !sources.iter().any(|src| src.starts_with(module)),
            "{module}: {sources:?}"
        );
    }
}

//! Text and JSON rendering of a small hand-built report.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use turboflex_graph::{EdgeGroup, EdgeKind, ItemPath, ModulePath, Provenance, Span};
use turboflex_report::{
    Absence, Conflict, Divergence, EdgeSample, Facts, GroupCount, Headline, ModelLocation, Report,
    SkipReason, UnmappedModule, View,
};

fn sample(src: &str, dst: &str, kind: EdgeKind, line: Option<u32>) -> EdgeSample {
    EdgeSample {
        src: ItemPath::new(src),
        dst: ItemPath::new(dst),
        kind,
        provenance: Provenance::Source,
        span: line.map(|line| Span {
            file: "src/report.rs".to_owned(),
            line: NonZeroU32::new(line).expect("lines are 1-based"),
        }),
    }
}

/// Every skip reason, with the given counts and 0 for the rest, as the service fills them.
fn skipped(counts: &[(SkipReason, usize)]) -> BTreeMap<SkipReason, usize> {
    let mut skipped: BTreeMap<SkipReason, usize> = SkipReason::ALL
        .into_iter()
        .map(|reason| (reason, 0))
        .collect();
    skipped.extend(counts.iter().copied());
    skipped
}

fn view() -> View {
    View {
        model: "arch.kdl".to_owned(),
        headline: Headline {
            convergences: 2,
            divergences: 1,
            absences: 1,
            unmapped: 3,
            conflicts: 1,
        },
        skipped: skipped(&[(SkipReason::TestOnly, 4), (SkipReason::SameComponent, 7)]),
        divergences: vec![Divergence {
            from: "report".to_owned(),
            to: "ledger".to_owned(),
            edges: 4,
            divergent: 3,
            groups: BTreeMap::from([
                (
                    EdgeGroup::Types,
                    GroupCount {
                        allowed: 1,
                        divergent: 0,
                    },
                ),
                (
                    EdgeGroup::Calls,
                    GroupCount {
                        allowed: 0,
                        divergent: 2,
                    },
                ),
                (
                    EdgeGroup::Matches,
                    GroupCount {
                        allowed: 1,
                        divergent: 1,
                    },
                ),
            ]),
            samples: vec![
                sample(
                    "app::report::render",
                    "app::db::get",
                    EdgeKind::Calls,
                    Some(12),
                ),
                sample(
                    "app::report::render",
                    "app::db::Mode",
                    EdgeKind::MatchesOn,
                    None,
                ),
            ],
        }],
        absences: vec![Absence {
            from: "cli".to_owned(),
            to: "report".to_owned(),
            kinds: Some(vec![EdgeGroup::Types, EdgeGroup::Calls]),
            location: ModelLocation {
                file: "arch.kdl".to_owned(),
                line: 9,
                column: 14,
            },
        }],
        unmapped: vec![
            UnmappedModule {
                module: ModulePath::new("app::stray"),
                entities: 2,
                edges: 5,
            },
            UnmappedModule {
                module: ModulePath::new("app::tests"),
                entities: 1,
                edges: 0,
            },
        ],
        conflicts: vec![Conflict {
            path: ItemPath::new("app::m::helper"),
            claims: vec!["a".to_owned(), "b".to_owned()],
            edges: 3,
        }],
    }
}

fn clean_view() -> View {
    View {
        model: "view.kdl".to_owned(),
        headline: Headline {
            convergences: 1,
            divergences: 0,
            absences: 0,
            unmapped: 0,
            conflicts: 0,
        },
        skipped: skipped(&[]),
        divergences: Vec::new(),
        absences: Vec::new(),
        unmapped: Vec::new(),
        conflicts: Vec::new(),
    }
}

fn report(views: Vec<View>) -> Report {
    Report {
        facts: Facts {
            units: 2,
            entities: 10,
            edges: 20,
        },
        views,
    }
}

fn text(report: &Report) -> String {
    let mut out = Vec::new();
    report.write_text(&mut out).expect("writing to a Vec");
    String::from_utf8(out).expect("UTF-8")
}

#[test]
fn text_lists_every_section_under_a_header_per_view() {
    let expected = "\
facts: 2 units, 10 entities, 20 edges

== arch.kdl
convergences 2, divergences 1, absences 1, unmapped 3, conflicts 1
skipped edges: group_filtered 0, provenance_filtered 0, test_only 4, unmapped 0, conflict 0, \
out_of_scope 0, not_targeted 0, same_component 7

divergences (group:allowed+divergent!):
  report -> ledger  4 edges, 3 divergent  [types:1 calls:2! matches:1+1!]
      app::report::render -> app::db::get (calls, source) src/report.rs:12
      app::report::render -> app::db::Mode (matches_on, source)
      ... 1 more

absences:
  cli -> report kinds=types,calls  arch.kdl:9:14

unmapped (module: entities, unchecked edges):
  app::stray: 2 entities, 5 edges
  app::tests: 1 entities, 0 edges

conflicts:
  app::m::helper: claimed by a, b; 3 edges

== view.kdl
convergences 1, divergences 0, absences 0, unmapped 0, conflicts 0
skipped edges: group_filtered 0, provenance_filtered 0, test_only 0, unmapped 0, conflict 0, \
out_of_scope 0, not_targeted 0, same_component 0
";
    assert_eq!(text(&report(vec![view(), clean_view()])), expected);
}

#[test]
fn findings_are_divergences_or_conflicts() {
    assert!(report(vec![clean_view(), view()]).has_findings());
    assert!(!report(vec![clean_view()]).has_findings());

    let mut absences_only = clean_view();
    absences_only.headline.absences = 1;
    absences_only.headline.unmapped = 4;
    assert!(!absences_only.has_findings());
    let mut conflict = clean_view();
    conflict.headline.conflicts = 1;
    assert!(conflict.has_findings());
}

#[test]
fn json_uses_wire_names() {
    let mut out = Vec::new();
    report(vec![view()])
        .write_json(&mut out)
        .expect("writing to a Vec");
    let json: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
    let view = &json["views"][0];
    assert_eq!(view["model"], "arch.kdl");
    assert_eq!(view["headline"]["divergences"], 1);
    assert_eq!(view["skipped"]["same_component"], 7);
    let divergence = &view["divergences"][0];
    assert_eq!(divergence["groups"]["matches"]["divergent"], 1);
    assert_eq!(divergence["samples"][0]["kind"], "calls");
    assert_eq!(divergence["samples"][0]["span"]["line"], 12);
    assert_eq!(divergence["samples"][1]["span"], serde_json::Value::Null);
    assert_eq!(
        view["absences"][0]["kinds"],
        serde_json::json!(["types", "calls"])
    );
    assert_eq!(view["absences"][0]["location"]["column"], 14);
    assert_eq!(view["unmapped"][0]["module"], "app::stray");
    assert_eq!(
        view["conflicts"][0]["claims"],
        serde_json::json!(["a", "b"])
    );
    assert_eq!(json["facts"]["units"], 2);
}

#[test]
fn every_skip_reason_is_listed_even_at_zero() {
    let report = report(vec![clean_view()]);
    assert!(text(&report).contains(
        "skipped edges: group_filtered 0, provenance_filtered 0, test_only 0, unmapped 0, \
         conflict 0, out_of_scope 0, not_targeted 0, same_component 0\n"
    ));

    let mut out = Vec::new();
    report.write_json(&mut out).expect("writing to a Vec");
    let json: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
    assert_eq!(
        json["views"][0]["skipped"],
        serde_json::json!({
            "group_filtered": 0,
            "provenance_filtered": 0,
            "test_only": 0,
            "unmapped": 0,
            "conflict": 0,
            "out_of_scope": 0,
            "not_targeted": 0,
            "same_component": 0,
        })
    );
    // In declaration order, as in the text.
    let out = String::from_utf8(out).expect("UTF-8");
    let (_, skipped) = out.split_once("\"skipped\": {").expect("a skipped map");
    let (skipped, _) = skipped.split_once('}').expect("a flat map");
    let positions: Vec<usize> = SkipReason::ALL
        .iter()
        .map(|reason| {
            let name = serde_json::to_string(reason).expect("a wire name");
            skipped
                .find(&format!("{name}:"))
                .expect("every reason is a key")
        })
        .collect();
    assert!(positions.is_sorted(), "{skipped}");
}

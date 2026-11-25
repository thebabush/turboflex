# turboflex

turboflex checks the dependencies a Rust workspace has against an architecture model you write.
It follows reflexion models (Murphy, Notkin and Sullivan, 1995). A rustc driver reads each
compiled crate's HIR and MIR and writes item-level dependency facts as JSONL, one file per
compile unit: type uses, calls, impls, macros, globals, constructions and matches. `turboflex
check` maps the facts onto the components declared in KDL model files and reports
convergences (allowed and used), divergences (used, not allowed), absences (allowed, not used),
unmapped entities and conflicts.

Status: experimental. The model language and the fact schema still change.

## Install

From a checkout. The driver links against rustc and builds with its own pinned nightly
(`nightly-2026-09-17`, with `rustc-dev`; see `crates/driver/rust-toolchain.toml`), so install
it from its directory.

```sh
cargo install --locked --path crates/cli
(cd crates/driver && cargo install --locked --path .)
```

## Usage

```sh
turboflex-driver WORKSPACE --out FACTS_DIR --all-targets
turboflex check --facts FACTS_DIR MODEL.kdl... [--format json] [--samples N]
```

The driver runs `cargo check` over the workspace with itself as the compiler wrapper. Arguments
it does not know go to `cargo check`. Successful runs publish a snapshot under `FACTS_DIR/.turboflex/`. The checker and scorer
select the snapshot named in `current` there. Older snapshots are retained and can be removed
when no readers are using them. Both readers also accept legacy directories of JSONL files.

`check` prints each model's findings. Exit status is 0 when no model has divergences or
conflicts, 1 when one has, 2 when the check could not run. `--format json` prints one JSON
object instead of text. `--samples N` sets how many divergent edges are shown per component
pair.

## Model

A layering of turboflex itself, with one rule left out on purpose:

```kdl
component "graph" {
    doc "Fact schema"
    modules "turboflex_graph" "turboflex_graph::**"
}
component "model" { modules "turboflex_model" "turboflex_model::**"; }
component "engine" { modules "turboflex_engine" "turboflex_engine::**"; }
component "front" {
    modules "turboflex_service" "turboflex_service::**" "turboflex_report" "turboflex_report::**"
    modules "turboflex" "turboflex::**"
}
external "io" { modules "std::fs" "std::fs::**" "std::process" "std::process::**"; }

allow "model" -> "graph"
allow "engine" -> "model"
allow "front" -> "graph" "model" "engine" "io"

policy {
    test-edges "exclude"
    provenance "source" "macro_expanded" "desugared"
}
```

Checked over turboflex's own facts with `--samples 2` (trimmed):

```
== layers.kdl
convergences 6, divergences 1, absences 0, unmapped 320, conflicts 0

divergences (group:allowed+divergent!):
  engine -> graph  68 edges, 68 divergent  [types:47! calls:18! constructs:1! matches:2!]
      turboflex_engine::dependency::Dependency::by_kind -> <turboflex_graph::kind::EdgeKind as core::cmp::Ord>::cmp (implicit_op, source) crates/engine/src/dependency.rs:151
      turboflex_engine::dependency::Dependency::by_kind -> turboflex_graph::kind::EdgeKind (uses_type_body, source) crates/engine/src/dependency.rs:151
      ... 66 more
```

Every dependency needs an `allow` rule; `kinds="types,calls"` narrows one to some edge groups.

## Views

Several model files can run over one fact base, each answering one question. `policy` narrows
what a view looks at: `edges` (edge groups), `provenance` (`source`, `macro_expanded`,
`desugared`, `derive`), `test-edges` and `targets` (only edges into the listed components).
Views share components and rules through `include "PATH.kdl"`. The full grammar is in the
module docs of `crates/model/src/lib.rs`.

## Layout

- `crates/graph`: fact schema shared by the driver and the analysis: entities, edges, kinds, provenance.
- `crates/driver`: the rustc driver. Its own workspace, on the pinned nightly.
- `crates/model`: the KDL model language and the mapping of entities to components.
- `crates/engine`: the reflexion engine.
- `crates/report`: text and JSON rendering.
- `crates/service`: loads facts and models and runs each view.
- `crates/cli`: the `turboflex` binary.
- `turboflex.kdl`, `turboflex-construction.kdl`: turboflex's own models. The tool checks itself.
- `fixtures/`: `tricky` (hard cases for extractors, with expected edges) and `taint` (sink and sanitizer views).
- `docs/decisions.md`, `docs/backlog.md`: design decisions with their reasons, and open work.
- `skills/turboflex-architecture`: instructions for coding agents that run turboflex.

## Development

```sh
scripts/check.sh
cargo test --workspace
```

`scripts/check.sh` is the gate: rustfmt, clippy with `-D warnings` for the workspace and the
driver, a check that the driver's lint policy matches the workspace's, and ast-grep rule tests
and scan.

## CI and releases

Pull requests and pushes to main run formatting, Clippy with warnings denied, ast-grep
rule tests and scans, and both Rust workspaces' tests. Run `scripts/check.sh` for the
lint gates and `scripts/self-check.sh` to build the CLI and driver from source and check
both workspaces against the two repository models.

Pushing a `v*` tag runs the same checks and publishes a GitHub Release with the Linux
x86-64 CLI and its SHA-256 checksum. Install the driver from the tagged source using
the instructions above. Its runtime depends on the pinned rustc toolchain.

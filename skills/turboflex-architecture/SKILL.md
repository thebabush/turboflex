---
name: turboflex-architecture
description: Check a Rust workspace's module and crate dependencies against its architecture model (turboflex, KDL files). Use when asked whether code respects layering or boundaries, which modules depend on or call which, who may call a sink or construct a sanitized type (capabilities, coarse taint), why an edge is a divergence, or to explain turboflex check output. Not for Cargo package versions (use cargo tree).
---

# turboflex

turboflex compares the dependencies a Rust workspace has (extracted facts) with the ones its
model allows. People own the model files. You run checks, verify findings and explain them.

## Run

Both binaries must be on `PATH`. If either is missing, see Install below.

1. Extract facts. They are a snapshot, so extract again after the code changes.

   ```sh
   turboflex-driver WORKSPACE --out FACTS_DIR --all-targets
   ```

   Keep `FACTS_DIR` outside the workspace. Extra arguments go to `cargo check`, and so does the
   environment it needs (for example, offline mode for compile-time-checked SQL). Expect about a
   minute cold for 100k lines. Each run deletes the fact files in `FACTS_DIR` first. For a
   repository with several workspaces, extract each to its own directory, then copy the
   `*.jsonl` files into one. A crate
   that is a member in one workspace and a dependency in another gets two different names, so
   edges across that boundary are out of scope.

2. Check one or more models. Each model is a view that answers one question.

   ```sh
   turboflex check --facts FACTS_DIR MODEL.kdl... [--format json] [--samples N]
   ```

   Exit 0: clean. Exit 1: findings, which is a result, not a failure. Exit 2: could not run.
   Prefer `--format json` when you will process the output.

## Read the results

- **Divergence**: code has an edge the model does not allow. Samples give source, target, edge
  kind, provenance and file:line. Open the line before calling it real.
- **Absence**: an allowed dependency no code uses. Often a stale rule or a component not built yet.
- **Unmapped**: entities no pattern claims. Their edges are not checked at all.
- **Skipped edges** `not_targeted`: edges outside the view's `targets`. A clean view with
  `targets` speaks only for edges into its targets.
- **Conflict**: helper-follow placed one entity in two components.
- **Provenance** `derive`, `desugared` or `macro_expanded` means the compiler wrote that edge.
  Say so before attributing it to someone's code.

## Rules

- Never edit a model to make a finding go away. Propose the rule change and its reason.
- Report per view, with the view's question, before merging conclusions across views.
- Views over one codebase share declarations through `include "PATH.kdl"`, not copies (copies
  drift). The path is relative to the including file; `policy` stays in the view's own file;
  ids are global across files. A view that asks one question includes the shared layout, adds
  its rules and sets `policy { targets "ID" }`: rules into unrelated components are then out
  of scope.
- Every dependency is an explicit `allow` rule. Never write `foundation`: it was removed.
- Before writing or proposing model text, read the grammar: the module docs of
  `crates/model/src/lib.rs` in the turboflex repository.

## Recipes: sinks and sanitizers

Coarse taint checks as plain views. They see references between items, not value flow; for
real dataflow taint use a dedicated tool (Paralegal, CodeQL).

- **Who may call a sink.** Make the sink APIs an `external` and allow one component to reach it.
  Set `policy { edges "calls" }`: naming a sink's type is not using it. Only direct calls are
  seen, so map third-party crates that call the sink internally into it by `crates`. A sink
  listed by `items` misses every API not listed.
- **Who may build a sanitized type.** Sinks take a newtype. Set `policy { edges "constructs" }`
  and keep `derive` in `provenance`: a derived `Default` or `Deserialize` is a real bypass.
  Put the type alone in a component by an `items` pattern, and its module in another component
  with no rule to the type, so derives and constructors written next to it diverge. Claim the
  vetted ones (a checked constructor, a derived `Clone`) into the sanitizer by item patterns,
  spelled as the driver prints them (e.g. `serde_core::Deserialize`, not `serde::`). Set
  `targets` to the type's component so only edges into it are checked.
- **Only one definition may touch X.** Put that definition alone in a component with an
  `items` pattern and allow only it to reach X. Set `targets "X"` so only edges into X are
  checked. Without it, also allow the rest to call the definition, and its use of helpers from
  its own module (or claim them with `follow=#true`).
- Map everything with a catch-all, `modules "CRATE" "CRATE::**"` per workspace crate.
- Not caught: mutating an existing value's field (`cmd.0.push_str(..)`), `transmute`, unsafe
  casts. Keep the field private: then only the type's own module can mutate it.

Worked views: `fixtures/taint/` in the turboflex repository.

## Install

From a turboflex checkout. The driver builds with its own pinned nightly, so install it from
its directory.

```sh
cargo install --locked --path crates/cli
(cd crates/driver && cargo install --locked --path .)
```

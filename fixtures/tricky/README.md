# tricky — dependency-extractor fixture

A small Cargo workspace full of Rust dependency patterns that are hard for
extractors to get right, plus a hand-written ground truth
(`expected_edges.tsv`) for scoring precision/recall of e.g. Charon (MIR →
LLBC) vs. rust-analyzer SCIP.

## Layout

```
Cargo.toml                 workspace (edition 2024, resolver 3)
expected_edges.tsv         ground truth
crates/core_api            cross-crate target: core_helper, Greeter, Storage, Version, Level
crates/macro_kit           cross-crate #[macro_export] macro exported_double! + double_it
crates/app                 the fixture proper
  build.rs                 writes $OUT_DIR/generated.rs (case c17)
  src/cases/cNN_*.rs       one module per case (the edge *sources*)
  src/*.rs, src/infra/*.rs target modules (the edge *destinations*)
```

Every case file starts with a `//!` comment saying what it tests, and every
target module says which case(s) use it.

## Build

```
cargo build --workspace
cargo build --workspace --all-features      # enables app/extra (c16)
cargo test  --workspace --no-run            # compiles #[cfg(test)] (c16)
cargo clippy --workspace --all-targets
```

No database or async runtime is needed: sqlx is used with
`default-features = false, features = ["sqlite", "macros"]` and only the
runtime API (`query_as::<_, T>`), never `query!`.

## Cases

| case | what it tests | targets |
|---|---|---|
| c01_direct_call | free-fn call, cross-module and cross-crate | `util`, `core_api` |
| c02_method_autoderef | inherent method; autoderef via Box, Arc, custom `Deref`; deref coercion at an argument | `shapes`, `wrappers` |
| c03_trait_dispatch | trait call via `T: Trait`, `dyn Trait`, concrete receiver, assoc type + where-clause, `impl Trait` arg/return | `core_api`, `animals` |
| c04_impl_location | impl of trait from B/other crate for type from A, written in C | `traits_b`, `model_a`, `impls_c` |
| c05_macros | crate-local `macro_rules!` (M) calling N, invoked in P; cross-crate `#[macro_export]` | `macros_m`, `sink_n`, `macro_kit` |
| c06_derive | Debug/Clone/PartialEq/Serialize/Deserialize derives reaching field types' impls | `kinds` (derived), `tags` (hand-written) |
| c07_try_from | `?` invoking `From<ErrA> for ErrB` defined in a third module | `errors_a`, `errors_b`, `error_conv` |
| c08_iter_ops | `for` over user `IntoIterator`/`Iterator`; `+` and `[]` via user `Add`/`Index` | `collection`, `iteration`, `vector`, `vector_ops` |
| c09_drop | user `Drop` run only at scope end | `resources` |
| c10_fmt | `format!`/`println!` using a `Display` impl in another module | `money`, `money_fmt` |
| c11_type_only | field/sig/body/generic-arg type uses, type alias, associated const | `types_t`, `aliases` |
| c12_reexport | type named through a `pub use` facade | `reexp_a`, `reexp_b` |
| c13_globals | const and static reads; write to an `AtomicU32` static | `config` |
| c14_closures_fnptr | call inside a capturing closure; fn item passed as fn pointer | `callbacks`, `math` |
| c15_async | async fn awaiting async fn; async block; native `async fn` in trait | `infra::net`, `service`, `service_impl` |
| c16_cfg | `#[cfg(test)]` call and `#[cfg(feature = "extra")]` call | `checks`, `extras` |
| c17_build_rs | build.rs-generated code `include!`d from `OUT_DIR` | `gen_target` |
| c18_serde_with | `serialize_with` / `with` attributes pointing at helper fns | `serde_helpers` |
| c19_sqlx | `derive(FromRow)` with manual `Type`/`Decode` and `try_from` columns; `query_as` | `infra::db_types`, `infra::db_codec` |
| c20_blanket_impl | method resolved via `impl<T: Greeter> Shout for T` | `blanket`, `animals` |
| c21_default_method | call resolving to a trait's default method body, which calls another module | `defaults`, `render`, `report_impl` |
| c22_split_impl | inherent impl blocks for one type in two modules | `split`, `split_ext` |
| c23_generic_inst | generic struct instantiated with another module's type | `containers`, `shapes` |
| c24_ufcs | `<T as Trait>::m(..)`, `Trait::m(&x)`, `Type::m(&x)` | `animals`, `core_api`, `shapes` |
| c25_constructs | struct literal, tuple-struct constructor, unit/tuple/struct-like variants, variant constructor as a fn value, cross-crate struct and variant | `payload`, `core_api` |
| c26_matches | `match`, `if let`, `let else`, `while let`, `matches!`, `match` on a field; cross-crate enum | `payload`, `core_api` |

## `expected_edges.tsv`

Columns: `case src_item src_module dst_item dst_module kind provenance notes`.

Conventions:

- Paths are fully qualified with the crate name. Impl blocks are written
  `impl Trait for Type` (with full paths) and their module is where the block is
  *written*. Trait-impl methods are `<Type as Trait>::method`; inherent
  methods are `Type::method` with `dst_module` = module of the impl block.
- One row per `(src_item, dst_item)` pair; `kind` is the most specific
  applicable kind and is informational. Score on the pair (or on the module
  pair after mapping).
- Only workspace → workspace edges, and only between distinct modules.
- Bodies without a name of their own (closures, async fn/block coroutines,
  local items generated by derives such as serde's `__Visitor` /
  `__SerializeWith`) are folded into the nearest enclosing named item.
- Not listed (scorer should ignore, neither TP nor FP): `use` declarations;
  edges from an item inside an impl to that impl's own Self type or trait
  (implied by the impl's `impl_for_type`/`impl_of_trait` rows); edges to
  std/core/alloc/serde/sqlx items.
- `notes` tags:
  - `[alt:G]` — rows sharing group `G` are alternative answers to the same
    question: matching any one counts as the recall hit, and none of them is a
    false positive.
  - `[optional]` — defensible edge; not required for recall, not a false
    positive if reported.
  - `[cfg:test]`, `[cfg:feature=extra]` — only present in that build
    configuration.

Deliberately **absent** (reporting it is a false positive): `app::callbacks::apply → app::math::triple`
(only via a runtime fn pointer).

## Debatable attributions

| case | question | how it is encoded |
|---|---|---|
| c03, c04, c15, c24 | statically-dispatchable trait call: impl method or trait declaration? | both, `[alt]` |
| c20 | blanket-impl method vs trait declaration; plus mono-only `Dog::greet` | `[alt]` + `[optional]` |
| c04 | which module "owns" an impl of B's trait for A's type written in C | impl rows have `src_module = app::impls_c` |
| c05 | macro edge P→M (invocation) vs expansion edge P→N; M→N textual edge | P→M and P→N both required, M→N `[optional]` |
| c06 | derived Serialize/Deserialize only pass field types as generic args to serde | generic_arg vs resolved impl method, `[alt]` |
| c06 | derived Debug reaches field impls via `&dyn Debug` coercion, not a call | listed as `implicit_fmt`, required |
| c07 | `?` calls std `from_residual`; the user `From::from` is one level down | listed on `run`, required |
| c10 | `format_args!` stores `<T as Display>::fmt` as a fn pointer | listed as `implicit_fmt`, required |
| c11 | alias vs aliased type (`aliases::Id` vs `types_t::Inner`) | `[alt]`; alias → Inner row required |
| c12 | re-export facade (`reexp_b::Foo`) vs definition (`reexp_a::Foo`) | definition required; facade + `pub use` item `[optional]` |
| c13 | const reads are inlined in MIR; atomic "write" is a `&` borrow + std call | listed as reads_global / writes_global, required |
| c14 | fn pointer reification vs call; closure body attribution | reification listed as `calls`; closure folded into enclosing fn |
| c19 | FromRow impl is generic over `R: Row`; manual `Decode`/`Type` impls only selected after monomorphization | `[optional]` |
| c22 | `Thing::extra` path is under the type, definition is in `split_ext` | `dst_module = app::split_ext` |
| c25 | a variant (or its constructor) built: edge to the variant or the enum? | the enum (variants are not entities) |
| c26 | derived `PartialEq` on `payload::Job` matches on `Job` itself | derive-provenance `matches_on` from `<Job as PartialEq>::eq`; own Self type, not listed |

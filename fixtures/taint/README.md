# taint — coarse taint checks as model views

A small Cargo workspace showing two "taint" patterns turboflex checks with
plain models, without any change to the model language, the engine or the
driver:

1. **Capability containment** (`capability.kdl`): sink APIs are external
   components, and only one workspace component may depend on each: `exec` on
   `std::process`, `storage` on `std::fs` writes.
2. **Type-enforced taint** (`sanitizer.kdl`): the process sink takes a
   sanitized newtype, `SafeCmd(String)`, and only the sanitizer may construct
   one. The view considers only the `constructs` edge group and keeps the
   `derive` provenance, because a derived `Default` or `Deserialize` is a real
   way to make a `SafeCmd` without the check.

`SafeCmd`'s field is `pub(crate)`, so the type system lets any module of the
crate build one: that is the weakening the sanitizer view polices, and what
lets every bypass case below compile.

## Layout

```
Cargo.toml              workspace (edition 2024, resolver 3); its own workspace,
                        excluded from the repository's (`fixtures` is in `exclude`)
capability.kdl          view 1
sanitizer.kdl           view 2
crates/app/src
  exec.rs               process sink: runs a SafeCmd (and case v03)
  storage.rs            fs-write sink
  sanitizer.rs          the policy (`is_safe`) and the free-fn constructor
  safe_cmd.rs           SafeCmd, its derives and inherent impl (cases b06-b08)
  cases/*.rs            one module per case; each starts with a `//!` comment
```

## Build and run

```
cd fixtures/taint && cargo build && cargo clippy --all-targets
# from the repository root
crates/driver/target/release/turboflex-driver fixtures/taint --out runs/taint-facts --all-targets
cargo run -q --release -p turboflex -- check --facts runs/taint-facts \
    fixtures/taint/capability.kdl fixtures/taint/sanitizer.kdl --samples 10
cargo test -p turboflex-service --test taint -- --ignored
```

The facts (`runs/taint-facts`) are gitignored; the ignored test
`crates/service/tests/taint.rs` pins the findings below.

## Cases

| case | where | what it tests | capability view | sanitizer view |
|---|---|---|---|---|
| l01 | `cases::l01_run_via_exec` | sanitize, then run through `exec` | not flagged | not flagged |
| l02 | `cases::l02_save_via_storage` | write files through `storage` | not flagged | – |
| l03 | `cases::l03_read_file` | `fs::read_to_string` outside `storage` (a read) | not flagged | – |
| l04 | `cases::l04_pass_safe_cmd` | `SafeCmd::parse`, `clone`, `as_str`, pass it on | – | not flagged |
| l05 | `exec::run` | `let SafeCmd(line) = cmd` (destructuring builds nothing) | – | not flagged |
| l06 | `safe_cmd` | `#[derive(Debug, Clone)]`, `SafeCmd::parse` next to the type | – | not flagged (claimed by the sanitizer) |
| v01 | `cases::v01_spawn_directly` | `Command::new` outside `exec` | `rest -> process` | – |
| v02 | `cases::v02_write_directly` | `fs::write` outside `storage` | `rest -> fs-write` | – |
| v03 | `exec::run_logged` | the process sink writes a log file | `exec -> fs-write` | – |
| b01 | `cases::b01_tuple_literal` | `SafeCmd(raw)` outside the sanitizer | – | `rest -> safe-cmd` |
| b02 | `cases::b02_ctor_as_fn` | `.map(SafeCmd)` | – | `rest -> safe-cmd` |
| b03 | `cases::b03_from_string` | `impl From<String> for SafeCmd` written outside the sanitizer | – | `rest -> safe-cmd` (the `from` impl, not `convert`, its caller) |
| b04 | `cases::b04_assoc_elsewhere` | `impl SafeCmd { fn from_trusted }` written in another module | – | `rest -> safe-cmd` (`from_trusted`, not `trusted_pair`, its caller) |
| b05 | `cases::b05_braced_literal` | `SafeCmd { 0: raw }` | – | `rest -> safe-cmd` (`braced`; `empty` only calls the derived `Default`) |
| b06 | `safe_cmd` | `#[derive(Default)]` | – | `safe-cmd-home -> safe-cmd` |
| b07 | `safe_cmd` | `#[derive(serde::Deserialize)]` | – | `safe-cmd-home -> safe-cmd` |
| b08 | `safe_cmd` | `SafeCmd::new_unchecked`, next to the type | – | `safe-cmd-home -> safe-cmd` |
| b09 | `cases::b09_field_mutation` | `cmd.0.push_str(arg)` on an existing SafeCmd | – | **not caught** (see Gaps) |

b06-b08 cannot have modules of their own: a derive always sits on the type,
and b08 is the "next to the type" variant of b04.

## Expected findings

Real output (`--samples 10`); exit status 1.

```
== fixtures/taint/capability.kdl
convergences 5, divergences 3, absences 0, unmapped 0, conflicts 0
skipped edges: group_filtered 200, out_of_scope 55, same_component 11

divergences (group:allowed+divergent!):
  exec -> fs-write  1 edges, 1 divergent  [calls:1!]
      app::exec::run_logged -> std::fs::write (calls, source) crates/app/src/exec.rs:26
  rest -> fs-write  1 edges, 1 divergent  [calls:1!]
      app::cases::v02_write_directly::dump -> std::fs::write (calls, source) crates/app/src/cases/v02_write_directly.rs:4
  rest -> process  3 edges, 3 divergent  [calls:3!]
      app::cases::v01_spawn_directly::list_dir -> std::process::Command::arg (calls, source) crates/app/src/cases/v01_spawn_directly.rs:6
      app::cases::v01_spawn_directly::list_dir -> std::process::Command::new (calls, source) crates/app/src/cases/v01_spawn_directly.rs:6
      app::cases::v01_spawn_directly::list_dir -> std::process::Command::status (calls, source) crates/app/src/cases/v01_spawn_directly.rs:6

== fixtures/taint/sanitizer.kdl
convergences 1, divergences 2, absences 0, unmapped 0, conflicts 0
skipped edges: group_filtered 272, out_of_scope 2

divergences (group:allowed+divergent!):
  rest -> safe-cmd  5 edges, 5 divergent  [constructs:5!]
      <app::safe_cmd::SafeCmd as core::convert::From<alloc::string::String>>::from -> app::safe_cmd::SafeCmd (constructs, source) crates/app/src/cases/b03_from_string.rs:8
      app::cases::b01_tuple_literal::run_raw -> app::safe_cmd::SafeCmd (constructs, source) crates/app/src/cases/b01_tuple_literal.rs:7
      app::cases::b02_ctor_as_fn::wrap_all -> app::safe_cmd::SafeCmd (constructs, source) crates/app/src/cases/b02_ctor_as_fn.rs:6
      app::cases::b05_braced_literal::braced -> app::safe_cmd::SafeCmd (constructs, source) crates/app/src/cases/b05_braced_literal.rs:8
      app::safe_cmd::SafeCmd::from_trusted -> app::safe_cmd::SafeCmd (constructs, source) crates/app/src/cases/b04_assoc_elsewhere.rs:8
  safe-cmd-home -> safe-cmd  3 edges, 3 divergent  [constructs:3!]
      <app::safe_cmd::SafeCmd as core::default::Default>::default -> app::safe_cmd::SafeCmd (constructs, derive) crates/app/src/safe_cmd.rs:11
      <app::safe_cmd::SafeCmd as serde_core::Deserialize>::deserialize -> app::safe_cmd::SafeCmd (constructs, derive) crates/app/src/safe_cmd.rs:12
      app::safe_cmd::SafeCmd::new_unchecked -> app::safe_cmd::SafeCmd (constructs, source) crates/app/src/safe_cmd.rs:23
```

Capability, per pair: `exec -> fs-write` is v03; `rest -> fs-write` is v02;
`rest -> process` is v01 (one edge per `Command` method). The convergences are
`exec -> process` (l01's sink), `storage -> fs-write` (l02's sink) and the
plumbing rules (`rest -> exec`, `rest -> storage`, `exec -> rest`).

Sanitizer, per pair: `rest -> safe-cmd` is b03 (`from`), b01, b02, b05, b04
(`from_trusted`: an inherent method's path is the type's, but its module is
where the impl block is written, `cases::b04_assoc_elsewhere`);
`safe-cmd-home -> safe-cmd` is b06, b07, b08. The one convergence,
`sanitizer -> safe-cmd`, is `sanitizer::sanitize`, `SafeCmd::parse` and the
derived `Clone::clone`.

Every bypass a caller merely *uses* (b03's `convert`, b04's `trusted_pair`,
b05's `empty`) is not flagged: the view flags the code that constructs, so each
bypass is reported once, where it is defined.

## Placement: where the type goes decides what is caught

Same-component edges are not checked, and an item pattern claims only the item,
not its impls or methods. Derives and an inherent impl written next to the type
live in the type's module. So the mapping of the type's module decides whether
b06-b08 are seen. Each alternative below was run over the same facts (the
mapping is what matters: putting the type's module in the sanitizer component
is the same as writing the type inside `sanitizer.rs`):

| placement | b01-b05 | b06 Default, b07 Deserialize, b08 new_unchecked | legit `parse`, derived `Clone` |
|---|---|---|---|
| type's module in the sanitizer component (type written in the sanitizer) | caught (`rest -> sanitizer`) | **missed**: same component | fine |
| type's module is the `safe-cmd` component (module pattern) | caught | **missed**: same component | fine |
| item pattern for the type, its module in the sanitizer | caught | **missed**: allowed as `sanitizer -> safe-cmd` | fine |
| item pattern for the type, its module in `rest` | caught | caught, mixed into `rest -> safe-cmd` (8 edges) | fine (claimed by item patterns) |
| **chosen**: item pattern for the type, its module its own component `safe-cmd-home`, no rule from it | caught (`rest -> safe-cmd`) | caught, as their own pair `safe-cmd-home -> safe-cmd` | fine (claimed by item patterns) |
| chosen, without the two item claims | caught | caught | **false positives**: `Clone::clone` and `parse` in `safe-cmd-home -> safe-cmd` |
| chosen, `provenance` without `derive` | caught | b08 only: b06, b07 dropped with every derive edge | fine (Clone dropped too) |

The chosen placement is default-deny for the type's home: any new derive or
associated fn there that builds a `SafeCmd` is a divergence until someone
claims it for the sanitizer by an item pattern. Item patterns name methods
exactly as the driver prints them, e.g.
`<app::safe_cmd::SafeCmd as core::clone::Clone>::clone` (note `serde_core::`
for `Deserialize`, not `serde::`). Rules cannot tell a derived `Clone` from a
derived `Default` (same pair, same group, same provenance); claiming the vetted
method by item pattern is what separates them.

## Gaps

- **Mutation (b09), not caught.** `cmd.0.push_str(arg)` changes a sanitized
  value without constructing one. The driver only emits `uses_type_*` edges and
  `String::push_str` calls for it; there is no field-level edge, so no view can
  see it. Closing it needs a driver and schema change: a field-write edge kind
  (say `writes_field`, to the ADT, in its own group) the sanitizer view could
  then include. Until then, keep the field private: only the type's own module
  can then mutate it, a small module to review by hand. The type system covers
  mutation elsewhere; the view covers construction.
- **Constructions with no `constructs` edge**: `mem::transmute`, pointer casts
  through a `#[repr(transparent)]` type, `mem::zeroed`. Not in the fixture
  (unsound or unsafe); not caught for the same reason as b09.
- **Capability edges are direct only**: a workspace call into a third-party
  crate that spawns or writes internally is an edge to that crate, not to
  `std::process`; map such crates into the sink externals by `crates`.
  `fs-write` lists writing functions by item; one it does not list (or an
  OS-specific extension trait outside `std::fs`/`std::process`, such as
  `std::os::unix::process::CommandExt`) is out of scope, not flagged.

# Decisions

Design decisions and the reasons for them, newest first. Each entry says what was decided, why,
and what would reopen it.

## 2026-11-23: Drop `foundation`

`foundation "ID"...` is removed. A model that still has it fails to parse, with an error naming
the replacement.

- Every dependency is an `allow` rule. A view about one or a few components uses
  `policy { targets ... }` instead of hiding unrelated pairs.

Why: `foundation` let every component depend on the listed ones, including components added
later, so a new component inherited permissions nobody wrote. It was also the only broad allow,
and without it a deny rule has nothing to carve from (see No deny rules).

Reopen if: a model needs so many identical rules into one component that they hide the rest.

## 2026-11-23: Policy targets

A view can check only the edges into some components: `policy { targets "ID"... }`.

- An edge is considered when its destination is owned by a listed component or one nested in
  it. Other edges are skipped and counted as `not_targeted`. The report's skipped line lists
  every reason, zeros included.
- An unmapped or out-of-scope destination is not filtered: it may belong to a target.
- A rule whose target is neither within a target nor an ancestor of one is out of the view's
  scope. It is never an absence and is not an error, so a view can `include` a shared layout
  and ask one question of it.
- It is a scope filter, like `edges`, not a permission. It allows nothing: a new component that
  builds a target without a rule still diverges.

Why:

- Coarse components (the one component plus a catch-all `rest`) need no feature and are fine
  for a standalone view. But such a view cannot include a shared layout: the layout's
  fine-grained components bring every unrelated pair back as a divergence. And a broad `rest`
  rule must be policed by review (never `rest -> target`).
- `foundation` hid the unrelated pairs by allowing them, so it also covered components added
  later. `targets` hides them by not looking.
- A deny rule would need a broad allow to carve from (see No deny rules).
- On turboflex itself, the `Model`/`Graph` construction view (`turboflex-construction.kdl`)
  dropped its `foundation` line for `targets "model-type" "graph-type"` and gave the same
  verdict: 2 rules, both converge, 249 edges `not_targeted`. Deleting the parser rule gave the
  expected divergence.

Reopen if: a view needs to filter by source as well, or a target list grows long enough that
coarse components read better.

## 2026-11-23: No deny rules

Rules stay an allow-list. A view that should check only some dependencies narrows its scope with
a `policy` filter instead (see Policy targets).

Why:

- turboflex follows reflexion models (Murphy, Notkin and Sullivan), which are closed-world:
  anything not allowed is already a divergence, so a deny rule adds nothing.
- Deny rules belong to open-world constraint languages (DCL, ArchUnit). Mixing both needs
  precedence rules, and an absence has no meaning for a deny rule.
- Deny is only needed to carve exceptions out of a broad allow. The only broad allow,
  `foundation`, has been dropped (see Drop `foundation`).

Reopen if: a real view needs an exception that a scope filter cannot express.

## 2026-11-23: Model includes

Views over one codebase share declarations through `include "RELATIVE/PATH.kdl"`, not copies.

- `include` is a top-level node. Its path is relative to the directory of the including file.
- An included file may hold `component`, `external`, `allow`, `link` and `include`, but not
  `policy` or `foundation`: those make the root file a view and stay in it. (`foundation` has
  since been removed.)
- The model is all its files together: ids, patterns and rule pairs are global, so a duplicate
  across files is the same error as within one file.
- An include cycle is an error naming the chain. A file reached twice is loaded once.
- Problems and report locations name the file they are in.

Known limitation: files are told apart by their lexically normalized path, not by what is on
disk. A file reached both directly and through a symlink is loaded twice, and its declarations
then clash as duplicates.

Multiple views can describe the same component. When each view maintains its own crate list,
the same dependency can be classified differently across views. Includes let those views
share one declaration.

Not a replacement for `foundation`: an include shares declarations across files, while
`foundation` shares one rule across components inside a file.

## 2026-11-23: Sink containment and sanitizer types, not value-flow taint

turboflex checks two coarse taint patterns as plain views, with no change to the language, the
engine or the driver: who may call a sink (sink APIs as externals, `calls` only), and who may
construct a sanitized newtype that sinks take (`constructs` only, `derive` kept). It does not
track how values flow. `fixtures/taint` shows both, with the cases each one catches and misses.

Why:

- The facts are references between items, not values. Following a value through assignments
  and calls is a different analysis.
- The type-enforced pattern is exact for construction: every literal, constructor-as-function,
  `From` impl, derived `Default` or `Deserialize` that builds the type is a divergence, reported
  where it is written. It runs in CI like any other view.
- Dataflow taint tools exist for Rust: Paralegal (Flowistry-based, markers on sources and sinks
  plus policies) and CodeQL's Rust taint tracking. Use them when that is the question.

Not caught:

- Mutating a field of an existing value (`cmd.0.push_str(..)`). There is no field-write edge.
- `mem::transmute`, pointer casts and `mem::zeroed`. They make a value without a `constructs`
  edge.
- Sinks reached indirectly. A third-party crate that spawns or writes internally is an edge to
  that crate; map it into the sink by `crates`. A sink listed by `items` misses unlisted APIs.

Reopen if: a target needs real dataflow and the external tools do not fit, or a field-write
edge kind in the driver becomes worth adding.

## 2026-10-28: No MCP server; agents use the CLI through a skill

Agents run the `turboflex` CLI, guided by an agent-neutral skill in
`skills/turboflex-architecture/SKILL.md`. The `mcp` component is gone from `turboflex.kdl`.

Why:

- Tool discovery is fragile across agents. Claude Code defers MCP tools behind bare names and
  finds them by lexical search, and every client shows, names and ranks tools differently. A
  skill's description is in context from the start in every agent that supports skills.
- The rules an agent needs ("never edit a model to hide a finding") are prose either way.
- CI and pre-commit use the CLI. A third front end would be one more thing to keep in step.
- A long-running server would hold facts that go stale as the code changes.

What MCP would buy, and the cheaper alternative:

- A resident fact graph. Each check reloads the fact files. If repeated interactive queries make that
  cost significant, add a query subcommand or a cached fact index first.
- Clients without a shell. None is a target today.

Skill install: one canonical copy in this repository, symlinked into each agent's skill
directory (`~/.claude/skills`, `~/.agents/skills`). Claude Code does not read `.agents/skills`
(checked on 2.1.312) but follows a symlink from `.claude/skills`. Codex, Copilot and Cursor read
`.agents/skills`.

Reopen if: a target client has no shell, or interactive queries become the main use.

## 2026-10-17: Views are separate model files

Several models run over one fact base (`turboflex check --facts DIR a.kdl b.kdl`), each a view
that answers one question. A model's `policy` can limit the edge groups and provenances it
considers. Nested components stay in the language, but new models use flat components and views
instead.

## 2026-10-17: KDL is the model language

Models are KDL v2 documents with strict parsing: unknown nodes, repeated properties and missing
arguments are errors with a location. The grammar is in the module docs of
`crates/model/src/lib.rs`.

## 2026-10-17: A custom rustc driver extracts facts

`crates/driver` is a rustc driver run through `RUSTC_WORKSPACE_WRAPPER`, in its own workspace
with a pinned nightly. It found every module-level edge in `fixtures/tricky`, including async,
macro use and desugaring, and records provenance.

Alternatives:

- **rust-analyzer SCIP** (`extractors/scip-edges`): has macro-use edges, but misses edges from
  macro expansion, `?` to `From`, `Drop`, `Display` and `for` loop iterators. Kept as a cheap
  cross-check.
- **Charon**: full desugaring, but no async or coroutines and no macro-use edges. Removed.

Reopen if: the pinned nightly becomes too costly to maintain.

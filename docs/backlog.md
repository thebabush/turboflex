# Backlog

Open work, grouped and ranked. Decisions behind these are in `decisions.md`.

## Model language

1. **Item patterns do not claim a type's methods and impls.** Narrowing a rule to one type
   (say `ExitCode`) needs this. See the `io` comment in `turboflex.kdl`.
2. **No catch-all crate pattern.** A catch-all needs one `modules "CRATE" "CRATE::**"` line per
   crate, and test crates show up as unmapped noise.

## Driver and facts

1. **A crate gets two names across workspaces.** As a member it is `turboflex_graph::graph::Graph`,
   as a dependency `turboflex_graph::Graph`. Cross-workspace edges fall out of scope and show as
   false absences (turboflex's own `driver -> graph`). About 2 to 3 hours.
2. **Field-write edges.** Mutating a sanitized value's field (`cmd.0.push_str(..)`) is invisible.
   A `writes_field` edge kind would close the gap. About half a day with a fixture case.

## CLI and report

1. **Repeatable `--facts`.** Each extraction empties its output directory and `check` reads one,
   so repositories with several workspaces need a manual copy. About 30 minutes.
2. **Convergences in the JSON report.** A clean view cannot show what it allowed. About 30
   minutes.

## Maybe

1. **Call-path rules**: "every call path from handlers to a sink passes through the sanitizer".
   Possible on today's call facts, expected to be noisy. A cheap experiment.

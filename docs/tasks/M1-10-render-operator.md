# M1-10 — `render` operator semantics

**Status:** todo
**Depends on:** M1-16, M1-06
**Crates:** `datascript-compiler` (spec + validation), `datascript-engine` (carries it)
**Diagnostic codes:** DS5001–DS5049 (D-018)

## Goal
Make `render` a validated part of the language that produces a `RenderSpec` frontends can draw.

## Scope
- `RenderSpec` type (serde): kind, x, y columns, series, title, axis titles, legend, y range, stacking.
- Binder validation of kinds and properties (did-you-mean), and of column suitability against the
  output schema (DS5002 explains what each chart needs and what the result has).
- Default column inference per LANGUAGE.md §6, resolved at compile time so the spec is always explicit.
- `render` line in golden `run` output (TESTING.md format).
- CLI behaviour: print the table and DS5001 info once.

Design-review additions: DS5003 unknown render kind and DS5004 unknown or repeated property, both
with did-you-mean. `render` not last is DS0120 in the parser (M1-03), not here.

## Acceptance criteria
- [ ] Golden cases for every chart kind (default inference + explicit props) and every render error.
- [ ] RenderSpec JSON schema documented (the web UI consumes it in M4).

## Carried over
- [ ] (from M1-16) `render` is the last operator the binder still binds as `hir::Op::Unsupported` with DS9001 (`Binder::bind_operator`). Give it a real HIR op that passes its input columns through.
- [ ] (from M1-06) `ir::QueryPlan::render` is an `Option<ir::RenderSpec>` and `ir::RenderSpec` is an empty placeholder struct (serde, `#[non_exhaustive]`). Fill it in, and set the field in `plan::plan_query` (always `None` today). Show it in `Program::explain` if useful.
- [ ] (from M1-07) `Session::run` already copies `QueryPlan::render` into `QueryResult::render`. Add a session test (`datascript-engine/src/tests.rs`) that a `render` query's result carries the spec once it is filled in.
- [ ] (from M1-11) `ds run examples/top_customers.ds` fails with DS9001 until `render` works. Remove the `#[ignore]` on `run_the_readme_example` in `crates/datascript-cli/tests/cli.rs` and make it check the table and the DS5001 info line. The CLI prints results with `output::write_results`; print DS5001 once there (or in `run_script` in `main.rs`) for a result whose `render` is set.

## Follow-ups

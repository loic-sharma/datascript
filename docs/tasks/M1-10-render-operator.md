# M1-10 — `render` operator semantics

**Status:** done — `render` binds to `hir::Op::Render` with every default resolved into `ir::RenderSpec` (DS5002–DS5008, DS1001), shown by golden tests and `explain`; `ds run` prints DS5001 once per run.
**Depends on:** M1-16, M1-06 (also touches code from M1-07 and M1-11, both done)
**Crates:** `datascript-compiler` (binder, spec, plan), `datascript-engine` (carries it), `datascript-cli` (DS5001), `datascript-tests` (golden `render:` line)
**Diagnostic codes:** DS5001–DS5049 (D-018). Registered: DS5001–DS5008. Also reuses DS1001.

## Goal
Make `render` a validated part of the language that produces an explicit `RenderSpec` frontends can
draw.

## Spec
The rules are settled; implement them, don't re-decide them (D-047):
- Semantics, properties, roles, type classes and defaults: [LANGUAGE.md §6](../LANGUAGE.md#6-render).
- RenderSpec JSON: [ARCHITECTURE.md, "Render layer"](../ARCHITECTURE.md#render-layer).
- Golden `render:` line and `explain` line: [TESTING.md](../TESTING.md), "Expected output rules".
- Codes DS5001–DS5008: [DIAGNOSTICS.md](../DIAGNOSTICS.md), "Code registry".

## Scope
- `hir::Op::Render` replacing the `Unsupported`/DS9001 path in `Binder::bind_operator`. Its output
  schema is its input schema.
- Binder validation, in this order so one root cause gives one error:
  1. `render` outside a query statement → DS5008 (and skip the rest).
  2. Kind → DS5003 with did-you-mean over the nine kinds.
  3. Each property: unknown or repeated → DS5004; not for this kind → DS5006; bad value → DS5005;
     unknown column → DS1001 (did-you-mean over output columns).
  4. Roles: one column in two roles → DS5007; explicit column of the wrong type or a required role
     that defaults cannot fill → DS5002.
  An error in an earlier property does not stop checking later ones (recovery), but a bad `kind`
  skips role checks.
- Default inference per §6.2, resolved in the binder/planner so the spec is always explicit.
- `ir::RenderSpec` filled in (serde, camelCase, absent-not-null, `#[non_exhaustive]`) and set by
  `plan::plan_query`.
- One shared formatter for the summary line (`render: timechart x=… y=[…] …`), used by the golden
  harness and `Program::explain`.
- CLI: DS5001 once per program run on stderr, for any `--format`, pointing at the first non-`table`
  render, with a note counting the others.

## Acceptance criteria
- [x] Golden `run` cases under `tests/cases/render/` for every kind: defaults only, and explicit
      properties (including `title`, `xtitle`/`ytitle`, `legend = hidden`, `ymin`/`ymax`,
      `kind = stacked`, quoted column names).
- [x] Golden cases for defaults that skip used columns (explicit `ycolumns` with default `series`,
      explicit `xcolumn` on a `scatterchart`).
- [x] One error case per code DS5002–DS5008 and for DS1001 in a property, plus a recovery case with
      two independent render errors.
- [x] KQL alignment cases: `kind = unstacked` resolves to `default`; `card` shows every typed
      column; bar/column/scatter default to all numeric y columns; a KQL-only kind
      (`anomalychart`), property (`ysplit`) and `kind = map` each get the "not supported yet"
      message, not a did-you-mean.
- [x] DS5002 cases for a missing role (timechart without datetime) and a wrongly typed explicit
      column; the message lists the result's columns with types.
- [x] A `plan` case shows the `render:` line in `explain` (re-bless `plan/readme_example.dstest`).
- [x] RenderSpec JSON is serialized as ARCHITECTURE.md documents (a unit test on serde output).
- [x] Every carried-over item below is done.

## Carried over
- [x] (from M1-16) `render` is the last operator the binder still binds as `hir::Op::Unsupported` with DS9001 (`Binder::bind_operator`). Give it a real HIR op that passes its input columns through.
- [x] (from M1-06) `ir::QueryPlan::render` is an `Option<ir::RenderSpec>` and `ir::RenderSpec` is an empty placeholder struct (serde, `#[non_exhaustive]`). Fill it in, and set the field in `plan::plan_query` (always `None` today). Show it in `Program::explain` (decided: same line as golden output, after the plan).
- [x] (from M1-07) `Session::run` already copies `QueryPlan::render` into `QueryResult::render`. Add a session test (`datascript-engine/src/tests.rs`) that a `render` query's result carries the spec once it is filled in.
- [x] (from M1-11) `ds run examples/top_customers.ds` fails with DS9001 until `render` works. Remove the `#[ignore]` on `run_the_readme_example` in `crates/datascript-cli/tests/cli.rs` and make it check the table and the DS5001 info line. The CLI prints results with `output::write_results`; print DS5001 once there (or in `run_script` in `main.rs`) for a result whose `render` is set. Add a CLI test that two charts give one DS5001 and that `render table` gives none.

## Follow-ups
Handed off:
- REPL use of `DS5001` → M1-12 "Carried over".

Notes (informational):
- The `render` operator's span travels beside the spec as `QueryPlan::render_span` and
  `QueryResult::render_span` (not in the JSON), so the CLI can point `DS5001` at it. ARCHITECTURE.md
  "Render layer" says so.
- The binder code is in `crates/datascript-compiler/src/binder/render.rs`; the spec types and the
  shared summary line (`RenderSpec::summary`) are in `crates/datascript-compiler/src/render.rs`,
  re-exported from `ir`. `datascript_syntax::display_name` prints column names as in the language.
- Golden cases: `tests/cases/render/` (run cases for every kind, check cases for each code,
  recovery, `explain_render`) and `tests/cases/plan/readme_example.dstest`.

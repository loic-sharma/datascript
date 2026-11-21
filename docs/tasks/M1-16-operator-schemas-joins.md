# M1-16 — Binder: operator schemas, column naming and joins

**Status:** done — every ● operator but `render` has its output schema in the HIR (`binder/ops.rs`, `binder/join.rs`), with §3.3 naming, `join` per D-019 and DS1005/DS1007; new codes DS1030–DS1032 and DS2030 (D-040); golden cases use the new `show: schema` harness header.
**Depends on:** M1-05
**Crates:** `datascript-compiler`
**Diagnostic codes:** DS1030–DS1049, DS2030–DS2039; also implements the registered DS1005 and
DS1007 (D-018)

## Goal
Compute the output schema of every ● operator, following the column naming and ordering rules,
on top of the binder core from M1-05. Split out of M1-05 so each task fits one session.

## Scope
- Per-operator schema transformation for all ● operators: `project*`, `extend` (replace in place),
  `summarize` (keys first, distinct keys with no aggregates), `sort`/`top`, `take`, `count`,
  `distinct`, `union` (by name, widening, DS2003), `getschema`.
- Column naming rules of LANGUAGE.md §3.3: unnamed aggregates and keys, `Column1`, …, DS1005.
- Aggregate context rules (DS2004, DS2005) where they depend on the operator.
- `join`: `$left`/`$right`, key type compatibility, `on key` producing one key column (D-019:
  left position, right value for `rightouter`, coalesced for `fullouter`), `$left.a == $right.b`
  keeping both, DS1007 for other collisions, semi/anti returning one side.
- `stage_schemas` filled for every operator.

## Out of scope
Source resolution and the capability pass (M1-17). `render` (M1-10).

## Acceptance criteria
- [x] Golden `check` cases for each operator (happy path + at least one error each), using the
      harness's `schema:` header with a mock provider until a real provider exists.
- [x] The README example's `join` yields one `customerId` column and no DS1007.
- [x] Golden cases for each join kind's key column, including `fullouter` with nulls on both sides.

## Carried over
- [x] (from M1-15) `project` takes expressions and renames, with DS1005 for duplicate names (the skeleton takes bare columns, and DataFusion then fails with DS4002).
- [x] (from M1-15) Un-skip `tests/cases/harness/check.dstest` (needs `summarize` and aggregates) and `harness/run.dstest` once M1-09 can run it.
- [x] (from M1-05) Replace `hir::Op::Unsupported` (and its DS9001) with real schemas. The binder already checks the expressions of `extend`, `summarize`, `sort`, `top` and `distinct` (`Binder::bind_operator`); reuse `Binder::aggregate_item` (DS2005) and `Agg::Row("`by`")` (DS2004) rather than re-checking. `project` still rejects named and computed items with DS9001 (`Binder::bind_project`).
- [x] (from M1-05) Un-skip `tests/cases/harness/check.dstest` and re-bless the cases that show DS9001 for `summarize` today: `errors/types/ds2004_aggregate_outside_summarize`, `errors/types/ds2005_not_aggregated`, `skeleton/not_implemented`.

## Follow-ups

- **Testing schemas.** `check` output showed only diagnostics, so the harness gained `show: schema`
  (TESTING.md): it prints each query's output columns. Cases are under `tests/cases/operators/`.
- **`fullouter` with nulls on both sides** is checked for its schema (`operators/join/kinds`,
  `key_widening`); the run case `operators/join/run_fullouter_nulls.dstest` is skipped until `join`
  runs, handed to M1-09. Planning the new operators is handed to M1-06, `render` to M1-10.
- `harness/check.dstest` is un-skipped; `harness/run.dstest` still needs M1-09 (handed over).
- The planner now runs `project` with names and expressions, `project-away`, `project-rename` and
  `extend` (`operators/project/run_project_family.dstest`); other new operators give DS9001
  "cannot run yet" at plan time instead of DS9001 from the binder.

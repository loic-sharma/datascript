# M1-06 — Logical plan IR and `explain`

**Status:** done — every operator the binder types now plans to the IR (`Values, Aggregate, Sort, TopN, Distinct, Join, Union` added), with serde, a tree printer and golden `explain` cases in `tests/cases/plan/`.
**Depends on:** M1-16, M1-17
**Crates:** `datascript-compiler`

## Goal
Lower the typed HIR into the engine-neutral relational IR (D-002) and pretty-print it. Extends the skeleton from M1-15 rather than starting from nothing.

## Scope
- IR nodes per ARCHITECTURE.md: `Scan, Filter, Project, Aggregate, Sort, Limit, TopN, Join,
  Union, Distinct, Values`, plus an expression IR. Every node and expression carries the `Span` it
  came from (for mapping runtime errors back).
- Desugaring: `extend` → `Project`, `count` → `Aggregate`, `top` → `TopN`, `project-away`/`rename`
  → `Project`, tabular `let` → inlined subplans (shared plan nodes are fine).
- `Program { queries: Vec<QueryPlan> }`, where `QueryPlan { root, output_schema, render: Option<RenderSpec> }`.
  The `RenderSpec` type itself is defined in M1-10. Leave a placeholder field.
- Deterministic tree printer for `ds explain` and `mode: explain` golden tests.
- `compile(src, provider, caps) -> Result<Program, Vec<Diagnostic>>` (ARCHITECTURE.md).
- `join ... on key` lowers to an equi-join plus a `Project` that keeps one key column (D-019);
  `fullouter` projects `coalesce(left.key, right.key)`.

## Acceptance criteria
- [x] Golden `explain` cases covering each operator and the README example (`tests/cases/plan/`; `where`/`take` in `harness/explain.dstest`, `let` inlining in `let/explain_inlined.dstest`). The README's final `render` is left out until M1-10.
- [x] Serde serialization of the IR (`Serialize` + `Deserialize`, round-trip test in `plan_tests.rs`).
- [x] Builds for wasm32.

## Carried over
- [x] (from M1-15) Move planning out of `binder.rs` into the IR builder. *(Done in M1-05: `plan.rs` lowers the HIR to the IR; extend it there.)*
- [x] (from M0-03) Confirm `harness/syntax.dstest` is blessed. *(`harness/explain.dstest` was un-skipped and blessed in M1-17; re-bless it if the plan format changes.)* The format of existing nodes did not change, and `harness/syntax.dstest` is blessed.
- [x] (from M1-16) Plan the operators the binder now types but `plan.rs` rejects with DS9001 "`<op>` cannot run yet" (`plan::cannot_run_yet`): `hir::Op::{Summarize, Sort, Top, Count, Distinct, Join, Union, GetSchema}`. Each stage's `columns` already holds its output schema. `extend` is planned as one `Project` per item (D-040) and the `project` family as `Project`; replace that with the real IR nodes if you prefer. Re-bless `tests/cases/skeleton/not_implemented.dstest` (it shows the DS9001 for `summarize`).
- [x] (from M1-16) `hir::JoinKey::merged` keys are one output column (D-019): the left value, the right one for `rightouter`, `coalesce(left, right)` for `fullouter`; `hir::Join::renamed` maps right columns to their output names (`name1`, …).
- [x] (from M1-17) Plan projection-only columns: `hir::Column::arrow_only` holds the Arrow type of a column with no DataScript type (its `ty` is `Type::Error`). `from 'x.parquet' | project tags` passes `check`, but planning it gives the internal DS9001 "an expression without a type"; the scan and `Project` should carry the Arrow type through.

## Follow-ups
- Decisions are in D-042 (summarize hidden columns, join renames, union alignment, `getschema`
  ordinals from 0, explicit null placement, one `JoinKind`).
- `ds explain` itself is wired by M1-11; `Program::explain` is the printer it uses.
- Handed off: lowering the new nodes and removing `cannot_run_yet` (M1-09), the `RenderSpec`
  placeholder (M1-10), decimal precision in `getschema` (M1-08).

# M1-06 — Logical plan IR and `explain`

**Status:** todo
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
- [ ] Golden `explain` cases covering each operator and the README example.
- [ ] Serde serialization of the IR (useful for debugging and future remote engines).
- [ ] Builds for wasm32.

## Carried over
- [x] (from M1-15) Move planning out of `binder.rs` into the IR builder. *(Done in M1-05: `plan.rs` lowers the HIR to the IR; extend it there.)*
- [ ] (from M0-03) Un-skip and bless `tests/cases/harness/explain.dstest` (needs host tables from M1-17) and confirm `harness/syntax.dstest` is blessed.
- [ ] (from M1-16) Plan the operators the binder now types but `plan.rs` rejects with DS9001 "`<op>` cannot run yet" (`plan::cannot_run_yet`): `hir::Op::{Summarize, Sort, Top, Count, Distinct, Join, Union, GetSchema}`. Each stage's `columns` already holds its output schema. `extend` is planned as one `Project` per item (D-040) and the `project` family as `Project`; replace that with the real IR nodes if you prefer. Re-bless `tests/cases/skeleton/not_implemented.dstest` (it shows the DS9001 for `summarize`).
- [ ] (from M1-16) `hir::JoinKey::merged` keys are one output column (D-019): the left value, the right one for `rightouter`, `coalesce(left, right)` for `fullouter`; `hir::Join::renamed` maps right columns to their output names (`name1`, …).

## Follow-ups

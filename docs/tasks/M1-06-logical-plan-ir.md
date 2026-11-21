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
- [ ] (from M1-15) Move planning out of `binder.rs` into the IR builder.
- [ ] (from M0-03) Un-skip and bless `tests/cases/harness/explain.dstest` (needs host tables from M1-17) and confirm `harness/syntax.dstest` is blessed.

## Follow-ups

# M1-06 — Logical plan IR and `explain`

**Status:** todo
**Depends on:** M1-05
**Crates:** `datascript-compiler`

## Goal
Lower the typed HIR into the engine-neutral relational IR (D-002) and pretty-print it.

## Scope
- IR nodes per ARCHITECTURE.md: `Scan, Filter, Project, Aggregate, Sort, Limit, TopN, Join,
  Union, Distinct, Values`, plus an expression IR. Every node and expression carries the `Span` it
  came from (for mapping runtime errors back).
- Desugaring: `extend` → `Project`, `count` → `Aggregate`, `top` → `TopN`, `project-away`/`rename`
  → `Project`, tabular `let` → inlined subplans (shared plan nodes are fine).
- `Program { queries: Vec<QueryPlan> }`, where `QueryPlan { root, output_schema, render: Option<RenderSpec> }`.
  The `RenderSpec` type itself is defined in M1-10. Leave a placeholder field.
- Deterministic tree printer for `ds explain` and `mode: explain` golden tests.
- `compile(src, provider) -> Result<Program, Diagnostics>`.

## Acceptance criteria
- [ ] Golden `explain` cases covering each operator and the README example.
- [ ] Serde serialization of the IR (useful for debugging and future remote engines).
- [ ] Builds for wasm32.

## Follow-ups

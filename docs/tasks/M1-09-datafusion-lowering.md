# M1-09 — DataFusion backend: lowering and execution

**Status:** todo
**Depends on:** M1-08
**Crates:** `datascript-datafusion`

## Goal
Translate the IR into DataFusion `LogicalPlan`s, execute them, and return Arrow results.

## Scope
- Lower every IR node with `LogicalPlanBuilder`. Lower expressions to DataFusion `Expr`.
- Function mapping table: every M1 catalog function maps to a DataFusion builtin or a custom UDF
  (e.g. KQL `bin`, `startofmonth`, `ago`, case-insensitive `contains`/`has`, `dcount`).
  `Capabilities` is derived from this table, so a missing mapping causes DS4001, not a panic.
- Semantics checks against KQL expectations where we follow KQL: `has` term matching, `bin` on
  datetimes, null handling in aggregates, integer division (document whatever we choose in LANGUAGE.md).
- Runtime errors mapped to DS4002 with the IR span when available. DataFusion's message goes in a note.
- The harness `Driver` wires `run` mode.

## Acceptance criteria
- [ ] Golden `run` cases for every ● operator and every M1 function in `tests/cases/operators/`
      and `tests/cases/functions/`.
- [ ] `now()`/`ago()` honor the injected clock (golden header `now:`).
- [ ] A division-by-zero or cast failure produces a readable DS4002 pointing at the expression.

## Follow-ups

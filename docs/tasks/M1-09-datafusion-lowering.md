# M1-09 — DataFusion backend: lowering and execution

**Status:** todo
**Depends on:** M1-08
**Crates:** `datascript-datafusion`

## Goal
Translate the IR into DataFusion `LogicalPlan`s, execute them, and return Arrow results. Extends the skeleton from M1-15 rather than starting from nothing.

## Scope
- Lower every IR node with `LogicalPlanBuilder`. Lower expressions to DataFusion `Expr`.
- Function mapping table: every M1 catalog function maps to a DataFusion builtin or a custom UDF
  (e.g. KQL `bin`, `startofmonth`, `ago`, case-insensitive `contains`/`has`, `dcount`).
  `Capabilities` is derived from this table, so a missing mapping causes DS4001, not a panic.
- Semantics checks against KQL expectations where we follow KQL: `has` term matching, `bin` on
  datetimes, null handling in aggregates, integer division (document whatever we choose in LANGUAGE.md).
- Runtime errors mapped to DS4002 with the IR span when available. DataFusion's message goes in a note.
- The harness `Driver` wires `run` mode.

Design-review additions:
- Semantics are now pinned in LANGUAGE.md and D-014: integer division truncates, integer division
  by zero and overflow are DS4003, regexes use Rust `regex`, `has` matches whole alphanumeric
  terms, `percentile` and `dcount` are exact, `startofweek` is Sunday, `round` is half away from
  zero. Write a golden case for each; record any deviation in LANGUAGE.md and DECISIONS.md.
- Null placement defaults (asc → nulls first, desc → nulls last). DataFusion's own defaults
  differ, so always lower an explicit null ordering.
- Checked integer arithmetic (D-014): DataFusion wraps on overflow by default, so use checked
  kernels or a UDF; use what M0-05 found ([checked arithmetic](../spikes/datafusion-wasm.md#checked-integer-arithmetic-for-m1-09-d-014)). Answer Q-008 (does KQL error here?) and record it.
- `timespan`: a golden case for every timespan row of LANGUAGE.md §4.1 and for `sum`/`avg`/`min`/
  `max` over durations. Cast DataFusion `Interval` results back to `Duration(µs)` (M0-05 findings).
  Per the [timespan probe](../spikes/datafusion-wasm.md#timespan-as-durationµs-for-m1-09), `-ts`,
  `ts * n`, `ts / n`, `ts / ts` and `bin(datetime, ts)` need UDFs or casts. Lower column references
  with `Column::new`, not `col()`, which lowercases.
- Result fields carry extension type metadata where needed (D-024).
- Anything this backend knowingly does not follow goes in `Capabilities::deviations`.
- Honour `DriverConfig` (`target_partitions = 1`) in the test driver.

## Acceptance criteria
- [ ] Golden `run` cases for every ● operator and every M1 function in `tests/cases/operators/`
      and `tests/cases/functions/`.
- [ ] `now()`/`ago()` honor the injected clock (golden header `now:`).
- [ ] A division-by-zero or cast failure produces a readable DS4002 pointing at the expression.

## Follow-ups

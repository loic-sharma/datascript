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

## Carried over
- [ ] (from M1-15) `lower_op` lowers every operator and expression faithfully (the skeleton maps only comparisons).
- [ ] (from M1-15) DataFusion runtime errors are traced to the node that caused them, not reported as DS4002 on the query's last node.
- [ ] (from review of M1-04) DataFusion built-ins that differ from the catalog; each needs a mapping or UDF and a golden case: `log` is base-10 in DataFusion (natural log is `ln`); `pow(Int64, Int64)` returns Int64 (catalog says `real`); `floor`/`ceil`/`round` on integers return Float64; `substr`/`strpos` are 1-based but `substring`/`indexof` are 0-based; `date_trunc('week')` starts Monday but `startofweek` is Sunday; `percentile` needs exact nearest-rank (a UDAF, not `percentile_cont`/`approx_percentile_cont`); `to*` conversions need `try_cast`; `trim(regex, s)` needs `regexp_replace`; `isempty` on non-strings needs custom logic; integer `/0` is a DataFusion error that must surface as DS4003, not DS4002.
- [ ] (from M1-05) The binder now accepts every operator of LANGUAGE.md §4.1 and catalog calls; `lower_expr` returns DS9001 "not implemented in the DataFusion engine yet" for `ExprKind::Call`, `in~`/`!in~`, `=~`/`!~`, the string operators and `matches regex`. Unary minus on `long` must raise DS4003 on overflow (D-014).
- [ ] (from M1-16) Un-skip `tests/cases/operators/join/run_fullouter_nulls.dstest` once `join` runs. Its expected table was written by hand from the fixtures (`fullouter` with nulls on both sides: `c-999` has no customer, `c-200` no orders); check the rows rather than blessing blindly.
- [ ] (from M1-16) Un-skip `tests/cases/harness/run.dstest` once it can run (needs `summarize`).
- [ ] (from M1-06) Lower the IR nodes M1-06 added: `Values`, `Aggregate`, `Sort`, `TopN`, `Distinct`, `Join` (one expression per side in each `JoinCondition`) and `Union`. Until then `Lowering::plan` reports them with `cannot_run_yet` (DS9001 "`summarize` and `count` cannot run yet", …); delete that function when every node lowers, and replace `tests/cases/skeleton/not_implemented.dstest` (it shows that DS9001) with a real `run` case. Plans are trees now: walk `Plan::inputs()`, not just `first_input()`.
- [ ] (from M1-06) A reference to a projection-only column (no DataScript type) has `Type::Error` in the IR; take its type from the input schema, which keeps the Arrow type (D-042).

## Follow-ups

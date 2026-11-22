# M1-09 — DataFusion backend: lowering and execution

**Status:** done
**Depends on:** M1-08
**Crates:** `datascript-datafusion`

**Summary:** Every IR node and every M1 catalog function lowers to DataFusion (own span-carrying
UDFs/UDAFs for scalar functions, checked arithmetic, `sum` and `percentile`); runtime errors point
at the failing expression (`DS4003`, new `DS4004`, `DS3008` for malformed rows); golden `run`
cases cover every ● operator and function.

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
- [x] Golden `run` cases for every ● operator and every M1 function in `tests/cases/operators/`
      and `tests/cases/functions/`.
- [x] `now()`/`ago()` honor the injected clock (golden header `now:`).
- [x] A division-by-zero or cast failure produces a readable DS4002 pointing at the expression. (Division by zero is `DS4003` per D-014 and casts cannot fail; see Follow-ups.)

## Carried over
- [x] (from M1-15) `lower_op` lowers every operator and expression faithfully (the skeleton maps only comparisons).
- [x] (from M1-15) DataFusion runtime errors are traced to the node that caused them, not reported as DS4002 on the query's last node.
- [x] (from review of M1-04) DataFusion built-ins that differ from the catalog; each needs a mapping or UDF and a golden case: `log` is base-10 in DataFusion (natural log is `ln`); `pow(Int64, Int64)` returns Int64 (catalog says `real`); `floor`/`ceil`/`round` on integers return Float64; `substr`/`strpos` are 1-based but `substring`/`indexof` are 0-based; `date_trunc('week')` starts Monday but `startofweek` is Sunday; `percentile` needs exact nearest-rank (a UDAF, not `percentile_cont`/`approx_percentile_cont`); `to*` conversions need `try_cast`; `trim(regex, s)` needs `regexp_replace`; `isempty` on non-strings needs custom logic; integer `/0` is a DataFusion error that must surface as DS4003, not DS4002.
- [x] (from M1-05) The binder now accepts every operator of LANGUAGE.md §4.1 and catalog calls; `lower_expr` returns DS9001 "not implemented in the DataFusion engine yet" for `ExprKind::Call`, `in~`/`!in~`, `=~`/`!~`, the string operators and `matches regex`. Unary minus on `long` must raise DS4003 on overflow (D-014).
- [x] (from M1-16) Un-skip `tests/cases/operators/join/run_fullouter_nulls.dstest` once `join` runs. Its expected table was written by hand from the fixtures (`fullouter` with nulls on both sides: `c-999` has no customer, `c-200` no orders); check the rows rather than blessing blindly.
- [x] (from M1-16) Un-skip `tests/cases/harness/run.dstest` once it can run (needs `summarize`).
- [x] (from M1-06) Lower the IR nodes M1-06 added: `Values`, `Aggregate`, `Sort`, `TopN`, `Distinct`, `Join` (one expression per side in each `JoinCondition`) and `Union`. Until then `Lowering::plan` reports them with `cannot_run_yet` (DS9001 "`summarize` and `count` cannot run yet", …); delete that function when every node lowers, and replace `tests/cases/skeleton/not_implemented.dstest` (it shows that DS9001) with a real `run` case. Plans are trees now: walk `Plan::inputs()`, not just `first_input()`.
- [x] (from M1-06) A reference to a projection-only column (no DataScript type) has `Type::Error` in the IR; take its type from the input schema, which keeps the Arrow type (D-042).
- [x] (from M1-08) Errors from reading rows (a malformed CSV row, an invalid JSON line, a value that does not parse as the inferred type) surface as DS4002 on the query. Report them as DS3008 naming the file, at the scan's span (LANGUAGE.md §3.1, DIAGNOSTICS.md).
- [x] (from M1-08) The scan is `Lowering::scan` over `sources::Resolved` (D-044): one table when every file has the same schema, else one per file under `UNION ALL`. Keep that shape when lowering the other nodes; `scan_column` does the casts, `nullif` for `null_value`, and the checked `UInt64` conversion.

## Follow-ups

- Acceptance "division by zero or cast failure gives DS4002": after the design review, integer
  division by zero is `DS4003` (D-014), shown in `errors/runtime/ds4003_division_by_zero.dstest`
  pointing at the division inside a `where` that is not the last operator. DataScript casts cannot
  fail: IR casts only widen and the `to*` conversions return null. `DS4002` remains for engine
  failures that have no span, at the query's last node.
- Function cases are under `tests/cases/functions/<group>/`, one file per function (or pair);
  semantics pinned by D-014 have their own cases: `expressions/run_integer_arithmetic`,
  `run_string_operators` (`has` terms, Rust regex), `run_timespan` (every §4.1 timespan row),
  `functions/aggregates/percentile` and `dcount` (exact), `functions/datetime/startof` (Sunday),
  `functions/math/round` (half away from zero), `run_nulls` and `aggregates/sum` (nulls in
  aggregates), `datetime/bin` (epoch bins).
- `sum`, `sumif` and `avg` gained a `timespan` signature in the catalog (as in KQL) so the
  duration aggregates the task asked for can be written; `min`/`max` already took timespans.
- The projection-only column carry-over needed no code: lowering never asks for the type of a
  column reference (`operators/project/run_arrow_only.dstest` passes one through `join`, `union`
  and `sort`).
- Runtime errors stop the run at the first one, so there is no runtime recovery case with two
  errors; checking still reports every independent error.
- Q-008: KQL's documentation is silent; the rule is kept and the remaining check is carried over
  to M1-13. Its long-form explanation, like every code's, is M2-08's (`docs/errors/`).
- Checking the wasm build of this crate needs `CC_wasm32_unknown_unknown` pointing at an LLVM
  clang locally (docs/spikes/datafusion-wasm.md); it passes with it.

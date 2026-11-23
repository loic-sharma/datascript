# M2-15 — Golden `run` coverage gaps

**Status:** done — added 25 `mode: run` golden cases across operators, sources, joins and runtime errors (23 passing, 2 skipped for newly filed bugs M2-17 and M2-18).
**Depends on:** M1-13
**Crates:** `datascript-tests` (golden cases under `tests/cases/` only)
**Diagnostic codes:** none new expected (tests only). If a test finds a bug whose fix needs a
code, write a new task for the fix instead of allocating one here (D-018).

## Goal
A review of the integration tests (November 2026) found features that `syntax` or `check` cases,
or unit tests, cover but that no `mode: run` golden case runs against DataFusion. Add run cases so
the engine's behaviour for each is pinned down.

## Scope
Add `mode: run` cases (happy path and the likely error) for:

### Operators
- [x] `!=` (every run case today uses `==` or `!~`).
- [x] `!between` (`tests/cases/expressions/run_in_between.dstest` has only `between` and `!in`).
- [x] The negated string operators `!contains_cs`, `!has_cs`, `!startswith`, `!startswith_cs`,
  `!endswith`, `!endswith_cs` (`run_string_operators.dstest` covers only `!has`, `!contains`,
  `!in~`). Include a null and a case-sensitivity row for each.

### Sources
- [x] `read_parquet(...)` called as a function (files ending `.parquet` already run through
  `from '…'`; the function form has only `check` cases).
- [x] `tests/data/edge/empty.csv` and `tests/data/edge/header_only.csv` (only `check` today, in
  `tests/cases/sources/edge_csv.dstest`).
- [x] An NDJSON glob, `tests/data/events/*.ndjson` (only `check` today, in
  `tests/cases/sources/glob_schema.dstest`; single `.ndjson` files already run).

### Joins
- [x] A multi-key join (`on a, b`, and `on $left.a == $right.x, $left.b == $right.y`).
- [x] A join whose key columns widen (`int` vs `long`), the run version of
  `tests/cases/operators/join/key_widening.dstest`.

### Runtime errors (`tests/cases/errors/runtime/`)
- [x] `DS4003` from date arithmetic in `crates/datascript-datafusion/src/calendar.rs`
  (`Fail::Overflow` in `startof*`/`endof*` and month/year offsets), from `i64::MIN / -1`, and from
  `sumif` overflowing a `long`.
- [x] Runtime `DS4004` for a computed bad argument to `trim`, `extract` and `datetime_diff`
  (today only `matches regex`, `datetime_part`, `format_datetime` and `percentile` have one).

Out of scope: `DS3008` for a malformed row in a query that reads several sources belongs to M2-12.

## Acceptance criteria
- [x] Every item above has at least one `mode: run` case, or a note under "Follow-ups" saying why
      it cannot (for example, the error cannot happen at run time).
- [x] Any wrong behaviour a new case exposes gets its own task file and index row; the case is
      added with the correct expected output and `skip:` naming that task.

## Follow-ups
- [M2-17](M2-17-sumif-overflow-message.md): `sumif` overflow reports `` `sum` `` instead of
  `` `sumif` `` (`tests/cases/errors/runtime/ds4003_sumif_overflow.dstest` added with `skip: M2-17`).
- [M2-18](M2-18-calendar-year-range-min.md): `calendar::year_range` panics on `i64::MIN` year
  (`tests/cases/errors/runtime/ds4003_startofyear_min_offset.dstest` added with `skip: M2-18`).

# M2-15 — Golden `run` coverage gaps

**Status:** todo
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
- [ ] `!=` (every run case today uses `==` or `!~`).
- [ ] `!between` (`tests/cases/expressions/run_in_between.dstest` has only `between` and `!in`).
- [ ] The negated string operators `!contains_cs`, `!has_cs`, `!startswith`, `!startswith_cs`,
  `!endswith`, `!endswith_cs` (`run_string_operators.dstest` covers only `!has`, `!contains`,
  `!in~`). Include a null and a case-sensitivity row for each.

### Sources
- [ ] `read_parquet(...)` called as a function (files ending `.parquet` already run through
  `from '…'`; the function form has only `check` cases).
- [ ] `tests/data/edge/empty.csv` and `tests/data/edge/header_only.csv` (only `check` today, in
  `tests/cases/sources/edge_csv.dstest`).
- [ ] An NDJSON glob, `tests/data/events/*.ndjson` (only `check` today, in
  `tests/cases/sources/glob_schema.dstest`; single `.ndjson` files already run).

### Joins
- [ ] A multi-key join (`on a, b`, and `on $left.a == $right.x, $left.b == $right.y`).
- [ ] A join whose key columns widen (`int` vs `long`), the run version of
  `tests/cases/operators/join/key_widening.dstest`.

### Runtime errors (`tests/cases/errors/runtime/`)
- [ ] `DS4003` from date arithmetic in `crates/datascript-datafusion/src/calendar.rs`
  (`Fail::Overflow` in `startof*`/`endof*` and month/year offsets), from `i64::MIN / -1`, and from
  `sumif` overflowing a `long`.
- [ ] Runtime `DS4004` for a computed bad argument to `trim`, `extract` and `datetime_diff`
  (today only `matches regex`, `datetime_part`, `format_datetime` and `percentile` have one).

Out of scope: `DS3008` for a malformed row in a query that reads several sources belongs to M2-12.

## Acceptance criteria
- [ ] Every item above has at least one `mode: run` case, or a note under "Follow-ups" saying why
      it cannot (for example, the error cannot happen at run time).
- [ ] Any wrong behaviour a new case exposes gets its own task file and index row; the case is
      added with the correct expected output and `skip:` naming that task.

## Follow-ups

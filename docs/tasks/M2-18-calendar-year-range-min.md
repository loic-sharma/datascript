# M2-18 — `calendar::year_range` panics on `i64::MIN` year

**Status:** done — replaced `year.abs() > 400_000` with `!(-400_000..=400_000).contains(&year)` in `calendar::year_range` (and used `checked_sub(1)` in `calendar::end_of`), added unit tests for `i64::MIN` year/offset, and unskipped `ds4003_startofyear_min_offset.dstest`.
**Depends on:** M2-15
**Crates:** `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** none new (`DS4003` is already used).

## Goal
`calendar::year_range` in `crates/datascript-datafusion/src/calendar.rs` checks
`year.abs() > 400_000`. When `startofyear` (or `endofyear`) adds an offset that lands on
`i64::MIN` (for example from year 0 with offset `-9223372036854775808`), `year.abs()` panics in
debug builds (`DS9001`: `attempt to negate with overflow`) and wraps to `i64::MIN` in release
builds, bypassing the range check. Return `Err(Fail::Overflow)` (`DS4003`) instead.

## Scope
- [x] Fix `year_range` in `crates/datascript-datafusion/src/calendar.rs` to avoid `i64::abs()`
  overflow (e.g. `!(-400_000..=400_000).contains(&year)` or `year.unsigned_abs() > 400_000`).
- [x] Add a unit test in `calendar.rs` covering `year == i64::MIN`.
- [x] Remove `skip:` from `tests/cases/errors/runtime/ds4003_startofyear_min_offset.dstest` and
  verify it passes.

## Acceptance criteria
- [x] `tests/cases/errors/runtime/ds4003_startofyear_min_offset.dstest` runs unskipped and passes.
- [x] No overflow panic in `calendar::year_range` in debug or release builds.

## Follow-ups

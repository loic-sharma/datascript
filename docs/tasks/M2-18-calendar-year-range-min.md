# M2-18 — `calendar::year_range` panics on `i64::MIN` year

**Status:** todo
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
- [ ] Fix `year_range` in `crates/datascript-datafusion/src/calendar.rs` to avoid `i64::abs()`
  overflow (e.g. `!(-400_000..=400_000).contains(&year)` or `year.unsigned_abs() > 400_000`).
- [ ] Add a unit test in `calendar.rs` covering `year == i64::MIN`.
- [ ] Remove `skip:` from `tests/cases/errors/runtime/ds4003_startofyear_min_offset.dstest` and
  verify it passes.

## Acceptance criteria
- [ ] `tests/cases/errors/runtime/ds4003_startofyear_min_offset.dstest` runs unskipped and passes.
- [ ] No overflow panic in `calendar::year_range` in debug or release builds.

## Follow-ups

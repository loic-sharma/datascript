# M2-17 — `sumif` overflow names `sum` instead of `sumif`

**Status:** done — `udaf::checked_sum` now takes the aggregate function name (`sum` / `sumif`) so `CheckedSum` uses distinct UDAF names (`ds_sum` / `ds_sumif`) and reports `` `sumif` `` in `DS4003` overflow diagnostics for both `long` and `timespan`; unskipped `ds4003_sumif_overflow.dstest` and added `ds4003_sumif_timespan_overflow.dstest` and a UDAF identity unit test.
**Depends on:** M2-15
**Crates:** `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** none new (`DS4003` is already used).

## Goal
When `sumif` overflows a `long` (or `timespan`), its `DS4003` diagnostic currently says
``the result of `sum` does not fit in a `long` `` while pointing at `sumif(...)`, because
`lower.rs` lowers both `sum` and `sumif` through `udaf::checked_sum` and `CheckedSum` hardcodes
``"`sum`"`` as `what`. Report `` `sumif` `` when the user called `sumif`.

## Scope
- [x] Pass the aggregate function name (`sum` / `sumif`) into `udaf::checked_sum`
  (`crates/datascript-datafusion/src/udaf.rs`, `crates/datascript-datafusion/src/lower.rs`), matching
  how scalar UDFs carry `what`.
- [x] Remove `skip:` from `tests/cases/errors/runtime/ds4003_sumif_overflow.dstest` and verify it
  passes.

## Acceptance criteria
- [x] `tests/cases/errors/runtime/ds4003_sumif_overflow.dstest` runs unskipped and passes.
- [x] `sum` overflow continues to report `` `sum` `` (`ds4003_sum_overflow.dstest`).

## Follow-ups

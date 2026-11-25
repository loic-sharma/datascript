# M2-21 — `DS4003` help: guard the divisor per row

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** none new; reuses `DS4003`.

## Goal
Integer division or modulo by zero, and `timespan` division by zero, stop the query with `DS4003`.
The help only says how to avoid the zero; it should also say how to keep the other rows by
guarding the divisor per row, e.g. `iff(b == 0, null, a / b)`.

This came out of a review of DataScript against KQL (November 2026); it was split out of M2-20.

## Scope
- [ ] First add a `mode: run` golden test showing that `iff(b == 0, null, a / b)` really avoids
  `DS4003` on rows where `b` is zero. DataFusion may evaluate both branches of `CASE`. If it does,
  do not change the help: write that down under Follow-ups and in DECISIONS.md (or Q-008).
- [ ] If the guard works, broaden the help in `Fail::DivisionByZero`
  (`crates/datascript-datafusion/src/errors.rs`) in both branches: integer `/` and `%`, and
  `timespan` `/`. Use phrasing that fits `%` as well as `/`.
- [ ] Update the 4 existing cases in `tests/cases/errors/runtime/`: `ds4003_division_by_zero`, `ds4003_modulo_by_zero`,
  `ds4003_timespan_division_by_zero`, `ds4003_timespan_division_by_zero_long`.

## Acceptance criteria
- [ ] The `mode: run` guard test passes, and the help is either updated or the reason it is not
  is recorded.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` pass.

## Follow-ups

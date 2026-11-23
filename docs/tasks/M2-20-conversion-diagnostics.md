# M2-20 — Conversions that can never succeed

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-compiler`, `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** DS2320–DS2329 (types) (D-018)

## Goal
Today every conversion function accepts `ANY` (`conversion()` in
`crates/datascript-compiler/src/builtins.rs`), so a call that can never give a value passes
`ds check` and returns null on every row. A KQL user writing `tolong(duration)` expects 100 ns ticks
(LANGUAGE.md §7, ≠ KQL) and gets a column of nulls with no warning. Catch these calls when the query
is checked, and point users at what works.

This came out of a review of DataScript against KQL (November 2026).

## Scope
- [ ] List every pair of input type and conversion function where the result is always null (for
  example `tolong`/`toint`/`toreal` on `timespan` or `datetime`, `todatetime` on a number,
  `totimespan` on a number or `bool`). Check each against `udf.rs` instead of guessing.
- [ ] Make these calls a check-time **error** with a new code from this task's block, not a lint:
  the result is known to be null before the query runs. Record the choice in DECISIONS.md (next
  free D-id), including why this is not the ticks behaviour KQL has.
- [ ] Help text says what to write instead: for `timespan` → number, divide by a unit
  (`duration / 1s`, `duration / 1ms`), which already works. For `datetime` → number, suggest
  `datetime_diff` or subtracting a reference `datetime(...)` and dividing.
- [ ] A bare `null` argument or a `dynamic` argument (M2-03) must not trigger the error.
- [ ] Broaden the `DS4003` division-by-zero help (`crates/datascript-datafusion/src/errors.rs`) to
  mention guarding per row with `iff(b == 0, null, a / b)`, so rows with a zero divisor can be
  kept. **Only after** a `mode: run` golden test shows the guard really stops `DS4003` (DataFusion
  may evaluate both branches of `CASE`). If it doesn't, write that down under Follow-ups instead.
- [ ] Update LANGUAGE.md §7 (conversion functions) to say these calls are errors.

## Acceptance criteria
- [ ] `mode: check` golden error cases for each always-null pair, with the fix in the help.
- [ ] `mode: run` happy-path cases showing the suggested rewrites give the expected values.
- [ ] Conversions that can succeed (`tolong("42")`, `toreal(5)`, `tostring(1d)`, …) are unchanged.
- [ ] LANGUAGE.md and DECISIONS.md updated in the same change.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and the wasm32 check pass.

## Follow-ups

# M2-20 — Conversions that can never succeed

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-diagnostics`, `datascript-compiler`, `datascript-datafusion`, `datascript-tests`
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
  example `tolong`/`toint`/`toreal`/`tobool` on `timespan` or `datetime`, `todatetime` on a number,
  `bool` or `timespan`, `totimespan` on a number, `bool` or `datetime`). Check each against `udf.rs`
  instead of guessing, keeping M1's typed-null-only types in mind: `decimal` converts to numbers,
  `bool` and `string` once M2-10 lands (and `operators.rs`/`catalog.rs` already suggest `toreal` for
  `decimal`), whereas `decimal` → `todatetime`/`totimespan` and `guid` → anything except `string`
  (and `guid` in M2-10) can never succeed.
- [ ] Make these calls a check-time **error** with a new code from this task's block, not a lint:
  the result is known to be null before the query runs. Register the code in
  `crates/datascript-diagnostics/src/codes.rs` and `docs/DIAGNOSTICS.md`, and record the choice in
  DECISIONS.md (next free D-id), including why this is not the ticks behaviour KQL has.
- [ ] Help text says what to write instead:
  - `timespan` → number: divide by a unit (`duration / 1s`, `duration / 1ms`), which already works.
  - `datetime` → number: suggest `datetime_diff` or subtracting a reference `datetime(...)` and
    dividing.
  - number → `totimespan`: multiply by a unit (`n * 1s`, `n * 1ms`) or write a literal with a unit
    (`60s`).
  - number → `todatetime`: add a scaled timespan to an epoch (`datetime(1970-01-01) + n * 1s`).
- [ ] A bare `null` argument, a `dynamic` argument (M2-03), or `Type::Error` (DIAGNOSTICS.md rule
  8) must not trigger the error.
- [ ] If M2-10 has already landed, apply the same rule to `todecimal` and `toguid`; otherwise leave
  the `(from M2-20)` handoff in `docs/tasks/M2-10-decimal-guid.md`.
- [ ] Broaden the `DS4003` division-by-zero help (`crates/datascript-datafusion/src/errors.rs`,
  both the integer `/` + `%` branch and the `timespan` branch) to mention guarding per row with
  `iff(b == 0, null, a / b)` (or phrasing that also fits `%`), so rows with a zero divisor can be
  kept, and update the 4 existing `ds4003_*_by_zero*.dstest` cases. **Only after** a `mode: run`
  golden test shows the guard really stops `DS4003` (DataFusion may evaluate both branches of
  `CASE`). If it doesn't, write that down under Follow-ups instead.
- [ ] Update existing golden tests that currently expect `null` for always-null pairs
  (`tests/cases/functions/conversion/{tolong,toreal,todatetime,totimespan}.dstest`), update
  LANGUAGE.md §7 (conversion functions) to say these calls are errors, and update the stale
  "`toreal(timestamp)` is null" remarks in `docs/DIAGNOSTICS.md` and `catalog.rs`.

## Acceptance criteria
- [ ] `mode: check` golden error cases for each always-null pair, with the fix in the help.
- [ ] `mode: run` happy-path cases showing the suggested rewrites give the expected values.
- [ ] Conversions that can succeed (`tolong("42")`, `toreal(5)`, `tostring(1d)`, …) are unchanged.
- [ ] LANGUAGE.md, DIAGNOSTICS.md, and DECISIONS.md updated in the same change.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and
  `cargo check -p datascript-diagnostics -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`
  pass.

## Follow-ups

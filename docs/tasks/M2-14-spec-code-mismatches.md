# M2-14 — Spec/code mismatches from the docs review

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-datafusion`, `datascript-compiler`, `datascript-syntax`
**Diagnostic codes:** none new expected (reuse the existing codes named below); reserve a block if needed (D-018)

## Goal
A review of `docs/` (November 2026) found places where LANGUAGE.md and the code disagree. For each,
decide whether the spec or the code is right, fix the other, and add a golden test. When the spec
changes, record the reason in DECISIONS.md.

## Scope

### Confirmed against the code
- [ ] **`timespan / real` rounds.** LANGUAGE.md §4.1 says a `timespan` divided by a number
  truncates toward zero. `k_div` in `crates/datascript-datafusion/src/udf.rs` truncates for integer
  divisors but sends real divisors through `real_to_int`, which calls `round()`, so `5us / 2.0` is
  `3us`. Likely fix: truncate in the real branch of `k_div`, keeping rounding for
  `timespan * real` as the spec says. Also state that `timespan / 0.0` is DS4003.
- [ ] **CSV one-character options and non-ASCII.** LANGUAGE.md §3.1 says `delimiter`, `quote` and
  `escape` are one character. The binder accepts any single Unicode character
  (`s.chars().count() == 1` in `crates/datascript-compiler/src/binder/sources.rs`), but the engine
  takes the first byte (`s.bytes().next()` in `crates/datascript-datafusion/src/sources.rs`), so
  `delimiter = '§'` reads with a broken UTF-8 byte. Require one ASCII character in the binder,
  using the existing "must be a single character" diagnostic with a label that says why, and
  say "one ASCII character" in the spec.
- [ ] **`matches regex` errors.** LANGUAGE.md §4 says a pattern that does not compile is DS4004
  "when the query runs". Since D-049 the binder reports constant patterns at check time
  (`crates/datascript-compiler/src/binder.rs`); only computed patterns fail at run time. Fix the
  spec sentence; §7 already says this.
- [ ] **Stale DS9001 filter.** `every_example_resolves` in
  `crates/datascript-compiler/src/catalog_tests.rs` drops DS9001 "is not implemented yet" for
  `extend`/`summarize` "until M1-16". M1-16 is done; remove the filter and check the test still passes.

### Check first, then fix spec or code
- [ ] Short-form `timespan` fractions: the spec says digits past nanosecond precision are ignored,
  but `crates/datascript-syntax/src/literal.rs` keeps at most 9 decimal digits whatever the unit
  (`0.0000000005d` is 43.2 µs but reads as 0).
- [ ] Glob format detection: the spec says all matches must share a format; the binder looks for a
  format extension in the pattern itself before expanding, so `'logs/*'` is DS3002 even when every
  match is a CSV file.
- [ ] Arrow `Null` columns: LANGUAGE.md §5.1 maps them to `string`, and the DataFusion engine does,
  but `datascript_compiler::arrow::from_arrow_source` still rejects `DataType::Null`.
- [ ] `"1" + 1`: LANGUAGE.md §4.2 suggests `tolong()`; the DS2003 help suggests `toreal()`. Make them agree.

## Acceptance criteria
- [ ] Each item is fixed in code or spec, or marked as already correct, with the reason.
- [ ] Golden tests cover `timespan / real`, a non-ASCII `delimiter`, and any other behaviour that changed.

## Follow-ups

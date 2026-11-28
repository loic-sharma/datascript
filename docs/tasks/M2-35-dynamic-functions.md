# M2-35 — `dynamic` functions

**Status:** todo
**Depends on:** M2-03
**Crates:** `datascript-compiler` (catalog), `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** DS2240–DS2244 (types), DS4070–DS4074 (runtime) (D-018)

## Goal
The functions that make and inspect `dynamic` values: `parse_json`, `todynamic`, `bag_keys`,
`array_length`, `split`, and the aggregates `make_list` and `make_set`.

Split from the original M2-03 (2026-11-28). [M2-06](M2-06-function-library.md) does not add
these; `make-series` ([M2-04](M2-04-make-series.md)) builds on `make_list`'s array output.

## Scope
Write each function's behaviour in LANGUAGE.md §7 before implementing it, and record anything that
differs from KQL in DECISIONS.md:
- **`parse_json(s)` / `todynamic(s)`.** Aliases (as in KQL) or one name plus a tailored error for
  the other (as `todouble` → `toreal`, D-014)? What invalid JSON returns: null, a runtime error,
  or the input as a `dynamic` string (KQL's behaviour)? What a `dynamic` argument returns
  (unchanged). Duplicate keys: last wins, as D-061 does at the top level.
- **`bag_keys(d)`.** Returns a `dynamic` array of strings. Key order: as written, or sorted? What
  a non-object returns (KQL: null).
- **`array_length(d)`.** Returns `long`. What a non-array returns (KQL: null).
- **`split(s, sep [, index])`.** Returns a `dynamic` array of strings. Empty separator, empty input,
  and whether the optional `index` argument is supported.
- **`make_list(x [, max])` / `make_set(x [, max])`.** Returns a `dynamic` array. Whether nulls are
  skipped (KQL skips them), the optional size cap and its default, and how `make_set` compares
  `dynamic` elements (the canonical equality from M2-03). **Element order:** DataFusion's
  partial aggregation does not keep input order across partitions, so either specify an order
  (input order within a single partition, sorted, or "unspecified") and make golden cases
  deterministic (sort the array, or use one partition in tests), or say in the spec that order is
  unspecified.

Each function gets a catalog entry with docs and an example, so hover, completion and signature
help pick it up.

## Acceptance criteria
- [ ] Every rule above is in LANGUAGE.md §7.
- [ ] Each function has a catalog entry and golden cases for its happy path, a null argument, and
      a wrong-kind argument (e.g. `array_length` of an object).
- [ ] Wrong argument types give a type error suggesting the right conversion (e.g. `split` of a
      `long` suggests `tostring`).
- [ ] `make_list` and `make_set` golden cases are deterministic across runs.
- [ ] If `parse_json` reports invalid input as an error, the error names the column and row the way
      other runtime errors do, with a golden case.

## Carried over
- [ ] (from M2-03 split) If M2-03 did not already do it, add `todynamic(x)` and `parse_json(x)` to the `DS2012` help (`needs_dynamic` in `crates/datascript-compiler/src/binder.rs`) and to its line in DIAGNOSTICS.md.

## Follow-ups

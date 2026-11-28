# M2-03 — `dynamic` type: representation, literals, access and sources

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`, `datascript-engine` (table and JSON output), `datascript-tests`
**Diagnostic codes:** DS0230–DS0236 (syntax), DS2230–DS2239 (types); also rewords the registered DS2012 (D-018)

## Goal
The `dynamic` type itself: its storage, `dynamic(...)` literals, member and index access
(`d.a.b`, `d[0]`, `d['key']`), equality, conversions to and from scalar types, and reading nested
columns from JSON and Parquet as `dynamic`.

Split from the original M2-03 (2026-11-28). Not this task's: the functions `parse_json`,
`todynamic`, `bag_keys`, `array_length`, `split`, `make_list` and `make_set` belong to
[M2-35](M2-35-dynamic-functions.md); the `mv-expand` operator belongs to
[M2-36](M2-36-mv-expand.md). Both depend on this task.

## Scope

### 1. Decide Q-003 first
Record the decision in DECISIONS.md (closing Q-003) and update LANGUAGE.md §5 before writing
code. Weigh at least:
- support for the Arrow Variant type in the arrow-rs and DataFusion versions the workspace uses;
- the wasm32 build (size, and no new non-wasm dependencies in core crates);
- the cost of member access and equality on each representation;
- the impact on M2-33's JSON reader, which writes `Utf8` + `arrow.json` (D-061). If Variant is
  picked, the reader's output type changes in this task, and D-024's extension name changes.

### 2. Settle the semantics in LANGUAGE.md
Each of these is open today. Decide it, write it in LANGUAGE.md, and record anything that differs
from KQL in DECISIONS.md:
- **Member and index access.** Result type (`dynamic`). What a missing key, `.a` on a non-object,
  `[i]` on a non-array, an out-of-range index and a negative index return (KQL: null; negative
  indexes count from the end). Which index types are allowed (`long` for arrays, `string` for
  objects) and the error for others.
- **Comparing `dynamic` with scalars.** §4.2 allows no implicit conversions, so
  `where props.page == "/docs"` is a type error unless this task says otherwise. KQL accepts it.
  Decide whether `==`/`!=` (and `<`, `>`, `in`, string operators) accept a `dynamic` operand next
  to a scalar, and if so how the value is compared. If not, the error must suggest the conversion
  (`tostring(props.page)`).
- **Equality between `dynamic` values.** D-061 requires comparing parsed values, not raw text
  (`"é"` vs `"é"`). Define the canonical form: is `1` equal to `1.0`; does object key order
  matter; how are nested duplicate keys resolved (last wins, as at the top level).
- **Where `dynamic` is allowed.** `sort by`, `summarize by`, `distinct`, `join on`, `union` type
  widening, `getschema` output. For each, allowed (with canonical comparison) or an error with help.
- **Conversions.** What `tostring`, `tolong`, `toreal`, `tobool`, `todatetime` and `totimespan`
  return for a `dynamic` value of each JSON kind (e.g. `tostring` of a JSON string is the string
  without quotes; of an object it is compact JSON text).
- **Literals.** The grammar of `dynamic(...)` bodies: arrays, objects, scalar literals, `null`,
  nested values, duplicate keys, trailing commas (D-059), and whether non-literal expressions are
  allowed inside (KQL: no).

### 3. Implement
- Parser: `dynamic(...)` bodies, member access, index access, `a['key']`.
- Binder: `Type::Dynamic` in expressions, the access and comparison rules above, DS2012 rewording.
- Arrow mapping: `from_arrow_source` maps `arrow.json` and Parquet Struct/List/LargeList/Map to
  `dynamic`. Parquet nested columns need a runtime conversion to the chosen representation (to
  JSON text if Q-003 keeps `arrow.json`).
- Runtime: access, equality and conversions in `datascript-datafusion`. JSON parsing must not add
  a filesystem or tokio dependency to `datascript-compiler` (ARCHITECTURE.md layering).
- Output: CLI table output shows `dynamic` values as compact JSON; JSON output nests them as JSON
  values, not as quoted strings. Result fields carry the canonical extension type (D-024).

## Test data
`tests/data/events/*.ndjson` has only a flat `props` object (`page`, `ref`), with no arrays and
no nesting. Either add nested and array fields to it (and re-bless every golden case whose output
changes, such as `getschema` and `take` over events), or add a new fixture under
`tests/data/json/` and document it in `tests/data/README.md`. No Parquet fixture has nested
columns (`types.parquet` has none), so add one with Struct, List and Map columns, including nulls
and an empty list, built the same reproducible way as the existing fixtures. Add a
second file with the same nested schema, so the glob (`Merged`) path can be tested too.

## Acceptance criteria
- [ ] Q-003 is closed in DECISIONS.md and every rule in "Settle the semantics" is in LANGUAGE.md.
- [ ] The `dynamic` representation lives behind one internal module in `datascript-datafusion`
      (parse, path access, canonical compare, conversion to and from Arrow, display as JSON text).
      Functions, operators (M2-35, M2-36) and source conversion go through it and never touch the
      storage format directly, so a later switch (e.g. JSON text to Variant) changes that module
      and the readers only. The decision record for Q-003 says so.
- [ ] Golden cases over JSON data: member access, nested paths, indexing (including missing key,
      out-of-range and negative index), `dynamic(...)` literals, equality between two `dynamic`
      values that differ only in escapes or key order, and `getschema` showing `dynamic`.
- [ ] Golden cases over Parquet Struct, List and Map columns, and over a Parquet glob whose
      `arrow.json` column stays `dynamic` (not `string`).
- [ ] Type errors when using `dynamic` values in typed contexts suggest the right conversion.
- [ ] Syntax errors for malformed `dynamic(...)` bodies (unclosed, non-literal element, bad key)
      and type errors for indexing with the wrong type, each with a golden case.
- [ ] CLI table and JSON output for `dynamic` columns are covered by tests.

## Carried over
- [ ] (from planning, 2026-11-22) If M3-02 or M4-09 is done, show `dynamic` values in hover and in the web results grid (as JSON text, collapsed when long), with tests. (M3-02 is done, so hover is in scope. M4-09 owns the grid; its "Carried over" item covers it if M4-09 lands after this task.)
- [ ] (from M1-01) Inside `dynamic(...)`, read a `QUOTED_IDENT` token such as `["a"]` as a one-element array (D-028).
- [ ] (from M1-02) `a['key']` lexes the index as a `QUOTED_IDENT`, so it does not parse as indexing yet; `dynamic([...])` bodies are not parsed. (The `mv-expand` part of this item moved to M2-36.)
- [ ] (from M1-23) `arrow::from_arrow_source` returns `UnsupportedArrowType` (DS2008) for Struct, List, LargeList, Map and Utf8 with `arrow.json` in M1 (D-039). Map them to `dynamic` again, update LANGUAGE.md §5.1, and flip the `nested_is_unsupported_until_m2`, `json_text_is_unsupported_until_m2` and `canonical_round_trip` tests in `arrow.rs`.
- [ ] (from M2-13) Reword `DS2012` (`needs_dynamic` in `crates/datascript-compiler/src/binder.rs`): its note says "`dynamic` values are not available yet; they are planned for M2". Once `dynamic` exists, say how to get one (`todynamic(x)`, `parse_json(x)`) instead, and update the registry line in DIAGNOSTICS.md. (Those functions land in M2-35; mention them in the help anyway, or have M2-35 add them to the help when it lands.)
- [ ] (from M2-19) Accept optional trailing commas in `dynamic([...])` arrays and `dynamic({...})` objects (including `}` as a list terminator) by parsing them with `list()` (or matching terminator logic). (The `mv-expand` part of this item moved to M2-36.)
- [ ] (from M2-33) The JSON reader now produces `Utf8` + `arrow.json` for nested and mixed columns, never `Struct`/`List` (Parquet still produces `Struct`/`List`, so mapping those stays in scope); remove the `Type::Dynamic` guard in `crates/datascript-compiler/src/binder/sources.rs` and `ide/mod.rs` when `from_arrow_source` maps `arrow.json`; nested values are stored as written minus whitespace, so `dynamic` equality, grouping and duplicate-key handling must work on parsed values (see D-061); JSON sources skip the glob merge, but Parquet columns carrying `arrow.json` still go through `Merged`, whose `Merged::Arrow` drops field metadata so they read as `string`: make `Merged` keep `arrow.json` (`dynamic`) when `from_arrow_source` maps it.

## Follow-ups

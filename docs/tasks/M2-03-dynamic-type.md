# M2-03 — `dynamic` type, JSON, and `mv-expand`

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`
**Diagnostic codes:** DS0230–DS0239 (syntax), DS2230–DS2249 (types); also rewords the registered DS2012 (D-018)

## Goal
Semi-structured data: the `dynamic` type, member/index access (`d.a.b`, `d[0]`), `parse_json`,
`todynamic`, `bag_keys`, `array_length`, `split`, `make_list`, `make_set`, and `mv-expand`.
This task owns `split`, `make_list` and `make_set` (M2-06 does not add them).

## Scope
Decide Q-003 (representation) first and record it in DECISIONS.md. Nested Parquet/JSON columns
should become usable through this type. Result fields carry the matching canonical extension type
(`arrow.json`, or the Variant extension type if Q-003 picks it, D-024).

## Acceptance criteria
- [ ] Golden cases over `tests/data/events/*.ndjson`.
- [ ] Type errors when using dynamic values in typed contexts suggest the right conversion.

## Carried over
- [ ] (from planning, 2026-11-22) If M3-02 or M4-02 is done, show `dynamic` values in hover and in the web results grid (as JSON text, collapsed when long), with tests.
- [ ] (from M1-01) Inside `dynamic(...)`, read a `QUOTED_IDENT` token such as `["a"]` as a one-element array (D-028).
- [ ] (from M1-02) `a['key']` lexes the index as a `QUOTED_IDENT`, so it does not parse as indexing yet; `dynamic([...])` bodies are not parsed; `mv-expand` parses as a planned unknown operator.
- [ ] (from M1-23) `arrow::from_arrow_source` returns `UnsupportedArrowType` (DS2008) for Struct, List, LargeList, Map and Utf8 with `arrow.json` in M1 (D-039). Map them to `dynamic` again, update LANGUAGE.md §5.1, and flip the `nested_is_unsupported_until_m2`, `json_text_is_unsupported_until_m2` and `canonical_round_trip` tests in `arrow.rs`.
- [ ] (from M2-13) Reword `DS2012` (`needs_dynamic` in `crates/datascript-compiler/src/binder.rs`): its note says "`dynamic` values are not available yet; they are planned for M2". Once `dynamic` exists, say how to get one (`todynamic(x)`, `parse_json(x)`) instead, and update the registry line in DIAGNOSTICS.md.

## Follow-ups

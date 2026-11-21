# M2-03 — `dynamic` type, JSON, and `mv-expand`

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-compiler`, `datascript-datafusion`

## Goal
Semi-structured data: the `dynamic` type, member/index access (`d.a.b`, `d[0]`), `parse_json`,
`todynamic`, `bag_keys`, `array_length`, `split`, `make_list`, `make_set`, and `mv-expand`.

## Scope
Decide Q-003 (representation) first and record it in DECISIONS.md. Nested Parquet/JSON columns
should become usable through this type. Result fields carry the matching canonical extension type
(`arrow.json`, or the Variant extension type if Q-003 picks it, D-024).

## Acceptance criteria
- [ ] Golden cases over `tests/data/events/*.ndjson`.
- [ ] Type errors when using dynamic values in typed contexts suggest the right conversion.

## Follow-ups

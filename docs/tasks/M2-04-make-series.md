# M2-04 — `make-series`

**Status:** todo
**Depends on:** M2-03, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`
**Diagnostic codes:** DS0240–DS0249 (syntax), DS2250–DS2259 (types), DS5050–DS5054 (render) (D-018)

## Goal
`make-series agg on ts [from .. to ..] step span [by keys]` with gap filling (`default=`).

## Acceptance criteria
- [ ] Output shape matches KQL (arrays per series, as dynamic). Render `timechart` understands it.
- [ ] Golden cases including empty ranges and missing buckets.

## Carried over
- [ ] (from M1-02) `make-series` is in `LATER_OPERATORS` (`crates/datascript-syntax/src/parser/grammar.rs`), so it parses as an unknown-operator error with a "planned" note. Replace that with real parsing and remove it from the list.
- [ ] (from M2-13) M4 runs before M2, so `web/` charts (M4-03) know only scalar columns. LANGUAGE.md §6.2 fills chart roles from typed scalar columns; make-series output is a `dynamic` array per series. Update §6.2 for it, and, if M4-03 is done, teach the web charts (and the VS Code panel, M3-06, which reuses them) to draw it, with a screenshot test.

## Follow-ups

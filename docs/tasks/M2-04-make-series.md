# M2-04 — `make-series`

**Status:** todo
**Depends on:** M2-03
**Crates:** `datascript-compiler`, `datascript-datafusion`

## Goal
`make-series agg on ts [from .. to ..] step span [by keys]` with gap filling (`default=`).

## Acceptance criteria
- [ ] Output shape matches KQL (arrays per series, as dynamic). Render `timechart` understands it.
- [ ] Golden cases including empty ranges and missing buckets.

## Follow-ups

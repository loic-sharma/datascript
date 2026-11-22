# M2-09 — Reader schema overrides

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-datafusion`

## Goal
`read_csv('x.csv', schema = (price: real, ts: datetime))` for partial or full schema
declaration, overriding inference. The DS2003 help in DIAGNOSTICS.md already suggests it.

## Acceptance criteria
- [ ] Partial schemas merge with inferred columns. Unknown column names in the schema give did-you-mean.
- [ ] Parse failures at runtime point at the schema entry.

## Follow-ups

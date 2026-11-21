# M2-05 — User-defined functions

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-syntax`, `datascript-compiler`

## Goal
`let f = (x: long, y: real = 1.0) { x * y };` for scalar functions, and tabular functions
`let recent = (T: (timestamp: datetime)) { T | where timestamp > ago(1d) };`.

## Acceptance criteria
- [ ] Inlined during planning (no engine support needed).
- [ ] Errors point both at the call site and at the definition (secondary label).
- [ ] Recursion is rejected with a clear message.

## Follow-ups

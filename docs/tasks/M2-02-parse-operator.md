# M2-02 — `parse` operator

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`

## Goal
KQL-style `parse <expr> with "lit" col:type "lit" ...` (simple and `kind=regex` modes).

## Acceptance criteria
- [ ] Lowered to regex extraction in the backend.
- [ ] Errors for patterns that can never match (two adjacent columns with no separator) explain why.
- [ ] Golden cases including type conversion failures (null + warning, as in KQL).

## Follow-ups

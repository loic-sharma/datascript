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
- [ ] Sources inside function bodies follow the same constant rule as elsewhere (D-020, D-022): a
      path built from a parameter is DS3006 unless the argument is constant at every call site
      after inlining. Source resolution stays in the binder; no pre-pass.

## Carried over
- [ ] (from planning, 2026-11-22) If M3-02 or M3-03 is done, extend the language server to UDFs: completion and signature help for UDF calls, and go to definition, find references and rename for UDFs and their parameters, each with an LSP test.

## Follow-ups

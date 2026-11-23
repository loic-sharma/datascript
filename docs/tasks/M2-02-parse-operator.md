# M2-02 — `parse` operator

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`
**Diagnostic codes:** DS0220–DS0229 (syntax), DS2220–DS2229 (types), DS4050–DS4054 (runtime) (D-018)

## Goal
KQL-style `parse <expr> with "lit" col:type "lit" ...` (simple and `kind=regex` modes).

## Acceptance criteria
- [ ] Lowered to regex extraction in the backend.
- [ ] Errors for patterns that can never match (two adjacent columns with no separator) explain why.
- [ ] Golden cases including type conversion failures: the column is `null`, with no warning, as
      the `to*` functions do. A runtime warning is not possible today: an error in a `BatchStream`
      ends the query (D-055), and there is no channel for per-row warnings during a run. If
      warnings are wanted later, that is an engine design task of its own (M2-13).

## Carried over
- [ ] (from M1-02) `parse` is in `LATER_OPERATORS` (`crates/datascript-syntax/src/parser/grammar.rs`), so it parses as an unknown-operator error with a "planned" note. Replace that with real parsing and remove it from the list.

## Follow-ups

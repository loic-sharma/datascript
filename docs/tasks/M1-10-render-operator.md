# M1-10 — `render` operator semantics

**Status:** todo
**Depends on:** M1-05, M1-06
**Crates:** `datascript-compiler` (spec + validation), `datascript-engine` (carries it)

## Goal
Make `render` a validated part of the language that produces a `RenderSpec` frontends can draw.

## Scope
- `RenderSpec` type (serde): kind, x, y columns, series, title, axis titles, legend, y range, stacking.
- Binder validation of kinds and properties (did-you-mean), and of column suitability against the
  output schema (DS5002 explains what each chart needs and what the result has).
- Default column inference per LANGUAGE.md §6, resolved at compile time so the spec is always explicit.
- `render` line in golden `run` output (TESTING.md format).
- CLI behaviour: print the table and DS5001 info once.

## Acceptance criteria
- [ ] Golden cases for every chart kind (default inference + explicit props) and every render error.
- [ ] RenderSpec JSON schema documented (the web UI consumes it in M4).

## Follow-ups

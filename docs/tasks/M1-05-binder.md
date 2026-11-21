# M1-05 — Binder core and expression checking

**Status:** todo
**Depends on:** M1-15
**Crates:** `datascript-compiler`
**Diagnostic codes:** DS1001–DS1029, DS2001–DS2029 (M1-04 owns DS2002/2003/2006/2009) (D-018)

## Goal
Grow the skeleton binder from M1-15 into the core of semantic analysis: scopes, name resolution,
expression type-checking, error recovery and did-you-mean, producing a typed HIR and semantic
diagnostics. Operator schemas and joins are M1-16; sources and capabilities are M1-17.

## Scope
- Scopes: program-level `let` bindings, scalar and tabular. A `let`'s kind is decided from its
  right side alone (D-020). No shadowing (DS1003).
- Expression checking against the catalog (M1-04), including the operator typing table of
  LANGUAGE.md §4.1 (integer arithmetic yields `long`, D-021), `in`, `between`, typed nulls and
  DS2009.
- Aggregate vs scalar context (DS2004, DS2005) as a reusable check that M1-16 calls per operator.
- Error recovery: unknown names get the error type, so there are no cascades (DIAGNOSTICS rule 8).
- Did-you-mean over columns, lets and functions in scope, plus the "columns available at this
  point" note.
- `check(src, provider, caps) -> Analysis { hir, diagnostics, stage_schemas, pending_sources }`
  (ARCHITECTURE.md). Keep `stage_schemas` (schema after each operator, keyed by text range) for LSP
  completion later.

## Acceptance criteria
- [ ] Golden `check` cases for names and expressions (happy path + errors), using the harness's
      `schema:` header and mock provider until a real provider exists (TESTING.md).
- [ ] The DS1001 example in DIAGNOSTICS.md renders exactly as documented.
- [ ] Multi-error case: unknown column in one statement and a type error in another both reported.
- [ ] `int` arithmetic cases show a `long` result type.

## Follow-ups

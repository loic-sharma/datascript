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

## Carried over
- [ ] (from M1-04) Golden `check` cases for the type-error builders (`BinaryTypeError`, `ArityError`, `ArgTypeError`, `operators::untyped_null`: DS2002/DS2003/DS2006/DS2009), with labels saying where each type came from.
- [ ] (from M1-04) Turn `UnknownFunction` into DS1004, using its did-you-mean and foreign-name equivalents.
- [ ] (from M1-15) Replace `Analysis::hir: Vec<BoundQuery>` (plans built directly by the binder) with a typed HIR.
- [ ] (from M1-15) A non-`bool` `where` predicate gives DS2001, not DS9001. Bind all operators, functions, `let`, `null`, and timespan/datetime literals (all DS9001 in the skeleton).
- [ ] (from M0-03) Replace the stand-in DataScript ↔ Arrow type-name mapping in `datascript-tests::schema` with M1-04's mapping (`arrow.rs`).
- [ ] (from review of M1-03) Run the binder only on a usable tree (ARCHITECTURE.md), or skip subtrees with syntax errors, so a syntax error adds no semantic errors. Today `where 1 < a < 3` also gives DS2003, `summerize count()` also gives DS9001 (D-030 says bind it as `summarize`), `t > 1x` also gives DS9001, and `project let, true` gives 3 errors.
- [ ] (from review of M1-04) Functions can return the internal `Null` type (`abs(null)`, `iff(p, null, null)`, `coalesce(null, null)`, …). Turn it into DS2009 or a default type so Arrow `Null` never reaches `to_arrow`, with a golden case.
- [ ] (from review of M1-04) Use M1-23's planned-function list so `make_list` etc. say "not available yet (M2)" instead of DS1004 "unknown function".

## Follow-ups

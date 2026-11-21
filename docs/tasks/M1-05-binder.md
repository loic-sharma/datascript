# M1-05 — Binder core and expression checking

**Status:** done — typed HIR (`hir.rs`) and planner (`plan.rs`); the binder resolves `let`s and
columns, checks every expression kind against §4.1 and the catalog, aggregate context, did-you-mean,
and skips nodes with syntax errors; DS1006, DS2010–DS2012 added (D-038).
**Depends on:** M1-15, M1-24
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
- [x] Golden `check` cases for names and expressions (happy path + errors), using the harness's
      `schema:` header and mock provider until a real provider exists (TESTING.md).
- [x] The DS1001 example in DIAGNOSTICS.md renders exactly as documented.
- [x] Multi-error case: unknown column in one statement and a type error in another both reported.
- [x] `int` arithmetic cases show a `long` result type.

## Carried over
- [x] (from M1-04) Golden `check` cases for the type-error builders (`BinaryTypeError`, `ArityError`, `ArgTypeError`, `operators::untyped_null`: DS2002/DS2003/DS2006/DS2009), with labels saying where each type came from.
- [x] (from M1-04) Turn `UnknownFunction` into DS1004, using its did-you-mean and foreign-name equivalents.
- [x] (from M1-15) Replace `Analysis::hir: Vec<BoundQuery>` (plans built directly by the binder) with a typed HIR.
- [x] (from M1-15) A non-`bool` `where` predicate gives DS2001, not DS9001. Bind all operators, functions, `let`, `null`, and timespan/datetime literals (all DS9001 in the skeleton). *(Expressions inside every operator are checked; the schemas of operators other than `where`/`project`/`take` are M1-16, handed over there.)*
- [x] (from M0-03) Replace the stand-in DataScript ↔ Arrow type-name mapping in `datascript-tests::schema` with M1-04's mapping (`arrow.rs`).
- [x] (from review of M1-03) Run the binder only on a usable tree (ARCHITECTURE.md), or skip subtrees with syntax errors, so a syntax error adds no semantic errors. Today `where 1 < a < 3` also gives DS2003, `summerize count()` also gives DS9001 (D-030 says bind it as `summarize`), `t > 1x` also gives DS9001, and `project let, true` gives 3 errors.
- [x] (from review of M1-04) Functions can return the internal `Null` type (`abs(null)`, `iff(p, null, null)`, `coalesce(null, null)`, …). Turn it into DS2009 or a default type so Arrow `Null` never reaches `to_arrow`, with a golden case.
- [x] (from M1-22) Once `let` binds, `ds run` on a program of only `let` statements gives the DS0004 warning (`Session::run` already emits it when the program has no query). Add the golden `tests/cases/skeleton/ds0004_let_only.dstest` (`let x = 1;`, `mode: run`); today it gives DS9001.
- [x] (from review of M1-04) Use M1-23's planned-function list so `make_list` etc. say "not available yet (M2)" instead of DS1004 "unknown function".
- [x] (from M1-24) A `let`'s kind is syntactic (D-036): it is tabular iff its value is a `PIPELINE` or one in parentheses (`LetStmt::is_tabular`). Bind table `let`s and scalar `let`s from that, not from the right side's names or types.
- [x] (from M1-24) `let alias = orders;` where `orders` is a table is a scalar `let` now. Report it with a binder code from this task's block, with a hint and a fix to write `let alias = from orders;`, and a golden case.
- [x] (from M1-24) `let x = (from t | count) + 1` and other scalar expressions that contain a pipeline: reject them with a clear error (a table is not a value), with a golden case.

## Follow-ups

- The planned-function list (`catalog::PLANNED`) was added here rather than waiting for M1-23;
  M1-23 has a carried-over note.
- Named and computed `project` items, operator schemas and the remaining DS9001s for operators
  are handed to M1-16; host-table names in `from` to M1-17; running calls, `in~` and the string
  operators on DataFusion to M1-09.
- The `let`-only DS0004 case is `tests/cases/skeleton/ds0004_let_only.dstest`.

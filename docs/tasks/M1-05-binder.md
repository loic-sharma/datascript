# M1-05 — Binder and semantic analysis

**Status:** todo
**Depends on:** M1-02, M1-04
**Crates:** `datascript-compiler`

## Goal
Resolve names, infer schemas through each pipeline stage, and type-check expressions, producing a
typed HIR and semantic diagnostics (DS1xxx, DS2xxx).

## Scope
- `SourceRef` (path, glob, or reader call with options, or a host table name) and the
  `SchemaProvider` trait, **defined in this crate** (the engine crate depends on the compiler, not
  the other way round). A `MockSchemaProvider` for tests.
- Scopes: program-level `let` bindings (scalar and tabular). No shadowing (DS1003).
- Per-operator schema transformation for all ● operators (column naming rules in LANGUAGE.md §3.3).
- Expression checking against the catalog. Aggregate context rules (DS2004, DS2005).
- Join: `$left`/`$right`, key type compatibility, output column renaming with an info diagnostic.
- Error recovery: unknown names get the error type, so there are no cascades (DIAGNOSTICS rule 8).
- Did-you-mean over columns in scope, plus the "columns available at this point" note.
- Source schema errors from the provider (DS3xxx) attached to the source span.
- `check(src, provider) -> Analysis { hir, diagnostics, stage_schemas }`. Keep `stage_schemas`
  (schema after each operator, keyed by text range) for LSP completion later.

## Acceptance criteria
- [ ] Golden `check` cases for each operator (happy path + at least one error each), using
      fixtures through a real provider once M1-08 lands. Until then use `tables:` with a mock
      provider (harness support may be needed; coordinate with M0-03).
- [ ] The DS1001 example in DIAGNOSTICS.md renders exactly as documented.
- [ ] Multi-error case: unknown column in one statement and a type error in another both reported.

## Follow-ups

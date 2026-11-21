# M1-04 — Type system and function catalog

**Status:** done — `Type`, Arrow mappings, §4.1 operator table, `FunctionCatalog` with the M1 set and overload resolution (DS2002/2003/2006/2009 builders), `Capabilities`.
**Depends on:** M0-02
**Crates:** `datascript-compiler`
**Diagnostic codes:** DS2002, DS2003, DS2006, DS2009 (inside M1-05's block) (D-018)

## Goal
Define DataScript scalar types and the single registry of functions that the binder, backends,
LSP, and docs all read. Can run in parallel with the parser tasks.

## Scope
- `Type` enum per LANGUAGE.md §5, plus an internal `Error` type that unifies with everything (to
  prevent cascading errors) and `Null`. Arrow mapping functions (`to_arrow`, `from_arrow` →
  `Result<Type, UnsupportedArrowType>`). Arrow types come from `arrow-schema` only.
- Coercion rules: int → long → real widening, null coercion. Everything else is explicit.
- `FunctionCatalog`: entries with `name`, `kind` (scalar/aggregate), signatures (with
  variadics and optional args), a return-type rule (fixed, same-as-arg-N, or a closure), a doc
  string, and examples. Lookup by name with did-you-mean.
- Overload resolution that produces precise errors: wrong arity (DS2002) says the expected
  signatures; wrong types (DS2003) points at the bad argument.
- Register the M1 function set from LANGUAGE.md §7. Implementation lives in the backend; the
  catalog only describes.

Design-review additions:
- Widening is `int` → `long` → `real` (LANGUAGE.md §4.2), and the full operator typing table in
  §4.1 (datetime/timespan arithmetic, integer division, string comparison) lives here as data plus
  unit tests. Integer arithmetic always yields `long`, and `sum` of integers is `long` (D-021).
- The Arrow → DataScript **read** mapping of LANGUAGE.md §5.1 (`from_arrow_source`), separate from
  the canonical `to_arrow`. All integer types read as `long` (D-021).
- `to_arrow` returns a `Field` with Arrow canonical extension type metadata where the storage type
  is ambiguous (`arrow.json`, `arrow.uuid`, D-024), and `from_arrow_source` reads it back.
- DS2006 (argument type mismatch, pointing at the argument) is separate from DS2003 (operator).
  DS2009 for an untyped `null` with no context.
- `Capabilities` (functions, join kinds, formats, and `deviations` from LANGUAGE.md semantics,
  D-014) is a plain data type defined here, not in the engine crate (ARCHITECTURE.md).

## Acceptance criteria
- [x] Unit tests for coercion and overload resolution, including the error-type short circuit.
- [x] A test that every catalog entry has docs and at least one example.
- [x] Builds for wasm32.
- [x] Every row of LANGUAGE.md §4.1 and §5.1 has a unit test.

## Follow-ups
- The diagnostics are builders (`BinaryTypeError::diagnostic`, `ArityError::diagnostic`,
  `ArgTypeError::diagnostic`, `operators::untyped_null`); golden `check` cases for them come with
  the binder (M1-05), which also adds where a type came from to the labels.
- `UnknownFunction` carries did-you-mean and foreign-name equivalents; M1-05 owns DS1004 and
  turns it into the diagnostic.

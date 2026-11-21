# M1-04 — Type system and function catalog

**Status:** todo
**Depends on:** M0-02
**Crates:** `datascript-compiler`

## Goal
Define DataScript scalar types and the single registry of functions that the binder, backends,
LSP, and docs all read. Can run in parallel with the parser tasks.

## Scope
- `Type` enum per LANGUAGE.md §5, plus an internal `Error` type that unifies with everything (to
  prevent cascading errors) and `Null`. Arrow mapping functions (`to_arrow`, `from_arrow` →
  `Result<Type, UnsupportedArrowType>`). Arrow types come from `arrow-schema` only.
- Coercion rules: long → real widening, null coercion. Everything else is explicit.
- `FunctionCatalog`: entries with `name`, `kind` (scalar/aggregate), signatures (with
  variadics and optional args), a return-type rule (fixed, same-as-arg-N, or a closure), a doc
  string, and examples. Lookup by name with did-you-mean.
- Overload resolution that produces precise errors: wrong arity (DS2002) says the expected
  signatures; wrong types (DS2003) points at the bad argument.
- Register the M1 function set from LANGUAGE.md §7. Implementation lives in the backend; the
  catalog only describes.

## Acceptance criteria
- [ ] Unit tests for coercion and overload resolution, including the error-type short circuit.
- [ ] A test that every catalog entry has docs and at least one example.
- [ ] Builds for wasm32.

## Follow-ups

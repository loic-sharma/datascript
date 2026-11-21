# M1-07 — Engine API and Session

**Status:** todo
**Depends on:** M1-06
**Crates:** `datascript-engine`
**Diagnostic codes:** DS4001–DS4049 (shared with M1-09) (D-018)

## Goal
The backend-neutral contract between frontends and engines (ARCHITECTURE.md § Engine abstraction).
Extends the skeleton from M1-15 rather than starting from nothing.

## Scope
- `Engine` trait: futures are `Send` on native and `?Send` on wasm32 (D-011). Native `Session` is
  `Send + Sync`. `ResultStream` / `QueryResult { schema, batches, render, diagnostics }`.
- `Capabilities` is the compiler's type (M1-04) and the capability check is the compiler's pass
  (M1-05, DS4001). This crate only passes `engine.capabilities()` to the compiler.
- `Session` facade: holds the engine, a schema cache (implements `SchemaProvider` by delegating to
  `Engine::schema_of`), and an injected `Clock`, but not one document's text, so the LSP and server
  can share one `Session`. Methods `check`, `explain`, `run` take a `SourceFile`. Schema lookups are
  async but the compiler's are sync, so run the source-resolution loop (D-022): bind against the
  cache, fetch `pending_sources`, bind again; a third pass with pending sources is DS9001.
- `Engine::schema_of` returns `Result<Schema, SourceError>` (D-023); the compiler builds the
  diagnostics.
- `ExecContext` (clock value for the run, base directory, optional path sandbox, optional row
  limit). `now()` is read once per program run (D-015).
- A panic boundary that turns panics into DS9001. Dropping a `ResultStream` cancels execution.
- Arrow ASCII table and CSV/JSON writers for frontends (move or reuse from M0-03).
- A fake in-memory engine for tests of the session logic.

## Acceptance criteria
- [ ] Unit tests with the fake engine: capability errors, schema-cache hits, clock injection, and
      the resolution loop (two passes, then done; one `schema_of` call per distinct source).
- [ ] No DataFusion dependency in this crate.
- [ ] A compile-time test (`fn assert_send<T: Send>()`) for the native `Session` and its futures.

## Follow-ups

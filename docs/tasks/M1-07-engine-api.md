# M1-07 — Engine API and Session

**Status:** todo
**Depends on:** M1-06
**Crates:** `datascript-engine`

## Goal
The backend-neutral contract between frontends and engines (ARCHITECTURE.md § Engine abstraction).

## Scope
- `Engine` trait (`?Send` async for WASM), `Capabilities` (supported functions, join kinds, formats),
  `ResultStream` / `QueryResult { schema, batches, render, diagnostics }`.
- A capability check pass: walk a `Program` and produce DS4001 for anything the engine lacks,
  pointing at the call site.
- `Session` facade: holds the source, the engine, a schema cache (implements `SchemaProvider` by
  delegating to `Engine::schema_of`), and an injected `Clock`. Methods: `check`, `explain`, `run`.
  Schema lookups are async but the compiler's are sync, so pre-resolve: collect `SourceRef`s from
  the parse, fetch their schemas, then run the binder.
- Arrow ASCII table and CSV/JSON writers for frontends (move or reuse from M0-03).
- A fake in-memory engine for tests of the session logic.

## Acceptance criteria
- [ ] Unit tests with the fake engine: capability errors, schema-cache hits, clock injection.
- [ ] No DataFusion dependency in this crate.

## Follow-ups

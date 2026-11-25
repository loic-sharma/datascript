# M7-05 — In-memory host tables on a live session

**Status:** todo
**Depends on:** M2-13
**Crates:** `datascript-engine`, `datascript-datafusion`, `datascript-compiler`
**Diagnostic codes:** none expected (an unknown table is still DS1002)

## Goal
Let a host register an in-memory Arrow table on a running `Session`, and remove or replace it,
so the Python package (M7-03) and the C API (M7-04) can pass dataframes and Arrow streams in.
Both need this; do it once, first.

## Scope
- Today `DataFusionEngine::with_table` (`crates/datascript-datafusion/src/lib.rs`) accepts only a
  file-backed `SourceRef`, and only while building the engine. Add registration on a live
  `Session`: `register_batches(name, schema, Vec<RecordBatch>)` (a DataFusion `MemTable` or
  equivalent), `register_source(name, SourceRef)` for the existing file case, and
  `deregister(name)`. Replacing a name is allowed.
- Thread safety: a `Session` is shared (`Send + Sync`, D-011). Registration must not race with a
  query that is being checked or run; say what a running query sees (the table as it was when the
  run started).
- The session's schema cache (D-043) and the compiler's `SchemaProvider` must see a new or replaced
  table at once: invalidate the name on every change.
- In-memory columns go through the same widening as file columns (LANGUAGE.md §5.1, D-012), so a
  pandas `int32` column is `long` and a `Timestamp(ns)` is `datetime`.
- Host tables are listed with their schemas, for completion and `.tables`-style listings.
- Note in ARCHITECTURE.md (§ Engine abstraction) how hosts register tables.

## Acceptance criteria
- [ ] Engine tests: register, query, replace and deregister an in-memory table; a query that joins
      a file with an in-memory table; widening of narrow integer and timestamp types.
- [ ] A test that a query started before a replacement sees the old table.

## Follow-ups

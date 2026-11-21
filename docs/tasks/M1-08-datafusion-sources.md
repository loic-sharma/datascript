# M1-08 — DataFusion backend: sources and schemas

**Status:** todo
**Depends on:** M1-07, M0-04, M0-05
**Crates:** `datascript-datafusion`
**Diagnostic codes:** DS3001–DS3049 (D-018)

## Goal
Implement `Engine::schema_of` and table providers for local files on DataFusion. Extends the skeleton from M1-15 rather than starting from nothing.

## Scope
- DataFusion is in the workspace from M1-15. Settle its feature set: current release, default
  features you don't need disabled, and a set that will later work on wasm32 (M0-05).
- Map `SourceRef` to `ListingTable`: path literals and globs (relative to a base directory given
  by the session), format by extension, `read_csv`/`read_parquet`/`read_json` options per
  LANGUAGE.md §3.1.
- Host-registered tables (`--table name=path`).
- Source errors as structured `SourceError` values (D-023), which the compiler turns into DS3001
  file not found (did-you-mean over `nearby` files in the same directory), DS3002, DS3003
  (did-you-mean over option names), DS3004 empty glob, DS3005 incompatible schemas (the two files
  and the differing column). This crate owns the DS3xxx block and writes the golden cases, even
  though the message text is built in the compiler.
- Map Arrow types to DataScript types and report unsupported column types lazily (only if used).

Design-review additions:
- Widen source columns at the scan per LANGUAGE.md §5.1 (D-012): `Timestamp(ns)` and `Date32`
  → `datetime`, `Utf8View`/dictionary → `string`, every integer type → `long` (D-021), `Float32` →
  `real`, decimals → `real` in M1. `schema_of` returns the widened schema.
- Glob merging by **name**, from each file's own schema. For CSV/TSV, infer each file's header and
  map columns by name before merging; DataFusion's CSV reader applies a table schema by position,
  so a single `ListingTable` over files with different header orders would misalign silently. Use
  per-file scans projected to the merged schema (or a schema adapter) when headers differ.
- Compressed CSV/TSV/JSON (`.gz`, `.bz2`, `.xz`, `.zst`) and the `compression` option.
- Globs: `**`, sorted matches, type widening across files before DS3005.
- Path resolution against `ExecContext`'s base directory and optional sandbox (DS3007 is
  implemented in M4-01, but the hook belongs here). DS3008 for unreadable files.
- Add a `wasm` feature and the CI step
  `cargo check -p datascript-datafusion --no-default-features --features wasm --target wasm32-unknown-unknown`
  (use what M0-05 learned).

## Acceptance criteria
- [ ] Golden `check` cases in `tests/cases/sources/` for every source form and every DS3xxx error.
- [ ] The harness `Driver` wires `check` mode to the real session with this backend.
- [ ] Golden cases over a Parquet fixture with `Timestamp(ns)`, `Int16`, `Float32` and a dictionary string column.
- [ ] `'../../data/reordered/*.csv'` returns correctly aligned rows (golden `run` once M1-09 lands;
      `check` of the merged schema here).
- [ ] The wasm32 check of this crate runs in CI.

## Follow-ups

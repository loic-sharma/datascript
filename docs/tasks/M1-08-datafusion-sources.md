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
  (use what M0-05 learned: [feature set and wasm32 workarounds](../spikes/datafusion-wasm.md#feature-set-for-m1-08).
  The CI runner needs a wasm-capable `clang` for zstd-sys, plus the getrandom rustflag).

## Acceptance criteria
- [ ] Golden `check` cases in `tests/cases/sources/` for every source form and every DS3xxx error.
- [ ] The harness `Driver` wires `check` mode to the real session with this backend.
- [ ] Golden cases over a Parquet fixture with `Timestamp(ns)`, `Int16`, `Float32` and a dictionary string column.
- [ ] `'../../data/reordered/*.csv'` returns correctly aligned rows (golden `run` once M1-09 lands;
      `check` of the merged schema here).
- [ ] The wasm32 check of this crate runs in CI.

## Carried over
- [ ] (from M1-15) Replace the temporary `SourceError::Unreadable { reason }` (reported as DS9001) with specific cases and DS3008.
- [ ] (from M1-15) Read all supported extensions (not only `.csv`), globs, reader options and host tables; the scan reuses the schema read during checking instead of reading it again.
- [ ] (from M0-04) Confirm DataFusion's CSV inference reads `2024-02-29T23:59:59Z` timestamps as UTC timestamps, not strings, with a golden case.
- [ ] (from review of M1-04) Struct, List and Map columns must not be widened to `arrow.json` text by `widen()` in M1 (the text is not JSON, or the cast fails); report them as unsupported (DS2008) until M2-03, matching M1-23.
- [ ] (from review of M1-04) A `UInt64` value above `i64::MAX` fails the scan cast with a generic DS4002. Report DS4003 naming the column, as LANGUAGE.md §5.1 requires.
- [ ] (from M1-17) `SourceRef` now has `Path`, `Glob`, `Reader(ReaderCall)` and `Table`. `CsvTable::open` handles only `Path` and `read_csv` without options; other cases return `SourceError::Unreadable` ("not supported yet"), and `Table` returns `NotFound` (the binder reports DS1002 "unknown table"). Implement them, and register host tables so `Table` lookups find them.
- [ ] (from M1-17) `DataFusionEngine`'s `Capabilities::formats` is `{Csv}`, so other formats are DS4001 before any read (`tests/cases/sources/datafusion_formats.dstest`). Add each format as it can be read, and re-bless that case.
- [ ] (from M1-17) The binder already builds DS3002 (unknown extension) and DS3003 (unknown or planned reader option) and checks option values (DS2041–DS2043) from `source::READERS` (D-041), so `SourceError` needs no `UnknownFormat` case. Golden cases for them are in `tests/cases/sources/`; add the engine-side DS3xxx cases next to them.

## Follow-ups

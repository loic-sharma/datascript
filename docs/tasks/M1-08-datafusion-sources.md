# M1-08 — DataFusion backend: sources and schemas

**Status:** todo
**Depends on:** M1-07, M0-04
**Crates:** `datascript-datafusion`

## Goal
Implement `Engine::schema_of` and table providers for local files on DataFusion.

## Scope
- Add DataFusion to the workspace (current release; disable default features you don't need, and
  keep a feature set that will later work on wasm32).
- Map `SourceRef` to `ListingTable`: path literals and globs (relative to a base directory given
  by the session), format by extension, `read_csv`/`read_parquet`/`read_json` options per
  LANGUAGE.md §3.1.
- Host-registered tables (`--table name=path`).
- Diagnostics: DS3001 file not found (with did-you-mean over files in the same directory), DS3002,
  DS3003 (did-you-mean over option names), DS3004 empty glob, DS3005 incompatible schemas
  (name the two files and the differing column).
- Map Arrow types to DataScript types and report unsupported column types lazily (only if used).

## Acceptance criteria
- [ ] Golden `check` cases in `tests/cases/sources/` for every source form and every DS3xxx error.
- [ ] The harness `Driver` wires `check` mode to the real session with this backend.

## Follow-ups

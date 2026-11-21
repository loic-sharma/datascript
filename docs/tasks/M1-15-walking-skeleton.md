# M1-15 — Walking skeleton: one query through every layer

**Status:** done — `ds run/check/explain` run the skeleton query through the binder, IR, `Session` and
DataFusion; golden cases in `tests/cases/skeleton/`.
**Depends on:** M1-02, M1-04, M0-04, M0-05
**Crates:** `datascript-compiler`, `datascript-engine`, `datascript-datafusion`, `datascript-cli`
**Diagnostic codes:** none of its own. It may give a minimal form of codes already in the registry
(DS1001, DS3001); the owning tasks finish them.

## Goal
Run `'orders.csv' | where amount > 100 | project orderId, amount | take 5` end to end with
`ds run`, through the real binder, IR, `Engine`/`Session` and DataFusion, before any layer is
built out. The point is to check early that the interfaces between layers have the right shape
(ARCHITECTURE.md), not to cover the language. M1-05 to M1-11 extend this code instead of starting
from nothing.

## Scope
- Binder: one CSV path source, column references, literal comparisons, `where`, `project`,
  `take`. Unknown column gives a minimal DS1001.
- Source resolution loop (D-022) with a `SchemaProvider` that answers `Pending`, and
  `Engine::schema_of` returning `SourceError` (D-023), even if only `NotFound` exists yet.
- IR: `Scan`, `Filter`, `Project`, `Limit`, with spans. `ds explain` prints them.
- `Engine` trait, `Session` with an injected clock, `ExecContext`, the native `Send` bound (D-011).
- Adding DataFusion 55 to the workspace raises `rust-version` to at least 1.94 (its MSRV;
  [M0-05 findings](../spikes/datafusion-wasm.md#versions-and-toolchain)).
- DataFusion: CSV `ListingTable` and lowering of those four nodes, with the widening at the scan
  for the fixture's columns.
- CLI: `ds run <file>` printing the ASCII table; `ds check <file>`.
- Harness `Driver` wired for `check`, `explain` and `run` modes.

## Out of scope
Every other operator, function, format and diagnostic. Leave `todo!()`-free stubs that return a
DS9001 "not implemented yet" diagnostic, never a panic.

## Acceptance criteria
- [x] Golden `run`, `explain` and `check` cases for the query above pass in CI.
- [x] The native `Session` passes the `assert_send` test, and the core crates still pass the
      wasm32 check.
- [x] Any interface change from ARCHITECTURE.md is written back to ARCHITECTURE.md in this task.

## Follow-ups
Shortcuts taken in the skeleton that the owning tasks replace:

- **M1-05 / M1-06:** the binder builds IR plans directly. `Analysis::hir` is a `Vec<BoundQuery>`
  holding each query's plan. M1-05 introduces a typed HIR and M1-06 moves planning out of
  `binder.rs`.
- **M1-05:** a non-`bool` `where` predicate gets DS9001 instead of DS2001. Only comparisons are
  bound; other operators, functions, `let`, `null`, timespan and datetime literals are DS9001.
- **M1-16:** `project` takes bare columns only, with no DS1005 for duplicate names (DataFusion
  then fails with DS4002).
- **M1-17:** a table name as a source is DS9001. Columns with no DataScript type get the error type
  silently instead of DS2008, and `caps` is not consulted.
- **M1-08:** `SourceError::Unreadable { reason }` is temporary. The binder reports it as DS9001;
  M1-08 replaces it with specific cases and DS3008. Only `.csv` paths are read (other extensions
  are DS9001), with no globs, reader options or host tables. The scan reads the schema a second
  time instead of reusing the one from checking.
- **M1-07:** the session caches only successful schema lookups and never invalidates them.
  `run` keeps no warnings when there are no errors (none exist yet). There is no panic boundary,
  CSV/JSON writer or `render` field yet.
- **M1-09:** `lower_op` maps only comparisons faithfully; DataFusion runtime errors are DS4002 on
  the query's last node, because the backend does not trace them to a node yet.
- **M1-11:** the CLI takes one file per command, with no `-e`, stdin, `--format`, `--color` or
  `--table`.

Workspace changes in this task: `rust-version` is now 1.94 (DataFusion 55's MSRV), which turned on
`clippy::manual_is_multiple_of` (fixed in `catalog.rs`). Workspace lints allow `linker_messages`
(macOS ld notes that binaries linking DataFusion have a large `__eh_frame`) and
`clippy::result_large_err` (`Diagnostic` is every stage's error type). The harness samples
`harness/{check,explain,run}.dstest` are now `skip:`ped, since they need `summarize` or host tables.

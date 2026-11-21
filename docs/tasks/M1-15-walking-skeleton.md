# M1-15 — Walking skeleton: one query through every layer

**Status:** todo
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
- [ ] Golden `run`, `explain` and `check` cases for the query above pass in CI.
- [ ] The native `Session` passes the `assert_send` test, and the core crates still pass the
      wasm32 check.
- [ ] Any interface change from ARCHITECTURE.md is written back to ARCHITECTURE.md in this task.

## Follow-ups

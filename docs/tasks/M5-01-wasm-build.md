# M5-01 — Build the engine for wasm32

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-wasm`, `datascript-datafusion`

## Goal
Compile `datascript-datafusion` (with a wasm feature set) and the core crates to
`wasm32-unknown-unknown`, with files served from an in-memory `ObjectStore`.

## Acceptance criteria
- [ ] `wasm-pack build` succeeds in CI. Binary size is recorded and tracked.
- [ ] A headless test (`wasm-bindgen-test`) runs a query over an in-memory CSV and a Parquet file.

## Follow-ups

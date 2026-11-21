# M5-01 — Build the engine for wasm32

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-wasm`, `datascript-datafusion`

## Goal
Compile `datascript-datafusion` (with a wasm feature set) and the core crates to
`wasm32-unknown-unknown`, with files served from an in-memory `ObjectStore`. Build **two modules**
(ARCHITECTURE.md § WASM constraints): a small compiler module (diagnostics, syntax, compiler) and
the engine module (DataFusion), so the editor works before the engine has downloaded.

## Acceptance criteria
- [ ] `wasm-pack build` succeeds in CI for both modules. Both sizes are recorded and tracked.
- [ ] A headless test (`wasm-bindgen-test`) runs a query over an in-memory CSV and a Parquet file.

## Follow-ups

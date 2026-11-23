# M5-01 — Build the engine for wasm32

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-wasm`, `datascript-datafusion`

## Goal
Compile `datascript-datafusion` (with a wasm feature set) and the core crates to
`wasm32-unknown-unknown`, with files served from an in-memory `ObjectStore`. Build **two modules**
(ARCHITECTURE.md § WASM constraints): a small compiler module (diagnostics, syntax, compiler) and
the engine module (DataFusion), so the editor works before the engine has downloaded.

Start from the [M0-05 spike](../spikes/datafusion-wasm.md): feature set, wasm32 workarounds,
size baseline (5.8 MB gzipped for DataFusion alone), and the open points (headless Chrome in CI,
disable the disk manager on wasm, keep `target_partitions = 1`).

## Acceptance criteria
- [ ] `wasm-pack build` succeeds in CI for both modules. Both sizes are recorded and tracked.
- [ ] A headless test (`wasm-bindgen-test`) runs a query over an in-memory CSV and a Parquet file.

## Carried over
- [ ] (from M1-07) `catch_panic` / `catch_panic_async` in `datascript-engine` use `catch_unwind`, which catches nothing on wasm32 (panics abort). Install a panic hook that reports DS9001 to the JS side instead (ARCHITECTURE.md § Engine abstraction).
- [ ] (from M1-08) `datascript-datafusion` builds for wasm32 (`--no-default-features --features wasm`, CI job `wasm-datafusion`), but source resolution uses `std::fs` and the `glob` crate: on wasm32, resolve paths and globs against the registered files (an `InMemory` object store at `memory://`, docs/spikes/datafusion-wasm.md) instead. Compressed files are off there (`compression` is a native default); decide whether to turn it on after testing decoding on wasm32.
- [ ] (from M0-05) Set up headless Chrome in CI for `wasm-pack test --headless --chrome` (the spike ran its tests under Node only), disable DataFusion's disk manager on wasm32, and keep `target_partitions = 1` (docs/spikes/datafusion-wasm.md).
- [ ] (from M2-13) Test JSON, NDJSON and globs on wasm32, not only CSV and Parquet: `is_json_array` in `crates/datascript-datafusion/src/sources.rs` opens the file with `std::fs` to tell a JSON array from NDJSON, so it must read from the in-memory store there.

## Follow-ups

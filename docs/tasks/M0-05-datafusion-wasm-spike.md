# M0-05 — Spike: DataFusion on wasm32

**Status:** done. DataFusion 55.3 runs CSV and Parquet queries on wasm32 (Node and Chromium) at 5.8 MB gzipped. Findings, feature set, and the duration and checked-arithmetic probes are in `docs/spikes/datafusion-wasm.md`.
**Depends on:** M0-01
**Crates:** throwaway crate under `spikes/datafusion-wasm/` (not a workspace member)

## Goal

Check the riskiest assumption in the plan early: that DataFusion builds for
`wasm32-unknown-unknown` and can query in-memory CSV and Parquet bytes, at a size we can ship. M5
depends on it, and the M1 backend should pick its features knowing the answer.

## Scope

- A minimal crate that depends on DataFusion with default features off plus what CSV and Parquet
  need, registers files in an in-memory `ObjectStore`, and runs `SELECT count(*), sum(x)` over each.
- Build with `wasm-pack build --release` and run it with `wasm-bindgen-test` in headless Chrome or Node.
- Also probe, natively, how DataFusion handles `timespan` as `Duration(µs)`: `timestamp - timestamp`,
  `timestamp ± duration`, `duration * int`, `duration / duration`, and `sum`/`avg`/`min`/`max` over a
  duration column. Note which return `Interval` instead of `Duration`, and which need a UDF.
- Also probe checked integer arithmetic: how to make `Int64` overflow and integer division by zero
  an error rather than wrap (D-014).
- Record: DataFusion version, the feature set, any patches or `getrandom`/`js` workarounds,
  whether tokio is still pulled in, and the `.wasm` size raw, after `wasm-opt -Oz`, and gzipped.

## Out of scope

Any DataScript code. The spike is not maintained after this task.

## Acceptance criteria

- [x] Findings written to `docs/spikes/datafusion-wasm.md`, including the exact feature set M1-08
      should use for its `wasm` feature, and the duration and checked-arithmetic findings for M1-09.
- [x] (Not triggered: it works, at 5.8 MB gzipped.) If DataFusion does not work on wasm32, or the gzipped size is above ~15 MB, ARCHITECTURE.md
      § WASM constraints and the M5 tasks are updated with the alternative (and a DECISIONS entry).

## Follow-ups
- `wasm-pack test --headless --chrome` was not run (no chromedriver on the dev machine). The tests
  ran under Node, and the web build ran in the desktop app's Chromium. M5-01 sets up headless
  Chrome in CI (noted in its task file).
- DataFusion 55 needs Rust 1.94, but the workspace says `rust-version = "1.85"`. Noted in M1-15,
  which adds DataFusion.
- The repo needs a `.cargo/config.toml` (or CI env) with the getrandom rustflag, plus a
  wasm-capable clang on the CI runner, before M1-08's wasm32 check can pass. Noted in M1-08.
- The spike crate is committed under `spikes/datafusion-wasm/` so the measurements can be
  reproduced. It is not a workspace member and nothing builds it in CI.

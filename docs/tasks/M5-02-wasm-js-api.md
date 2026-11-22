# M5-02 — JS/TS API (`@datascript/wasm`)

**Status:** todo
**Depends on:** M5-01, M3-02 (`complete` lives in the compiler), M2-13
**Crates:** `datascript-wasm`

## Goal
A typed JS API: `init()`, `registerFile(name, bytes | File | URL)`, `check(text)`, `run(text)`
(Arrow IPC + RenderSpec + diagnostics), `complete(text, offset)`. Runs in a Web Worker so the UI
stays responsive. `check` and `complete` use the compiler module and are available as soon as it
loads; the engine module loads lazily on the first `run` or schema lookup. Cancelling a run stops
the worker (a CPU-bound query cannot observe a dropped stream on one thread). Diagnostic positions
are UTF-16 (D-025).

## Acceptance criteria
- [ ] Generated `.d.ts` plus hand-written docs. Vitest tests in Node and in a browser.

## Follow-ups

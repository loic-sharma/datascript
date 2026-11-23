# M5-03 — WASM runtime adapter and playground

**Status:** todo
**Depends on:** M5-02, M4-02, M4-03, M4-04, M2-13
**Crates:** `web/`

## Goal
A `WasmRuntime` implementation of the web `Runtime` interface, plus a static playground build
(drag-drop files, sample datasets, shareable query URLs that never include data).

## Acceptance criteria
- [ ] The same Playwright smoke tests pass against both runtimes.
- [ ] Deployed to a static host by CI (e.g. GitHub Pages) once the maintainer approves.

## Follow-ups

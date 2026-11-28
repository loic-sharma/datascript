# M5-03 — WASM runtime adapter and playground

**Status:** todo
**Depends on:** M5-02, M4-02, M4-09, M4-14, M4-03, M4-04, M2-13
**Crates:** `web/`

## Goal
A `WasmRuntime` implementation of the web `Runtime` interface, plus a static playground build
(drag-drop files, sample datasets, shareable query URLs that never include data). Notebook calls
(`check`, `run`, IDE requests) go through M5-02's notebook API, never the plain-text `run`, so
cell runs, `DS1085`, and statement runs behave exactly as in `ds ui`. `WasmRuntime` exports with
client-side blob downloads and reports `capabilities.exportFormats` as the formats the WASM build's
writers support (at least `csv` and `ndjson`), so the same export menu works unchanged.

## Acceptance criteria
- [ ] The same Playwright smoke tests pass against both runtimes.
- [ ] Deployed to a static host by CI (e.g. GitHub Pages) once the maintainer approves.

## Follow-ups

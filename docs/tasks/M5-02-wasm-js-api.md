# M5-02 — JS/TS API (`@datascript/wasm`)

**Status:** todo
**Depends on:** M5-01
**Crates:** `datascript-wasm`

## Goal
A typed JS API: `init()`, `registerFile(name, bytes | File | URL)`, `check(text)`, `run(text)`
(Arrow IPC + RenderSpec + diagnostics), `complete(text, offset)`. Runs in a Web Worker so the UI
stays responsive.

## Acceptance criteria
- [ ] Generated `.d.ts` plus hand-written docs. Vitest tests in Node and in a browser.

## Follow-ups

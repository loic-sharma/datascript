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

## Scope
The sketch above (and the one in ARCHITECTURE.md § WASM constraints) is the minimum. The API must
also cover what `web/` (M4-02's `Runtime` interface, M5-03) and the parity tests (M5-04) need:
- programs with several statements: `run` yields one result per query, with its statement index,
  and can run one statement by index (`Cmd/Ctrl+Alt+Enter` in the web UI, D-063);
- the 10,000-row UI display limit decided in D-064 (closed Q-007), and the `truncated` flag;
- `schema(source)`, `hover(text, offset)`, `signatureHelp(text, offset)`, `explain(text)`;
- `registerTable` for host tables, and listing and removing registered files;
- an injected `now` (core crates read no clock, ARCHITECTURE.md) so `now()`/`ago()` work and
  parity tests are stable;
- an `AbortSignal` on `run`.
Write the final API into ARCHITECTURE.md § WASM constraints and the API contract doc M4-01 writes.

## Acceptance criteria
- [ ] Generated `.d.ts` plus hand-written docs. Vitest tests in Node and in a browser.

## Carried over
- [ ] (from planning, 2026-11-24) If M7-01 is done, URL sources are allowed natively (`ExecContext::allow_remote`). In the browser, fetch them with `fetch` (CORS permitting) instead of needing `registerFile`, honour `allow_remote`, and turn fetch failures into M7-01's errors, with a Vitest test. (If M7-01 lands after this task, M7-01 adds this item's work to M5-03 or a new task.)
- [ ] (from planning, 2026-11-24) If M2-25 is done, `output` writes into the registered-file store; report written files from `run` (path, format, rows) so the UI can offer a download, with a Vitest test. (If M2-25 lands after this task, M2-25 does it.)

## Follow-ups

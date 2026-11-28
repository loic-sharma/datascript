# M5-02 — JS/TS API (`@datascript/wasm`)

**Status:** todo
**Depends on:** M5-01, M3-02 (`complete` lives in the compiler), M4-08 (`.dsnb` parser and `StitchedProgram`), M2-13
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
  and can run one statement by index (D-063). The index form checks the whole program, as
  `Session::run_statement` does, so it is for plain `.ds` scripts only;
- **notebooks** (D-062), all through M4-08's `.dsnb` parser and `StitchedProgram`, matching
  `ds ui` exactly ([WEB_UI.md §3](../WEB_UI.md#transitive-upstream-let-prepending--ds1085)):
  `parseNotebook(text)` / `serializeNotebook(cells, baseText?)` (the same content-diff
  serializer), `checkNotebook(cells)` (whole-notebook check, diagnostics grouped by `cellIndex`),
  and `runCell(cells, cellIndex, { statementIndex?, now })` using single-cell, single-statement,
  or bindings-only stitching with `DS1085` and `DS0004` suppression, so a broken neighbouring
  statement never blocks a statement run. `complete`/`hover`/`signatureHelp` also accept
  `(cells, cellIndex, offset)` using prefix stitching;
- the 10,000-row UI display limit decided in D-064 (closed Q-007), and the `truncated` flag;
- `schema(source)`, `hover(text, offset)`, `signatureHelp(text, offset)`, `explain(text)`;
- `registerTable` for host tables, and listing and removing registered files;
- an injected `now` (core crates read no clock, ARCHITECTURE.md) so `now()`/`ago()` work and
  parity tests are stable;
- an `AbortSignal` on `run`.
Write the final API into ARCHITECTURE.md § WASM constraints and [WEB_UI.md §6](../WEB_UI.md#the-runtime-abstraction-web-wasm-desktop-vs-code) (M4-01/M4-10).

## Acceptance criteria
- [ ] Generated `.d.ts` plus hand-written docs. Vitest tests in Node and in a browser.
- [ ] Notebook Vitest tests: `DS1085` (direct and indirect), a statement run succeeding next to a
      broken statement in the same cell, cross-cell `let` completion, and a `.dsnb` round trip
      through `parseNotebook`/`serializeNotebook`, with results identical to M4-08's fixtures.

## Carried over
- [ ] (from planning, 2026-11-24) If M7-01 is done, URL sources are allowed natively (`ExecContext::allow_remote`). In the browser, fetch them with `fetch` (CORS permitting) instead of needing `registerFile`, honour `allow_remote`, and turn fetch failures into M7-01's errors, with a Vitest test. (If M7-01 lands after this task, M7-01 adds this item's work to M5-03 or a new task.)
- [ ] (from planning, 2026-11-24) If M2-25 is done, `output` writes into the registered-file store; report written files from `run` (path, format, rows) so the UI can offer a download, with a Vitest test. (If M2-25 lands after this task, M2-25 does it.)

## Follow-ups

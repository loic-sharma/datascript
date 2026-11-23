# M3-08 — LSP integration test gaps

**Status:** done — added 9 JSON-RPC integration tests in `crates/datascript-lsp/tests/lsp.rs` covering `relatedInformation`, location-less diagnostics and empty programs, multi-change `didChange`, CRLF documents, `datascript/run` warning+runtime merging / `statement: 0` / `maxRows: 0` / compile errors, and multi-line / error / closed-or-unknown CodeLens; all passed with no LSP bugs exposed.
**Depends on:** M3-05
**Crates:** `datascript-lsp` (`tests/lsp.rs`)
**Diagnostic codes:** none new expected (tests only). If a test finds a bug whose fix needs a
code, write a new task for the fix instead of allocating one here (D-018).

## Goal
A review of the integration tests (November 2026) found LSP behaviour that only unit tests in
`src/convert.rs` cover, or that nothing tests. Add tests over JSON-RPC in `tests/lsp.rs`.

## Scope

### Diagnostics
- [x] `relatedInformation` for a diagnostic with secondary spans (for example `DS2003` or
  `DS1003`) is published with the right URIs and ranges.
- [x] A diagnostic with no location (`DS0004`, empty program) is published at a sensible range
  (see Follow-ups: `check` does not publish `DS0004`; covered via `datascript/run` for `DS0004`
  and via `DS9001`/`DS4002` for location-less `publishDiagnostics`).

### Document sync
- [x] One `textDocument/didChange` with several `contentChanges`, applied in order.
- [x] A CRLF document: diagnostic ranges and CodeLens lines are correct.

### `datascript/run`
- [x] A runtime error in a document that already has a check warning (for example `DS2103` plus
  `DS4003`): both are published, so `Doc::published` in `src/server.rs` keeps the check
  diagnostics when it adds runtime ones. Today the runtime test only sees `["DS4003"]`.
- [x] `statement: 0` (out of range) and `maxRows: 0` (no limit).
- [x] A document with a compile error returns `failed: true` and adds no runtime diagnostics.

### CodeLens
- [x] A query statement over several lines: the lens range covers the whole statement.
- [x] A document with syntax errors, and a closed or unknown URI.

## Acceptance criteria
- [x] Every item above has a test, or a note under "Follow-ups" saying why it cannot.
- [x] Any wrong behaviour a new test exposes gets its own task file and index row; the test is
      added with `#[ignore = "<task id>"]`.

## Follow-ups
- Note: `Session::check` intentionally does not emit `DS0004` for empty or `let`-only programs
  (LANGUAGE.md §2); `DS0004` is emitted by `datascript/run` in its output (with `failed: false`).
  `location_less_diagnostics_and_empty_program` verifies both that empty and `let`-only programs
  publish no check diagnostics and report `DS0004` on `datascript/run`, and that location-less
  diagnostics (`primary: None`) published via `textDocument/publishDiagnostics` (check-time
  `DS9001` from a caught engine panic and run-time untraceable `DS4002`) land at range `0:0-0:0`.
- Note: all 9 new integration tests pass against the current server; no LSP bugs were found.

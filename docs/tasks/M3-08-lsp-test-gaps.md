# M3-08 — LSP integration test gaps

**Status:** todo
**Depends on:** M3-05
**Crates:** `datascript-lsp` (`tests/lsp.rs`)
**Diagnostic codes:** none new expected (tests only). If a test finds a bug whose fix needs a
code, write a new task for the fix instead of allocating one here (D-018).

## Goal
A review of the integration tests (November 2026) found LSP behaviour that only unit tests in
`src/convert.rs` cover, or that nothing tests. Add tests over JSON-RPC in `tests/lsp.rs`.

## Scope

### Diagnostics
- [ ] `relatedInformation` for a diagnostic with secondary spans (for example `DS2003` or
  `DS1003`) is published with the right URIs and ranges.
- [ ] A diagnostic with no location (`DS0004`, empty program) is published at a sensible range.

### Document sync
- [ ] One `textDocument/didChange` with several `contentChanges`, applied in order.
- [ ] A CRLF document: diagnostic ranges and CodeLens lines are correct.

### `datascript/run`
- [ ] A runtime error in a document that already has a check warning (for example `DS2103` plus
  `DS4003`): both are published, so `Doc::published` in `src/server.rs` keeps the check
  diagnostics when it adds runtime ones. Today the runtime test only sees `["DS4003"]`.
- [ ] `statement: 0` (out of range) and `maxRows: 0` (no limit).
- [ ] A document with a compile error returns `failed: true` and adds no runtime diagnostics.

### CodeLens
- [ ] A query statement over several lines: the lens range covers the whole statement.
- [ ] A document with syntax errors, and a closed or unknown URI.

## Acceptance criteria
- [ ] Every item above has a test, or a note under "Follow-ups" saying why it cannot.
- [ ] Any wrong behaviour a new test exposes gets its own task file and index row; the test is
      added with `#[ignore = "<task id>"]`.

## Follow-ups

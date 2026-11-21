# M3-01 — Language server: core and diagnostics

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-lsp`, `datascript-diagnostics`

## Goal
`ds lsp` speaks LSP over stdio and publishes diagnostics as the user types.

## Scope
- `tower-lsp-server`, incremental text sync, a debounced re-check per document.
- `Diagnostic` → `lsp_types::Diagnostic` conversion (UTF-16 positions, related information from
  secondary labels, code descriptions linking to error docs).
- Syntax diagnostics instantly, semantic ones once the schemas resolve (cached, with file
  watching so a changed CSV invalidates the cache).

## Acceptance criteria
- [ ] In-process LSP tests: open/change/close produce the expected diagnostics.
- [ ] Re-check latency < 100 ms for a 500-line script with warm schemas (measured).

## Carried over
- [ ] (from M0-02) Diagnostic lines split on `\n` only; a lone `\r` is not a line break, while LSP treats it as one. Align the two.
- [ ] (from M1-07) The session caches schemas and failed lookups until told otherwise (D-043). Watch the workspace's data files and call `Session::invalidate_source` / `invalidate_schemas` when they change, then re-check open documents. Call `parse` and `complete` through `datascript_engine::catch_panic`.

## Follow-ups

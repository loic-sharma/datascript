# M3-01 — Language server: core and diagnostics

**Status:** done — `ds lsp` serves the LSP over stdio (new `datascript-lsp` crate): incremental sync, debounced checks with syntax errors first while schemas load, schema cache cleared on watched file changes, UTF-16 conversion with related information and code links (D-052).
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
- [x] In-process LSP tests: open/change/close produce the expected diagnostics.
      (`crates/datascript-lsp/tests/lsp.rs`: open/change/close, debounce, syntax first, file
      watching, `untitled:` documents; `ds lsp` over real stdio in `datascript-cli/tests/cli.rs`.)
- [x] Re-check latency < 100 ms for a 500-line script with warm schemas (measured).
      `recheck_latency_of_a_long_script`, change to publish with no debounce, 100 queries over
      `orders.csv` (Apple M-series, 2026-11-22): median 1.4 ms in release, 14.6 ms in debug. The
      test asserts < 100 ms in release builds and < 500 ms in debug builds.

## Carried over
- [x] (from planning, 2026-11-22) `docs/errors/DSxxxx.md` does not exist until M2-08. Until then, point each diagnostic's code description at its row in the DIAGNOSTICS.md code registry, built in one function so M2-08 can switch it.
- [x] (from M0-02) Diagnostic lines split on `\n` only; a lone `\r` is not a line break, while LSP treats it as one. Align the two.
- [x] (from M1-07) The session caches schemas and failed lookups until told otherwise (D-043). Watch the workspace's data files and call `Session::invalidate_source` / `invalidate_schemas` when they change, then re-check open documents. Call `parse` and `complete` through `datascript_engine::catch_panic`.

## Follow-ups
- Code links: `datascript_diagnostics::codes::docs_url` builds every link; it points at the
  DIAGNOSTICS.md code registry (table rows have no anchors). M2-08's carried-over item now names it.
- Lone `\r`: `SourceFile` ends a line at `\n`, `\r\n` or a lone `\r`, matching the LSP and the
  parser; renderers and the JSON form follow.
- Schema invalidation is whole-cache on any non-`.ds` file change (D-052). `parse` runs through
  `catch_panic`; `Session::check` already does. Calling `complete` through it is carried over
  to M3-02.
- Clients that cannot register file watchers get no invalidation until the server restarts; a
  failed registration is logged to the client.
- Handed over: a syntax-only mode for untrusted workspaces (M3-04), quick fixes from suggestions
  (already M3-03's scope).

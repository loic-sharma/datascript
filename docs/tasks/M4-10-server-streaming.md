# M4-10 — Local server (`ds ui`): WebSocket streaming runs & row-limit helper

**Status:** todo
**Depends on:** M4-01
**Crates:** `datascript-server`, `datascript-engine`, `datascript-diagnostics`
**Diagnostic codes:** DS4060–DS4064 (`DS4060` results truncated per D-066) (D-018)

## Goal
Add streaming query execution (`WS /api/ws`), a reusable streaming row-limit helper in
`datascript-engine`, and workspace file watching to `datascript-server` (export is M4-14) ([WEB_UI.md §3](../WEB_UI.md#3-execution-model--cross-cell-state),
[§6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-14-m4-04--m4-16)).

## Scope
- **Streaming row-limit helper** (`datascript-engine`, D-066):
  - `ExecContext::row_limit` is currently enforced only inside `ResultStream::collect`
    (`crates/datascript-engine/src/lib.rs`), which buffers. **Extract** that logic into a streaming
    helper that counts rows as batches arrive, slices the final batch at `row_limit`, drops the
    `QueryStream` on the next row to cancel execution, and reports `truncated = true`. Then make
    `collect` call the helper, so the CLI and every UI runtime share one implementation (reused by
    `datascript-server`, M3-06, and M5). The existing `row_limit_*` tests in
    `crates/datascript-engine/src/tests.rs` must pass unchanged.
- **WebSocket streaming execution & file watcher** (`WS /api/ws`, requiring the `ds_session_<port>` cookie +
  `Origin` check):
  - Executes a `run` request `{ cells, targetCellIndex, mode, statementIndex?, nowMicros? }` with
    `mode` = `"cell"` (every statement), `"statement"` (one cell-relative query `statementIndex`;
    single-statement stitching, so the cell's other query statements cannot block it), or
    `"bindings"` (validate the cell's `let`s; no queries, `DS0004` suppressed). It stitches with
    M4-08's `StitchedProgram` (every upstream `let`, then the part of the target cell the mode
    needs, [WEB_UI.md §3](../WEB_UI.md#upstream-let-prepending--ds1085)), runs it with
    `Session::start` + the streaming row-limit helper, and passes the diagnostics through M4-08's
    `DS1085` step.
  - Emits `started` carrying `nowMicros` (accepting an optional shared `nowMicros` for Run all) and
    `rowLimit`, then for each result its `statementIndex` and `sourceCount` (distinct `SourceRef`s
    found by walking `PlanNode::Scan` nodes in that query's `QueryPlan::root`; per result, not a
    union across the cell). `cellIndex`/`targetCellIndex` count every cell, Markdown included
    ([WEB_UI.md §2](../WEB_UI.md#serialization-grammar--canonical-layout)).
  - Streams Arrow IPC batches, `RenderSpec`, timing, and diagnostics (`DS4060` when truncated, with
    the message, span and help in [WEB_UI.md §3](../WEB_UI.md#result-size-limits--streaming-export-d-066-closes-q-007)
    — mark active in `codes.rs` & `DIAGNOSTICS.md`; **suppress `DS0004`** when validating a
    `let`-only cell).
  - Dropping/cancelling the run drops `RunStream` immediately (D-055).
  - Watches the workspace root, notifies connected clients of file changes, and invalidates
    `Session`'s schema cache. **Drops watch events caused by the server's own saves**: M4-01's
    `PUT` records the `version` it wrote per path, and an event whose resulting `version` matches is
    not broadcast, so auto-save never shows the "changed on disk" banner for its own writes.

## Acceptance criteria
- [ ] Unit tests for `datascript-engine`'s streaming row-limit helper (exact limit, slice across
      batch boundary, cancellation on next row).
- [ ] API/WS integration tests covering:
  - WS cookie + `Origin` auth, streaming batches, cancellation, `sourceCount`/`rowLimit`, `DS4060`
    truncation (default & custom `row_limit`), per-result `sourceCount` in a multi-statement
    cell, `DS1085` propagation (one `DS1085` for a broken upstream `let` even when the target does not
    use it; none for an upstream query error), a statement run succeeding while another query statement in the
    same cell has an error (and a whole-cell run of that cell blocked by it),
    `mode: "bindings"` validating a `let` while a half-typed query in the same cell is ignored,
    `DS0004` suppression on `let`-only cells, workspace file-watch invalidation,
    and **no** change notification after the server's own `PUT`.
- [ ] Exact JSON/WebSocket wire contract documented alongside [WEB_UI.md](../WEB_UI.md) (shared
      with M5 WASM).

## Carried over
- [ ] (from M2-11) The streaming API exists: `Session::start` returns a `RunStream` (`Send`, owns its engine and context) whose `next_query()` yields a `QueryStream` per query, batches arriving as they are produced, behind the panic boundary (D-055). Use it for streaming and cancellation over WebSocket (dropping it cancels the query) instead of building another.
- [ ] (from M1-07) `ExecContext::row_limit` is enforced by `collect` and sets `QueryResult::truncated` (D-043). Extend it with the streaming-limit helper above, implement D-066's `DS4060` truncation info diagnostic (from this task's block, DS4060–DS4064), and show it. Invalidate the session's schema cache when files under the root change, since failed lookups are cached too.

## Follow-ups

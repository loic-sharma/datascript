# M4-10 — Local server (`ds ui`): WebSocket streaming runs, row-limit helper & export

**Status:** todo
**Depends on:** M4-01
**Crates:** `datascript-server`, `datascript-engine`, `datascript-diagnostics`
**Diagnostic codes:** DS4060–DS4064 (`DS4060` results truncated per D-064) (D-018)

## Goal
Add streaming query execution (`WS /api/ws`), a reusable streaming row-limit helper in
`datascript-engine`, workspace file watching, and two-step zero-buffering streaming export to
`datascript-server` ([WEB_UI.md §3](../WEB_UI.md#3-execution-model--cross-cell-state),
[§6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-04--m4-05)).

## Scope
- **Streaming row-limit helper** (`datascript-engine`, D-064):
  - `ExecContext::row_limit` is currently enforced only inside `ResultStream::collect`
    (`crates/datascript-engine/src/lib.rs`), which buffers. **Extract** that logic into a streaming
    helper that counts rows as batches arrive, slices the final batch at `row_limit`, drops the
    `QueryStream` on the next row to cancel execution, and reports `truncated = true`. Then make
    `collect` call the helper, so the CLI and every UI runtime share one implementation (reused by
    `datascript-server`, M3-06, and M5). The existing `row_limit_*` tests in
    `crates/datascript-engine/src/tests.rs` must pass unchanged.
- **WebSocket streaming execution & file watcher** (`WS /api/ws`, requiring the `ds_session_<port>` cookie +
  `Origin` check):
  - Executes single cells (or a single cell-relative `statementIndex`) using M4-08's
    `StitchedProgram` (transitive upstream `let` prepending + `DS1085`; single-statement stitching
    for a `statementIndex` run, so the cell's other query statements cannot block it, per
    [WEB_UI.md §3](../WEB_UI.md#transitive-upstream-let-prepending--ds1085)) and `Session::start` +
    the streaming row-limit helper.
  - Emits `started` carrying `nowMicros` (accepting an optional shared `nowMicros` for Run all) and
    `rowLimit`, then for each result its `statementIndex` and `sourceCount` (distinct `SourceRef`s
    found by walking `PlanNode::Scan` nodes in that query's `QueryPlan::root`; per result, not a
    union across the cell). `cellIndex`/`targetCellIndex` count every cell, Markdown included
    ([WEB_UI.md §2](../WEB_UI.md#serialization-grammar--canonical-layout)).
  - Streams Arrow IPC batches, `RenderSpec`, timing, and diagnostics (`DS4060` when truncated, with
    the message, span and help in [WEB_UI.md §3](../WEB_UI.md#result-size-limits--streaming-export-d-064-closes-q-007)
    — mark active in `codes.rs` & `DIAGNOSTICS.md`; **suppress `DS0004`** when validating a
    `let`-only cell).
  - Dropping/cancelling the run drops `RunStream` immediately (D-055).
  - Watches the workspace root, notifies connected clients of file changes, and invalidates
    `Session`'s schema cache. **Drops watch events caused by the server's own saves**: M4-01's
    `PUT` records the `version` it wrote per path, and an event whose resulting `version` matches is
    not broadcast, so auto-save never shows the "changed on disk" banner for its own writes.
- **Two-step zero-buffering streaming export** (D-064):
  - `POST /api/export` (cookie + `Origin`) accepts the run-time cell snapshot
    `{ path, cells, targetCellIndex, statementIndex, nowMicros, format }` (`statementIndex` 0-based
    among the target cell's own query statements), checks, plans and starts the query (returning
    diagnostics and no ticket if that fails), and mints a single-use ticket
    `/api/export/<ticket>` holding the started stream (60 s expiry drops it).
  - `GET /api/export/<ticket>` requires the session cookie, verifies
    `Sec-Fetch-Site: same-origin` when present, consumes the ticket atomically, and streams the
    uncapped result via `datascript_engine::writers` as `Content-Disposition: attachment`. On a
    runtime or I/O error after the headers are sent it **aborts the connection** (never a clean
    end, so the browser marks the download failed) and sends an `exportFailed` WS event with the
    `Diagnostic` ([WEB_UI.md §6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-04--m4-05)).
  - **Formats.** v1 export offers `csv` and `ndjson`, the existing streaming row writers in
    `writers.rs`; this task adds no writer. `json` is added by the carried-over item below once
    M2-26 makes `write_json` stream (today it takes every batch at once), `parquet` once M2-27
    lands, and `xlsx` once M2-28 lands. `GET /api/info` lists the formats this server can write, and the UI shows only
    those.

## Acceptance criteria
- [ ] Unit tests for `datascript-engine`'s streaming row-limit helper (exact limit, slice across
      batch boundary, cancellation on next row).
- [ ] API/WS integration tests covering:
  - WS cookie + `Origin` auth, streaming batches, cancellation, `sourceCount`/`rowLimit`, `DS4060`
    truncation (default & custom `row_limit`), per-result `sourceCount` in a multi-statement
    cell, `DS1085` propagation (not on an unreferenced broken `let` or upstream
    query error), a statement run and an export succeeding while another query statement in the
    same cell has an error (and a whole-cell run of that cell blocked by it),
    `DS0004` suppression on `let`-only cells, workspace file-watch invalidation,
    and **no** change notification after the server's own `PUT`;
  - two-step streaming export reusing the run-time snapshot, `nowMicros` (`now()`/`ago()`),
    cell-relative `statementIndex`, cookie + one-shot ticket replay/expiry, and `Sec-Fetch-Site`;
  - export failures: a check or missing-source error returned by `POST /api/export` with no
    ticket, and an error mid-stream aborting the connection (the client sees an incomplete body,
    not a clean end) plus an `exportFailed` WS event;
  - each v1 format (`csv`, `ndjson`) streaming a multi-batch result larger than
    `row_limit`, read back and compared row for row.
- [ ] Exact JSON/WebSocket wire contract documented alongside [WEB_UI.md](../WEB_UI.md) (shared
      with M5 WASM).

## Carried over
- [ ] (from M2-11) The streaming API exists: `Session::start` returns a `RunStream` (`Send`, owns its engine and context) whose `next_query()` yields a `QueryStream` per query, batches arriving as they are produced, behind the panic boundary (D-055). Use it for streaming and cancellation over WebSocket (dropping it cancels the query) instead of building another.
- [ ] (from M1-07) `ExecContext::row_limit` is enforced by `collect` and sets `QueryResult::truncated` (D-043). Extend it with the streaming-limit helper above, implement D-064's `DS4060` truncation info diagnostic (from this task's block, DS4060–DS4064), and show it. Invalidate the session's schema cache when files under the root change, since failed lookups are cached too.
- [ ] (from planning, 2026-11-24) D-064's "export the full result" should stream through `datascript_engine::writers`, the same writers the `output` operator uses (M2-25…M2-27, M2-28 for Excel), so the export offers every format they can write. If M2-26 is done (`write_json` streams), add `json` to the export formats with a read-back test (M2-26's note covers it if it lands after this task). If M2-27 is done, add `parquet` likewise (M2-27's "Hosts" note covers it if it lands after this task); likewise `xlsx` for M2-28.

## Follow-ups

# M4-14 — Export full result (server & UI)

**Status:** todo
**Depends on:** M4-09
**Crates:** `datascript-server`, `web/`
**Diagnostic codes:** none (export failures reuse existing diagnostics)

## Goal
Let users export the full, uncapped result behind a truncated grid, streamed straight to disk
through `datascript_engine::writers` (D-066,
[WEB_UI.md §3](../WEB_UI.md#result-size-limits--streaming-export-d-066-closes-q-007),
[§6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-14-m4-04--m4-16)).
Split out of M4-10 and M4-09: export is one feature across server and UI, and it is off the path
to running a cell and seeing a grid.

## Scope
- **Server: two-step zero-buffering streaming export** (`datascript-server`, D-066):
  - `POST /api/export` (cookie + `Origin`) accepts the run-time cell snapshot
    `{ path, cells, targetCellIndex, statementIndex, nowMicros, format }` (`statementIndex` 0-based
    among the target cell's own query statements), stitches it exactly like a
    `mode: "statement"` run (M4-10), checks, plans and starts the query (returning diagnostics and no ticket if that fails), and mints a single-use ticket
    `/api/export/<ticket>` holding the started stream (60 s expiry drops it).
  - `GET /api/export/<ticket>` requires the session cookie, verifies
    `Sec-Fetch-Site: same-origin` when present, consumes the ticket atomically, and streams the
    uncapped result via `datascript_engine::writers` as `Content-Disposition: attachment`. On a
    runtime or I/O error after the headers are sent it **aborts the connection** (never a clean
    end, so the browser marks the download failed) and sends an `exportFailed` WS event with the
    `Diagnostic` ([WEB_UI.md §6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-14-m4-04--m4-16)).
  - **Formats.** v1 export offers `csv` and `ndjson`, the existing streaming row writers in
    `writers.rs`; this task adds no writer. `json` is added by the carried-over item below once
    M2-26 makes `write_json` stream (today it takes every batch at once), `parquet` once M2-27
    lands, and `xlsx` once M2-28 lands. Add `exportFormats` to `GET /api/info`: the formats this
    server can write.
- **UI: Export full result** (`web/`, D-066):
  - Capture the immutable cell snapshot (`cells`, `targetCellIndex`, `nowMicros`, cell-relative
    `statementIndex`) at run time and wire the **Export full result** menu to `POST /api/export` →
    `GET /api/export/<ticket>` (authenticated by session cookie + one-shot ticket; re-executes
    against current disk files), started from a programmatic `<a download>` click so the page
    never unloads; show `POST` diagnostics and `exportFailed` WS events in the cell footer.
  - The menu lists exactly `Runtime.capabilities.exportFormats` (from
    `GET /api/info`; v1: CSV, NDJSON; JSON, Parquet and `.xlsx` once M2-26 / M2-27 / M2-28 land),
    never a hard-coded list.

## Acceptance criteria
- [ ] API integration tests covering:
  - two-step streaming export reusing the run-time snapshot, `nowMicros` (`now()`/`ago()`),
    cell-relative `statementIndex`, cookie + one-shot ticket replay/expiry, and `Sec-Fetch-Site`;
  - an export succeeding while a neighbouring query statement in the target cell has an error;
  - export failures: a check or missing-source error returned by `POST /api/export` with no
    ticket, and an error mid-stream aborting the connection (the client sees an incomplete body,
    not a clean end) plus an `exportFailed` WS event;
  - `GET /api/info` returning `exportFormats` (`["csv", "ndjson"]` in v1, growing as the
    carried-over writers land);
  - each v1 format (`csv`, `ndjson`) streaming a multi-batch result larger than
    `row_limit`, read back and compared row for row.
- [ ] `exportFailed` added to the WebSocket wire contract doc that M4-10 writes.
- [ ] Vitest tests for run-time export snapshot capture (including after editing a cell into Stale
      state), and a Playwright test of a streaming CSV export download with the menu showing exactly
      the server's `exportFormats` (Parquet appears only when the server reports it).

## Carried over
- [ ] (from planning, 2026-11-24) D-066's "export the full result" should stream through `datascript_engine::writers`, the same writers the `output` operator uses (M2-25…M2-27, M2-28 for Excel), so the export offers every format they can write. If M2-26 is done (`write_json` streams), add `json` to the export formats with a read-back test (M2-26's note covers it if it lands after this task). If M2-27 is done, add `parquet` likewise (M2-27's "Hosts" note covers it if it lands after this task); likewise `xlsx` for M2-28.

## Follow-ups

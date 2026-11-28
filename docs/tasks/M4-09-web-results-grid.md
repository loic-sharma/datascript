# M4-09 — Web UI: virtualized Arrow results grid, multi-result tabs & export

**Status:** todo
**Depends on:** M4-02
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Build the decoupled `web/src/components/results/` package (reused directly by VS Code's M3-06
results webview) implementing the virtualized Arrow grid, multi-statement sub-tabs (D-063),
truncation-aware sorting + **Sort in query**, and two-step streaming export (D-064,
[WEB_UI.md §3–§4.2](../WEB_UI.md#42-center-pane--notebook-canvas)).

## Scope
- **Virtualized Arrow grid** (`apache-arrow` + `@tanstack/react-virtual`):
  - Row index gutter, compact monochrome column headers (type glyph + subtle null indicator whose
    click opens the right-hand profiler), italic `NULL` cell styling, column resizing, click-to-copy
    cell/row, and fullscreen expand.
- **Multi-statement sub-tabs** (D-063):
  - Compact output sub-tabs (`Result 1`, `Result 2`, …, or `render` title) whenever a cell run
    produces more than one result.
- **Truncation UX & "Sort in query"** (D-064):
  - `DS4060` truncation notice and client-side sort label using the run's actual `rowLimit`
    (`"Showing first {N} rows"` / `"sorted first {N} rows"`).
  - Column-header **Sort in query** action that inserts `| sort by <col> desc` (or `asc`, always
    explicit so `DS2103` never fires, D-005) **before any trailing `| render ...`** or updates an
    existing `sort` in place, then re-runs over all rows.
- **Two-step streaming export** (D-064):
  - Capture the immutable cell snapshot (`cells`, `targetCellIndex`, `nowMicros`, cell-relative
    `statementIndex`) at run time and wire the **Export full result** menu to `POST /api/export` →
    `GET /api/export/<ticket>` (authenticated by session cookie + one-shot ticket; re-executes
    against current disk files; CSV, JSON, NDJSON, Parquet, Arrow IPC).

## Acceptance criteria
- [ ] Vitest unit tests for Arrow cell formatting, "Sort in query" rewrites (including a query
      ending in `| render timechart`, updating an existing `sort`, and explicit `asc`/`desc`),
      dynamic truncation labels with a non-default `rowLimit`, and run-time export snapshot capture
      (including after editing a cell into Stale state).
- [ ] Playwright test against `ds ui` verifying virtualized scrolling, multi-statement sub-tabs,
      truncation banner + "Sort in query", and streaming CSV/Parquet export download.

## Follow-ups

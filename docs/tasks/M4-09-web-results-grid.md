# M4-09 — Web UI: cell execution, virtualized Arrow results grid, multi-result tabs & export

**Status:** todo
**Depends on:** M4-02, M4-10
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Wire cell execution over M4-10's WebSocket (`ServerRuntime.run`) and build the decoupled
`web/src/components/results/` package (reused directly by VS Code's M3-06 results webview)
implementing the virtualized Arrow grid, multi-statement sub-tabs (D-063),
truncation-aware sorting + **Sort in query**, and two-step streaming export (D-064,
[WEB_UI.md §3–§4.2](../WEB_UI.md#42-center-pane--notebook-canvas)).

## Scope
- **Execution & staleness** (moved from M4-02 so M4-02 and M4-10 can run in parallel):
  `ServerRuntime.run`/cancel over `WS /api/ws`; Run cell (`Cmd/Ctrl+Enter`), Run & advance
  (`Shift+Enter`), Run statement at cursor (`Cmd/Ctrl+Alt+Enter` / gutter button), Run cell &
  below / Run all (one shared `nowMicros`, stopping at the first failing cell), Cancel
  (`Cmd/Ctrl+.` / Stop button), per-result status line (`rows · duration · N sources`),
  transitive-`let` Stale badge, `let`-only validation line (`DS0004` suppressed), and `DS1085`
  cards whose secondary label jumps to the upstream cell.
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
    against current disk files), started from a programmatic `<a download>` click so the page
    never unloads; show `POST` diagnostics and `exportFailed` WS events in the cell footer. The menu lists the formats the server reports (v1: CSV, NDJSON,
    JSON, Arrow IPC; Parquet and `.xlsx` once M2-27 / M2-28 land).

## Acceptance criteria
- [ ] Vitest unit tests for transitive-`let` staleness fingerprinting, Run-statement-at-cursor
      selection, Arrow cell formatting, "Sort in query" rewrites (including a query
      ending in `| render timechart`, updating an existing `sort`, and explicit `asc`/`desc`),
      dynamic truncation labels with a non-default `rowLimit`, and run-time export snapshot capture
      (including after editing a cell into Stale state).
- [ ] Playwright test against `ds ui` verifying Run / Run all / Cancel, `let`-only cell showing no
      `DS0004` warning, `DS1085` jump, virtualized scrolling, multi-statement sub-tabs (each with
      its own source count), truncation banner + "Sort in query", and streaming CSV export
      download (plus Parquet if M2-27 is done).

## Carried over
- [ ] (from M2-03) If M2-03 is done, `dynamic` values exist: show them in the grid as JSON text,
      collapsed when long, with a Vitest test. (If M2-03 lands after this task, M2-03 does it.)

## Follow-ups

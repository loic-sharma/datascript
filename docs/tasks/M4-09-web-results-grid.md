# M4-09 — Web UI: cell execution, virtualized Arrow results grid & multi-result tabs

**Status:** todo
**Depends on:** M4-15, M4-10
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Wire cell execution over M4-10's WebSocket (`ServerRuntime.run`) and build the decoupled
`web/src/components/results/` package (reused directly by VS Code's M3-06 results webview)
implementing the virtualized Arrow grid, multi-statement sub-tabs (D-065),
and truncation-aware sorting + **Sort in query** (D-066; export is M4-14,
[WEB_UI.md §3–§4.2](../WEB_UI.md#42-center-pane--notebook-canvas)).

## Scope
- **Execution & staleness** (moved from M4-02 so M4-02 and M4-10 can run in parallel):
  `ServerRuntime.run`/cancel over `WS /api/ws`; Run cell (`Cmd/Ctrl+Enter`), Run & advance
  (`Shift+Enter`), Run statement at cursor (`Cmd/Ctrl+Alt+Enter` / gutter button), Run cell &
  below / Run all (one shared `nowMicros`, stopping at the first failing cell), Cancel
  (`Cmd/Ctrl+.` / Stop button), per-result status line (`rows · duration · N sources`),
  upstream-`let` Stale badge (own source plus every upstream `let`, the same `let`s a run compiles), `let`-only validation line (`DS0004` suppressed), and `DS1085`
  cards whose secondary label jumps to the upstream cell.
- **Live workspace sync** (needs M4-10's watcher): implement `Runtime.subscribeWorkspace` in
  `ServerRuntime` over `WS /api/ws` file-change events; refresh the file tree on additions,
  renames, and deletions; and show the "changed on disk" banner as soon as the open file changes
  externally, ignoring the server's own saves
  ([WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables)).
- **Virtualized Arrow grid** (`apache-arrow` + `@tanstack/react-virtual`):
  - Row index gutter, compact monochrome column headers (type glyph + subtle null indicator whose
    click opens the right-hand profiler), italic `NULL` cell styling, column resizing, click-to-copy
    cell/row, and fullscreen expand.
- **Multi-statement sub-tabs** (D-065):
  - Compact output sub-tabs (`Result 1`, `Result 2`, …, or `render` title) whenever a cell run
    produces more than one result.
- **Truncation UX & "Sort in query"** (D-066):
  - `DS4060` truncation notice and client-side sort label using the run's actual `rowLimit`
    (`"Showing first {N} rows"` / `"sorted first {N} rows"`).
  - Column-header **Sort in query** action that inserts `| sort by <col> desc` (or `asc`, always
    explicit so `DS2103` never fires, D-005) **before any trailing `| render ...`** or updates an
    existing `sort` in place, then re-runs over all rows.

## Acceptance criteria
- [ ] Vitest unit tests for upstream-`let` staleness fingerprinting, Run-statement-at-cursor
      selection, Arrow cell formatting, "Sort in query" rewrites (including a query
      ending in `| render timechart`, updating an existing `sort`, and explicit `asc`/`desc`),
      and dynamic truncation labels with a non-default `rowLimit`.
- [ ] Playwright test against `ds ui` verifying Run / Run all / Cancel, `let`-only cell showing no
      `DS0004` warning, `DS1085` jump, virtualized scrolling, multi-statement sub-tabs (each with
      its own source count), truncation banner + "Sort in query", live tree refresh after a file is added and
      deleted on disk, the "changed on disk" banner appearing without a save after an external
      edit, and **no** banner after the server's own auto-save.

## Carried over
- [ ] (from M2-03) If M2-03 is done, `dynamic` values exist: show them in the grid as JSON text,
      collapsed when long, with a Vitest test. (If M2-03 lands after this task, M2-03 does it.)

## Follow-ups

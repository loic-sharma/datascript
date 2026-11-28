# M4-02 — Web UI: notebook shell, `Runtime`, editor & execution (M4 v1)

**Status:** todo
**Depends on:** M4-01, M2-13
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Scaffold the shared React + Vite + Tailwind + CodeMirror 6 notebook SPA (`D-061`, `D-062`), wire
`Runtime` / `ServerRuntime`, and ship the M4 v1 workspace shell, notebook/script editing,
auto-save, and cell execution loop ([WEB_UI.md §1–§4.2](../WEB_UI.md)). The virtualized Arrow grid
and export UI land in M4-09; ECharts in M4-03; inspector & IDE completion in M4-04; power-user
tabs/shortcuts/file management in M4-05.

## Scope
- **Scaffold & `Runtime`**:
  - Initialize `web/` with React, TypeScript, Vite, Tailwind CSS, CodeMirror 6, monochrome SVG
    icons, and dark/light theme variables (`localStorage` preferences keyed by workspace root,
    including `row_limit`).
  - Implement the `Runtime` interface and `ServerRuntime` ([WEB_UI.md §6](../WEB_UI.md#the-runtime-abstraction-web-wasm-desktop-vs-code)),
    stripping `?token=` via `history.replaceState` on startup.
- **Notebook shell, file tree & Scratchpad**:
  - Left pane: read-only working-directory tree + **New Notebook (`.dsnb`)**, data-file
    click/double-click actions (insert `from '<relpath>'` / preview cell), and optional Attached
    Tables section ([WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables)).
  - Center pane: single active `.dsnb`/`.ds` view with path breadcrumb, Scratchpad empty-state
    quick-preview strip ([WEB_UI.md §2](../WEB_UI.md#plain-ds-scripts--scratchpads)), drag-and-drop
    cell reordering, and hover `+ DataScript` / `+ Markdown` dividers.
  - Right pane shell starting collapsed by default (populated in M4-04).
  - Load/save structured `.dsnb` cells directly via `Runtime` (no duplicate TS parser), with
    debounced auto-save (~500 ms), `Cmd/Ctrl+S` flush, and `409 Conflict` banner.
- **Cells, diagnostics & execution**:
  - **Markdown cells**: GFM rendered via DOMPurify with remote `http(s)://` images blocked by
    default; click/double-click edits in CodeMirror (`Cmd/Ctrl+Enter`, `Shift+Enter`, `Esc` per
    [WEB_UI.md §4.2](../WEB_UI.md#42-center-pane--notebook-canvas)).
  - **DataScript cells**: CodeMirror 6 with DataScript highlighting (D-026), whole-notebook live
    checking (`POST /api/check`), inline squiggles, and rustc/Claude-Code-style diagnostic cards
    with one-click **Apply fix** and `DS4061` upstream jump links.
  - **Execution & staleness**: Run cell (`Cmd/Ctrl+Enter`), Run & advance (`Shift+Enter`), Run
    statement at cursor (`Cmd/Ctrl+Alt+Enter` / gutter button per [WEB_UI.md §3](../WEB_UI.md#explicit-execution-staleness--clocks)),
    Run cell & below / Run all (one shared `nowMicros`), Cancel (`Cmd/Ctrl+.` / Stop button),
    cell status bar (`rows · duration · N sources`), Stale badge, and `let`-only validation line.
    Display a minimal placeholder table slot until M4-09 plugs in the virtualized Arrow grid.

## Acceptance criteria
- [ ] Vitest unit tests for `ServerRuntime`, token URL stripping, cross-cell staleness
      fingerprinting, Markdown remote-image sanitization, and Run-statement-at-cursor selection.
- [ ] Playwright smoke test against a real `ds ui`: Scratchpad quick-preview, multi-cell `.dsnb`
      with shared `let` (`sourceCount` & `DS4061` jump), drag reorder, auto-save, and `409` banner.
- [ ] Dark and light themes work cleanly with monochrome icons and keyboard accessibility.

## Follow-ups

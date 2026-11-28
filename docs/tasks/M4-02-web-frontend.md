# M4-02 — Web UI: scaffold, `Runtime`, notebook shell & auto-save (M4 v1)

**Status:** todo
**Depends on:** M4-01, M2-13
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Scaffold the shared React + Vite + Tailwind + CodeMirror 6 notebook SPA (`D-063`, `D-064`), wire
`Runtime` / `ServerRuntime`, and ship the M4 v1 workspace shell: file tree, Scratchpad, a cell
list with plain CodeMirror editors, and auto-save ([WEB_UI.md §1–§4.2](../WEB_UI.md)). This task
needs only M4-01's HTTP API, so it runs in parallel with M4-10 (WebSocket runs). Cell editing
proper (DataScript mode, Markdown rendering, reordering, live checking) lands in M4-15; running
cells and the grid in M4-09; export in M4-14; ECharts in M4-03; inspector & IDE completion in
M4-04; keyboard power features in M4-05, file management in M4-16, tabs and Outline/Scope in
M4-17. `Runtime.run` is declared here but `ServerRuntime.run` is implemented in M4-09.

## Scope
- **Scaffold & `Runtime`**:
  - Initialize `web/` with React, TypeScript, Vite, Tailwind CSS, CodeMirror 6, monochrome SVG
    icons, and dark/light theme variables (preferences loaded from `GET /api/info` and persisted
    via `PATCH /api/prefs`, including `row_limit` and theme).
  - Implement the `Runtime` interface and `ServerRuntime` ([WEB_UI.md §6](../WEB_UI.md#the-runtime-abstraction-web-wasm-desktop-vs-code))
    using cookie session auth (verifying page reload preserves the session and SPA works
    under the server's strict CSP header).
- **Notebook shell, file tree & Scratchpad**:
  - Left pane: read-only working-directory tree + **New Notebook (`.dsnb`)**, data-file actions
    inserting paths **relative to the active notebook's parent directory** (`../data/orders.parquet`
    inside `notebooks/sales.dsnb`, [WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables)),
    and optional Attached Tables section.
  - Center pane: single active `.dsnb`/`.ds` view with path breadcrumb, Scratchpad empty-state
    quick-preview strip (Save defaults to workspace root, warning if saved into a subfolder,
    [WEB_UI.md §2](../WEB_UI.md#plain-ds-scripts--scratchpads)), and a cell list (add and delete
    DataScript and Markdown cells, each a plain CodeMirror editor; M4-15 adds the real editing).
  - Right pane shell starting collapsed by default (populated in M4-04).
  - Load/save structured `.dsnb` cells (`{ kind, text }`) directly via `Runtime` (no duplicate
    TS parser), with debounced auto-save (~500 ms), `Cmd/Ctrl+S` flush, and `409 Conflict` banner.
- **Saving**:
  - Use the fresh `version` returned by every `PUT /api/file` for the next auto-save, keeping **at most one `PUT` per path in flight** and merging edits made meanwhile
    into one follow-up save ([WEB_UI.md §2](../WEB_UI.md#serialization-grammar--canonical-layout)).

## Acceptance criteria
- [ ] Vitest unit tests for `ServerRuntime` (including edits made while a slow `PUT` is in
      flight producing exactly one follow-up `PUT` with the fresh `version` and no `409`, a timed-out `PUT`
      resent with the same cells and old `version`, with newer edits sent only after it answers,
      and no banner whether or not the original had landed), notebook-relative
      path calculation (root notebook, nested subfolder notebook, Scratchpad save-location
      warning).
- [ ] Playwright smoke test against a real `ds ui`: verifies strict CSP header, page reload
      retaining session cookie, two `ds ui` instances open at once both staying signed in,
      multi-cell `.dsnb` in a subfolder with relative `../` path insertion, two consecutive auto-saves of different cells leaving untouched
      cells byte-identical, and the `409` banner when saving after an external edit. (Watch-driven
      behaviour, including no banner after the server's own save, is tested in M4-09.)
- [ ] Dark and light themes both work, with monochrome icons.

## Follow-ups

# M4-02 — Web UI: notebook shell, `Runtime`, editor & live checking (M4 v1)

**Status:** todo
**Depends on:** M4-01, M2-13
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Scaffold the shared React + Vite + Tailwind + CodeMirror 6 notebook SPA (`D-061`, `D-062`), wire
`Runtime` / `ServerRuntime`, and ship the M4 v1 workspace shell, notebook/script editing,
auto-save, and whole-notebook live checking ([WEB_UI.md §1–§4.2](../WEB_UI.md)). This task needs
only M4-01's HTTP API, so it runs in parallel with M4-10 (WebSocket runs). Running cells, the
virtualized Arrow grid and export land in M4-09; ECharts in M4-03; inspector & IDE completion in
M4-04; power-user tabs/shortcuts/file management in M4-05. `Runtime.run` is declared here but
`ServerRuntime.run` is implemented in M4-09.

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
    [WEB_UI.md §2](../WEB_UI.md#plain-ds-scripts--scratchpads)), drag-and-drop cell reordering, and
    hover `+ DataScript` / `+ Markdown` dividers.
  - Right pane shell starting collapsed by default (populated in M4-04).
  - Load/save structured `.dsnb` cells with opaque `originId` directly via `Runtime` (no duplicate
    TS parser), with debounced auto-save (~500 ms), `Cmd/Ctrl+S` flush, and `409 Conflict` banner.
- **Cells & diagnostics**:
  - **Markdown cells**: GFM rendered via DOMPurify (no external `http(s)://` or relative workspace
    images in v1; `data:`/`blob:` only); click/double-click edits in CodeMirror (`Cmd/Ctrl+Enter`,
    `Shift+Enter`, `Esc` per [WEB_UI.md §4.2](../WEB_UI.md#42-center-pane--notebook-canvas)).
  - **DataScript cells**: CodeMirror 6 with DataScript highlighting (D-026), whole-notebook live
    checking (`POST /api/check`), inline squiggles, and rustc/Claude-Code-style diagnostic cards
    with one-click **Apply fix** and `DS1085` upstream jump links.
  - Swap in the fresh `originId`s and `version` returned by every `PUT /api/file` before the next
    auto-save, keeping **at most one `PUT` per path in flight** and merging edits made meanwhile
    into one follow-up save ([WEB_UI.md §2](../WEB_UI.md#serialization-grammar--canonical-layout)).

## Acceptance criteria
- [ ] Vitest unit tests for `ServerRuntime` (including `originId` swap after save, and edits made
      while a slow `PUT` is in flight producing exactly one follow-up `PUT` with the fresh
      `originId`s and no `409`), notebook-relative
      path calculation (root notebook, nested subfolder notebook, Scratchpad save-location
      warning), and Markdown sanitization.
- [ ] Playwright smoke test against a real `ds ui`: verifies strict CSP header, page reload
      retaining session cookie, two `ds ui` instances open at once both staying signed in,
      multi-cell `.dsnb` in a subfolder with relative `../` path insertion, cross-cell live
      diagnostics, drag reorder, two consecutive auto-saves of different cells leaving untouched
      cells byte-identical, no conflict banner after the server's own save, and the `409` banner
      after an external edit.
- [ ] Dark and light themes both work, with monochrome icons and keyboard accessibility.

## Follow-ups

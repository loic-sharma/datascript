# M4-02 — Web UI: notebook shell, `.dsnb` engine, editor & results grid

**Status:** todo
**Depends on:** M4-01, M2-13
**Crates:** `web/`

## Goal
Build the shared React + Vite + Tailwind + CodeMirror 6 notebook SPA specified in
[WEB_UI.md](../WEB_UI.md) (D-061–D-064), ready for `ds ui`, the M5 WASM playground, VS Code's M3-06
results webview, and a future Tauri desktop shell.

## Scope
- **Architecture & runtime layer**:
  - Scaffold `web/` with React, TypeScript, Vite, Tailwind CSS, CodeMirror 6, `apache-arrow`, and
    `@tanstack/react-virtual`.
  - Define the `Runtime` TypeScript interface ([WEB_UI.md §6](../WEB_UI.md#the-runtime-abstraction-web-wasm-desktop-vs-code))
    and implement `ServerRuntime` (HTTP + WebSocket client + two-step ticketed export download
    against M4-01).
  - Keep result rendering primitives (`web/src/components/results/`: virtualized Arrow grid and
    diagnostic cards) decoupled from the notebook shell so M3-06 can import them directly.
- **`.dsnb` notebook parser & lossless serializer** (D-062):
  - Parse source-only plain-text Markdown `.dsnb` files (YAML frontmatter preserved, ` ```ds `
    fences → DataScript cells, prose → Markdown cells, `<!-- ds:cell -->` adjacent Markdown
    delimiter) with round-trip unit tests.
  - Open plain `.ds` scripts as single-editor tabs, and open an unsaved **Scratchpad** notebook tab
    on launch when no notebook is open.
  - Debounced auto-save (~500 ms) + `Cmd/Ctrl+S` flush for saved files, plus `409 Conflict`
    external-edit banner (`Reload from disk` / `Overwrite disk`).
- **Three-pane layout & dark/light theming**:
  - Calm, dark-first Claude-Code-inspired monochrome theme + crisp light theme with semantic CSS
    variables and collapsible/resizable left and right sidebars.
  - **Left pane**: full working-directory tree (hiding `.git`, `node_modules`, `target`, dotfiles by
    default, with filter box & toggle), create/rename/delete for `.dsnb`, `.ds`, and safe folders,
    read-only data files (double-click / action inserts `from '<relpath>'` or `'<relpath>'` in table
    position, D-036, or creates a preview cell), and optional Attached Tables section.
  - **Center pane**: slim tab bar + path breadcrumb keeping open tabs' outputs, scroll position,
    and undo history alive; cell list with pointer drag-and-drop handles and hover `+ DataScript` /
    `+ Markdown` insert dividers (keyboard reordering in Command Mode lands in M4-05).
  - **Right pane shell**: collapsible inspector frame (populated in M4-04).
- **Cells, diagnostics & virtualized results grid**:
  - **Markdown cells**: rendered cleanly (GFM, sanitized HTML); click/double-click edits in
    CodeMirror; `Cmd/Ctrl+Enter` renders in place, `Shift+Enter` renders & advances, `Esc` renders
    & leaves Edit Mode.
  - **DataScript cells**: CodeMirror 6 editor with DataScript syntax highlighting (matching D-026),
    inline diagnostic squiggles, and rustc/Claude-Code-style diagnostic cards with one-click
    **Apply fix** buttons for machine-applicable `Suggestion`s (including jump links for `DS4061`
    upstream cell errors).
  - **Execution & staleness**: Run cell (`Cmd/Ctrl+Enter`), Run & advance (`Shift+Enter`), Run
    statement at cursor (`Cmd/Ctrl+Alt+Enter`), Run cell & below, Run all, Stop/cancel
    (`Cmd/Ctrl+.` or Stop button); cross-cell dependency fingerprinting with subtle **Stale** badge;
    `let`-only validation status line; compact output sub-tabs for multi-statement cells (D-063).
  - **Virtualized Arrow results grid**: row numbers, column type badges, `NULL` styling, column
    resizing, client-side sort, click-to-copy cell/row, fullscreen expand (`[ ]`), `DS4060`
    truncation banner (10k default, D-064), and streaming **Export full result** menu (passing
    `nowMicros`, `statementIndex`, and upstream `let` context).

## Acceptance criteria
- [ ] Vitest unit tests for `.dsnb` lossless round-trip parsing/serialization, cross-cell staleness
      fingerprinting, `ServerRuntime`, and virtualized grid formatting.
- [ ] Playwright smoke test against a real `ds ui`: opens Scratchpad, inspects/inserts a fixture
      CSV from the left tree, runs a multi-cell notebook with shared `let`, reorders cells, saves
      as `.dsnb`, and exports results.
- [ ] Dark and light themes work cleanly with no contrast regressions. Keyboard accessible.

## Follow-ups

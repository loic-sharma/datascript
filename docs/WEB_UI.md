# DataScript Web UI Design Specification

This document specifies the design, UX, notebook format, execution semantics, API contract, and
front-end architecture for DataScript's local web UI (`ds ui`, Milestone M4). The same React front
end (`web/`) also powers the in-browser WASM playground (M5), shares its result grid and chart
components with the VS Code results panel (M3-06), and is structured to wrap cleanly in a native
desktop shell (Tauri) later without rewriting the UI.

Related documents: [ARCHITECTURE.md](ARCHITECTURE.md), [DECISIONS.md](DECISIONS.md) (D-060–D-063),
[LANGUAGE.md](LANGUAGE.md), [DIAGNOSTICS.md](DIAGNOSTICS.md), and
[tasks/README.md](tasks/README.md).

---

## 1. Product Vision & Aesthetic Direction

DataScript's web UI is a **notebook-first, local-files-first analytical workbench** that blends the
best ergonomics of five tools:

| Inspiration | What we take |
|---|---|
| **DuckDB UI** | Minimal notebook cells, execution timing summaries (`4,589 rows in 105ms`), virtualized data grid with column type badges & `NULL` styling, live right-hand column profiler (distinct counts, null %, distributions), fullscreen expand (`[ ]`), one-click copy/export |
| **Obsidian** | Local working-directory tree, collapsible sidebars, debounced auto-save, Quick Open (`Cmd/Ctrl+P`), Command Palette, Markdown prose cells, document outline, calm distraction-free typography |
| **VS Code** | Slim multi-file tab bar, LSP-grade inline diagnostics/squiggles with one-click Quick Fixes, rich autocomplete/hover/signature help, keyboard-first workflows |
| **Azure Data Explorer (ADX)** | Pipe-based exploration ergonomics, language-native `| render` visualizations directly below the query with Chart/Table toggle, optional run-statement-at-cursor shortcut |
| **Claude Code** | Calm, high-craft terminal aesthetic: muted dark-first monochrome surfaces, sharp typography, minimal chrome, subtle inline keyboard hints (`⌘↵`, `⎋`), and clear diff/quick-fix cards for diagnostics |

### Visual principles

- **Dark-first, dual-theme.** Deep charcoal/neutral surfaces (not pure `#000`), hairline 1px
  borders, restrained accent colors reserved for active focus, execution state, and data viz; paired
  with a crisp, high-contrast light theme.
- **Typography-led hierarchy.** Crisp monospace for DataScript code, table cells, column types,
  timings, and diagnostic snippets; clean sans-serif for prose, headings, sidebar labels, and UI
  controls. Tabular numerals (`font-variant-numeric: tabular-nums`) in every grid cell, row count,
  and profiler stat.
- **Low chrome, progressive disclosure.** Cell action bars, drag handles, and insert dividers stay
  quiet until hover or focus. Keybindings appear as subtle keycap hints (`⌘↵`, `⇧↵`, `⎋`) where
  relevant rather than cluttering toolbars with icons.
- **Diagnostics as a first-class UI surface.** Errors and warnings render both inline in the
  CodeMirror editor (squiggles + hover tooltip) and as clean, rustc/Claude-Code-style diagnostic
  cards below the cell editor—complete with code (`DS1001`), span context, `help`/`note` lines, and
  one-click **Apply fix** buttons for machine-applicable `Suggestion`s (D-025).

---

## 2. Notebook File Format (`.dsnb`)

Notebooks are stored on disk as **strictly source-only, plain-text Markdown files** with the
`.dsnb` extension (D-061).

### Why `.dsnb` Markdown?

- **Dedicated extension** gives clean OS/desktop file associations later and unambiguous discovery
  in repos and tooling without colliding with `README.md` or `docs/*.md`.
- **Plain-text Markdown inside** keeps notebooks human-readable, effortless to diff/review in git,
  and trivial to export to `.md`.
- **Strictly source-only**: query outputs, row counts, and chart images are never written into
  `.dsnb` files, keeping git diffs spotless.

### Serialization grammar

A `.dsnb` file is a sequence of cells parsed from and serialized to UTF-8 Markdown:

1. **Optional YAML frontmatter** at the top of the file (`--- ... ---`) is preserved verbatim on
   round-trip (usable for title, tags, or future metadata).
2. **DataScript cells** are fenced code blocks tagged with `ds` (or `datascript`):
   ````markdown
   ```ds
   let orders = from 'data/orders.parquet' | where status == "paid";
   ```
   ````
   Adjacent ` ```ds ` blocks parse directly as separate DataScript cells.
3. **Markdown cells** are prose blocks between fenced ` ```ds ` blocks.
   - Non-`ds` fenced code blocks (e.g. ` ```json `, ` ```bash `) inside prose stay part of the
     Markdown cell.
   - Two adjacent Markdown cells are delimited by an HTML comment marker on its own line:
     ```markdown
     <!-- ds:cell -->
     ```
   - Optional lightweight cell metadata (only emitted when non-default, e.g. collapsed editor) may
     be attached via fence info or an HTML comment (`<!-- ds:cell collapsed -->`) without breaking
     standard Markdown readability.
4. **Lossless round-trip.** Unmodified cells preserve their original text and line endings; saving
   only updates changed cells so git diffs stay minimal.

### Plain `.ds` scripts & Scratchpads

- **`.ds` files**: Opening any `.ds` file from the left tree opens it in a tab as a single-editor
  DataScript script view with the exact same diagnostics, completion, Run controls, result grid,
  charts, and right-hand inspector.
- **Launch Scratchpad**: When `ds ui` opens in a directory with no open notebooks (e.g. a folder of
  raw CSV/Parquet files), it immediately opens an unsaved **Scratchpad** notebook tab with one
  empty DataScript cell so the user can query right away. Pressing `Cmd/Ctrl+S` prompts for a
  filename and saves it as `<name>.dsnb` in the workspace.

### Saving & external edits

- **Saved files on disk** (`.dsnb`, `.ds`) use **debounced auto-save** (~500 ms after edits stop,
  Obsidian-style), with full undo/redo history and immediate flush on `Cmd/Ctrl+S`.
- **Scratchpad tabs** stay in memory until explicitly saved.
- **External-change protection**: Writes carry the last-known file version/etag (mtime + content
  hash). If the file changed on disk (e.g. edited in VS Code or switched by `git checkout`), auto-save
  pauses and displays a clear non-destructive banner (`Reload from disk` / `Overwrite disk`).

---

## 3. Execution Model & Cross-Cell State

### Top-to-bottom lexical `let` scope (D-061)

DataScript `let` bindings are immutable and lexically scoped. A notebook behaves exactly like a
single DataScript program split into cells:

- Every DataScript cell automatically sees all scalar and tabular `let` declarations from
  DataScript cells **above it** in document order.
- A cell does **not** need its upstream `let` cells to be executed first for checking, completion,
  hover, or execution—running Cell *k* compiles and executes Cell *k* with all upstream `let`
  statements prepended (using a combined-program span mapper modeled after `ReplProgram` in
  `datascript-cli`).
- **Live cross-cell IDE intelligence**: Editing a `let` in Cell 1 immediately updates inferred
  schemas, diagnostics, column completion, and hover info in Cells 2, 3, … on the next debounced
  check.
- **Reordering cells** immediately updates scope and diagnostics deterministically—there is never
  hidden Jupyter-style execution-order state.
- **No shadowing across cells**: Re-declaring an existing `let` name in a lower cell reports
  DataScript's standard `DS1003` diagnostic with a secondary label pointing back to the earlier
  cell where the name was defined.

> **Upstream error handling:** If an upstream `let` cell has a syntax or semantic error, that error
> is shown on the upstream cell (not duplicated as noise on every downstream cell), while downstream
> cells that reference the broken binding indicate the upstream error clearly and link directly to
> that cell.

### Explicit execution with staleness tracking

- Queries **never auto-run** on edit or reorder (protecting users on large local Parquet/CSV
  datasets).
- Each cell tracks a fingerprint of its own source plus the upstream `let` definitions it depends
  on. When either changes after a cell has produced output, the output header shows a subtle
  **Stale** indicator.
- Execution actions:
  - **Run cell** (`Cmd/Ctrl+Enter`) — runs all query statements in the focused cell.
  - **Run cell & advance** (`Shift+Enter`) — runs the focused cell and moves focus to the next cell
    (inserting a new DataScript cell if at the bottom).
  - **Run statement at cursor** (`Cmd/Ctrl+Alt+Enter`) — ADX-style escape hatch for multi-statement
    cells and `.ds` scripts.
  - **Run cell & below** / **Run all** — executes cells sequentially top-to-bottom, stopping on the
    first error cell (or continuing on request).
  - **Cancel / Stop** (`Esc` while running, or the inline Stop button) — cancels the active
    `RunStream` immediately (D-055).

### `let`-only cells & multi-statement cells (D-062, closes Q-004)

- **`let`-only cell**: Running a cell that defines only `let` statements checks/validates its
  bindings and displays a compact status line (`2 bindings defined · validated in 3ms`) with no
  empty table clutter.
- **Multi-statement cell**: Running a cell with multiple query statements renders compact output
  sub-tabs (`Result 1`, `Result 2`, … — labelled with the `render` title if present) inside the
  cell's output frame.

### Result size limits & streaming export (D-063, closes Q-007)

- UI executions pass `row_limit = 10_000` by default (configurable per-workspace or per-run in UI
  settings).
- When `QueryResult::truncated` is true, the runtime emits the Q-007 info diagnostic (`DS4060`) and
  the cell footer renders a subtle truncation notice (e.g. `Showing first 10,000 rows · truncated`)
  with a one-click **Export full result** menu.
- **Export full result** re-executes and streams the complete, uncapped query directly through
  `datascript_engine::writers` (CSV, JSON, NDJSON, Parquet, Arrow IPC, and Excel once M2-28 lands)
  as a streamed download—never buffering the full dataset in browser memory.

---

## 4. Three-Pane Workspace Layout

```
┌──────────────────────┬─────────────────────────────────────────────┬──────────────────────────┐
│  [Files]       [Q][+]│ [sales.dsnb ×] [Scratchpad ×]           [+] │ [Profile] [Outline] [Let]│
│ ──────────────────── │  data / sales.dsnb               ▶ Run all  │ ──────────────────────── │
│  🔍 Filter files...  │ ┌─────────────────────────────────────────┐ │  4,589 Rows · 4 Columns  │
│  ▾ data/             │ │ # Monthly Revenue Analysis              │ │  🔍 Filter columns...    │
│      orders.parquet  │ │ Exploring top customers by region.      │ │  ─────────────────────── │
│      customers.csv   │ └─────────────────────────────────────────┘ │  T   id          4.59K - │
│  ▾ notebooks/        │ ┌─────────────────────────────────────────┐ │  123 downloads   null 88%│
│    ● sales.dsnb      │ │ ▶  let orders = from 'data/orders...';  │ │  T   region        4 - │
│      adhoc.ds        │ │    validated · 1 binding (4ms)          │ │  📅  month        12 - │
│                      │ └─────────────────────────────────────────┘ │                          │
│  ATTACHED TABLES     │ ┌─────────────────────────────────────────┐ │  (Switches to Source     │
│    live_events       │ │ ▶  from orders                          │ │   Schema when a data     │
│                      │ │    | summarize rev = sum(amount) by ... │ │   file is selected in    │
│                      │ │    | render timechart                   │ │   the left tree)         │
│                      │ │  4,589 rows in 105ms  [Chart|Table] [⤢] │ │                          │
│                      │ │  ┌───────────────────────────────────┐  │ │                          │
│                      │ │  │       (ECharts / Arrow Grid)      │  │ │                          │
│                      │ │  └───────────────────────────────────┘  │ │                          │
│                      │ └─────────────────────────────────────────┘ │                          │
└──────────────────────┴─────────────────────────────────────────────┴──────────────────────────┘
```

All three panes have smooth drag-resizable splitters, and both sidebars collapse cleanly via
shortcuts (`Cmd/Ctrl+B` left, `Cmd/Ctrl+Alt+B` right) or top-bar icons.

### 4.1 Left Pane — Working Directory Tree & Attached Tables

- **Full working directory view**: Displays the directory passed to `ds ui [dir]` (default `.`).
  Hidden files and common clutter (`.git`, `node_modules`, `target`, `.DS_Store`) are hidden by
  default with a quick toggle and live filename filter box.
- **Visual distinction**: Clear, restrained file icons distinguish `.dsnb` notebooks, `.ds` scripts,
  supported data files (`.parquet`, `.csv`, `.tsv`, `.json`, `.jsonl`, `.ndjson`, `.xlsx`),
  folders, and other files (subtly dimmed).
- **Safe file CRUD** (confined strictly to the workspace root, D-016):
  - Users can **create, rename, and delete** `.dsnb` notebooks, `.ds` scripts, and folders from the
    sidebar (toolbar `+` button, context menu, or Command Palette).
  - **Data files are strictly read-only** in the UI—never deleted or modified from the tree.
- **Data file interactions**:
  - **Single-click** selects the file and opens its **Source Schema** in the right-hand inspector
    (columns, DataScript/Arrow types, file size).
  - **Double-click** (or inline hover action) inserts `from '<relpath>'` (or `'<relpath>'` when the
    cursor is already in a table position, D-036) into the active cell, or creates a new preview
    cell (`from '<relpath>' | take 100`).
- **Attached Tables section**: Shown only when host tables are registered (e.g. `ds ui --table
  orders=data/orders.parquet`), listing their names and schemas (DuckDB UI style).
- **Live workspace sync**: File additions, renames, and deletions on disk update the tree and
  invalidate `Session`'s schema cache automatically.

### 4.2 Center Pane — Tabbed Notebook Canvas

- **Slim tab bar + breadcrumb**: Multiple `.dsnb` notebooks, `.ds` scripts, and Scratchpads can stay
  open simultaneously. Switching tabs preserves each tab's rendered outputs, scroll position, active
  cell selection, and undo history. Supports tab pinning, close (`Cmd/Ctrl+W`), and Quick Open
  (`Cmd/Ctrl+P`).
- **Cell list & reordering**:
  - Every cell has a quiet left gutter drag handle (for pointer drag-and-drop reordering) and supports
    keyboard reordering (`Alt+Up` / `Alt+Down`).
  - Subtle `+ DataScript  + Markdown` insert dividers appear between cells on hover or at the end
    of the notebook.
- **Markdown cells**:
  - Rendered cleanly by default (GFM: headings, tables, task lists, inline code, links, code blocks).
  - Click/double-click or press `Enter` (in Command Mode) to edit raw Markdown in CodeMirror; press
    `Shift+Enter` or `Esc` to render back.
- **DataScript cells**:
  - **Editor**: CodeMirror 6 with DataScript syntax highlighting (matching the TextMate/lexer rules
    in D-026), inline diagnostic squiggles, hover documentation, signature help, and context-aware
    autocomplete backed by `datascript_compiler::ide`.
  - **Cell header / status bar**: Compact Play/Stop button, execution duration & row count
    (`4,589 rows · 105ms`), Stale badge (when applicable), Chart/Table toggle (for `| render`
    queries), fullscreen expand (`[ ]`), and overflow menu (duplicate, split, copy query, collapse
    output, delete).
  - **Diagnostics panel**: Renders any compiler or runtime `Diagnostic`s in rustc/Claude-Code style
    with clickable **Apply fix** buttons for machine-applicable `Suggestion`s.
  - **Tabular output (virtualized Arrow grid)**:
    - Reads Arrow IPC `RecordBatch`es directly via `apache-arrow` with row and column virtualization
      (`@tanstack/react-virtual`).
    - Row index gutter, column header type badges (`T` string, `123` long/int/real/decimal, `📅`
      datetime, `⏱` timespan, `✓` bool, `{}` dynamic), subtle italic `NULL` styling, column
      resizing, client-side column sort (on loaded rows), click-to-copy cell/row, and quick export
      menu.
  - **Visualization output (`| render`)**:
    - Queries ending with `| render <kind>` default to Chart view with an instant **Chart / Table**
      toggle and PNG/SVG download.
    - Supports all 9 render kinds in [LANGUAGE.md §6](LANGUAGE.md#6-render): `table`, `timechart`,
      `linechart`, `areachart`, `barchart` (horizontal), `columnchart` (vertical), `piechart`,
      `scatterchart`, and `card` (KPI summary card).
    - Plain table queries without `| render` display Table view only; a quiet **Visualize (`| render`)**
      helper can insert a starter `| render ...` line into the query so the DataScript code always
      remains the single source of truth.
    - Unrenderable specs (e.g. >500 distinct series or empty chart data) degrade gracefully with a
      clear explanatory banner and one-click switch to Table view—never hanging the tab.

### 4.3 Right Pane — Contextual Inspector (Collapsible)

The right pane provides three slim tabs, automatically switching to the most relevant context while
allowing manual override:

1. **Inspector (default contextual tab)**:
   - **When a cell with results is focused → Live Result Profile (DuckDB UI style)**:
     - Header: total rows (with truncation note if capped), column count, copy-schema button, column
       search input, and sort selector (`Default`, `Name`, `Null %`, `Distinct`).
     - Per-column row: type icon, column name, distinct count, null % pill, and compact distribution
       summary (inline sparkbar / top values / min–max on expand).
     - Computed **client-side** over the cell's loaded Arrow `RecordBatch`es, so it works with zero
       extra server round-trips in `ds ui`, WASM, and desktop alike.
   - **When a data file or attached table is selected in the left tree → Source Schema**:
     - File path, format, size, and searchable column list (`name`, DataScript type, and underlying
       Arrow type when projection-only).
     - Quick actions: **Insert `from '...'`**, **New preview cell (`take 100`)**, and **Describe**
       (`from '...' | describe`, wired once M2-23 lands).
2. **Outline (Obsidian style)**:
   - Interactive jump list of Markdown headings (`#`, `##`, `###`) and DataScript cells (showing
     defined `let` names or `render` title / first line) for fast navigation in long notebooks.
3. **Scope (`let`s)**:
   - Lists every scalar `let` (name, type, constant value if known) and tabular `let` (name,
     inferred output columns & types, defining cell link) visible to the currently focused cell.

---

## 5. Keyboard Shortcuts & Command Mode

The UI uses a **hybrid modeless + cell Command Mode** model (D-061). Every action is also searchable
from the Command Palette.

### Global & Modeless Shortcuts (work anywhere)

| Action | macOS | Windows / Linux |
|---|---|---|
| Run current cell | `Cmd+Enter` | `Ctrl+Enter` |
| Run cell & advance (or insert below) | `Shift+Enter` | `Shift+Enter` |
| Run statement at cursor (ADX style) | `Cmd+Alt+Enter` | `Ctrl+Alt+Enter` |
| Run all cells | `Cmd+Shift+Enter` | `Ctrl+Shift+Enter` |
| Reorder cell up / down | `Alt+Up` / `Alt+Down` | `Alt+Up` / `Alt+Down` |
| Command Palette | `Cmd+Shift+P` or `Cmd+K` | `Ctrl+Shift+P` or `Ctrl+K` |
| Quick Open file / notebook | `Cmd+P` | `Ctrl+P` |
| Toggle left sidebar (Files) | `Cmd+B` | `Ctrl+B` |
| Toggle right sidebar (Inspector) | `Cmd+Alt+B` | `Ctrl+Alt+B` |
| Save immediately / Save Scratchpad | `Cmd+S` | `Ctrl+S` |
| Close active tab | `Cmd+W` | `Ctrl+W` |
| New Scratchpad notebook | `Cmd+Shift+N` | `Ctrl+Shift+N` |
| Toggle line comment (in cell editor) | `Cmd+/` | `Ctrl+/` |

### Cell Command Mode (`Esc` from editor → cell frame focused; `Enter` re-enters editor)

| Key | Action |
|---|---|
| `j` / `Down`, `k` / `Up` | Move selection to next / previous cell |
| `Enter` | Enter Edit Mode on selected cell |
| `a` / `b` | Insert new DataScript cell above / below |
| `m` / `y` | Convert selected cell to Markdown / DataScript |
| `d d` | Delete selected cell |
| `z` | Undo last cell structure change (delete/move/convert) |
| `x` / `c` / `v` | Cut / copy / paste cell |
| `o` | Toggle output collapsed / expanded |
| `Shift+Up` / `Shift+Down` | Reorder cell up / down |
| `?` | Open keyboard shortcuts cheat sheet |

---

## 6. Architecture, Stack & Multi-Target Reuse

### Frontend stack (D-060, closes Q-002)

- **UI Framework**: React + TypeScript + Vite
- **Styling**: Tailwind CSS with semantic CSS custom properties (`--surface-*`, `--border-*`,
  `--text-*`, `--accent-*`) for zero-flash dark/light theming
- **Editor**: CodeMirror 6 (lightweight, multi-instance friendly for notebooks, native diagnostics &
  completion extensions, small bundle for WASM)
- **Data interchange**: Apache Arrow IPC (`apache-arrow`)
- **Grid virtualization**: `@tanstack/react-virtual`
- **Charts**: Apache ECharts (tree-shaken Canvas modules covering all 9 `RenderSpec` kinds)
- **Markdown rendering**: `marked` or `markdown-it` + safe HTML sanitization (DOMPurify)
- **Testing**: Vitest (unit/component tests) + Playwright (E2E & chart visual regression tests)

### The `Runtime` abstraction (Web, WASM, Desktop, VS Code)

All UI logic talks exclusively to a TypeScript `Runtime` interface, never calling `fetch` or
`WebSocket` directly from components:

```ts
export interface Runtime {
  readonly capabilities: {
    workspaceFiles: boolean;      // true in ds ui & Tauri; false/virtual in WASM playground
    streamingExport: boolean;     // true in ds ui & Tauri; client-side blob export in WASM
    hostTables: boolean;
  };

  // Language & execution (cell-aware: accepts optional upstream `let` prefix cells)
  check(req: CheckRequest): Promise<CheckResponse>;
  run(req: RunRequest, onEvent: (ev: RunEvent) => void): RunHandle; // handle.cancel()
  complete(req: IdeRequest): Promise<CompletionResponse>;
  hover(req: IdeRequest): Promise<HoverResponse | null>;
  signatureHelp(req: IdeRequest): Promise<SignatureHelpResponse | null>;
  schemaOf(source: string): Promise<SchemaResponse>;
  exportResult(req: ExportRequest): Promise<void>;

  // Workspace files (when capabilities.workspaceFiles is true)
  listFiles(dir?: string): Promise<FileEntry[]>;
  readFile(path: string): Promise<FileContent>;                     // includes version/etag
  writeFile(path: string, content: string, expectedVersion?: string): Promise<WriteResult>;
  createFile(path: string, kind: "file" | "dir", content?: string): Promise<FileEntry>;
  renameFile(from: string, to: string): Promise<FileEntry>;
  deleteFile(path: string): Promise<void>;
  subscribeWorkspace?(onEvent: (ev: WorkspaceEvent) => void): () => void;
}
```

How each target plugs in:

| Surface | Runtime implementation | Notes |
|---|---|---|
| **Local Web UI (`ds ui`)** | `ServerRuntime` (M4) | HTTP + WebSocket to `datascript-server` (`axum`, 127.0.0.1, per-launch token, Origin check, D-016 sandbox) |
| **WASM Playground** | `WasmRuntime` (M5-03) | In-browser Web Worker (`@datascript/wasm`); dropped/sample files mapped as virtual workspace entries |
| **VS Code Results Panel** | Direct component import (M3-06) | Imports `web/src/components/results/` (Arrow grid, ECharts `RenderSpec` chart, diagnostic cards) into a dedicated Vite webview bundle |
| **Future Desktop App** | `TauriRuntime` (later) | Tauri 2 shell wrapping `web/` + Rust `Session` directly via Tauri IPC (no Node/Electron), adding native menus, folder picker, and OS `.dsnb` double-click file association |

### Server API contract (`datascript-server`, M4-01 & M4-04)

All endpoints require the per-launch session token (`Authorization: Bearer <token>` or `?token=`
for WebSocket/downloads) and verify the `Origin` header matches the local server origin.

| Endpoint | Purpose |
|---|---|
| `GET /api/info` | Root folder name, capabilities, default `row_limit`, registered host tables |
| `GET /api/files?dir=&includeHidden=` | Directory tree under root (sandboxed, D-016) |
| `GET /api/file?path=` | Read a `.dsnb` or `.ds` text file + `version` (etag) |
| `PUT /api/file` | Save a `.dsnb` or `.ds` file with optional `expectedVersion` (409 on conflict; refuses non-`.dsnb`/`.ds` paths) |
| `POST /api/file` / `PATCH /api/file` / `DELETE /api/file` | Create, rename, or delete `.dsnb`, `.ds`, or folders (refuses mutating/deleting data files) |
| `GET /api/schema?source=` | Inspect schema of a file or host table via `Session` |
| `POST /api/check` | Check a single script or notebook cell (accepts `{ path, cells, targetCellIndex }` so upstream `let`s are in scope with spans mapped back to each cell's UTF-16 offsets) |
| `WS /api/ws` | Multiplexed stream for: (1) query execution (`run` / `cancel` → streaming Arrow IPC batches + `RenderSpec` + diagnostics + timing), and (2) workspace file-change notifications |
| `POST /api/export` | Stream full uncapped query result via `datascript_engine::writers` (`csv`, `json`, `ndjson`, `parquet`, `arrow`) |
| `POST /api/complete`, `/api/hover`, `/api/signature-help` | IDE helpers backed by `datascript_compiler::ide` over the same cell-aware combined program (M4-04) |

### Cross-cell combined program (Rust)

To avoid duplicating `ReplProgram`'s offset-mapping logic, `datascript-server` (or a small shared
notebook/program-stitching helper) builds a virtual program from the notebook's DataScript cells up
to `targetCellIndex`:
- Only `let` statements from upstream cells (`0 .. targetCellIndex`) are prepended ahead of the
  target cell's full text, separated by `\n;\n` glue segments.
- Every `Diagnostic` span and IDE offset (`complete`, `hover`, `signature_help`) is mapped back from
  combined-program offsets to `{ cellIndex, start, end, line, column }` (UTF-16, D-025).
- Relative file paths inside the notebook resolve against the `.dsnb` file's parent directory (or
  the workspace root for unsaved Scratchpads), always enforced by the D-016 sandbox.

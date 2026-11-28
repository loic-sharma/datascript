# DataScript Web UI Design Specification

This document is the authoritative design spec for DataScript's local web UI (`ds ui`, Milestone
M4). The same React front end (`web/`) also powers the in-browser WASM playground (M5), shares its
result grid and chart components (`web/src/components/results/`) with the VS Code results panel
(M3-06), and is structured to wrap cleanly in a native desktop shell (Tauri) later without
rewriting the UI.

Related documents: [ARCHITECTURE.md](ARCHITECTURE.md), [LANGUAGE.md §2.1](LANGUAGE.md#21-notebooks-dsnb),
[DECISIONS.md](DECISIONS.md) (D-061–D-064), [DIAGNOSTICS.md](DIAGNOSTICS.md), and
[tasks/README.md](tasks/README.md).

---

## 1. Product Vision & Aesthetic Direction

DataScript's web UI is a **notebook-first, local-files-first analytical workbench** that blends the
best ergonomics of five tools:

| Inspiration | What we take |
|---|---|
| **DuckDB UI** | Minimal notebook cells, execution timing summaries (`4,589 rows · 105ms · 3 sources`), virtualized data grid with column type badges & `NULL` styling, live right-hand column profiler (distinct counts, null %, distributions), fullscreen expand, one-click copy/export |
| **Obsidian** | Local working-directory tree, collapsible sidebars, debounced auto-save, Quick Open (`Cmd/Ctrl+P`), Command Palette, Markdown prose cells, document outline, calm distraction-free typography |
| **VS Code** | Slim multi-file tab bar, LSP-grade inline diagnostics/squiggles with one-click Quick Fixes, rich autocomplete/hover/signature help, keyboard-first workflows |
| **Azure Data Explorer (ADX)** | Pipe-based exploration ergonomics, language-native `| render` visualizations directly below the query with Chart/Table toggle, optional run-statement-at-cursor shortcut |
| **Claude Code** | Calm, high-craft terminal aesthetic: muted dark-first monochrome surfaces, sharp typography, monochrome SVG icons, minimal chrome, subtle inline keyboard hints (`⌘↵`, `⎋`), and clear diff/quick-fix cards for diagnostics |

### Visual principles

- **Dark-first, dual-theme.** Deep charcoal/neutral surfaces (not pure `#000`), hairline 1px
  borders, monochrome SVG icons/glyphs (never color emoji in UI chrome), restrained accent colors
  reserved for active focus, execution state, and data viz; paired with a crisp, high-contrast
  light theme.
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
`.dsnb` extension (D-062). Language-level scoping rules are specified in
[LANGUAGE.md §2.1](LANGUAGE.md#21-notebooks-dsnb).

### Why `.dsnb` Markdown?

- **Dedicated extension** gives clean OS/desktop file associations later and unambiguous discovery
  in repos and tooling without colliding with `README.md` or `docs/*.md`.
- **Plain-text Markdown inside** keeps notebooks human-readable, effortless to diff/review in git,
  and trivial to export to `.md`.
- **Works outside the browser**: the single canonical `.dsnb` parser/serializer lives alongside
  `StitchedProgram` in `datascript-compiler` (a small, dependency-free, line-based CommonMark fence
  scanner that builds for `wasm32`) so `datascript-server`, `ds run`/`ds check`/`ds explain`
  (M4-06), `ds lsp`/VS Code (M4-06), and `datascript-wasm` (M5) all reuse it—`web/` never
  duplicates the parser.
- **Strictly source-only**: query outputs, row counts, and chart images are never written into
  `.dsnb` files, keeping git diffs spotless. (Optional local `IndexedDB` output caching outside the
  repo—showing Stale-badged previous outputs on reopen—is tracked separately in M4-07.)

### Serialization grammar & canonical layout

A `.dsnb` file is a sequence of cells parsed from and serialized to UTF-8 Markdown:

1. **Optional YAML frontmatter** at the top of the file (`--- ... ---` starting on line 1) is
   preserved verbatim on round-trip.
2. **DataScript cells** are fenced code blocks parsed per CommonMark fenced-code-block rules:
   - Accepts both backtick (`` ``` ``) and tilde (`~~~`) fences of length ≥ 3, tagged with info
     string `ds` or `datascript` (any extra tokens after `ds` on the opening line are ignored in v1
     and preserved on untouched cells).
   - A closing fence must use the same fence character and be at least as long as the opening
     fence, so a ```` ```ds ```` block nested inside a longer non-`ds` fence (e.g. ````` ````markdown `````)
     stays part of the Markdown cell.
   - An **unclosed fence** runs to EOF per CommonMark (checked as a normal DataScript cell; no
     extra `.dsnb`-specific diagnostic code in v1).
   - On save, newly created or modified DataScript cells emit `ds` as the canonical tag with a
     backtick fence strictly longer than any backtick run inside the cell (minimum 3), so a
     DataScript string containing ```` ``` ```` never terminates the fence early.
3. **Markdown cells** are non-whitespace prose blocks between DataScript cells:
   - **Whitespace-only text** (blank lines) between two DataScript cells is a separator, never an
     empty Markdown cell.
   - Non-`ds` fenced code blocks (e.g. `json`, `bash`) inside prose stay part of the Markdown cell.
   - Two adjacent Markdown cells are delimited by a standalone HTML comment on its own line:
     ```markdown
     <!-- ds:cell -->
     ```
   - If a Markdown cell contains a literal standalone `<!-- ds:cell -->` line, the serializer
     escapes it to `<!-- ds:cell:literal -->` (still invisible in Markdown renderers) and the
     parser unescapes it back on load.
   - **No persisted cell metadata in v1** (nothing in M4-01–M4-05 needs it, keeping the delimiter
     unambiguous).
4. **Canonical separators & lossless round-trip**:
   - Blank lines between cells belong to the separator (canonical separator for newly serialized
     cells is one blank line `\n\n`, using the file's dominant line ending, defaulting to `\n`).
   - Because `PUT /api/file` already reads the on-disk file to verify `expectedVersion`, the server
     diffs incoming cells against the on-disk parsed cells (at `expectedVersion`) and re-emits
     untouched cells and their surrounding separators verbatim (preserving original line endings,
     fence styles, and tags). Only modified, inserted, or reordered cells are re-serialized in
     canonical form.

### Plain `.ds` scripts & Scratchpads

- **`.ds` files**: Opening any `.ds` file from the left tree opens it as a single-editor DataScript
  script view with the same diagnostics, completion, Run controls, result grid, charts, and
  right-hand inspector.
- **Launch Scratchpad**: When `ds ui` opens in a directory with no open notebooks (e.g. a folder of
  raw CSV/Parquet files), it opens an unsaved **Scratchpad** notebook with one empty DataScript
  cell. While empty, a quiet, dismissible quick-preview strip lists data files discovered in the
  folder, each with a one-click **Preview** action that inserts `from '<relpath>' | take 100` and
  runs it immediately. Pressing `Cmd/Ctrl+S` prompts for a filename and saves it as `<name>.dsnb`.

### Saving & external edits

- **Saved files on disk** (`.dsnb`, `.ds`) use **debounced auto-save** (~500 ms after edits stop,
  Obsidian-style), with full undo/redo history and immediate flush on `Cmd/Ctrl+S`.
- **Scratchpads** stay in memory until explicitly saved.
- **External-change protection**: Writes carry the last-known file `version` (etag from mtime +
  content hash). If the file changed on disk (e.g. edited in VS Code or switched by `git checkout`),
  auto-save pauses and displays a non-destructive banner (`Reload from disk` / `Overwrite disk`).

---

## 3. Execution Model & Cross-Cell State

### Top-to-bottom lexical `let` scope (D-062)

A notebook behaves identically to a single DataScript program split into cells
([LANGUAGE.md §2.1](LANGUAGE.md#21-notebooks-dsnb)):

- Every DataScript cell automatically sees all scalar and tabular `let` declarations from
  DataScript cells **above it** in document order, compiled via `datascript_compiler::StitchedProgram`
  (§6).
- **Whole-notebook live checking**: `POST /api/check` checks the **entire notebook in one pass**
  (the same check `ds check notebook.dsnb` performs in M4-06) and returns diagnostics grouped by
  `cellIndex`, so editing a `let` in cell 1 updates diagnostics across cells 2, 3, … in a single
  request instead of $O(N^2)$ per-cell checks.
- **Reordering cells** immediately updates scope and diagnostics deterministically—there is never
  hidden Jupyter-style execution-order state.
- **No shadowing across cells**: Re-declaring an existing `let` name in a lower cell reports
  `DS1003` with a secondary label pointing back to the earlier cell (D-007).
- **Lazy `let` visibility**: Because `let` bindings are logical aliases (not yet materialized),
  running a downstream cell re-scans the sources behind upstream `let`s. Each cell's execution bar
  shows how many **distinct `SourceRef`s** the query plan scanned (computed by walking `Plan::Scan`
  nodes in `QueryPlan::root`, so `'data/*.parquet'` counts as 1 source — e.g. `4,589 rows · 105ms ·
  3 sources`). Materialized/cached `let` results are flagged as higher priority in
  [ROADMAP.md](ROADMAP.md).

### Upstream error handling (`DS4061`)

- During live **`check`**, errors in an upstream cell are reported on that upstream cell itself;
  downstream references to a broken `let` inherit `LetEntry::Error` silently (D-038) so errors do
  not cascade.
- When the user tries to **run** cell $k$, the server emits **`DS4061`** (`"cannot run cell because
  an upstream cell has errors"`, with a secondary label pointing at the first broken upstream cell)
  if and only if:
  1. any upstream cell ($0 \dots k-1$) has a **lexical or syntax error** (preventing reliable `let`
     extraction), **or**
  2. any prepended upstream **`let` statement** has an **error-severity** diagnostic.
- Warnings never block execution, and errors inside upstream **query** statements (which are not
  prepended when running cell $k$) do **not** block running cell $k$.

### Explicit execution, staleness & clocks

- Queries **never auto-run** on edit or reorder.
- Each cell tracks a fingerprint of its own source plus the upstream `let` definitions it depends
  on. When either changes after a cell has produced output, the output header shows a subtle
  **Stale** badge.
- Execution actions:
  - **Run cell** (`Cmd/Ctrl+Enter`) — runs all query statements in the focused cell.
  - **Run cell & advance** (`Shift+Enter`) — runs the focused cell and moves focus to the next cell
    (inserting a new DataScript cell if at the bottom).
  - **Run statement at cursor** (`Cmd/Ctrl+Alt+Enter` or cell gutter button) — if the cursor is on
    a query statement, runs that statement; if on a `let` statement (or in a `let`-only cell),
    validates the cell's bindings; if between statements, runs the nearest preceding query
    statement (or validates if none).
  - **Run cell & below** / **Run all** — executes cells sequentially top-to-bottom under **one
    shared `nowMicros` clock** (honoring D-015 so `ago()` aligns across every cell in the batch),
    stopping on the first error cell (or continuing on request).
  - **Cancel / Stop** (`Cmd/Ctrl+.`, or the inline Stop button) — cancels the active `RunStream`
    immediately (D-055).

### `let`-only cells & multi-statement cells (D-063, closes Q-004)

- **`let`-only cell**: Running a cell that defines only `let` statements validates its bindings and
  displays a compact status line (`2 bindings defined · validated in 3ms`) with no empty table
  clutter.
- **Multi-statement cell**: Running a cell with multiple query statements renders compact output
  sub-tabs (`Result 1`, `Result 2`, … — labelled with the `render` title if present) inside the
  cell's output frame.

### Result size limits & streaming export (D-064, closes Q-007)

- UI executions cap streamed output at `row_limit = 10_000` by default. Users can configure a
  per-workspace default (stored in browser `localStorage` keyed by workspace root) or override it
  per run. Because `Session::start`'s raw `QueryStream` does not cap rows on its own,
  `datascript-engine` provides a streaming-limit helper (counting rows per batch, slicing the final
  batch at the limit, dropping the stream to cancel execution on the next row, and returning
  `truncated: true`) shared across `datascript-server`, M3-06, and M5.
- When `truncated` is true, the runtime emits the `DS4060` info diagnostic and the cell footer
  renders a subtle truncation notice using the run's actual `rowLimit` (e.g.
  `Showing first {N} rows · truncated`) with a one-click **Export full result** menu.
- **Export full result** sends **the immutable cell snapshot captured at run time** (so exporting a
  Stale result still matches what the grid showed) and re-executes the exact statement with **the
  original run's `nowMicros` clock** (preserving `now()` / `ago()` windows per D-015), **the target
  `statementIndex`** (0-based among the target cell's own query statements, ignoring upstream and
  local `let` statements), and **the same upstream-`let` cell context**. To stream directly to disk
  without buffering the full file in browser memory, export uses a two-step ticketed download
  (`POST /api/export` → one-shot `GET /api/export/<ticket>`, §6) streaming uncapped rows through
  `datascript_engine::writers` (CSV, JSON, NDJSON, Parquet, Arrow IPC, and Excel once M2-28 lands).

---

## 4. Three-Pane Workspace Layout

```
┌──────────────────────┬─────────────────────────────────────────────┬──────────────────────────┐
│  [Files]        [+ ] │ [sales.dsnb ×] [Scratchpad ×]           [+] │ [Profile] [Outline] [Let]│
│ ──────────────────── │  data / sales.dsnb               ▶ Run all  │ ──────────────────────── │
│  Filter files...     │ ┌─────────────────────────────────────────┐ │  4,589 Rows · 4 Columns  │
│  ▾ data/             │ │ # Monthly Revenue Analysis              │ │  Filter columns...       │
│      orders.parquet  │ │ Exploring top customers by region.      │ │  ─────────────────────── │
│      customers.csv   │ └─────────────────────────────────────────┘ │  T   id          4.59K - │
│  ▾ notebooks/        │ ┌─────────────────────────────────────────┐ │  123 downloads   null 88%│
│    ● sales.dsnb      │ │ ▶  let orders = from 'data/orders...';  │ │  T   region        4 - │
│      adhoc.ds        │ │    validated · 1 binding (4ms)          │ │  dt  month        12 - │
│                      │ └─────────────────────────────────────────┘ │                          │
│  ATTACHED TABLES     │ ┌─────────────────────────────────────────┐ │  (Starts collapsed;      │
│    live_events       │ │ ▶  from orders                          │ │   switches to Source     │
│                      │ │    | summarize rev = sum(amount) by ... │ │   Schema when a data     │
│                      │ │    | render timechart                   │ │   file is selected)      │
│                      │ │  4,589 rows · 105ms · 3 sources         │ │                          │
│                      │ │  ┌───────────────────────────────────┐  │ │                          │
│                      │ │  │       (ECharts / Arrow Grid)      │  │ │                          │
│                      │ │  └───────────────────────────────────┘  │ │                          │
│                      │ └─────────────────────────────────────────┘ │                          │
└──────────────────────┴─────────────────────────────────────────────┴──────────────────────────┘
```

All three panes have smooth drag-resizable splitters, and both sidebars collapse cleanly via
shortcuts (`Cmd/Ctrl+B` left, `Cmd/Ctrl+Shift+B` right) or top-bar icons. **The right pane starts
collapsed by default** so the notebook and wide result grids get full horizontal breathing room on
laptop screens; users open it on demand via `Cmd/Ctrl+Shift+B`, the top-bar inspector toggle, or
clicking a column header's stats trigger.

> **Staged delivery:** M4 v1 (`M4-08`, `M4-01`–`M4-04`, `M4-09`) ships a focused single-notebook
> view with a read-only file tree + `.dsnb` creation, DataScript & Markdown cells, virtualized
> grid, ECharts, auto-save/conflict handling, and the collapsible Result Profiler + Source Schema
> inspector. Power-user and workspace features (multi-file tabs & pinning, file rename/delete, Cell
> Command Mode, Command Palette, Quick Open, and Outline/Scope tabs) land in `M4-05`; CLI/LSP
> `.dsnb` support lands in `M4-06`; optional local output caching lands in `M4-07`.

### 4.1 Left Pane — Working Directory Tree & Attached Tables

- **Full working directory view**: Displays the directory passed to `ds ui [dir]` (default `.`).
  Hidden files and common clutter (`.git`, `node_modules`, `target`, `.DS_Store`) are hidden by
  default with a quick toggle and live filename filter box.
- **Visual distinction**: Monochrome SVG file icons distinguish `.dsnb` notebooks, `.ds` scripts,
  supported data files (`.parquet`, `.csv`, `.tsv`, `.json`, `.jsonl`, `.ndjson`, `.xlsx`),
  folders, and other files (subtly dimmed).
- **Safe file CRUD** (confined strictly to the workspace root, D-016):
  - M4 v1 (`M4-01`–`M4-02`) supports creating and saving `.dsnb`/`.ds` files; folder creation,
    rename, and delete land in `M4-05`.
  - **The workspace file API never writes, renames, or deletes data files** (explicit query
    `output` operators from M2-25…M2-28 may still write data files inside the D-016 sandbox):
    - All mutating endpoints (`POST`, `PUT`, and M4-05's `POST /api/dir`, `PATCH`, `DELETE`) refuse
      any target path that itself or via any parent component under the root is a **symlink**
      (`symlink_metadata`/`O_NOFOLLOW`), so an in-root symlink such as `notes.ds -> data/orders.csv`
      can never bypass the extension guard.
    - Creating (`POST`) uses `O_CREAT|O_EXCL` and renaming (`PATCH`, M4-05) uses atomic no-clobber
      primitives (`renameat2(RENAME_NOREPLACE)` / `renamex_np(RENAME_EXCL)` / `MoveFileExW`
      without replace), refusing any non-`.dsnb`/`.ds` extension or already-existing path.
    - Deleting a folder (`DELETE`, M4-05) never calls `remove_dir_all`: it refuses symlinks and
      any non-`.dsnb`/`.ds` file, unlinking verified `.dsnb`/`.ds` files one by one followed by
      `rmdir` (which fails atomically if a new file appeared concurrently). Renaming a folder
      refuses any folder recursively containing non-`.dsnb`/`.ds` files or symlinks.
- **Data file interactions**:
  - **Single-click** selects the file and opens its **Source Schema** in the right-hand inspector.
  - **Double-click** (or inline hover action) inserts `from '<relpath>'` (or `'<relpath>'` when the
    cursor is already in a table position, D-036) into the active cell, or creates a new preview
    cell (`from '<relpath>' | take 100`).
- **Attached Tables section**: Shown only when host tables are registered (`ds ui --table ...`),
  listing their names and schemas (DuckDB UI style).
- **Live workspace sync**: File additions, renames, and deletions on disk update the tree and
  invalidate `Session`'s schema cache automatically.

### 4.2 Center Pane — Notebook Canvas

- **Header & tabs**: M4 v1 shows a clean path breadcrumb header for the active notebook/script;
  `M4-05` adds a slim multi-file tab bar keeping open tabs' outputs, scroll position, and undo
  history alive, with pinning and Quick Open (`Cmd/Ctrl+P`).
- **Cell list & reordering**:
  - Every cell has a quiet left gutter drag handle (pointer drag-and-drop reordering) and supports
    keyboard reordering in Command Mode (`Shift+Up`/`Shift+Down`, `Shift+K`/`Shift+J`, `M4-05`)
    plus the Command Palette—leaving `Alt+Up`/`Alt+Down` free for CodeMirror line movement.
  - Subtle `+ DataScript  + Markdown` insert dividers appear between cells on hover or at the end
    of the notebook.
- **Markdown cells**:
  - Rendered cleanly by default (GFM: headings, tables, task lists, inline code, links, code blocks)
    through DOMPurify. **External remote `http(s)://` images are blocked by default** (preventing
    tracking pixels when opening notebooks from cloned repos; relative workspace images or explicit
    opt-in allowed).
  - Click/double-click or press `Enter` (in Command Mode) to edit raw Markdown in CodeMirror.
  - `Cmd/Ctrl+Enter` renders in place; `Shift+Enter` renders and advances to the next cell; `Esc`
    renders and exits to Command Mode on that cell.
- **DataScript cells**:
  - **Editor**: CodeMirror 6 with DataScript syntax highlighting (matching D-026), inline
    diagnostic squiggles, hover documentation, signature help, and context-aware autocomplete
    backed by `datascript_compiler::ide`.
  - **Cell header / status bar**: Compact Play/Stop button, execution duration, row count, and
    distinct source-scan count (`4,589 rows · 105ms · 3 sources`), Stale badge (when applicable),
    Chart/Table toggle (for `| render` queries), fullscreen expand, and overflow menu (duplicate,
    split, copy query, collapse output, delete).
  - **Diagnostics panel**: Renders any compiler or runtime `Diagnostic`s in rustc/Claude-Code style
    with clickable **Apply fix** buttons for machine-applicable `Suggestion`s (including jump links
    for `DS4061` upstream cell errors).
  - **Tabular output (virtualized Arrow grid, `M4-09`)**:
    - Reads Arrow IPC `RecordBatch`es directly via `apache-arrow` with row and column virtualization
      (`@tanstack/react-virtual`).
    - Row index gutter, compact monochrome column headers with type glyphs (`T` string, `#`
      long/int/real/decimal, `dt` datetime, `ts` timespan, `b` bool, `{}` dynamic) and a subtle
      null indicator (clicking opens the column in the right-hand profiler), subtle italic `NULL`
      cell styling, column resizing, click-to-copy cell/row, and quick export menu.
    - **Truncation-aware sorting**: client-side column sort clearly labels *"sorted first {N}
      rows"* using the run's actual `rowLimit` whenever the result is truncated (D-064), and every
      column header offers a **Sort in query** action that inserts `| sort by <col> desc` (or
      `asc`, always explicit so `DS2103` never fires, D-005) **before any trailing `| render ...`**
      (or updates an existing `sort` in place) and re-runs over the full dataset so code remains
      the single source of truth.
  - **Visualization output (`| render`, `M4-03`)**:
    - Queries ending with `| render <kind>` default to Chart view with an instant **Chart / Table**
      toggle and PNG/SVG download.
    - Supports all 9 render kinds in [LANGUAGE.md §6](LANGUAGE.md#6-render): `table`, `timechart`,
      `linechart`, `areachart`, `barchart` (horizontal), `columnchart` (vertical), `piechart`,
      `scatterchart`, and `card` (KPI summary card).
    - Plain table queries without `| render` stay in Table view by default; a quiet
      **Visualize (`| render`)** helper can append a starter `| render ...` line to the query so
      DataScript code remains the single source of truth (D-062).
    - Unrenderable specs (e.g. >500 distinct series or empty chart data) degrade gracefully with a
      clear explanatory banner and one-click switch to Table view—never hanging the tab.

### 4.3 Right Pane — Contextual Inspector (Collapsed by Default)

The right pane starts collapsed so wide tables keep full horizontal space, and opens on demand via
`Cmd/Ctrl+Shift+B`, the top-bar inspector icon, selecting a data file in the left tree, or clicking
a column header's stats badge. It provides contextual views (with Outline and Scope tabs added in
`M4-05`):

1. **Inspector (default contextual tab, M4-04)**:
   - **When a cell with results is focused → Live Result Profile (DuckDB UI style)**:
     - Header: total rows (explicitly labelled `"of first {N} rows"` using the run's actual
       `rowLimit` when truncated per D-064), column count, copy-schema button, column search input,
       and sort selector (`Default`, `Name`, `Null %`, `Distinct`).
     - Per-column row: monochrome type glyph, column name, distinct count, null % pill, and compact
       distribution summary (inline sparkbar / top values / min–max on expand).
     - Computed **client-side** over the cell's loaded Arrow `RecordBatch`es, so it works with zero
       extra server round-trips in `ds ui`, WASM, and desktop alike.
   - **When a data file or attached table is selected in the left tree → Source Schema**:
     - File path, format, size, and searchable column list (`name`, DataScript type, and underlying
       Arrow type when projection-only).
     - Quick actions: **Insert `from '...'`**, **New preview cell (`take 100`)**, and **Describe**
       (`from '...' | describe`, wired once M2-23 lands).
2. **Outline (Obsidian style, M4-05)**:
   - Interactive jump list of Markdown headings (`#`, `##`, `###`) and DataScript cells (showing
     defined `let` names or `render` title / first line) for fast navigation in long notebooks.
3. **Scope (`let`s, M4-05)**:
   - Lists every scalar `let` (name, type, constant value if known) and tabular `let` (name,
     inferred output columns & types, defining cell link) visible to the currently focused cell.

---

## 5. Keyboard Shortcuts & Command Mode

The UI uses a **hybrid modeless + cell Command Mode** model (D-062). Shortcuts are chosen to avoid
CodeMirror 6 editing bindings (`Alt+Up/Down` move line, `Cmd/Ctrl+Shift+Up/Down` selection), OS
dead keys / `AltGr`, and browser-reserved chords (`Ctrl+W`, `Ctrl+Shift+N`, Firefox's
`Cmd/Ctrl+Shift+P`). Because browser chord reservation varies across platforms (for example,
Chrome's `Cmd/Ctrl+Shift+B` bookmarks bar toggle on macOS), M4-05 tests verify preventability and
every action remains reachable via the Command Palette (`Cmd/Ctrl+K`) and toolbar icons. Modifier
chords match on `KeyboardEvent.code`, browser-safe `Alt` chords (`Alt+W`, `Alt+N`) fire only
outside text inputs (preserving macOS `Option` dead keys inside CodeMirror), and `Ctrl+Alt+Enter`
ignores `AltGraph` events on Windows European layouts.

### Global & Modeless Shortcuts

| Action | macOS | Windows / Linux |
|---|---|---|
| Run DataScript cell / render Markdown cell in place | `Cmd+Enter` | `Ctrl+Enter` |
| Run / render cell & advance (or insert below) | `Shift+Enter` | `Shift+Enter` |
| Run statement at cursor *(ignores `AltGraph`; also in cell gutter)* | `Cmd+Alt+Enter` | `Ctrl+Alt+Enter` |
| Run all cells | `Cmd+Shift+Enter` | `Ctrl+Shift+Enter` |
| Cancel running query | `Cmd+.` | `Ctrl+.` |
| Leave Edit Mode (renders Markdown) → Command Mode | `Esc` | `Esc` |
| Command Palette *(primary; `Cmd/Ctrl+Shift+P` alias where browser allows)* | `Cmd+K` | `Ctrl+K` |
| Quick Open file / notebook *(M4-05)* | `Cmd+P` | `Ctrl+P` |
| Toggle left sidebar (Files) | `Cmd+B` | `Ctrl+B` |
| Toggle right sidebar (Inspector; verify `preventDefault` in Chrome, toolbar fallback) | `Cmd+Shift+B` | `Ctrl+Shift+B` |
| Save immediately / Save Scratchpad | `Cmd+S` | `Ctrl+S` |
| Close active tab *(M4-05; outside text inputs in browser; `Cmd/Ctrl+W` in desktop shell)* | `Alt+W` | `Alt+W` |
| New Scratchpad notebook *(M4-05; outside text inputs in browser; `Cmd/Ctrl+Shift+N` in desktop shell)* | `Alt+N` | `Alt+N` |
| Toggle line comment (in cell editor) | `Cmd+/` | `Ctrl+/` |

### Cell Command Mode (`Esc` from editor → cell frame focused; `Enter` re-enters editor, M4-05)

| Key | Action |
|---|---|
| `j` / `Down`, `k` / `Up` | Move selection to next / previous cell |
| `Shift+Up` / `Shift+Down` (or `Shift+K` / `Shift+J`) | Reorder selected cell up / down |
| `Enter` | Enter Edit Mode on selected cell |
| `a` / `b` | Insert new DataScript cell above / below |
| `m` / `y` | Convert selected cell to Markdown / DataScript |
| `d d` | Delete selected cell |
| `z` | Undo last cell structure change (delete/move/convert) |
| `x` / `c` / `v` | Cut / copy / paste cell |
| `o` | Toggle output collapsed / expanded |
| `?` | Open keyboard shortcuts cheat sheet |

---

## 6. Architecture, Stack & Multi-Target Reuse

### Frontend stack (D-061, closes Q-002)

- **UI Framework**: React + TypeScript + Vite
- **Styling**: Tailwind CSS with semantic CSS custom properties (`--surface-*`, `--border-*`,
  `--text-*`, `--accent-*`) for zero-flash dark/light theming and monochrome SVG icons
- **Editor**: CodeMirror 6 (lightweight, multi-instance friendly for notebooks, native diagnostics &
  completion extensions, small bundle for WASM)
- **Data interchange**: Apache Arrow IPC (`apache-arrow`)
- **Grid virtualization**: `@tanstack/react-virtual`
- **Charts**: Apache ECharts (tree-shaken Canvas modules covering all 9 `RenderSpec` kinds)
- **Markdown rendering**: `marked` or `markdown-it` + DOMPurify (blocking external remote images by
  default)
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

  // Language & execution
  // check checks the whole notebook in one pass ({ path, cells } -> diagnostics grouped by cellIndex)
  check(req: CheckRequest): Promise<CheckResponse>;
  run(req: RunRequest, onEvent: (ev: RunEvent) => void): RunHandle; // RunEvent includes nowMicros, rowLimit, sourceCount
  complete(req: IdeRequest): Promise<CompletionResponse>;
  hover(req: IdeRequest): Promise<HoverResponse | null>;
  signatureHelp(req: IdeRequest): Promise<SignatureHelpResponse | null>;
  schemaOf(source: string): Promise<SchemaResponse>;
  // ExportRequest carries the run-time snapshot { path, cells, targetCellIndex, statementIndex, nowMicros, format }
  exportResult(req: ExportRequest): Promise<void>;

  // Workspace files (when capabilities.workspaceFiles is true)
  listFiles(dir?: string): Promise<FileEntry[]>;
  // Returns structured cells (parsed by datascript-compiler) for `.dsnb`, or raw text for `.ds`, plus version/etag
  readFile(path: string): Promise<FileDocument>;
  writeFile(path: string, doc: FileDocumentInput, expectedVersion?: string): Promise<WriteResult>;
  createFile(path: string, doc?: FileDocumentInput): Promise<FileEntry>;
  // Added in M4-05:
  createDir?(path: string): Promise<FileEntry>;
  renameFile?(from: string, to: string): Promise<FileEntry>;
  deleteFile?(path: string): Promise<void>;
  subscribeWorkspace?(onEvent: (ev: WorkspaceEvent) => void): () => void;
}
```

How each target plugs in:

| Surface | Runtime implementation | Notes |
|---|---|---|
| **Local Web UI (`ds ui`)** | `ServerRuntime` (M4) | HTTP + WebSocket to `datascript-server` (`axum`, 127.0.0.1, per-launch token, Origin check, D-016 sandbox) |
| **WASM Playground** | `WasmRuntime` (M5-03) | In-browser Web Worker (`@datascript/wasm`); dropped/sample files mapped as virtual workspace entries; parses/serializes `.dsnb` via the same compiled-to-WASM `datascript-compiler` module |
| **VS Code Results Panel** | Direct component import (M3-06) | Imports `web/src/components/results/` (Arrow grid from M4-09, ECharts `RenderSpec` chart from M4-03, diagnostic cards) into a dedicated Vite webview bundle |
| **Future Desktop App** | `TauriRuntime` (later) | Tauri 2 shell wrapping `web/` + Rust `Session` directly via Tauri IPC (no Node/Electron), adding native menus, folder picker, and OS `.dsnb` double-click file association |

### Server API contract (`datascript-server`, M4-01, M4-04 & M4-05)

All JSON API and WebSocket endpoints require the per-launch session token (`Authorization: Bearer
<token>`, or `?token=` on the WebSocket handshake) and verify the `Origin` header matches the local
server origin. **Token hygiene**: immediately after reading the initial `?token=` from the launch
URL, the SPA stores it in memory and strips it from the address bar via `history.replaceState` so
it does not remain in browser history, screenshots, or `Referer` headers.

Streaming file downloads use a short-lived, single-use ticket (`GET /api/export/<ticket>`) minted
by `POST /api/export` so the browser streams straight to disk without buffering in JS memory; that
`GET` endpoint skips `Origin` (browsers omit `Origin` on GET navigations), verifies
`Sec-Fetch-Site: same-origin` when present, and consumes the ticket atomically.

| Endpoint | Milestone | Purpose |
|---|---|---|
| `GET /api/info` | M4-01 | Root folder name, capabilities, default `row_limit`, registered host tables |
| `GET /api/files?dir=&includeHidden=` | M4-01 | Directory tree under root (sandboxed, D-016) |
| `GET /api/file?path=` | M4-01 | Read a `.dsnb` (returned as canonical parsed cells + frontmatter) or `.ds` text file + `version` (etag) |
| `PUT /api/file` | M4-01 | Save a `.dsnb` (diffing against on-disk file at `expectedVersion` to preserve untouched cells/separators via `datascript-compiler`) or `.ds` file (`409` on conflict; refuses non-`.dsnb`/`.ds` paths and any path with a symlink component under root) |
| `POST /api/file` | M4-01 | Create a new `.dsnb` or `.ds` file atomically (`O_CREAT\|O_EXCL`; refuses non-`.dsnb`/`.ds` paths, existing files, and symlink components) |
| `POST /api/dir`, `PATCH /api/file`, `DELETE /api/file` | M4-05 | Create folders, rename (atomic no-clobber: `RENAME_NOREPLACE`/`RENAME_EXCL`), or delete `.dsnb`/`.ds` files or safe folders (unlinking only verified `.dsnb`/`.ds` regular files one by one followed by `rmdir`; refuses symlinks and any folder containing non-`.dsnb`/`.ds` files) |
| `GET /api/schema?source=` | M4-01 | Inspect schema of a file or host table via `Session` |
| `POST /api/check` | M4-01 | Check a `.ds` script or whole `.dsnb` notebook in one pass (`{ path, cells }` → diagnostics grouped by `cellIndex` in UTF-16 offsets + per-cell in-scope `let` summaries) |
| `WS /api/ws` | M4-01 | Multiplexed stream for: (1) query execution (`run` with optional shared `nowMicros` for Run all / `cancel` → `started` with `nowMicros`, `rowLimit`, `sourceCount`, streaming capped Arrow IPC batches + `RenderSpec` + diagnostics incl. `DS4060`/`DS4061` + timing), and (2) workspace file-change notifications |
| `POST /api/export` | M4-01 | Accepts the run-time cell snapshot `{ path, cells, targetCellIndex, statementIndex, nowMicros, format }` (`statementIndex` is 0-based among the target cell's own query statements), validates it, and returns a short-lived single-use download URL `/api/export/<ticket>` |
| `GET /api/export/<ticket>` | M4-01 | Consumes the one-shot ticket (checks `Sec-Fetch-Site` when present, no `Origin` header required) and streams the full uncapped result as `Content-Disposition: attachment` via `datascript_engine::writers` (`csv`, `json`, `ndjson`, `parquet`, `arrow`) |
| `POST /api/complete`, `/api/hover`, `/api/signature-help` | M4-04 | IDE helpers (`{ path, cells, targetCellIndex, offset }`) backed by `datascript_compiler::ide` over `StitchedProgram` |

### Cross-cell combined program & `.dsnb` parser (`datascript-compiler`, M4-08)

Both the dependency-free, line-based `.dsnb` Markdown fence scanner/serializer and the
segment-stitching / span-mapping primitive (`StitchedProgram`, extracted from `ReplProgram` in
`datascript-cli`, which delegates to it) live in **`datascript-compiler`** (pure, no I/O,
`wasm32`-compatible). `datascript-server` (M4-01 & M4-04), `datascript-cli` (`ds run`/`ds check`/
`ds explain` on `.dsnb`, M4-06), `datascript-lsp` (M4-06), and `datascript-wasm` (M5) all reuse
them:
- Whole-notebook check stitches every DataScript cell's statements in document order (mapping every
  diagnostic span back to `{ cellIndex, start, end, line, column }` in UTF-16, D-025).
- Single-cell execution and IDE requests (`run`, `export`, `complete`, `hover`, `signature_help`)
  prepend only upstream `let` statements (`0 .. targetCellIndex`) ahead of the target cell's full
  text, separated by `\n;\n` glue segments.
- Relative file paths inside the notebook resolve against the `.dsnb` file's parent directory (or
  the workspace root for unsaved Scratchpads), always enforced by the D-016 sandbox in `ds ui`.

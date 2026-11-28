# DataScript Web UI Design Specification

This document is the authoritative design spec for DataScript's local web UI (`ds ui`, Milestone
M4). The same React front end (`web/`) also powers the in-browser WASM playground (M5), shares its
result grid and chart components (`web/src/components/results/`) with the VS Code results panel
(M3-06), and is structured so a native desktop shell (Tauri) can wrap it later without
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
| **Claude Code** | Terminal aesthetic: muted dark-first monochrome surfaces, sharp typography, monochrome SVG icons, minimal chrome, subtle inline keyboard hints (`⌘↵`, `⎋`), and clear diff/quick-fix cards for diagnostics |

### Visual principles

- **Dark-first, dual-theme.** Deep charcoal/neutral surfaces (not pure `#000`), hairline 1px
  borders, monochrome SVG icons/glyphs (never color emoji in UI chrome), restrained accent colors
  reserved for active focus, execution state, and data viz; paired with a high-contrast
  light theme.
- **Typography-led hierarchy.** Monospace for DataScript code, table cells, column types,
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
- **Plain-text Markdown inside** keeps notebooks human-readable, easy to diff in git, and
  easy to export to `.md`. *(GitHub treats unknown extensions as plain text by default; repos
  that want rendered Markdown previews on GitHub can add `*.dsnb linguist-language=Markdown` to
  `.gitattributes`.)*
- **Works outside the browser**: the single canonical `.dsnb` parser/serializer lives alongside
  `StitchedProgram` in `datascript-compiler` (a small, dependency-free, line-based CommonMark fence
  scanner that builds for `wasm32`) so `datascript-server`, `ds run`/`ds check`/`ds explain`
  (M4-06), `ds lsp`/VS Code (M4-06), and `datascript-wasm` (M5) all reuse it—`web/` never
  duplicates the parser.
- **Strictly source-only**: query outputs, row counts, and chart images are never written into
  `.dsnb` files, so git diffs show only source changes. (Optional local output caching outside the repo—stored
  under the user's OS cache dir for `ds ui` or `IndexedDB` for the static WASM playground, showing
  Stale-badged previous outputs on reopen—is tracked separately in M4-07.)

### Serialization grammar & canonical layout

A `.dsnb` file is a sequence of cells parsed from and serialized to UTF-8 Markdown:

1. **Optional YAML frontmatter** at the top of the file (`--- ... ---` starting on line 1) is
   preserved verbatim on round-trip.
2. **DataScript cells** are **top-level** fenced code blocks (0–3 leading spaces of indentation,
   with CommonMark's content-indent stripping; `ds` fences inside blockquotes or list items stay
   part of the surrounding Markdown cell) parsed per CommonMark fenced-code-block rules:
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
     empty Markdown cell; an empty/whitespace-only Markdown cell created in the UI is dropped on
     save/reload.
   - Non-`ds` fenced code blocks (e.g. `json`, `bash`) inside prose stay part of the Markdown cell.
   - Two adjacent Markdown cells are delimited by a standalone HTML comment on its own line:
     ```markdown
     <!-- ds:cell -->
     ```
   - **Escaping is stacking, so it is lossless for every input**: on save, any standalone line in
     a Markdown cell of the form `<!-- ds:cell -->`, `<!-- ds:cell:literal -->`,
     `<!-- ds:cell:literal:literal -->`, … gains one more `:literal`; on load, a line of that form
     with at least one `:literal` loses one. So a user's literal `<!-- ds:cell:literal -->` is
     written as `<!-- ds:cell:literal:literal -->` and loads back unchanged. (All forms stay
     invisible in Markdown renderers.)
   - **The serializer never lets a cell change cell boundaries.** Every cell is checked with the
     same scanner in its new position before it is written, including untouched cells whose bytes
     are reused (item 4 below), because deleting, reordering, or appending cells changes their neighbours:
     - a **top-level fence tagged `ds` or `datascript`** inside a Markdown cell is written with
       the same stacking escape on its tag (`ds` → `ds:literal`, `ds:literal` →
       `ds:literal:literal`, likewise for `datascript`), and the parser removes one `:literal`
       from such a tag inside Markdown cells. A top-level fence tagged `ds:literal` is never a
       DataScript cell;
     - a **fence left open at the end of a cell** (a Markdown cell's fence in any language, or an
       unclosed DataScript fence that ran to EOF) gets a matching closing fence line appended when
       any cell follows it, so it cannot swallow the cells after it;
     - when the file has **no frontmatter** and the first cell is Markdown starting with a `---`
       line, the serializer writes one blank separator line first, so the cell is not read back as
       frontmatter.

     Property tests assert `parse(serialize(cells)) == normalize(cells)` for every cell list, where
     `normalize` drops empty/whitespace-only Markdown cells and trims leading and trailing blank
     lines from Markdown cells (those belong to the separator); the appended closing fence is the
     only other difference.
   - **No persisted cell metadata in v1** (nothing in M4-01–M4-05 needs it, keeping the delimiter
     unambiguous).
4. **Canonical separators & deterministic lossless round-trip**:
   - Blank lines between cells belong to the separator (canonical separator for newly serialized
     cells is one blank line `\n\n`, using the file's dominant line ending, defaulting to `\n`).
   - **Content diff against the base file (no cell IDs)**: the client sends only `expectedVersion`
     and its cells (`{ kind, text }`) on `PUT /api/file`. Under the per-path lock, after the `409`
     check, the server re-parses the on-disk file (which is the file at `expectedVersion`) and
     matches incoming cells to base cells by **content**: first a longest-common-subsequence diff
     over the `(kind, text)` sequences, then each still-unmatched incoming cell pairs with the
     first still-unmatched base cell of identical content (so a moved cell is matched too). The
     match is deterministic, and identical duplicate cells pair up in document order. A matched
     cell re-emits its exact original bytes (preserving line endings, fence style, and tag); an
     unmatched (modified or new) cell is serialized in canonical form. A cell's bytes never
     include the separator or a `<!-- ds:cell -->` delimiter: **separators are recomputed from
     each cell's new neighbours**. The original separator bytes are reused only when both
     neighbours are the same matched cells, in the same order, as on disk; otherwise the canonical
     separator is written, with `<!-- ds:cell -->` whenever two Markdown cells end up adjacent
     (e.g. after deleting the DataScript cell between them).
   - Nothing the client holds goes stale except `version`: a successful `PUT` returns
     `WriteResult { version }`, and the client sends that version with its next save.
   - **One save in flight per file**: because every save changes `version`, `ServerRuntime` never
     sends a second `PUT` for a path until the previous one has answered. Edits made meanwhile are
     merged into one follow-up save, sent with the fresh `version` (or, after a failure, retried
     with the old one). Without this, a slow disk turns two quick auto-saves into a `409` and a
     false "changed on disk" banner.
   - **`expectedVersion` is required** on every `PUT` to an existing file: a `PUT` without it is
     rejected with `428 Precondition Required`, because the `409` check and the content diff both
     need a base version. Only `POST` (create) omits it.
   - **Cell indexes**: `cellIndex` and `targetCellIndex` count **every** cell in document order,
     Markdown cells included, so the client can use the index of its own cell list directly.
     `statementIndex` counts only the query statements inside one DataScript cell (§3).

### Plain `.ds` scripts & Scratchpads

- **`.ds` files**: Opening any `.ds` file from the left tree opens it as a single-editor DataScript
  script view with the same diagnostics, completion, Run controls, result grid, charts, and
  right-hand inspector.
- **Launch Scratchpad**: When `ds ui` opens in a directory with no open notebooks (e.g. a folder of
  raw CSV/Parquet files), it opens an unsaved **Scratchpad** notebook with one empty DataScript
  cell (relative paths resolve against the workspace root). While empty, a quiet, dismissible
  quick-preview strip lists data files discovered in the folder, each with a one-click **Preview**
  action that inserts `from '<relpath>' | take 100` and runs it immediately. Pressing `Cmd/Ctrl+S`
  defaults the Save destination to the **workspace root** so root-relative paths remain valid (if
  the user selects a subfolder, the UI warns that relative paths in the Scratchpad may need
  updating to be relative to the subfolder).

### Saving, atomic writes & external edits

- **Saved files on disk** (`.dsnb`, `.ds`) use **debounced auto-save** (~500 ms after edits stop,
  Obsidian-style), with full undo/redo history and immediate flush on `Cmd/Ctrl+S`.
- **Scratchpads** stay in memory until explicitly saved.
- **Atomic writes & serialization**: Every `PUT /api/file` runs under a per-path `tokio::sync::Mutex`
  in the server so the `expectedVersion` check, content diff, and write never interleave. Writes
  go to a temporary file in the same directory, call `sync_all()`, preserve existing Unix
  permissions, and atomically rename into place—so a crash or kill mid-auto-save never truncates a
  notebook.
- **External-change protection**: Writes carry the last-known file `version` (etag from mtime +
  content hash). If the file changed on disk (e.g. edited in VS Code or switched by `git checkout`),
  auto-save pauses and displays a non-destructive banner (`Reload from disk` / `Overwrite disk`).
- **The server's own writes are not external changes**: the atomic rename fires a file-watch event
  for every save. The server remembers the `version` it last wrote per path and drops (does not
  broadcast) watch events whose resulting `version` matches it, so auto-save never trips the
  banner on its own writes.

---

## 3. Execution Model & Cross-Cell State

### Top-to-bottom lexical `let` scope (D-062)

A notebook behaves identically to a single DataScript program split into cells
([LANGUAGE.md §2.1](LANGUAGE.md#21-notebooks-dsnb)):

- Every DataScript cell sees all scalar and tabular `let` declarations from DataScript cells
  **above it** in document order, compiled via `datascript_compiler::StitchedProgram` (§6).
- **Whole-notebook live checking**: `POST /api/check` checks the **entire notebook in one pass**
  (the same check `ds check notebook.dsnb` performs in M4-06) and returns diagnostics grouped by
  `cellIndex`, so editing a `let` in cell 1 updates diagnostics across cells 2, 3, … in a single
  request instead of $O(N^2)$ per-cell checks.
- **Reordering cells** immediately updates scope and diagnostics deterministically—there is never
  hidden Jupyter-style execution-order state.
- **No shadowing across cells**: Re-declaring an existing `let` name in a lower cell reports
  `DS1003` with a secondary label pointing back to the earlier cell (D-007).
- **Lazy `let` visibility**: Because `let` bindings are logical aliases (not yet materialized),
  running a downstream cell re-scans the sources behind upstream `let`s. Each result's status line
  shows how many **distinct `SourceRef`s** its query plan scanned (computed by walking
  `PlanNode::Scan` nodes in `QueryPlan::root`, so `'data/*.parquet'` counts as 1 source — e.g.
  `4,589 rows · 105ms · 3 sources`). `sourceCount` is **per result** (per query statement), not a
  union across the cell; in a multi-statement cell each result sub-tab shows its own count.
  Materialized/cached `let` results are flagged as higher priority in [ROADMAP.md](ROADMAP.md).

### Transitive upstream `let` prepending & `DS1085`

This section is the single definition of when an interactive single-cell run is blocked.
[LANGUAGE.md §2.1](LANGUAGE.md#21-notebooks-dsnb) and D-062 link here rather than restating it.

- During live **`check`**, errors in an upstream cell are reported on that upstream cell itself;
  downstream references to a broken `let` receive the binder's error type silently (D-038) so
  errors do not cascade.
- When running cell $k$ interactively, `StitchedProgram` (in `datascript-compiler`, M4-08) parses
  every upstream DataScript cell **with recovery**, splitting it into statements, and computes cell
  $k$'s **transitive `let` dependencies** syntactically. It prepends **only** those upstream `let`
  statements, in document order, so an unrelated typo'd `let` in cell 1 never blocks cells below
  that don't reference it.
- **The dependency scan over-approximates on purpose**: any identifier token in cell $k$ (or in a
  prepended `let`) whose text equals an upstream `let` name counts as a reference, even when the
  binder would resolve it to a column. Prepending an unused `let` is harmless; missing a used one
  is not.
- If two upstream statements declare the same name, both are prepended; the later one carries
  `DS1003` (D-007), which blocks the run under the rule below.
- `StitchedProgram` emits **`DS1085`** (error) if and only if a transitively referenced upstream
  `let` statement has an **error-severity** diagnostic (lexical, syntax, or semantic, including
  `DS1003`). A broken `let` counts as declaring a name whenever recovery finds the name.

  Nothing else blocks: errors in unreferenced upstream `let`s, upstream query statements
  (including a broken query next to a valid `let` in the same cell), and warnings anywhere **do
  not block** running cell $k$. An upstream statement so broken that its name cannot be recovered
  (e.g. `let = …`, or a stretch whose statement boundaries are lost) declares nothing, so a name in
  cell $k$ that it might have defined is the ordinary unknown-name error (`DS1001`/`DS1002`) in
  cell $k$. Live check already shows the syntax error on the upstream cell, so no special
  diagnostic is needed.
- **`DS1085` shape** (DIAGNOSTICS.md style):
  - message: ``cannot run this cell because `orders` (cell 2) has errors``;
  - primary span: the first reference to the blocking name in cell $k$;
  - secondary label: the broken upstream statement (`` `orders` is defined here ``);
  - help: ``fix the error in cell 2, then run this cell again``. The web UI renders the secondary
    label as a jump link to that cell.
- **What the target cell contributes**: `Session::start` checks the whole stitched program and runs
  nothing if it has any error, so the stitched text decides what can block a run:
  - **Run cell** stitches every statement of cell $k$. An error in any statement of the cell
    blocks the whole cell run (no partial results), reported on that statement.
  - **Run statement at cursor** and **Export full result** stitch only the target query statement
    plus the `let` statements it transitively references (from cell $k$ and from upstream cells,
    in document order). Other query statements in cell $k$, and unreferenced `let`s in it, are left
    out, so a half-typed neighbouring statement never blocks running or exporting a working one.
    The target is then the only query in the program, so no `RunStream::select` is needed.

### IDE requests see every upstream `let`

Completion, hover, and signature help (`POST /api/complete`, `/api/hover`, `/api/signature-help`)
do **not** use the transitive stitching above, because completion must offer upstream `let` names
the cell does not reference yet. They use the whole-notebook stitching up to and including cell
$k$ (every DataScript cell above it, plus cell $k$), so every upstream `let` is in scope, exactly
as in live `check`.

### Explicit execution, staleness, `ds run` vs "Run all" & clocks

- Queries **never auto-run** on edit or reorder.
- Each cell tracks a fingerprint of its own source plus the transitively referenced upstream `let`
  definitions. When either changes after a cell has produced output, the output header shows a
  subtle **Stale** badge.
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
    stopping at the first failing cell so earlier cells' outputs remain visible. *(By contrast,
    `ds run notebook.dsnb` in M4-06 checks the entire notebook as one program upfront and refuses
    to run if any cell has an error; see [LANGUAGE.md §2.1](LANGUAGE.md#21-notebooks-dsnb).)*
  - **Cancel / Stop** (`Cmd/Ctrl+.`, or the inline Stop button) — cancels the active `RunStream`
    immediately (D-055).

### `let`-only cells & multi-statement cells (D-063, closes Q-004)

- **`let`-only cell**: Running a cell that defines only `let` statements validates its bindings,
  **suppresses `DS0004`** ("program has no query to run"), and displays a compact status line
  (`2 bindings defined · validated in 3ms`) with no empty table.
- **Multi-statement cell**: Running a cell with multiple query statements renders compact output
  sub-tabs (`Result 1`, `Result 2`, … — labelled with the `render` title if present) inside the
  cell's output frame.

### Result size limits & streaming export (D-064, closes Q-007)

- UI executions cap streamed output at `row_limit = 10_000` by default. Users can configure a
  per-workspace default (stored server-side in the user's OS config dir via `PATCH /api/prefs`,
  keyed by canonical workspace root, so it survives `ds ui`'s random port across launches; in the
  static WASM playground on a stable origin, stored in `localStorage`) or override it per run.
  Today the limit lives only inside `ResultStream::collect`, which buffers. M4-10 extracts that
  logic (count rows per batch, slice the final batch at the limit, drop the stream on the next row
  to cancel execution, report `truncated`) into one streaming-limit helper in `datascript-engine`,
  and `collect` calls the helper, so the CLI, `datascript-server`, M3-06, and M5 share a single
  implementation that cannot drift.
- When `truncated` is true, the runtime emits the `DS4060` info diagnostic
  (``result truncated to the first {N} rows``, where `{N}` is the run's `rowLimit`, e.g.
  `10,000`; primary span on the query statement, help:
  ``export the full result, or add `| take` or `| summarize` to the query``) and the cell footer
  shows a truncation notice using the run's actual `rowLimit` (e.g.
  `Showing first {N} rows · truncated`) with an **Export full result** menu.
- **Export full result** sends **the immutable cell snapshot captured at run time** (so exporting a
  Stale result still matches the code that produced the grid) and re-executes the exact statement
  with **the original run's `nowMicros` clock** (preserving `now()` / `ago()` windows per D-015),
  **the target `statementIndex`** (0-based among the target cell's own query statements, ignoring
  upstream and local `let` statements), and **the same upstream-`let` cell context**. Because
  export re-reads source files from disk, external changes to source files since the grid ran will
  be reflected in the exported file (noted in the export menu tooltip). To stream directly to disk
  without buffering in browser memory, export uses a two-step ticketed download (`POST /api/export`
  → one-shot `GET /api/export/<ticket>`, §6) streaming uncapped rows through
  `datascript_engine::writers`; a failed export is reported, never saved as a file that looks
  complete (§6). **v1 formats** are CSV and NDJSON, the existing streaming row
  writers, so M4-10 adds no writer. JSON is offered once M2-26 makes `write_json` stream (today it
  takes every batch at once), Parquet once M2-27 lands, and `.xlsx` once M2-28 lands; new formats
  come from the `output` operator's writers, never from export-only code. The export menu lists
  only the formats the running server can write.

---

## 4. Three-Pane Workspace Layout

```
┌──────────────────────┬─────────────────────────────────────────────┬──────────────────────────┐
│  [Files]        [+ ] │ [sales.dsnb ×] [Scratchpad ×]           [+] │ [Profile] [Outline] [Let]│
│ ──────────────────── │  notebooks / sales.dsnb          ▶ Run all  │ ──────────────────────── │
│  Filter files...     │ ┌─────────────────────────────────────────┐ │  4,589 Rows · 4 Columns  │
│  ▾ data/             │ │ # Monthly Revenue Analysis              │ │  Filter columns...       │
│      orders.parquet  │ │ Exploring top customers by region.      │ │  ─────────────────────── │
│      customers.csv   │ └─────────────────────────────────────────┘ │  T   id          4.59K - │
│  ▾ notebooks/        │ ┌─────────────────────────────────────────┐ │  #   downloads   null 88%│
│    ● sales.dsnb      │ │ ▶  let orders = from '../data/orders…'; │ │  T   region        4 -   │
│      adhoc.ds        │ │    validated · 1 binding (4ms)          │ │  dt  month        12 -   │
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

All three panes have drag-resizable splitters, and both sidebars collapse via
shortcuts (`Cmd/Ctrl+B` left, `Cmd/Ctrl+Shift+B` right) or top-bar icons. **The right pane starts
collapsed by default** so the notebook and wide result grids get the full width on
laptop screens; users open it on demand via `Cmd/Ctrl+Shift+B`, the top-bar inspector toggle, or
clicking a column header's stats trigger.

> **Staged delivery:** M4 v1 (`M4-08` → `M4-01` → `M4-10` ∥ `M4-02` → `M4-09` → `M4-03` ∥
> `M4-04`, where ∥ means the two can run in parallel) ships
> a focused single-notebook view with a read-only file tree + `.dsnb` creation, DataScript &
> Markdown cells, virtualized grid, ECharts, auto-save/conflict handling, and the collapsible
> Result Profiler + Source Schema inspector. Power-user and workspace features (multi-file tabs &
> pinning, file rename/delete, Cell Command Mode, Command Palette, Quick Open, and Outline/Scope
> tabs) land in `M4-05`; CLI/LSP `.dsnb` support lands in `M4-06`; optional local output caching
> lands in `M4-07`.

### 4.1 Left Pane — Working Directory Tree & Attached Tables

- **Full working directory view**: Displays the directory passed to `ds ui [dir]` (default `.`).
  Hidden files and common clutter (`.git`, `node_modules`, `target`, `.DS_Store`) are hidden by
  default with a quick toggle and live filename filter box.
- **Visual distinction**: Monochrome SVG file icons distinguish `.dsnb` notebooks, `.ds` scripts,
  supported data files (`.parquet`, `.csv`, `.tsv`, `.json`, `.jsonl`, `.ndjson`, `.xlsx`),
  folders, and other files (subtly dimmed).
- **Safe file CRUD** (confined strictly to the workspace root, D-016):
  - M4 v1 (`M4-01`, `M4-02`) supports creating and saving `.dsnb`/`.ds` files; folder creation,
    rename, and delete land in `M4-05`.
  - **The workspace file API never writes, renames, or deletes data files** (explicit query
    `output` operators from M2-25…M2-28 may still write data files inside the D-016 sandbox):
    - All mutating endpoints (`POST`, `PUT`, and M4-05's `POST /api/dir`, `PATCH`, `DELETE`) check
      every path component under the root via `symlink_metadata` and open the final path with
      `O_NOFOLLOW`, refusing any in-root symlink (such as `notes.ds -> data/orders.csv`). *(The
      tiny TOCTOU window of an external process swapping a parent directory component mid-request
      is accepted for a local single-user tool.)*
    - Creating (`POST`) uses `OpenOptions::create_new(true)` (`O_CREAT|O_EXCL`). Renaming
      (`PATCH`, M4-05) never overwrites, refusing any non-`.dsnb`/`.ds` extension or existing
      path:
      - **Files and folders use the same no-replace rename**: `rustix`'s
        `renameat_with(RENAME_NOREPLACE)` on Linux / `RENAME_EXCL` on macOS (safe wrapper), and
        `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` on Windows (a small `unsafe` FFI call
        with a comment explaining why, per AGENTS.md). It is one atomic step, so a crash never
        leaves the notebook under two names, and it works on filesystems without hard links
        (FAT32/exFAT drives, many SMB/NFS shares).
      - Where the no-replace rename is not supported (`EINVAL`/`ENOTSUP`, e.g. some network
        filesystems on Linux), **files** fall back to `std::fs::hard_link(from, to)` +
        `remove_file(from)` (not atomic: a crash between the two steps leaves both names, which
        is safe). **Folders** cannot be hard-linked, so a folder rename has no fallback. If the
        no-replace rename (or the file fallback) fails, refuse with a plain error that names the
        filesystem limitation; never fall back to a rename that can overwrite.
    - **OS clutter files** (`.DS_Store`, `Thumbs.db`, `desktop.ini`, `._*` AppleDouble files) are
      allowed inside a folder being deleted or renamed. Finder creates `.DS_Store` in every folder
      it opens, and the tree hides these files, so refusing them would block almost every folder
      on macOS with no visible reason.
    - Deleting a folder (`DELETE`, M4-05) never calls `remove_dir_all`. It runs in two phases:
      1. **Verify everything first**: walk the whole tree (nested subfolders included) with
         `symlink_metadata`, and refuse the request before deleting anything if it finds a
         symlink or any file other than `.dsnb`/`.ds` and OS clutter. The refusal names the first
         blocking path (relative to the root), including hidden files.
      2. **Delete bottom-up**: unlink the verified files, then `rmdir` each folder, deepest first.
         If a new file appeared concurrently, that `rmdir` fails and the request stops. Files
         already unlinked stay deleted, and the response lists what was deleted and what was
         left, so the UI can say so instead of claiming nothing happened.

      Renaming a folder runs the same verify phase (it never needs the delete phase).
- **Data file interactions (notebook-relative paths)**:
  - **Single-click** selects the file and opens its **Source Schema** in the right-hand inspector.
  - **Double-click** (or inline hover action) inserts `from '<rel>'` (or `'<rel>'` when the cursor
    is already in a table position, D-036) into the active cell, or creates a preview cell
    (`from '<rel>' | take 100`), where `<rel>` is the POSIX path **relative to the active
    notebook's parent directory** (e.g. `../data/orders.parquet` inside `notebooks/sales.dsnb`, or
    `data/orders.parquet` in a root-level notebook or Scratchpad).
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
  - Rendered by default (GFM: headings, tables, task lists, inline code, links, code blocks)
    through DOMPurify, backed by the SPA's strict Content-Security-Policy (§6). **External remote
    `http(s)://` images and relative workspace image files are not loaded in v1** (inline `data:`/
    `blob:` images only; prevents tracking pixels from cloned repos and avoids exposing an
    arbitrary file-serving route).
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
    for `DS1085` upstream cell errors).
  - **Tabular output (virtualized Arrow grid, `M4-09`)**:
    - Reads Arrow IPC `RecordBatch`es directly via `apache-arrow` with row and column virtualization
      (`@tanstack/react-virtual`).
    - Row index gutter, compact monochrome column headers with type glyphs (`T` string, `#`
      long/int/real/decimal, `dt` datetime, `ts` timespan, `b` bool, `id` guid, `{}` dynamic) and a subtle
      null indicator (clicking opens the column in the right-hand profiler), subtle italic `NULL`
      cell styling, column resizing, click-to-copy cell/row, and quick export menu.
    - **Truncation-aware sorting**: client-side column sort clearly labels *"sorted first {N}
      rows"* using the run's actual `rowLimit` whenever the result is truncated (D-064), and every
      column header offers a **Sort in query** action that inserts `| sort by <col> desc` (or
      `asc`, always explicit so `DS2103` never fires, D-005) **before any trailing `| render ...`**
      (or updates an existing `sort` in place) and re-runs over all rows so code remains the single
      source of truth.
  - **Visualization output (`| render`, `M4-03`)**:
    - Queries ending with `| render <kind>` default to Chart view with a **Chart / Table**
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
     - Quick actions: **Insert `from '<rel>'`** (relative to the active notebook's parent dir),
       **New preview cell (`take 100`)**, and **Describe** (`from '<rel>' | describe`, wired once
       M2-23 lands).
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
- **Charts**: Apache ECharts (tree-shaken modules covering all 9 `RenderSpec` kinds; the Canvas
  renderer draws on screen, and SVG download re-renders the chart off-screen with the
  `SVGRenderer`, since a Canvas chart can only export PNG)
- **Markdown rendering**: `marked` or `markdown-it` + DOMPurify, reinforced by the server's strict
  CSP header
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

  // Workspace info & preferences (persisted in OS config dir on ds ui; localStorage on WASM)
  getInfo(): Promise<WorkspaceInfo>;
  updatePrefs(patch: Partial<WorkspacePrefs>): Promise<WorkspacePrefs>;

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
  // Returns structured cells ({ kind, text }, parsed by datascript-compiler) for `.dsnb`, or raw text for `.ds`, plus version/etag
  readFile(path: string): Promise<FileDocument>;
  // expectedVersion is required (428 without it); at most one writeFile per path in flight (§2); WriteResult = { version }
  writeFile(path: string, doc: FileDocumentInput, expectedVersion: string): Promise<WriteResult>;
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
| **Local Web UI (`ds ui`)** | `ServerRuntime` (M4) | HTTP + WebSocket to `datascript-server` (`axum`, 127.0.0.1, `HttpOnly` session cookie, Origin check, CSP, D-016 sandbox) |
| **WASM Playground** | `WasmRuntime` (M5-03) | In-browser Web Worker (`@datascript/wasm`); dropped/sample files mapped as virtual workspace entries; parses/serializes `.dsnb` via the same compiled-to-WASM `datascript-compiler` module |
| **VS Code Results Panel** | Direct component import (M3-06) | Imports `web/src/components/results/` (Arrow grid from M4-09, ECharts `RenderSpec` chart from M4-03, diagnostic cards) into a dedicated Vite webview bundle |
| **Future Desktop App** | `TauriRuntime` (later) | Tauri 2 shell wrapping `web/` + Rust `Session` directly via Tauri IPC (no Node/Electron), adding native menus, folder picker, and OS `.dsnb` double-click file association |

### Server auth, CSP & API contract (`datascript-server`, M4-01, M4-10, M4-04 & M4-05)

- **Cookie-based session auth**: Opening the launch URL `http://127.0.0.1:<port>/?token=<token>`
  validates the per-launch token on the server, mints a **separate random session id**, sets
  `Set-Cookie: ds_session_<port>=<session id>; HttpOnly; SameSite=Strict; Path=/`, and responds
  with `302 Found` to `/`. Page reloads (`F5`), duplicate tabs, and restored tabs remain
  authenticated for the server's lifetime, and neither secret is readable from JavaScript.
  - **Why the port is in the cookie name**: cookies are not isolated by port (RFC 6265), and
    SameSite ignores the port too, so every server on `127.0.0.1` shares one cookie jar. A fixed
    name would let a second `ds ui` (on another port) overwrite the first one's cookie and sign
    its tabs out. The cookie value is a session id rather than the launch token because the launch
    token is printed in the terminal and lands in browser history.
  - **`GET /` without a valid session cookie** (and without a valid `?token=`) returns a small
    static page saying to open the URL that `ds ui` printed, rather than an SPA that then fails
    every API call with `401`.
- **`Origin` enforcement**: Every mutating HTTP endpoint (`POST`, `PUT`, `PATCH`, `DELETE`) and the
  `WS /api/ws` handshake require the session cookie AND verify that the `Origin` header
  matches the server's local origin. Read endpoints (`GET`) require the session cookie.
- **Content-Security-Policy (defense in depth)**: The SPA HTML response sends:
  ```text
  Content-Security-Policy: default-src 'self'; img-src 'self' data: blob:; connect-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; object-src 'none'; frame-src 'none'
  ```
  (`style-src 'unsafe-inline'` is required by CodeMirror 6 and ECharts runtime styling; this
  enforces remote-image blocking and blocks `<video poster>`, `<audio>`, CSS external `url()`,
  inline scripts, and iframes at the browser level even if Markdown sanitization ever had a bug.)
- **Streaming downloads**: `POST /api/export` (cookie + `Origin` check) mints a short-lived
  single-use download ticket (`/api/export/<ticket>`). `GET /api/export/<ticket>` requires the
  session cookie, verifies `Sec-Fetch-Site: same-origin` when present, consumes the ticket
  atomically, and streams `Content-Disposition: attachment` straight to disk without buffering in browser
  memory.
- **Export errors never produce a file that looks complete**, and never leave the page:
  - `POST /api/export` checks and plans the snapshot and starts the query (`Session::start` up to
    the target `QueryStream`) before it mints the ticket. Check, `DS1085`, sandbox, and
    source-not-found errors come back from the `POST` as diagnostics, with no ticket. The ticket
    holds the started stream; it expires after 60 s, and expiry drops the stream (cancelling the
    query).
  - The SPA starts the download by clicking a programmatic `<a href="/api/export/<ticket>"
    download>`. A same-origin `download` link never unloads the page, and a `4xx` answer (ticket
    used, expired, or unknown) shows as a failed download instead of replacing the SPA. This is why
    `frame-src 'none'` in the CSP costs nothing: no hidden iframe is needed.
  - If a runtime or I/O error happens after the `200` headers are sent, the server **aborts the
    connection** (no final chunk / no clean end), so the browser marks the download as failed
    rather than complete. The server also sends an `exportFailed` event with the `Diagnostic`
    over `WS /api/ws`, and the cell footer shows it.

| Endpoint | Milestone | Purpose |
|---|---|---|
| `GET /?token=...` | M4-01 | Validates launch token, sets the `HttpOnly; SameSite=Strict` `ds_session_<port>` cookie (random session id), `302` redirects to `/`; without a valid cookie or token, serves a static "open the printed URL" page |
| `GET /api/info` | M4-01 | Root folder name, capabilities, persisted preferences (`row_limit`, theme), registered host tables |
| `PATCH /api/prefs` | M4-01 | Persist per-root UI preferences (`row_limit`, theme) in the user's OS config dir (`dirs::config_dir()`) |
| `GET /api/files?dir=&includeHidden=` | M4-01 | Directory tree under root (sandboxed, D-016) |
| `GET /api/file?path=` | M4-01 | Read a `.dsnb` (returned as parsed cells `{ kind, text }` + frontmatter) or `.ds` text file + `version` (etag) |
| `PUT /api/file` | M4-01 | Atomic save (per-path lock, required `expectedVersion` (`428` without it), `409` check, lossless `.dsnb` content diff against the on-disk file via `datascript-compiler`, temp file + `sync_all` + rename; refuses non-`.dsnb`/`.ds` paths and symlink components). Returns `WriteResult { version }` |
| `POST /api/file` | M4-01 | Create a new `.dsnb` or `.ds` file atomically (`O_CREAT\|O_EXCL`; refuses non-`.dsnb`/`.ds` paths, existing files, and symlink components) |
| `POST /api/dir`, `PATCH /api/file`, `DELETE /api/file` | M4-05 | Create folders, rename (no-clobber: `rustix` no-replace rename / `MoveFileExW` for files and folders, `hard_link` fallback for files only), or delete `.dsnb`/`.ds` files or safe folders (verify the whole tree first, then per-file unlink + bottom-up `rmdir`; refuses symlinks and any folder containing files other than `.dsnb`/`.ds` and OS clutter such as `.DS_Store`) |
| `GET /api/schema?source=` | M4-01 | Inspect schema of a file or host table via `Session` |
| `POST /api/check` | M4-01 | Check a `.ds` script or whole `.dsnb` notebook in one pass (`{ path, cells }` → diagnostics grouped by `cellIndex` (every cell, Markdown included) in UTF-16 offsets + per-cell in-scope `let` summaries) |
| `WS /api/ws` | M4-10 | Multiplexed stream for: (1) query execution (`run` with optional shared `nowMicros` for Run all / `cancel` → `started` with `nowMicros` and `rowLimit`, then per result its `statementIndex` and `sourceCount`, streaming capped Arrow IPC batches + `RenderSpec` + diagnostics incl. `DS4060`/`DS1085` + timing, suppressing `DS0004` on `let`-only cells), and (2) workspace file-change notifications |
| `POST /api/export` | M4-10 | Accepts the run-time cell snapshot `{ path, cells, targetCellIndex, statementIndex, nowMicros, format }` (`statementIndex` is 0-based among the target cell's own query statements), checks, plans and starts the query (returning diagnostics instead of a ticket on failure), and returns a short-lived single-use download URL `/api/export/<ticket>` |
| `GET /api/export/<ticket>` | M4-10 | Requires the session cookie + one-shot ticket (checks `Sec-Fetch-Site` when present) and streams the full uncapped result as `Content-Disposition: attachment`, aborting the connection on a mid-stream error (plus a WS `exportFailed` event) via `datascript_engine::writers` (v1: `csv`, `ndjson`; `json` once M2-26 lands, `parquet` once M2-27 lands, `xlsx` once M2-28 lands) |
| `POST /api/complete`, `/api/hover`, `/api/signature-help` | M4-04 | IDE helpers (`{ path, cells, targetCellIndex, offset }`) backed by `datascript_compiler::ide` over the whole-notebook `StitchedProgram` up to the target cell (every upstream `let` in scope, §3) |

### Cross-cell combined program & `.dsnb` parser (`datascript-compiler`, M4-08)

Both the dependency-free, line-based `.dsnb` Markdown fence scanner/serializer and the
segment-stitching / span-mapping primitive (`StitchedProgram`, extracted from `ReplProgram` in
`datascript-cli`, which delegates to it) live in **`datascript-compiler`** (pure, no I/O,
`wasm32`-compatible). `datascript-server` (M4-01, M4-10, M4-04), `datascript-cli` (`ds run`/
`ds check`/`ds explain` on `.dsnb`, M4-06), `datascript-lsp` (M4-06), and `datascript-wasm` (M5)
all reuse them:
- Whole-notebook check stitches every DataScript cell's statements in document order (mapping every
  diagnostic span back to `{ cellIndex, start, end, line, column }` in UTF-16, D-025).
- Single-cell and single-statement execution (`run`, `export`) compute the target's **transitive
  upstream `let` dependencies**, prepend only those `let` statements in document order separated
  by `\n;\n` glue segments, and emit `DS1085` under the rules in
  [§3](#transitive-upstream-let-prepending--ds1085) (a statement run or export also leaves out the
  target cell's other query statements).
- IDE requests (`complete`, `hover`, `signature_help`) use the whole-notebook stitching up to and
  including the target cell, so completion offers every upstream `let`
  ([§3](#ide-requests-see-every-upstream-let)).
- Relative file paths inside the notebook resolve against the `.dsnb` file's parent directory (or
  the workspace root for unsaved Scratchpads), always enforced by the D-016 sandbox in `ds ui`.

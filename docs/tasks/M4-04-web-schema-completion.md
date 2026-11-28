# M4-04 — Web UI: contextual inspector, result profiler & IDE completion

**Status:** todo
**Depends on:** M4-02, M3-02, M2-13
**Crates:** `web/`, `datascript-server`, `datascript-compiler`, `datascript-cli`

## Goal
Build the collapsible right-hand **Contextual Inspector** (DuckDB-style live Result Profiler,
Source Schema viewer, Obsidian-style Outline, and In-Scope `let` browser) and wire CodeMirror 6
completion, hover, and signature help to the same `datascript_compiler::ide` APIs the LSP uses
([WEB_UI.md §4.3](../WEB_UI.md#43-right-pane--contextual-inspector-collapsible)).

## Scope
- **Server IDE endpoints** (`datascript-server`):
  - `POST /api/complete`, `POST /api/hover`, and `POST /api/signature-help`, backed by
    `datascript_compiler::ide::complete`, `hover`, and `signature_help` (M3-02, D-057) over the
    same cross-cell combined program built in M4-01 so upstream notebook `let`s and inferred stage
    schemas complete accurately inside any cell.
  - The server lists sandboxed files for `Completions::files` (D-016) as the LSP does, and passes
    registered `--table` host table names into `ide::complete`.
- **Editor IDE extensions** (`web/`):
  - CodeMirror 6 autocomplete (columns, operators, functions, `let`s, host tables, sandboxed file
    paths), hover tooltips, and function signature help popups driven through `Runtime`.
- **Right-hand Contextual Inspector** (`web/`):
  - **Inspector tab (default, contextual)**:
    - *Live Result Profile* (when a cell with results is focused): total rows (noting 10k cap if
      truncated) & column count, copy-schema button, column filter & sort (`Default`, `Name`,
      `Null %`, `Distinct`), and per-column rows showing type badge, distinct count, null % badge,
      and expandable distribution summary / min–max / top values—computed client-side over the
      cell's Arrow `RecordBatch`es so it works identically in WASM (M5-03).
    - *Source Schema* (when a data file or attached table is selected in the left tree): path,
      format, size, searchable column list (`name`, DataScript/Arrow type), and quick actions
      (**Insert `from '...'`**, **New preview cell**, and **Describe** once M2-23 lands).
  - **Outline tab**: jump list of Markdown headings and DataScript cells (showing `let` names or
    query titles).
  - **Scope (`let`s) tab**: all scalar and tabular `let` bindings (types/columns + jump link to
    defining cell) visible to the currently focused cell.

## Acceptance criteria
- [ ] Completion, hover, and signature-help results match the LSP for the same text and cursor
      (shared test vectors), plus tests for cross-cell `let` completion in notebooks.
- [ ] Vitest unit tests for the client-side Arrow column profiler (null %, distinct counts,
      numeric/string/datetime summaries, empty tables).
- [ ] Playwright tests verifying live Result Profile updates when switching focused cells, Source
      Schema inspection from the left tree, Outline navigation, and editor completion/hover.

## Carried over
- [ ] (from M3-02) `datascript_compiler::ide::complete` knows no host tables (the LSP has none). When `ds ui` gets host tables, give `complete` a way to receive their names so they are offered after `from`, next to tabular `let`s. File names come back as `Completions::files` for the server to list, as the LSP does in `crates/datascript-lsp/src/ide.rs`.
- [ ] (from M3-07) Once `datascript_compiler::ide::complete` accepts host tables, pass the REPL's `--table` names to it and remove the `is_table_position` / `add_host_tables` workaround in `crates/datascript-cli/src/repl/editor.rs` (and `CompletionContext::let_names_before` in `crates/datascript-cli/src/repl/session.rs`).
- [ ] (from planning, 2026-11-24) If M2-23 is done, add a "Describe" action to the schema browser that runs `from <file> | describe`, with a Playwright or Vitest test. (If M2-23 lands after this task, M2-23 does it.)

## Follow-ups

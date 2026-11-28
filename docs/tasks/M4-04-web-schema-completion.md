# M4-04 — Web UI: result profiler, source schema inspector & IDE completion

**Status:** todo
**Depends on:** M4-09, M3-02, M2-13
**Crates:** `web/`, `datascript-server`, `datascript-compiler`, `datascript-cli`
**Diagnostic codes:** none

## Goal
Populate the on-demand right-hand **Contextual Inspector** (DuckDB-style live Result Profiler and
Source Schema viewer, starting collapsed by default per
[WEB_UI.md §4.3](../WEB_UI.md#43-right-pane--contextual-inspector-collapsed-by-default)) and wire
CodeMirror 6 completion, hover, and signature help to `datascript_compiler::ide`. *(Outline and
Scope tabs follow in M4-05.)*

## Scope
- **Server IDE endpoints** (`datascript-server`):
  - `POST /api/complete`, `POST /api/hover`, and `POST /api/signature-help` backed by
    `datascript_compiler::ide` (M3-02, D-057) over `StitchedProgram` (M4-08), listing sandboxed
    files for `Completions::files` (D-016) and passing registered `--table` host tables.
- **Editor IDE extensions** (`web/`): CodeMirror 6 autocomplete, hover tooltips, and signature help
  through `Runtime`.
- **Right-hand Inspector** (`web/`, starts collapsed; opens via `Cmd/Ctrl+Shift+B`, top-bar icon,
  file click, or grid column-header stats click):
  - **Live Result Profile**: client-side profiling over loaded Arrow batches (distinct count, null
    %, distributions, search/sort, labelling `"of first {N} rows"` using the run's actual `rowLimit`
    when truncated per D-064).
  - **Source Schema**: file/host-table columns & types plus **Insert `from '<rel>'`** (relative to
    the active notebook's parent directory), **New preview cell**, and **Describe** (once M2-23
    lands).

## Acceptance criteria
- [ ] Completion, hover, and signature-help results match the LSP for the same text and cursor
      (shared test vectors), plus cross-cell `let` completion tests.
- [ ] Vitest unit tests for the client-side Arrow column profiler (including dynamic truncation
      labelling with both default and non-default `rowLimit`) and subfolder-relative `Insert from`
      paths.
- [ ] Playwright tests verifying the right pane starts collapsed, opens on demand for Result
      Profile / Source Schema, updates across cells, and drives editor completion/hover.

## Carried over
- [ ] (from M3-02) `datascript_compiler::ide::complete` knows no host tables (the LSP has none). When `ds ui` gets host tables, give `complete` a way to receive their names so they are offered after `from`, next to tabular `let`s. File names come back as `Completions::files` for the server to list, as the LSP does in `crates/datascript-lsp/src/ide.rs`.
- [ ] (from M3-07) Once `datascript_compiler::ide::complete` accepts host tables, pass the REPL's `--table` names to it and remove the `is_table_position` / `add_host_tables` workaround in `crates/datascript-cli/src/repl/editor.rs` (and `CompletionContext::let_names_before` in `crates/datascript-cli/src/repl/session.rs`).
- [ ] (from planning, 2026-11-24) If M2-23 is done, add a "Describe" action to the schema browser that runs `from <file> | describe`, with a Playwright or Vitest test. (If M2-23 lands after this task, M2-23 does it.)

## Follow-ups

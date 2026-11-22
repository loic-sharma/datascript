# M4-04 — Web UI: schema browser and completion

**Status:** todo
**Depends on:** M4-02, M3-02, M2-13
**Crates:** `web/`, `datascript-server`

## Goal
A schema panel (columns and types per file, expandable) and editor completion and hover backed by
the same compiler APIs the LSP uses (`/api/complete`).

## Acceptance criteria
- [ ] Completion results match the LSP for the same text and cursor (shared test vectors).

## Carried over
- [ ] (from M3-02) `datascript_compiler::ide::complete` knows no host tables (the LSP has none). When `ds ui` gets host tables, give `complete` a way to receive their names so they are offered after `from`, next to tabular `let`s. File names come back as `Completions::files` for the server to list, as the LSP does in `crates/datascript-lsp/src/ide.rs`.

## Follow-ups

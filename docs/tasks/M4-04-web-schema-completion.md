# M4-04 — Web UI: schema browser and completion

**Status:** todo
**Depends on:** M4-02, M3-02, M2-13
**Crates:** `web/`, `datascript-server`

## Goal
A schema panel (columns and types per file, expandable) and editor completion and hover backed by
the same compiler APIs the LSP uses.

## Scope
- `POST /api/complete` and `POST /api/hover` in `datascript-server`, backed by
  `datascript_compiler::ide::complete` and `hover` (M3-02, D-057). The server lists files for
  `Completions::files` as the LSP does. (Moved here from M4-01, which does not depend on M3-02.)
- Signature help (`ide::signature_help`) the same way, if the editor shows it.

## Acceptance criteria
- [ ] Completion and hover results match the LSP for the same text and cursor (shared test vectors).

## Carried over
- [ ] (from M3-02) `datascript_compiler::ide::complete` knows no host tables (the LSP has none). When `ds ui` gets host tables, give `complete` a way to receive their names so they are offered after `from`, next to tabular `let`s. File names come back as `Completions::files` for the server to list, as the LSP does in `crates/datascript-lsp/src/ide.rs`.

## Follow-ups

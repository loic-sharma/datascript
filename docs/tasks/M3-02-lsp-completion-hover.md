# M3-02 — Language server: completion, hover, signature help

**Status:** todo
**Depends on:** M3-01
**Crates:** `datascript-lsp`, `datascript-compiler`

## Goal
Context-aware completion: operators after `|`, columns from `stage_schemas` at the cursor,
functions (with snippets), lets, render kinds and properties, reader options, and file paths
inside source strings. Hover shows types and catalog docs. Signature help for function calls.

## Acceptance criteria
- [ ] Completion works in incomplete code (`orders | where am|`), relying on parser recovery.
- [ ] LSP tests for each completion context.

## Follow-ups

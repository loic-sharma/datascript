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

## Carried over
- [ ] (from M1-24) Completion after `from ` (a `PIPELINE` whose `from_token()` is followed by nothing or by a partial name) offers only tables and paths: table `let`s, host tables, scalar `let`s bound to strings, reader functions and files. The right side of `join`/`union`/`lookup` is the same kind of position, without `from`.

## Follow-ups

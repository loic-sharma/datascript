# M3-07 — REPL column completion

**Status:** done — REPL Tab completion now analyses the current input after the saved lets via `CompletionContext` (reusing `Session`'s schema cache without invalidating it) and delegates language items to `datascript_compiler::ide::complete`, adding REPL-only host tables, file listing and meta-command completion.
**Depends on:** M3-02
**Crates:** `datascript-cli`
**Diagnostic codes:** none expected

## Goal
Tab in the REPL completes column names, using the compiler's `complete` from M3-02, so the REPL,
the LSP, the `ds ui` server and WASM offer the same columns for the same text.

## Scope
- M1-12 left column completion out of the REPL's Tab completion; today it offers operators,
  functions, saved `let` names, host tables and meta commands only.
- Analyse the current input as its own `<repl:N>` snippet with the saved lets in front (D-048), the
  same way the REPL runs it, then call `complete` at the cursor. Map the cursor offset across the
  saved-lets prefix.
- Where `complete` already offers operators, functions and lets, prefer its items over the REPL's
  own lists, so the two cannot disagree. Meta command completion (input starting with `.`) stays
  REPL-only.
- Schema lookups go through the session's cache, as `ds run` does; Tab must not re-read a file the
  session already read.

## Acceptance criteria
- [x] `from 'tests/data/orders.csv' | where am<Tab>` offers `amount`.
- [x] Columns from a saved `let` table work (`let big = from '…' | where amount > 100;` then `from big | project <Tab>`).
- [x] A source that fails to load gives no columns and no error output on Tab.
- [x] Tests in `datascript-cli` cover these cases.

## Follow-ups
- None. Host tables are added in table-source positions (`from`, `join`, `union`, `.schema`) by the
  REPL until `datascript_compiler::ide::complete` accepts host tables (M4-04).

# M3-02 — Language server: completion, hover, signature help

**Status:** in-progress
**Depends on:** M3-01
**Crates:** `datascript-lsp`, `datascript-compiler`

## Goal
Context-aware completion: operators after `|`, columns from `stage_schemas` at the cursor,
functions (with snippets), lets, render kinds and properties, reader options, and file paths
inside source strings. Hover shows types and catalog docs. Signature help for function calls.

`hover(&Analysis, offset)` lives in `datascript-compiler` next to `complete`, for the same reason:
the LSP, the `ds ui` server (M4-04) and WASM (M5-02) must give the same answer for the same text.
The LSP only converts the result to LSP types.

## Acceptance criteria
- [ ] Completion works in incomplete code (`from orders | where am|`), relying on parser recovery.
- [ ] LSP tests for each completion context.
- [ ] ARCHITECTURE.md's facade sketch lists `hover` next to `complete`, with its final signature.

## Carried over
- [ ] (from M3-01) Call `complete` through `datascript_engine::catch_panic`, as the server already does for `parse`, so a compiler bug becomes `DS9001` instead of killing the server.
- [ ] (from M1-24) Completion after `from ` (a `PIPELINE` whose `from_token()` is followed by nothing or by a partial name) offers only tables and paths: table `let`s, host tables, scalar `let`s bound to strings, reader functions and files. The right side of `join`/`union`/`lookup` is the same kind of position, without `from`.

- [ ] (from M3-04) A server started with `initializationOptions.syntaxOnly` (untrusted workspace, D-053) must not read schemas for completion, hover or signature help: offer keywords, operators, functions and `let` names, but no columns. Test it next to `syntax_only_mode_reads_no_files` in `crates/datascript-lsp/tests/lsp.rs` (the `NoReads` engine panics on any read).

## Follow-ups

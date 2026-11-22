# M3-02 — Language server: completion, hover, signature help

**Status:** done — `datascript_compiler::ide` has `complete`, `hover` and `signature_help` over `(src, analysis, offset)`; the LSP serves them (fresh check per request, files listed by the server, nothing read in syntax-only mode, each call behind `catch_panic`) (D-057).
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
- [x] Completion works in incomplete code (`from orders | where am|`), relying on parser recovery.
- [x] LSP tests for each completion context. (`completion_in_each_context` and friends in
      `crates/datascript-lsp/tests/lsp.rs`; one unit test per context in
      `crates/datascript-compiler/src/ide/tests.rs`.)
- [x] ARCHITECTURE.md's facade sketch lists `hover` next to `complete`, with its final signature.

## Carried over
- [x] (from M3-01) Call `complete` through `datascript_engine::catch_panic`, as the server already does for `parse`, so a compiler bug becomes `DS9001` instead of killing the server.
- [x] (from M1-24) Completion after `from ` (a `PIPELINE` whose `from_token()` is followed by nothing or by a partial name) offers only tables and paths: table `let`s, host tables, scalar `let`s bound to strings, reader functions and files. The right side of `join`/`union`/`lookup` is the same kind of position, without `from`.

- [x] (from M3-04) A server started with `initializationOptions.syntaxOnly` (untrusted workspace, D-053) must not read schemas for completion, hover or signature help: offer keywords, operators, functions and `let` names, but no columns. Test it next to `syntax_only_mode_reads_no_files` in `crates/datascript-lsp/tests/lsp.rs` (the `NoReads` engine panics on any read).

## Follow-ups
- The signature is `(src: &SourceFile, analysis: &Analysis, offset: usize)`, not
  `(&Analysis, offset)`: the analysis holds no text, so each function parses `src` again (D-057).
- Host tables are not offered: the LSP has none, and `complete` has no input for them yet.
  Handed to M4-04.
- Handed over: new M2 operators (M2-01), user-defined functions (M2-05), `extend` items seeing
  earlier items and `$left.`/`$right.` (M3-03).
- Completion runs a fresh `Session::check` per request, which is cheap with warm schemas
  (M3-01 measured 1.4 ms for 500 lines).

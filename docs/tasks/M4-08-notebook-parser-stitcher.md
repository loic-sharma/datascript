# M4-08 — `.dsnb` notebook parser/serializer & shared `StitchedProgram`

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-diagnostics`, `datascript-cli`
**Diagnostic codes:** DS4061 (upstream cell has errors per D-062; allocated from M4's
DS4060–DS4064 block per D-018)

## Goal
Build the single canonical, pure, `wasm32`-compatible `.dsnb` Markdown parser/serializer and
`StitchedProgram` helper in `datascript-compiler` (including `DS4061` emission) so
`datascript-server` (M4-01, M4-10, M4-04), `ds run`/`ds check`/`ds lsp` (M4-06), `datascript-cli`'s
REPL, and `datascript-wasm` (M5) all share one implementation
([WEB_UI.md §2](../WEB_UI.md#2-notebook-file-format-dsnb),
[§3](../WEB_UI.md#transitive-upstream-let-prepending--ds4061),
[§6](../WEB_UI.md#cross-cell-combined-program--dsnb-parser-datascript-compiler-m4-08), D-062).

## Scope
- **Line-based `.dsnb` parser & lossless serializer** (`datascript-compiler`, zero new crate
  dependencies so `wasm32` stays small):
  - Implement the top-level CommonMark fence scanner (0–3 leading spaces with content-indent
    stripping; fences inside blockquotes or lists stay Markdown), canonical layout, empty-Markdown
    drop, and `originId`-based lossless diff serializer specified in
    [WEB_UI.md §2](../WEB_UI.md#serialization-grammar--canonical-layout):
    - YAML frontmatter preserved verbatim;
    - backtick and tilde fences (`ds`/`datascript`), nested fences per CommonMark, unclosed fences
      running to EOF, extra info tokens preserved on untouched cells and ignored in v1;
    - whitespace-only text between DataScript cells treated as separator (empty Markdown cells
      dropped on save/reload), canonical `\n\n` separator for newly serialized cells, no persisted
      cell metadata in v1;
    - `<!-- ds:cell -->` adjacent-Markdown delimiter with lossless `<!-- ds:cell:literal -->`
      escaping/unescaping;
    - opaque per-cell `originId` (byte range + content hash) returned by the parser and matched on
      serialize-with-base so untouched cells and separators retain exact original bytes and line
      endings unambiguously even with duplicate identical cells or reorders. Serializing also
      returns the fresh `originId` of every cell in the written output (the server sends these back
      from `PUT`), and an `originId` not found in the base is a reported mismatch, never silently
      re-serialized.
- **Shared `StitchedProgram` & `DS4061`** (`datascript-compiler`, `datascript-diagnostics`):
  - Extract the pure segment-stitching and offset-mapping primitive from `datascript-cli`'s
    `ReplProgram`, supporting:
    1. whole-notebook stitching (for `POST /api/check`, `ds check`, `ds run`);
    2. single-cell stitching that syntactically computes the target cell's **transitive upstream
       `let` dependencies** (parsing upstream cells with recovery, over-approximating references
       by identifier text), prepends only those `let` statements in document order, and emits
       **`DS4061`** (register active in `codes.rs` & `DIAGNOSTICS.md`) exactly per rules (a) and
       (b) in [WEB_UI.md §3](../WEB_UI.md#transitive-upstream-let-prepending--ds4061), with the
       message, primary span, secondary label, and help given there.
  - Map diagnostic spans back to `(cell_index, local_span)` and UTF-16 positions (D-025).
  - Have `datascript-cli`'s `ReplProgram` delegate to `datascript_compiler::StitchedProgram`.

## Acceptance criteria
- [ ] Fixture-driven round-trip tests in `datascript-compiler` covering: YAML frontmatter, CRLF and
      LF preservation on untouched cells, whitespace-only gaps and empty Markdown drop, top-level
      0–3 space indent stripping, `ds` fence inside a blockquote/list staying Markdown,
      `datascript` and `~~~ds` fences, extra info tokens, DataScript cells containing ```` ``` ````,
      ` ```ds ` nested inside a 4-backtick non-`ds` fence, unclosed fence to EOF, adjacent Markdown
      cells with `<!-- ds:cell -->`, literal `<!-- ds:cell -->` escaping, and `originId`
      partial-edit diff preservation across duplicate identical cells and reorders.
- [ ] Repeated-save test: parse, edit cell A, serialize, re-parse to get fresh `originId`s, edit
      cell B, serialize again. Cell A (CRLF, `~~~datascript` fence) and every untouched cell keep
      their bytes after the second save, and a stale `originId` is reported as a mismatch rather
      than silently canonicalized.
- [ ] Unit tests for `StitchedProgram` whole-notebook and transitive single-cell stitching, span
      mapping (multi-byte UTF-8/UTF-16), and `DS4061`: fires under rule (a) (referenced `let` with
      a type error; referenced name declared twice upstream, `DS1003`) and rule (b) (upstream
      `let` with a missing name or `=`, and the target references an otherwise undeclared name);
      does *not* fire on an unreferenced broken `let`, a broken query next to a valid `let` in the
      same cell, an upstream warning, or an undeclared name with no upstream syntax error (that is
      the ordinary unknown-name diagnostic); a column whose name matches an upstream `let`
      prepends that `let` harmlessly.
- [ ] Rendered-output snapshot tests (error output is first-class behaviour, AGENTS.md): `.dsnb`
      fixtures under `crates/datascript-compiler/tests/notebooks/` whose expected output is the
      rendered diagnostic text (code, message, primary span, secondary label, help, `.dsnb` line
      and column) for `DS4061` under both rules and for cross-cell `DS1003`. `.dstest` is not used
      because its `---- query` section models one snippet, not a multi-cell file; M4-06 adds the
      same cases end to end through `ds check`.
- [ ] `datascript-cli` REPL tests pass unchanged, and `datascript-compiler` still passes
      `cargo check --target wasm32-unknown-unknown`.

## Follow-ups

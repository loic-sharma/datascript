# M4-08 — `.dsnb` notebook parser/serializer & shared `StitchedProgram`

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-cli`
**Diagnostic codes:** none

## Goal
Build the single canonical, pure, `wasm32`-compatible `.dsnb` Markdown parser/serializer and
segment-stitching helper (`StitchedProgram`) in `datascript-compiler` so `datascript-server`
(M4-01/M4-04), `ds run`/`ds check`/`ds lsp` (M4-06), `datascript-cli`'s REPL, and `datascript-wasm`
(M5) all share one implementation ([WEB_UI.md §2](../WEB_UI.md#2-notebook-file-format-dsnb),
[§6](../WEB_UI.md#cross-cell-combined-program--dsnb-parser-datascript-compiler-m4-08), D-062).

## Scope
- **Line-based `.dsnb` parser & lossless serializer** (`datascript-compiler`, zero new crate
  dependencies so `wasm32` stays small):
  - Implement the CommonMark fenced-code-block scanner, canonical layout, and diff-based lossless
    round-trip rules specified in
    [WEB_UI.md §2](../WEB_UI.md#serialization-grammar--canonical-layout):
    - YAML frontmatter preserved verbatim;
    - backtick and tilde fences (`ds`/`datascript`), nested fences per CommonMark, unclosed fences
      running to EOF, extra info tokens preserved on untouched cells and ignored in v1;
    - whitespace-only text between DataScript cells treated as separator (never a Markdown cell),
      canonical `\n\n` separator for newly serialized cells, no persisted cell metadata in v1;
    - `<!-- ds:cell -->` adjacent-Markdown delimiter with lossless `<!-- ds:cell:literal -->`
      escaping/unescaping;
    - save-time diffing against an existing parsed `.dsnb` so untouched cells and separators retain
      their exact original bytes and line endings, while modified DataScript cells emit `ds` with a
      backtick fence strictly longer than any backtick run in the cell.
- **Shared `StitchedProgram`** (`datascript-compiler`):
  - Extract the pure segment-stitching and offset-mapping primitive from `datascript-cli`'s
    `ReplProgram`, supporting both whole-notebook stitching (for `check` / `ds check` / `ds run`)
    and single-cell stitching with upstream `let` statements prepended (for single-cell `run`,
    `export`, and IDE requests), mapping diagnostic spans back to `(cell_index, local_span)` and
    UTF-16 positions (D-025).
  - Have `datascript-cli`'s `ReplProgram` delegate to `datascript_compiler::StitchedProgram`.

## Acceptance criteria
- [ ] Fixture-driven round-trip tests in `datascript-compiler` covering: YAML frontmatter, CRLF and
      LF preservation on untouched cells, whitespace-only gaps between ` ```ds ` blocks,
      `datascript` and `~~~ds` fences, extra info tokens, DataScript cells containing ```` ``` ````,
      ` ```ds ` nested inside a 4-backtick non-`ds` fence, unclosed fence to EOF, adjacent Markdown
      cells with `<!-- ds:cell -->`, literal `<!-- ds:cell -->` escaping, and partial-edit diff
      preservation.
- [ ] Unit tests for `StitchedProgram` whole-notebook and single-cell stitching + span mapping
      (including multi-byte UTF-8 / UTF-16).
- [ ] `datascript-cli` REPL tests pass unchanged, and `datascript-compiler` still passes
      `cargo check --target wasm32-unknown-unknown`.

## Follow-ups

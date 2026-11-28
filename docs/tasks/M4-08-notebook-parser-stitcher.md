# M4-08 — `.dsnb` notebook parser/serializer & shared `StitchedProgram`

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-diagnostics`, `datascript-cli`
**Diagnostic codes:** DS1085–DS1089 (`DS1085` upstream cell has errors, D-062) (D-018)

## Goal
Build the single canonical, pure, `wasm32`-compatible `.dsnb` Markdown parser/serializer and
`StitchedProgram` helper in `datascript-compiler` (including `DS1085` emission) so
`datascript-server` (M4-01, M4-10, M4-04), `ds run`/`ds check`/`ds lsp` (M4-06), `datascript-cli`'s
REPL, and `datascript-wasm` (M5) all share one implementation
([WEB_UI.md §2](../WEB_UI.md#2-notebook-file-format-dsnb),
[§3](../WEB_UI.md#upstream-let-prepending--ds1085),
[§6](../WEB_UI.md#cross-cell-combined-program--dsnb-parser-datascript-compiler-m4-08), D-062).

## Scope
- **Line-based `.dsnb` parser & lossless serializer** (`datascript-compiler`, zero new crate
  dependencies so `wasm32` stays small):
  - Implement the top-level CommonMark fence scanner (0–3 leading spaces with content-indent
    stripping; fences inside blockquotes or lists stay Markdown), canonical layout, empty-Markdown
    drop, and content-diff lossless serializer specified in
    [WEB_UI.md §2](../WEB_UI.md#serialization-grammar--canonical-layout):
    - YAML frontmatter preserved verbatim;
    - backtick and tilde fences (`ds`/`datascript`), nested fences per CommonMark, unclosed fences
      running to EOF, extra info tokens preserved on untouched cells and ignored in v1;
    - whitespace-only text between DataScript cells treated as separator (empty Markdown cells
      dropped on save/reload), canonical `\n\n` separator for newly serialized cells, no persisted
      cell metadata in v1;
    - `<!-- ds:cell -->` adjacent-Markdown delimiter with stacking `:literal` escaping (add one
      `:literal` on save, remove one on load), so every input round-trips;
    - no cell can change cell boundaries, checked for every cell in its new position (untouched
      cells included): top-level `ds`/`datascript` fences inside a Markdown cell get the same
      stacking `:literal` escape on their tag, a fence left open at the end of any cell followed by
      another cell gets a closing fence appended, and a first Markdown cell starting with `---` (no
      frontmatter) gets a leading blank line;
    - `serialize_with_base(base_text, cells)`: no cell IDs. It parses the base, matches incoming
      cells to base cells by `(kind, text)` (an LCS diff, then each leftover incoming cell pairs
      with the first leftover identical base cell, so moved cells match; duplicates pair in
      document order), re-emits matched cells' exact original bytes and line endings, and
      serializes unmatched cells canonically. Separators are recomputed from each cell's new
      neighbours (original separator bytes reused only between the same two matched cells in the
      same order; `<!-- ds:cell -->` whenever two Markdown cells become adjacent). Cell text is
      normalized (LF line endings; DataScript text without fences or the newline before the
      closing fence; Markdown trimmed of leading/trailing blank lines) both in parser output and
      on incoming cells before matching, per
      [WEB_UI.md §2](../WEB_UI.md#serialization-grammar--canonical-layout).
- **Shared `StitchedProgram` & `DS1085`** (`datascript-compiler`, `datascript-diagnostics`):
  - Extract the pure segment-stitching and offset-mapping primitive from `datascript-cli`'s
    `ReplProgram`, supporting:
    1. whole-notebook stitching (for `POST /api/check`, `ds check`, `ds run`);
    2. cell stitching (runs, export, IDE requests): every upstream `let` statement in document
       order (upstream queries left out), parsing upstream cells with recovery, followed by the
       part of the target cell the request needs: all of it (`mode: "cell"`, IDE requests), its
       `let`s plus one query statement (`mode: "statement"`, export), or only its `let`s
       (`mode: "bindings"`);
    3. **`DS1085`** (register active in `codes.rs` & `DIAGNOSTICS.md`): a pure step over the
       diagnostics that checking the stitched program returns (`Session::start` already refuses
       to run when any of them is an error). When an error-severity diagnostic falls in the
       prepended upstream text, or an upstream cell cannot be split into statements, it reports one
       `DS1085` (first broken upstream statement, with the message,
       primary span, secondary label, and help in
       [WEB_UI.md §3](../WEB_UI.md#upstream-let-prepending--ds1085)) instead of those upstream
       diagnostics. No binder changes.
  - Map diagnostic spans back to `(cell_index, local_span)` and UTF-16 positions (D-025).
  - Have `datascript-cli`'s `ReplProgram` delegate to `datascript_compiler::StitchedProgram`.

## Acceptance criteria
- [ ] Fixture-driven round-trip tests in `datascript-compiler` covering: YAML frontmatter, CRLF and
      LF preservation on untouched cells, whitespace-only gaps and empty Markdown drop, top-level
      0–3 space indent stripping, `ds` fence inside a blockquote/list staying Markdown,
      `datascript` and `~~~ds` fences, extra info tokens, DataScript cells containing ```` ``` ````,
      ` ```ds ` nested inside a 4-backtick non-`ds` fence, unclosed fence to EOF, adjacent Markdown
      cells with `<!-- ds:cell -->`, literal `<!-- ds:cell -->` **and** literal
      `<!-- ds:cell:literal -->` escaping, a Markdown cell holding a top-level ```` ```ds ```` block,
      a Markdown cell ending in an unclosed ```` ```json ```` fence followed by a DataScript cell, a
      first Markdown cell starting with `---`, and content-diff
      preservation across partial edits, duplicate identical cells, and reorders.
- [ ] Serialize-with-base boundary tests where only neighbours change: deleting the DataScript
      cell between two untouched Markdown cells (they stay two cells, joined by
      `<!-- ds:cell -->`), appending a cell after an untouched last cell with an unclosed
      DataScript or Markdown fence (not swallowed), moving an untouched Markdown cell starting with
      `---` to the top of a file without frontmatter (not read as frontmatter).
- [ ] Property test: `parse(serialize(cells)) == normalize(cells)` (empty Markdown cells dropped,
      Markdown cells' leading/trailing blank lines trimmed) for generated cell lists (random
      Markdown and DataScript text including fences, `---`, and `ds:cell` lines), apart from
      appended closing fences; also for serialize-with-base after random deletes, reorders, and
      inserts.
- [ ] Repeated-save test: parse, edit cell A, serialize with base, then edit cell B and serialize
      again against the first save's output. Every untouched cell (one with CRLF and a
      `~~~datascript` fence) keeps its bytes after both saves.
- [ ] CRLF test: parse a CRLF file, convert every cell's text to LF as an editor would, edit one
      cell, and serialize with base: every other cell keeps its CRLF bytes and fence style.
- [ ] Unit tests for `StitchedProgram` whole-notebook and cell stitching (each `mode`), span
      mapping (multi-byte UTF-8/UTF-16), and `DS1085`: fires once for an upstream `let` with a type
      error, for a name declared twice upstream (`DS1003`), and for an upstream cell that cannot
      be split into statements, naming the first broken statement; does *not* fire for an
      upstream query error or an upstream warning; upstream errors are not repeated next to it.
      `mode: "statement"` leaves out a broken neighbouring query in the target cell, `mode:
      "bindings"` leaves out a half-typed query, and IDE stitching includes an upstream `let` the
      target does not reference.
- [ ] Rendered-output snapshot tests (error output is first-class behaviour, AGENTS.md): `.dsnb`
      fixtures under `crates/datascript-compiler/tests/notebooks/` whose expected output is the
      rendered diagnostic text (code, message, primary span, secondary label, help, `.dsnb` line
      and column) for `DS1085` and for cross-cell `DS1003`. `.dstest` is not used
      because its `---- query` section models one snippet, not a multi-cell file; M4-06 adds the
      cross-cell `DS1003` case end to end through `ds check` (`ds check` checks the whole
      notebook, so it never emits `DS1085`; M4-10 covers `DS1085` end to end over `WS /api/ws`).
- [ ] `datascript-cli` REPL tests pass unchanged, and `datascript-compiler` still passes
      `cargo check --target wasm32-unknown-unknown`.

## Follow-ups

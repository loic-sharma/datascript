# M4-13 — `.dsnb` notebook parser & lossless serializer

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`
**Diagnostic codes:** none

## Goal
Build the single canonical, pure, `wasm32`-compatible `.dsnb` Markdown parser and lossless
serializer in `datascript-compiler`, so `datascript-server` (M4-01), `ds run`/`ds check`/`ds lsp`
(M4-06), and `datascript-wasm` (M5-02) all share one implementation
([WEB_UI.md §2](../WEB_UI.md#2-notebook-file-format-dsnb), D-062). Split out of M4-08, which
builds `StitchedProgram`; the two can run in parallel.

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

- **Source positions**: each parsed cell also carries its byte range in the file and a mapping
  from positions in its normalized text back to file positions (line, column, and UTF-16 offset,
  D-025), accounting for fence lines, stripped indentation, and CRLF. `ds check` and the LSP
  (M4-06) use it to point diagnostics at exact `.dsnb` lines.

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
- [ ] Position-mapping tests: a diagnostic offset inside a cell maps to the right file line,
      column, and UTF-16 offset for an indented fence, a `~~~` fence, CRLF line endings, and a
      line with multi-byte characters.
- [ ] `datascript-compiler` still passes `cargo check --target wasm32-unknown-unknown`, with no new
      crate dependencies.

## Follow-ups

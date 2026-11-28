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
[§3](../WEB_UI.md#transitive-upstream-let-prepending--ds1085),
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
    - `<!-- ds:cell -->` adjacent-Markdown delimiter with stacking `:literal` escaping (add one
      `:literal` on save, remove one on load), so every input round-trips;
    - no cell can change cell boundaries, checked for every cell in its new position (untouched
      cells included): top-level `ds`/`datascript` fences inside a Markdown cell get the same
      stacking `:literal` escape on their tag, a fence left open at the end of any cell followed by
      another cell gets a closing fence appended, and a first Markdown cell starting with `---` (no
      frontmatter) gets a leading blank line;
    - opaque per-cell `originId` (byte range + content hash) returned by the parser and matched on
      serialize-with-base so untouched cells retain exact original bytes and line endings
      unambiguously even with duplicate identical cells or reorders. Separators are recomputed
      from each cell's new neighbours (original separator bytes reused only between the same two
      untouched cells in the same order; `<!-- ds:cell -->` whenever two Markdown cells become
      adjacent), and a repeated non-null `originId` is a reported error. Serializing also
      returns the fresh `originId` of every cell in the written output (the server sends these back
      from `PUT`), and an `originId` not found in the base is a reported mismatch, never silently
      re-serialized.
- **Shared `StitchedProgram` & `DS1085`** (`datascript-compiler`, `datascript-diagnostics`):
  - Extract the pure segment-stitching and offset-mapping primitive from `datascript-cli`'s
    `ReplProgram`, supporting:
    1. whole-notebook stitching (for `POST /api/check`, `ds check`, `ds run`);
    2. single-cell stitching that syntactically computes the target cell's **transitive upstream
       `let` dependencies** (parsing upstream cells with recovery, over-approximating references
       by identifier text), prepends only those `let` statements in document order, and emits
       **`DS1085`** (register active in `codes.rs` & `DIAGNOSTICS.md`) exactly per rules (a) and
       (b) in [WEB_UI.md §3](../WEB_UI.md#transitive-upstream-let-prepending--ds1085), with the
       message, primary span, secondary label, and help given there;
    3. single-statement stitching (Run statement at cursor, export): the same as 2, but keeping
       only the target query statement and the `let`s it transitively references from the target
       cell, so the cell's other query statements cannot block it;
    4. prefix stitching for IDE requests: every DataScript cell up to and including the target, so
       completion sees every upstream `let`
       ([WEB_UI.md §3](../WEB_UI.md#ide-requests-see-every-upstream-let)).
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
      first Markdown cell starting with `---`, and `originId`
      partial-edit diff preservation across duplicate identical cells and reorders.
- [ ] Serialize-with-base boundary tests where only neighbours change: deleting the DataScript
      cell between two untouched Markdown cells (they stay two cells, joined by
      `<!-- ds:cell -->`), appending a cell after an untouched last cell with an unclosed
      DataScript or Markdown fence (not swallowed), moving an untouched Markdown cell starting with
      `---` to the top of a file without frontmatter (not read as frontmatter), and a repeated
      `originId` reported as an error.
- [ ] Property test: `parse(serialize(cells)) == normalize(cells)` (empty Markdown cells dropped,
      Markdown cells' leading/trailing blank lines trimmed) for generated cell lists (random
      Markdown and DataScript text including fences, `---`, and `ds:cell` lines), apart from
      appended closing fences; also for serialize-with-base after random deletes, reorders, and
      inserts.
- [ ] Repeated-save test: parse, edit cell A, serialize, re-parse to get fresh `originId`s, edit
      cell B, serialize again. Cell A (CRLF, `~~~datascript` fence) and every untouched cell keep
      their bytes after the second save, and a stale `originId` is reported as a mismatch rather
      than silently canonicalized.
- [ ] Unit tests for `StitchedProgram` whole-notebook and transitive single-cell stitching, span
      mapping (multi-byte UTF-8/UTF-16), and `DS1085`: fires under rule (a) (referenced `let` with
      a type error; referenced name declared twice upstream, `DS1003`) and rule (b) (upstream
      `let` with a missing name, and the target has a name the binder cannot resolve; `DS1085`
      replaces that unknown-name error);
      does *not* fire on an unreferenced broken `let`, a broken query next to a valid `let` in the
      same cell, an upstream warning, or an undeclared name with no upstream syntax error (that is
      the ordinary unknown-name diagnostic), or a target that uses only existing columns and
      functions while an upstream `let` has a missing name; a column whose name matches an upstream `let`
      prepends that `let` harmlessly. Single-statement stitching leaves out a broken neighbouring
      query statement in the target cell, and prefix stitching includes an unreferenced upstream
      `let`.
- [ ] Rendered-output snapshot tests (error output is first-class behaviour, AGENTS.md): `.dsnb`
      fixtures under `crates/datascript-compiler/tests/notebooks/` whose expected output is the
      rendered diagnostic text (code, message, primary span, secondary label, help, `.dsnb` line
      and column) for `DS1085` under both rules and for cross-cell `DS1003`. `.dstest` is not used
      because its `---- query` section models one snippet, not a multi-cell file; M4-06 adds the
      cross-cell `DS1003` case end to end through `ds check` (`ds check` checks the whole
      notebook, so it never emits `DS1085`; M4-10 covers `DS1085` end to end over `WS /api/ws`).
- [ ] `datascript-cli` REPL tests pass unchanged, and `datascript-compiler` still passes
      `cargo check --target wasm32-unknown-unknown`.

## Follow-ups

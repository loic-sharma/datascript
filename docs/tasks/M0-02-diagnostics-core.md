# M0-02 — Diagnostics core

**Status:** done — diagnostic model, collection, plain/terminal/JSON renderers, did-you-mean and code registry landed in `datascript-diagnostics`
**Depends on:** M0-01
**Crates:** `datascript-diagnostics`
**Diagnostic codes:** DS0004, DS9001 (D-018)

## Goal
The single diagnostic model and renderers used by every surface. See [DIAGNOSTICS.md](../DIAGNOSTICS.md)
and the Diagnostics section of [ARCHITECTURE.md](../ARCHITECTURE.md).

## Scope
- Types: `Span` (byte range + `FileId`), `SourceFile`/`SourceMap` (line index, UTF-8 ↔ UTF-16
  column conversion for LSP), `Code`, `Severity`, `Label`, `Help`, `Suggestion`, `Applicability`,
  `Diagnostic` with a builder API (`Diagnostic::error(code, msg).primary(span, "...").help("...")`).
- `DiagnosticSink` / `Diagnostics` collection, sorted by source position, de-duplicated.
- Renderers:
  - **terminal**: color + Unicode box drawing (evaluate `annotate-snippets` or `ariadne`; pick
    one and record it in DECISIONS.md, or hand-roll it if neither renders the formats in
    DIAGNOSTICS.md well),
  - **plain**: ASCII, no color, stable (used by golden tests, format shown in TESTING.md),
  - **JSON**: `serde` serialization with UTF-16 offsets and 1-based line/column counted in UTF-16
    code units (D-025). No byte offsets.
- Edit-distance helper `did_you_mean(candidate, iter) -> Suggestions` (Damerau-Levenshtein
  plus case-insensitive match), shared by all later stages.
- Code registry: a `codes.rs` with a constant per code and its summary, matching the table in
  DIAGNOSTICS.md.

Design-review additions:
- `primary: Option<Label>` (None only for location-less diagnostics such as DS0004), and
  `Suggestion { edits: Vec<TextEdit>, applicability }` with non-overlapping edits (ARCHITECTURE.md).
- The JSON form carries `"version": 1`.
- The collection caps errors at 50 per file and adds one note with the omitted count.
- Register DS0004 and DS9001 (this task's block).

## Out of scope
LSP conversion (M3-01), long-form explanations (M2-08).

## Acceptance criteria
- [x] Unit tests for multi-line spans, spans at EOF, empty spans, tabs, and non-ASCII text (columns
      counted in characters for display, UTF-16 for LSP).
- [x] Snapshot tests (e.g. `insta`) of the terminal and plain renderers for the three examples in
      DIAGNOSTICS.md.
- [x] JSON schema documented in a doc comment. Round-trip test. A test with non-BMP text (emoji)
      checks UTF-16 positions.
- [x] Crate builds for wasm32.
- [x] Rendering of a diagnostic with no primary span, and of a multi-edit suggestion, is tested.

## Follow-ups
- Renderer choice: hand-written (D-027). Tests compare terminal output to the DIAGNOSTICS.md
  examples and plain output to the TESTING.md example verbatim.
- `codes.rs` mirrors the whole DIAGNOSTICS.md registry (not only this task's block); a unit test
  fails if the two disagree, so tasks adding a code must update both.
- Carets count characters, so lines with wide characters (CJK, emoji) are not visually aligned in
  a terminal. Use `unicode-width` if that turns out to matter.
- Lines are split on `\n` only; a lone `\r` is not a line break (LSP treats it as one). Revisit in
  M3-01 if it matters.

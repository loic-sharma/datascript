# M4-08 — Notebook `StitchedProgram` & `DS1085`

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-diagnostics`, `datascript-cli`
**Diagnostic codes:** DS1085–DS1089 (`DS1085` upstream cell has errors, D-062) (D-018)

## Goal
Build the pure, `wasm32`-compatible `StitchedProgram` helper in `datascript-compiler` (including
`DS1085` emission) so `datascript-server` (M4-01, M4-10, M4-04), `ds run`/`ds check`/`ds lsp`
(M4-06), `datascript-cli`'s REPL, and `datascript-wasm` (M5) all share one implementation. It works
on a list of cells (`{ kind, text }`), so it does not need the `.dsnb` parser (M4-13), and the two
tasks can run in parallel
([WEB_UI.md §3](../WEB_UI.md#upstream-let-prepending--ds1085),
[§6](../WEB_UI.md#cross-cell-combined-program--dsnb-parser-datascript-compiler-m4-08--m4-13), D-062).

## Scope
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
       to run when any of them is an error). It drops every diagnostic whose primary span falls in
       the prepended upstream text (errors and warnings; live check shows them on their own
       cells). If any dropped one was an error, or an upstream cell cannot be split into
       statements, it reports one `DS1085` (first broken upstream statement, with the message,
       primary span, secondary label, and help in
       [WEB_UI.md §3](../WEB_UI.md#upstream-let-prepending--ds1085)). No binder changes.
  - Map diagnostic spans back to `(cell_index, local_span)` and UTF-16 positions (D-025).
  - Have `datascript-cli`'s `ReplProgram` delegate to `datascript_compiler::StitchedProgram`.

## Acceptance criteria
- [ ] Unit tests for `StitchedProgram` whole-notebook and cell stitching (each `mode`), span
      mapping (multi-byte UTF-8/UTF-16), and `DS1085`: fires once for an upstream `let` with a type
      error, for a name declared twice upstream (`DS1003`), and for an upstream cell that cannot
      be split into statements, naming the first broken statement; does *not* fire for an
      upstream query error or an upstream warning; upstream errors and warnings are not repeated in run output.
      `mode: "statement"` leaves out a broken neighbouring query in the target cell, `mode:
      "bindings"` leaves out a half-typed query, and IDE stitching includes an upstream `let` the
      target does not reference.
- [ ] Rendered-output snapshot tests (error output is first-class behaviour, AGENTS.md): cell-list
      fixtures under `crates/datascript-compiler/tests/notebooks/` whose expected output is the
      rendered diagnostic text (code, message, primary span, secondary label, help, cell index and
      cell-local line and column) for `DS1085` and for cross-cell `DS1003`. `.dstest` is not used
      because its `---- query` section models one snippet, not a multi-cell file; M4-06 adds the
      cross-cell `DS1003` case end to end through `ds check` with real `.dsnb` line numbers (`ds check` checks the whole
      notebook, so it never emits `DS1085`; M4-10 covers `DS1085` end to end over `WS /api/ws`).
- [ ] `datascript-cli` REPL tests pass unchanged, and `datascript-compiler` still passes
      `cargo check --target wasm32-unknown-unknown`.

## Follow-ups

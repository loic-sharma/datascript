# M4-06 — `.dsnb` notebooks in CLI (`ds run`/`ds check`) and LSP / VS Code

**Status:** todo
**Depends on:** M4-08, M4-13, M3-01, M3-02
**Crates:** `datascript-cli`, `datascript-lsp`, `editors/vscode`
**Diagnostic codes:** none

## Goal
Make `.dsnb` notebooks first-class citizens outside the browser ([LANGUAGE.md §2.1](../LANGUAGE.md#21-notebooks-dsnb),
D-062) by reusing `datascript-compiler`'s canonical `.dsnb` parser (M4-13) and `StitchedProgram` (M4-08),
independent of `datascript-server`.

## Scope
- **CLI (`datascript-cli`)**:
  - `ds check notebook.dsnb`: parses (M4-13) and stitches (M4-08) all DataScript cells top-to-bottom and
    renders diagnostics pointing at exact `.dsnb` lines/columns.
  - `ds run notebook.dsnb`: executes all DataScript cells' queries top-to-bottom under one shared
    `now()` clock (D-015), streaming tables / CSV / JSON / NDJSON / Arrow like `ds run script.ds`.
  - `ds explain notebook.dsnb`: prints the logical plan for each query cell.
- **LSP (`datascript-lsp`) & VS Code (`editors/vscode`)**:
  - Register `.dsnb` in the VS Code extension and handle `.dsnb` documents in `datascript-lsp` using
    M4-13's `.dsnb` parser + M4-08's `StitchedProgram` for live diagnostics, completion, hover, and
    signature help inside ` ```ds ` fences with spans mapped back to `.dsnb` UTF-16 positions.

## Acceptance criteria
- [ ] CLI integration tests (`crates/datascript-cli/tests/`, used rather than `.dstest` because
      `.dstest` models single-snippet `---- query` programs while CLI file tests verify multi-file
      `.dsnb` fixtures end-to-end) covering `ds check`, `ds run`, and `ds explain` on `.dsnb` files
      (cross-cell `let`s, accurate `.dsnb` diagnostic line numbers, `DS1003` across cells, shared
      `now()`).
- [ ] LSP integration tests (`crates/datascript-lsp/tests/`) verifying diagnostics, completion, and
      hover inside ` ```ds ` fences of an open `.dsnb` document.
- [ ] VS Code extension activates on `.dsnb` files without interfering with regular `.md` files.

## Follow-ups

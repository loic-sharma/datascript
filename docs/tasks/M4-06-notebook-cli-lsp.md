# M4-06 — `.dsnb` notebooks in CLI (`ds run`/`ds check`) and LSP / VS Code

**Status:** todo
**Depends on:** M4-01, M3-01, M3-02
**Crates:** `datascript-cli`, `datascript-lsp`, `editors/vscode`

## Goal
Make `.dsnb` notebooks first-class citizens outside the browser (D-062) by reusing the canonical
`.dsnb` parser and `StitchedProgram` added to `datascript-compiler` in M4-01.

## Scope
- **CLI (`datascript-cli`)**:
  - `ds check notebook.dsnb`: parses the notebook's ` ```ds ` cells, stitches them top-to-bottom
    with `StitchedProgram`, checks them, and renders diagnostics pointing at the exact line/column
    inside the `.dsnb` file.
  - `ds run notebook.dsnb`: runs every DataScript cell's queries top-to-bottom (sharing upstream
    `let`s), streaming tables / CSV / JSON / NDJSON / Arrow just like `ds run script.ds`.
  - `ds explain notebook.dsnb`: prints the logical plan for each query cell.
- **LSP (`datascript-lsp`) & VS Code (`editors/vscode`)**:
  - Register `.dsnb` in the VS Code extension and teach `datascript-lsp` to recognize `.dsnb`
    documents using `datascript-compiler`'s `.dsnb` parser + `StitchedProgram`.
  - Provide live diagnostics, completion, hover, and signature help inside ` ```ds ` fenced blocks
    with upstream cells' `let`s in scope and spans mapped back to the `.dsnb` file's UTF-16
    positions.

## Acceptance criteria
- [ ] CLI integration tests running `ds check`, `ds run`, and `ds explain` on `.dsnb` fixtures
      (including cross-cell `let`s, multi-cell errors with accurate `.dsnb` line numbers, and
      `DS1003` across cells).
- [ ] LSP integration tests verifying diagnostics, completion, and hover inside ` ```ds ` fences of
      an open `.dsnb` document.
- [ ] VS Code extension activates on `.dsnb` files without interfering with regular `.md` files.

## Follow-ups

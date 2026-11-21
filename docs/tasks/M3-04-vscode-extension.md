# M3-04 — VS Code extension

**Status:** todo
**Depends on:** M3-01
**Crates:** `editors/vscode`

## Goal
A TypeScript extension: `.ds` language registration, a TextMate grammar (for highlighting before
the server starts), a language client that launches `ds lsp` (configurable path, or a bundled
binary later), and snippets.

## Acceptance criteria
- [ ] `vsce package` builds a .vsix. An extension test opens a file and receives diagnostics.
- [ ] Grammar covers every token kind of the lexer (tested against fixture files). Hyphenated
      operator names are matched as adjacent `ident-ident` after `|`, the same rule as the parser (D-026).
- [ ] Honors Workspace Trust: in an untrusted workspace, the server runs syntax-only (no schema reads, no Run). Tested.

## Follow-ups

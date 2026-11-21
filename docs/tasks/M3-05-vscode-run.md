# M3-05 — VS Code: Run CodeLens and results panel

**Status:** todo
**Depends on:** M3-04, M3-02
**Crates:** `editors/vscode`, `datascript-lsp`

## Goal
A grey "▶ Run" CodeLens above each query statement (like Flutter's run button). Running it calls a
custom `datascript/run` LSP request and shows the result in a webview panel: a grid, and charts once
M4-03 exists, reusing `web/` components.

## Acceptance criteria
- [ ] Run a single statement (with the `let`s it depends on) or the whole file.
- [ ] Runtime errors appear as diagnostics in the editor and in the panel.
- [ ] Cancellation works.

## Follow-ups

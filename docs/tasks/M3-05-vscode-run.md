# M3-05 — VS Code: Run CodeLens and results panel

**Status:** todo
**Depends on:** M3-04, M3-02, M4-02 (results grid component from `web/`)
**Crates:** `editors/vscode`, `datascript-lsp`

## Goal
A grey "▶ Run" CodeLens above each query statement (like Flutter's run button). Running it calls a
custom `datascript/run` LSP request and shows the result in a webview panel: a grid, and charts once
M4-03 exists, reusing `web/` components.

## Acceptance criteria
- [ ] Run a single statement (with the `let`s it depends on) or the whole file.
- [ ] Runtime errors appear as diagnostics in the editor and in the panel.
- [ ] Cancellation works.

## Carried over
- [ ] (from M3-04) No Run in an untrusted workspace (ARCHITECTURE.md § Frontends, D-053): show no Run CodeLens when `vscode.workspace.isTrusted` is false, and have the server reject `datascript/run` when it was started with `initializationOptions.syntaxOnly`. Add both to `editors/vscode/test/e2e/suite/untrusted.test.ts` and `crates/datascript-lsp/tests/lsp.rs`.

## Follow-ups

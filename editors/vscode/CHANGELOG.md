# Changelog

## 0.0.1

- `.ds` language registration, TextMate grammar, snippets and bracket configuration.
- Diagnostics from `ds lsp`; syntax errors only in untrusted workspaces.
- Run: a "▶ Run" CodeLens over each query statement, **DataScript: Run Statement at Cursor** and
  **DataScript: Run File**, printing what `ds run` prints in a "DataScript" terminal. Unsaved
  edits run; runtime errors show as diagnostics; Ctrl+C cancels. No Run in untrusted workspaces.

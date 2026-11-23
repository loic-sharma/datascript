# M3-06 — VS Code: results panel

**Status:** todo
**Depends on:** M3-05, M4-02, M4-03, M2-13
**Crates:** `datascript-lsp`, `datascript-engine` (Arrow output per result), `editors/vscode`, `web/`

## Goal
Show Run results in a webview panel: a virtualized grid and, for `render`, the chart, reusing the
`web/` components from M4-02 and M4-03. The terminal output from M3-05 stays available (D-054).

## Scope
- `datascript/run` gains a way to ask for data instead of text, e.g. `format: "arrow"`, returning
  each result as Arrow IPC with its `RenderSpec`. Follow the row limit decided for UI runtimes
  (Q-007).
- A setting chooses where results go: the terminal (M3-05) or the panel.
- The panel shows several results the way the web UI does (Q-004), and runtime errors as the web
  UI does.

## Acceptance criteria
- [ ] Run shows the grid in the panel, and a chart for `render`.
- [ ] Large results are truncated with a message, not buffered whole.
- [ ] Cancellation and runtime errors work as in M3-05.
- [ ] Still no Run in an untrusted workspace.

## Carried over
- [ ] (from M3-05) In the editor, `datascript/run`'s text is word for word what `ds run` prints, so a cut table says `show them with --max-rows N` and `DS5001` says charts are not drawn "on the command line". When the panel adds a setting for where results go, decide whether the terminal output should name the `datascript.run.maxRows` setting and the panel instead (a `Frontend` variant in `datascript_engine::printer`, and a `DS5001` wording per frontend), and record it in DECISIONS.md.

## Follow-ups

# M4-15 — Web UI: cell editing & live checking (M4 v1)

**Status:** todo
**Depends on:** M4-02
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Turn M4-02's plain cell list into the real notebook editor: DataScript and Markdown cells,
reordering, and whole-notebook live checking with diagnostic cards
([WEB_UI.md §4.2](../WEB_UI.md#42-center-pane--notebook-canvas), D-064). Split out of M4-02.

## Scope
- **Markdown cells**: GFM rendered via DOMPurify (no external `http(s)://` or relative workspace
  images in v1; `data:`/`blob:` only); click/double-click edits in CodeMirror (`Cmd/Ctrl+Enter`,
  `Shift+Enter`, `Esc` per [WEB_UI.md §4.2](../WEB_UI.md#42-center-pane--notebook-canvas)).
- **DataScript cells**: CodeMirror 6 with DataScript highlighting (D-026), whole-notebook live
  checking (`POST /api/check`, debounced), inline squiggles, and rustc/Claude-Code-style diagnostic
  cards with one-click **Apply fix** and cross-cell jump links for secondary labels (e.g. `DS1003`;
  `DS1085` comes from runs, M4-09).
- **Reordering & insertion**: drag-and-drop cell reordering and hover `+ DataScript` /
  `+ Markdown` dividers.

## Acceptance criteria
- [ ] Vitest unit tests for Markdown sanitization (remote and relative images blocked, scripts
      stripped) and for mapping grouped diagnostics onto cells.
- [ ] Playwright test against a real `ds ui`: cross-cell live diagnostics (editing a `let` in cell
      1 updates cell 3), an Apply fix, a `DS1003` jump link, drag reorder followed by a save that
      keeps untouched cells byte-identical, and Markdown edit/render with `Esc`.
- [ ] Keyboard accessibility of cells and dividers.

## Follow-ups

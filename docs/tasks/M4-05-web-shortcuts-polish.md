# M4-05 — Web UI: keyboard shortcuts, Command Palette, Quick Open & M4 polish

**Status:** todo
**Depends on:** M4-02, M4-03, M4-04
**Crates:** `web/`

## Goal
Complete the keyboard-first notebook ergonomics specified in [WEB_UI.md §5](../WEB_UI.md#5-keyboard-shortcuts--command-mode)
(hybrid modeless chords + `Esc` Cell Command Mode, Command Palette, Quick Open, shortcut hints) and
deliver M4 acceptance polish across the full end-to-end workflow.

## Scope
- **Cell Command Mode (`Esc` / `Enter`)**:
  - `Esc` from a cell editor (rendering Markdown if editing a Markdown cell) focuses the cell frame
    with a distinct subtle focus ring; `Enter` re-enters edit mode. (`Cmd/Ctrl+.` cancels a running
    query so `Esc` never conflicts with query cancellation.)
  - Full single-key command bindings: `j`/`k` (and `Down`/`Up`) cell navigation, `Shift+Up`/`Shift+Down`
    (and `Shift+K`/`Shift+J`) cell reordering (leaving `Alt+Up`/`Alt+Down` free for CodeMirror line
    movement), `a`/`b` insert above/below, `m`/`y` convert Markdown/DataScript, `dd` delete cell,
    `z` undo cell structure changes, `x`/`c`/`v` cut/copy/paste cell, `o` collapse/expand output,
    `?` shortcuts cheat sheet.
- **Browser-safe global chords**:
  - `Alt+W` (close tab) and `Alt+N` (new Scratchpad) in web browsers, avoiding browser-reserved
    `Ctrl+W` / `Ctrl+Shift+N` (structured so a future Tauri desktop shell can bind native
    `Cmd/Ctrl+W` and `Cmd/Ctrl+Shift+N`).
- **Command Palette (`Cmd/Ctrl+Shift+P`, `Cmd/Ctrl+K`)**:
  - Fuzzy-searchable palette exposing every workspace, notebook, cell, view, theme, and export
    action alongside its live platform-specific keybinding (`⌘` vs `Ctrl`).
- **Quick Open (`Cmd/Ctrl+P`)**:
  - Fuzzy-searchable file switcher over the workspace tree (prioritizing open tabs, `.dsnb`
    notebooks, `.ds` scripts, and data files).
- **Subtle inline shortcut hints & cheat sheet**:
  - Quiet Claude-Code-style keycap hints (`⌘↵`, `⇧↵`, `⎋`) in the status bar / focused cell chrome,
    plus a `?` modal cheat sheet.
- **M4 acceptance polish**:
  - Focus management, ARIA roles/keyboard accessibility across all three panes, responsive narrow
    viewport handling (auto-collapsing sidebars cleanly), empty states, and smooth loading/error
    transitions.

## Acceptance criteria
- [ ] Playwright E2E acceptance suite covering the full M4 journey using keyboard-only workflows
      (Scratchpad → Quick Open / tree schema inspect → multi-cell notebook with `let`s + Markdown →
      Command Mode reorder/delete/undo → `| render timechart` + Table toggle → Result Profile
      inspection → auto-save + external-change conflict handling → streaming CSV/Parquet export).
- [ ] Keyboard shortcut unit tests verifying macOS (`Cmd`) and Windows/Linux (`Ctrl`) bindings and
      zero conflicts with CodeMirror editing keys.
- [ ] M4 "Done when" criteria in [ROADMAP.md](../ROADMAP.md) verified end-to-end.

## Follow-ups

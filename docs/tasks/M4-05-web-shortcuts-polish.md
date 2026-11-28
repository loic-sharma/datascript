# M4-05 — Web UI: Command Mode, Command Palette & Quick Open

**Status:** todo
**Depends on:** M4-09, M4-03, M4-04, M4-16, M4-17
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Keyboard power features on top of M4 v1: Cell Command Mode, Command Palette, Quick Open, and the
full shortcut set ([WEB_UI.md §5](../WEB_UI.md#5-keyboard-shortcuts--command-mode)). File
management is M4-16; tabs and Outline/Scope are M4-17.

## Scope
- **`Alt+W`/`Alt+N` chords** (matched on `KeyboardEvent.code`, outside text inputs): `Alt+W`
  closes M4-17's active tab, `Alt+N` opens a new Scratchpad.
- **Command Palette coverage**: every action is reachable from the palette (WEB_UI §5), including
  M4-16's new folder, rename, and delete and M4-17's close and pin tab.
- **Cell Command Mode, Command Palette (`Cmd/Ctrl+K` primary) & Quick Open (`Cmd/Ctrl+P`)**: full
  bindings, shortcut cheat sheet (`?`), and subtle keycap hints per
  [WEB_UI.md §5](../WEB_UI.md#5-keyboard-shortcuts--command-mode) (verifying `Cmd/Ctrl+Shift+B`
  preventability in Chrome with toolbar/Command Palette fallback).

## Acceptance criteria
- [ ] Keyboard shortcut unit tests verifying `KeyboardEvent.code` matching, macOS/Win/Linux chords,
      `Alt+W`/`Alt+N` suppression inside text inputs, `AltGraph` ignored on `Ctrl+Alt+Enter`, and
      no CodeMirror editing conflicts.
- [ ] Playwright E2E suite covering Quick Open, Command Palette, and keyboard-only Command Mode editing.

## Follow-ups

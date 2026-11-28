# M4-05 — Web UI: power-user shortcuts, tabs, file management & inspector polish

**Status:** todo
**Depends on:** M4-02, M4-09, M4-03, M4-04
**Crates:** `web/`, `datascript-server`
**Diagnostic codes:** none

## Goal
Layer the power-user and multi-file workspace features from [WEB_UI.md](../WEB_UI.md) on top of M4
v1: multi-file tabs & pinning, safe file/folder creation, rename and delete, Outline & Scope
inspector tabs, Cell Command Mode, Command Palette, and Quick Open.

## Scope
- **Multi-file tabs & pinning** (`web/`): slim tab bar preserving open tabs' outputs, scroll
  position, and undo history, with pinning and browser-safe `Alt+W`/`Alt+N` chords (matched on
  `KeyboardEvent.code`, outside text inputs).
- **Safe file/folder management** (`datascript-server` + `web/`): add `POST /api/dir`,
  `PATCH /api/file`, and `DELETE /api/file` following the symlink, atomic no-clobber rename (via
  `rustix` on Unix and `MoveFileExW` on Windows), and per-file+`rmdir` folder deletion rules in
  [WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables).
- **Outline & Scope inspector tabs** (`web/`, [WEB_UI.md §4.3](../WEB_UI.md#43-right-pane--contextual-inspector-collapsed-by-default)).
- **Cell Command Mode, Command Palette (`Cmd/Ctrl+K` primary) & Quick Open (`Cmd/Ctrl+P`)**: full
  bindings, shortcut cheat sheet (`?`), and subtle keycap hints per
  [WEB_UI.md §5](../WEB_UI.md#5-keyboard-shortcuts--command-mode) (verifying `Cmd/Ctrl+Shift+B`
  preventability in Chrome with toolbar/Command Palette fallback).

## Acceptance criteria
- [ ] Server API tests for `POST /api/dir`, `PATCH`, and `DELETE`: data-file and in-root symlink
      rejection (`notes.ds -> data/orders.csv`), folder-with-data-files/symlinks rejection, atomic
      no-clobber rename refusal on existing files, and per-file+`rmdir` folder deletion.
- [ ] Keyboard shortcut unit tests verifying `KeyboardEvent.code` matching, macOS/Win/Linux chords,
      `Alt+W`/`Alt+N` suppression inside text inputs, `AltGraph` ignored on `Ctrl+Alt+Enter`, and
      no CodeMirror editing conflicts.
- [ ] Playwright E2E suite covering multi-tab workflows, file rename/delete, Outline & Scope tabs,
      Quick Open, Command Palette, and keyboard-only Command Mode editing.

## Follow-ups

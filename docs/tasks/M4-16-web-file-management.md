# M4-16 — Web UI: safe file & folder management

**Status:** todo
**Depends on:** M4-02
**Crates:** `datascript-server`, `web/`
**Diagnostic codes:** none

## Goal
Create folders and rename or delete `.dsnb`/`.ds` files and safe folders from the file tree, never
touching data files ([WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables)).
Split out of M4-05.

## Scope
- **Safe file/folder management** (`datascript-server` + `web/`): add `POST /api/dir`,
  `PATCH /api/file`, and `DELETE /api/file` following the symlink, atomic no-clobber rename (via
  `rustix` on Unix and `MoveFileExW` on Windows for files and folders, `hard_link` fallback for
  files only; a folder rename with no no-replace support is refused), OS
  clutter allow-list, and verify-then-delete folder rules in
  [WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables).

## Acceptance criteria
- [ ] Server API tests for `POST /api/dir`, `PATCH`, and `DELETE`: data-file and in-root symlink
      rejection (`notes.ds -> data/orders.csv`), folder-with-data-files/symlinks rejection, atomic
      no-clobber rename refusal on existing files (files and folders), verify-then-delete folder
      deletion with nested subfolders, a folder containing only notebooks and `.DS_Store` deleting
      cleanly, a folder with one hidden data file refused before anything is deleted (the error
      names the file), and a concurrent new file reported as a partial delete.
- [ ] Playwright test of rename and delete from the file tree.

## Follow-ups

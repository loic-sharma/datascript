# M4-05 — Web UI: power-user shortcuts, tabs, file management & inspector polish

**Status:** todo
**Depends on:** M4-02, M4-03, M4-04
**Crates:** `web/`, `datascript-server`

## Goal
Layer the power-user and multi-file workspace features from [WEB_UI.md](../WEB_UI.md) on top of M4
v1: multi-file tabs & pinning, safe file/folder rename and delete, Outline & Scope inspector tabs,
Cell Command Mode, Command Palette, Quick Open, and full keyboard-first E2E polish.

## Scope
- **Multi-file tabs & pinning** (`web/`):
  - Slim tab bar above the breadcrumb keeping open `.dsnb` notebooks, `.ds` scripts, and
    Scratchpads alive in memory (preserving rendered outputs, scroll position, and undo history),
    with tab pinning and browser-safe `Alt+W` (close tab) / `Alt+N` (new Scratchpad) chords
    matched on `KeyboardEvent.code` and active only outside text inputs so macOS `Option` dead keys
    continue working inside CodeMirror (structured so a future Tauri desktop shell can bind native
    `Cmd/Ctrl+W` and `Cmd/Ctrl+Shift+N`).
- **Safe file/folder rename & delete** (`datascript-server` + `web/`):
  - Add `POST /api/dir`, `PATCH /api/file` (rename), and `DELETE /api/file` (delete) in
    `datascript-server`, wired to sidebar context menus and inline rename in `web/`.
  - Strictly enforce read-only data files without check-then-act races (D-016, D-062):
    - Refuse `POST /api/dir`, `PATCH`, or `DELETE` whenever the source, target, or any parent
      component under root is a **symlink** (`symlink_metadata`/`O_NOFOLLOW`).
    - Refuse deleting or renaming any file whose extension is not `.dsnb` or `.ds`, or renaming
      onto a non-`.dsnb`/`.ds` extension.
    - Rename atomically with no-clobber OS primitives (`renameat2(RENAME_NOREPLACE)` on Linux,
      `renamex_np(RENAME_EXCL)` on macOS, `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` on
      Windows) so renaming can never race to overwrite an existing file.
    - Delete folders without `remove_dir_all`: recursively verify no symlinks or non-`.dsnb`/
      non-`.ds` files, unlink each verified `.dsnb`/`.ds` regular file one by one, and finish with
      `rmdir` (which fails atomically if a new file appeared concurrently). Refuse renaming any
      folder that recursively contains non-`.dsnb`/non-`.ds` files or symlinks.
- **Outline & Scope inspector tabs** (`web/`):
  - **Outline tab**: interactive jump list of Markdown headings (`#`, `##`, `###`) and DataScript
    cells.
  - **Scope (`let`s) tab**: all scalar and tabular `let` bindings (types/columns + jump link to
    defining cell) visible to the currently focused cell.
- **Cell Command Mode (`Esc` / `Enter`)**:
  - `Esc` from a cell editor (rendering Markdown if editing a Markdown cell) focuses the cell frame
    with a distinct subtle focus ring; `Enter` re-enters edit mode. (`Cmd/Ctrl+.` cancels a running
    query so `Esc` never conflicts with query cancellation.)
  - Full single-key command bindings: `j`/`k` (and `Down`/`Up`) cell navigation,
    `Shift+Up`/`Shift+Down` (and `Shift+K`/`Shift+J`) cell reordering (leaving `Alt+Up`/`Alt+Down`
    free for CodeMirror line movement), `a`/`b` insert above/below, `m`/`y` convert
    Markdown/DataScript, `dd` delete cell, `z` undo cell structure changes, `x`/`c`/`v` cut/copy/
    paste cell, `o` collapse/expand output, `?` shortcuts cheat sheet.
- **Command Palette (`Cmd/Ctrl+K` primary; `Cmd/Ctrl+Shift+P` alias) & Quick Open (`Cmd/Ctrl+P`)**:
  - Fuzzy-searchable Command Palette exposing every action with live platform keybindings (`⌘` vs
    `Ctrl`; `Cmd/Ctrl+K` is primary since Firefox reserves `Cmd/Ctrl+Shift+P`), plus Quick Open
    over workspace files and open tabs. Right sidebar toggle uses `Cmd/Ctrl+Shift+B` (avoiding
    Windows `AltGr`), and `Ctrl+Alt+Enter` ignores `AltGraph` events.
  - Quiet Claude-Code-style keycap hints (`⌘↵`, `⇧↵`, `⎋`) in the status bar / focused cell chrome,
    plus a `?` modal cheat sheet.

## Acceptance criteria
- [ ] Server API tests for `POST /api/dir`, `PATCH`, and `DELETE`: refusal to delete/rename data
      files or in-root symlinks (`notes.ds -> data/orders.csv`), refusal to delete/rename folders
      recursively containing data files or symlinks, atomic no-clobber refusal to rename onto an
      already-existing file or data-file extension, and per-file+`rmdir` folder deletion.
- [ ] Keyboard shortcut unit tests verifying `KeyboardEvent.code` matching, macOS (`Cmd`) and
      Windows/Linux (`Ctrl`) bindings, `Alt+W`/`Alt+N` suppressed inside text inputs (macOS
      `Option` dead-key safety), `AltGraph` ignored for `Ctrl+Alt+Enter`, and zero conflicts with
      CodeMirror editing keys or browser-reserved chords.
- [ ] Playwright E2E acceptance suite covering multi-tab workflows, file rename/delete, Outline &
      Scope tabs, Quick Open, Command Palette, and keyboard-only Command Mode notebook editing.

## Follow-ups

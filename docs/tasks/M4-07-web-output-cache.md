# M4-07 — Web UI: local output cache across reloads *(optional / future enhancement)*

**Status:** todo
**Depends on:** M4-09, M4-03
**Crates:** `datascript-server`, `web/`
**Diagnostic codes:** none

## Goal
Optionally persist each cell's last rendered result locally outside the repository so reopening a
`.dsnb` notebook shows previous outputs immediately (marked **Stale** until re-run) while keeping
`.dsnb` files strictly source-only on disk (D-062). *Optional for M4 "done".*

## Scope
- **Storage by runtime** (surviving `ds ui`'s random port across launches):
  - In `ds ui` (`ServerRuntime`): store cached Arrow IPC batches, `RenderSpec`, and
    timing/`sourceCount`/`rowLimit` metadata on the server under the user's OS cache directory
    (`dirs::cache_dir()` / `datascript/ui/`, never inside the working directory or next to `.dsnb`
    files), exposed via a lightweight cache endpoint on `Runtime`.
  - In the static WASM playground (`WasmRuntime`, stable origin): store in browser `IndexedDB`.
- **Keying**: key every cached entry by **canonical workspace root + notebook path + cell source &
  upstream-`let` fingerprint** (storing cell index only to disambiguate identical cells) so
  inserting, deleting, or reordering cells preserves cached outputs.
- Hydrate matching cells on open with a **Stale** badge and a bounded LRU cache + graceful quota/IO
  fallback, plus a "Clear local output cache" Command Palette action.

## Acceptance criteria
- [ ] Unit tests for cache serialization, LRU eviction, quota/IO-error handling, and fingerprint
      matching across cell insertion, deletion, and reordering (including duplicate identical
      cells).
- [ ] Playwright test verifying that restarting `ds ui` (on a new random port) and reopening a
      saved `.dsnb` restores cached outputs with a **Stale** badge without modifying the `.dsnb`
      file on disk.

## Follow-ups

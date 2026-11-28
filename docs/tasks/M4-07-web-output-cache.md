# M4-07 — Web UI: local output cache across reloads *(optional / future enhancement)*

**Status:** todo
**Depends on:** M4-09, M4-03
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Optionally persist each cell's last rendered result locally outside the repository so reopening a
`.dsnb` notebook shows previous outputs immediately (marked **Stale** until re-run) while keeping
`.dsnb` files strictly source-only on disk (D-062). *Optional for M4 "done".*

## Scope
- Store each cell's last Arrow IPC batches, `RenderSpec`, timing/`sourceCount`/`rowLimit` metadata,
  and cell source + upstream-`let` fingerprint in browser `IndexedDB`, **keyed by workspace root +
  notebook path + cell source fingerprint** (using cell index only to disambiguate identical cells)
  so inserting, deleting, or reordering cells preserves cached outputs.
- Never write cache files inside the working directory or next to `.dsnb` files.
- Hydrate matching cells on open with a **Stale** badge and a bounded LRU cache + graceful quota
  fallback, plus a "Clear local output cache" Command Palette action.

## Acceptance criteria
- [ ] Unit tests for cache serialization, LRU eviction, quota-error handling, and fingerprint
      matching across cell insertion, deletion, and reordering (including duplicate identical
      cells).
- [ ] Playwright test verifying that reloading a saved `.dsnb` restores cached outputs with a
      **Stale** badge without modifying the `.dsnb` file on disk.

## Follow-ups

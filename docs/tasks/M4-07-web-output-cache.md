# M4-07 — Web UI: local output cache across reloads (future enhancement)

**Status:** todo
**Depends on:** M4-02, M4-03
**Crates:** `web/`

## Goal
Optionally persist each cell's last rendered result locally outside the repository so reopening a
`.dsnb` notebook shows previous outputs immediately (marked **Stale** until re-run) while keeping
`.dsnb` files strictly source-only on disk (D-062).

## Scope
- Store each cell's last Arrow IPC batches, `RenderSpec`, timing/`sourceCount`/`rowLimit` metadata,
  and cell source + upstream-`let` fingerprint in browser `IndexedDB`, **keyed by workspace root +
  notebook path + cell source fingerprint** (storing cell index only as a disambiguation hint when
  two cells have identical source) so inserting, deleting, or reordering cells preserves cached
  outputs.
- Never write cache files inside the working directory or next to `.dsnb` files.
- On opening a `.dsnb` notebook, hydrate matching cells with their cached outputs marked **Stale**
  (and show a subtle "Cached from previous session · click Run to refresh" indicator).
- Enforce a bounded total cache size with LRU eviction and graceful fallback when storage quotas
  are exceeded or unavailable; provide a "Clear local output cache" action in the Command Palette.

## Acceptance criteria
- [ ] Unit tests for cache serialization, LRU eviction, quota-error handling, and fingerprint
      matching across cell insertion, deletion, and reordering (including duplicate identical
      cells).
- [ ] Playwright test verifying that reloading a saved `.dsnb` restores cached outputs with a
      **Stale** badge without modifying the `.dsnb` file on disk.

## Follow-ups

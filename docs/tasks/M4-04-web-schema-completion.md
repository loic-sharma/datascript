# M4-04 — Web UI: schema browser and completion

**Status:** todo
**Depends on:** M4-02, M3-02, M2-13
**Crates:** `web/`, `datascript-server`

## Goal
A schema panel (columns and types per file, expandable) and editor completion and hover backed by
the same compiler APIs the LSP uses (`/api/complete`).

## Acceptance criteria
- [ ] Completion results match the LSP for the same text and cursor (shared test vectors).

## Follow-ups

# M6-03 — Second backend: SQL emitter

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-sql` (new)

## Goal
Prove the engine abstraction with a backend that lowers the IR to SQL text (dialects:
DuckDB, then SQLite/Postgres). Its `Capabilities` honestly reports what it cannot express.

## Acceptance criteria
- [ ] The golden corpus runs against it (via DuckDB) with an `engine:` header matrix, and
      divergences are listed.
- [ ] No changes to the compiler were needed beyond bug fixes. If some were, document why in DECISIONS.md.
- [ ] Engine-specific differences use `---- expected[<engine>]` sections (TESTING.md § Several engines), each with a reason.

## Follow-ups

# M0-03 — Golden test harness

**Status:** todo
**Depends on:** M0-01, M0-02
**Crates:** `datascript-tests`

## Goal
Implement the `.dstest` runner specified in [TESTING.md](../TESTING.md) so later tasks can write
failing tests first.

## Scope
- `.dstest` parser (header keys, description, `---- query` / `---- expected` sections) with clear
  errors for malformed test files.
- A runner built on `libtest-mimic` (or `datatest-stable`) so every case is its own test, filterable by path.
- A `Driver` trait that the runner calls per mode (`syntax`, `check`, `explain`, `run`). Until the
  compiler exists, modes return "not implemented" and those cases are reported as **ignored**,
  not passed.
- Pretty diff on mismatch (`similar`). `DS_BLESS=1` rewrites the expected sections in place,
  keeping header and query untouched.
- ASCII table formatter for Arrow results (deterministic number/datetime formatting as specified).
  It can live here or in `datascript-engine`. Prefer the engine crate if the CLI will reuse it.
- One sample case per mode, ignored until implemented.

Design-review additions (see TESTING.md § Determinism, D-017):
- `order` defaults to `strict` only when every query's output order is defined (its last
  `sort`/`top` is followed only by order-preserving operators, LANGUAGE.md §3.4); otherwise `any`. Until the
  parser exists, read `order` from the header and default to `any`.
- Path normalization in output (`<test>/`, `<repo>/`).
- The `schema:` header and a mock `SchemaProvider` built from it, so M1-05 can write `check` cases
  before a real backend exists. The header's type names are parsed by the harness, not the compiler.
- A `DriverConfig` that later backends use to force `target_partitions = 1`.
- Parse `---- expected[<engine>]` sections now (used from M6-03), even if only one engine exists.

## Acceptance criteria
- [ ] `cargo test -p datascript-tests` discovers cases recursively under `tests/cases/`.
- [ ] Bless mode is idempotent and preserves the file's other content byte for byte.
- [ ] A malformed `.dstest` fails with a message naming the file and line.
- [ ] Documented in TESTING.md if anything deviates.

## Follow-ups

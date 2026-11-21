# M0-04 — Test fixture datasets

**Status:** todo
**Depends on:** M0-01
**Crates:** none (data only)

## Goal
Small, deterministic datasets under `tests/data/` used by golden tests, examples, and docs.

## Scope
Create the files listed in [TESTING.md § Fixtures](../TESTING.md#fixtures), plus
`tests/data/README.md` describing each file's columns, row count, and the interesting properties
tests rely on (nulls, ties for sort stability, duplicate keys for joins, a customer with no orders,
an order whose customer does not exist).

- Generate them with a committed script (`tests/data/generate.py` or a small Rust bin) using a fixed
  seed, so they can be regenerated. Commit the generated outputs too.
- `orders.parquet` must contain the same rows as `orders.csv`.
- `reordered/*.csv` (same columns, different header order, one file with an extra column) and
  `types.parquet` (the Arrow types listed in TESTING.md § Fixtures).

## Acceptance criteria
- [ ] Total size < 1 MB.
- [ ] README lists every file and its schema.
- [ ] Regenerating produces byte-identical files.

## Follow-ups

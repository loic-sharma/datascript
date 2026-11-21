# M0-03 — Golden test harness

**Status:** done — `.dstest` parser, libtest-mimic runner with a `Driver` trait (all modes ignored until implemented), bless, mock `schema:` provider, path/order normalization, and the Arrow table formatter in `datascript-engine::table`.
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
- [x] `cargo test -p datascript-tests` discovers cases recursively under `tests/cases/`.
- [x] Bless mode is idempotent and preserves the file's other content byte for byte.
- [x] A malformed `.dstest` fails with a message naming the file and line.
- [x] Documented in TESTING.md if anything deviates.

## Follow-ups
- `MockSchemaProvider` implements the compiler's `SchemaProvider` trait once M1-05 defines it.
- The DataScript ↔ Arrow type-name mappings in `datascript-tests::schema` and
  `datascript-engine::table` are small stand-ins; replace both with M1-04's type mapping.
- `Drivers::current()` registers only a stub `datafusion` driver. M1-15 adds the real driver
  (honoring `DriverConfig`) and implements `output_order_defined` from the parser.
- The `syntax` and `explain` samples have empty expected sections; bless them when M1-02 and
  M1-06 define the dump formats. The `run` sample's numbers are illustrative until M0-04's
  fixtures exist.
- Arrow is pinned to 59 in the workspace to match DataFusion 55; bump both together (M1-08).

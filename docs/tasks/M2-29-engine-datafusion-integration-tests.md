# M2-29 — Engine and DataFusion integration tests

**Status:** done — Added `datascript-datafusion/tests/integration.rs` (`Session` over the real engine on temp files) and `datascript-engine/tests/output_parity.rs` (every printer format parsed back over an edge-type dataset), golden join cases for null keys, empty sides and chained joins, the `compressed/events.ndjson.gz` fixture, and runtime `DS3008` for a file deleted between check and run.
**Depends on:** M2-15
**Crates:** `datascript-datafusion`, `datascript-engine`, `datascript-tests`
**Diagnostic codes:** none (tests for existing codes only; D-018).

## Goal
`Session` is tested only against the in-memory `FakeEngine`, neither `datascript-engine` nor
`datascript-datafusion` has a `tests/` integration suite, and the printer and writers have no
cross-format coverage of edge types. Add focused integration suites that run the real engine, and
a small set of golden `mode: run` cases for branches the lowering owns.

The sandbox (`DS3007`) bugs and tests found by the same audit belong to
[M4-01](M4-01-server-api.md), which owns the sandbox and `DS3007`'s promotion.

## Scope

### `Session` + `DataFusionEngine` suite (`crates/datascript-datafusion/tests/integration.rs`)
Add `tempfile` to `[dev-dependencies]` and build the suite on `tempfile::TempDir`:
- [x] **Schema cache invalidation.** `Session::invalidate_source` and `Session::invalidate_schemas`
      followed by `Session::run` pick up a changed schema and changed data; a cached `DS3001`
      clears once the file exists and is invalidated; a file changed or deleted between `check`
      and `run` gives a diagnostic, not a panic.
- [x] **Streaming and limits.** `Session::start` (`RunStream` / `QueryStream`) matches
      `Session::run` on a multi-batch (`batch_size = 2`), multi-query program. `row_limit` with
      `Some(0)`, an exact match (`truncated == false`) and a mid-batch cut (`truncated == true`),
      with each query in a multi-query program truncating on its own.
- [x] **Cancellation and rollback.** Dropping a `QueryStream` mid-stream and a `RunStream` between
      queries stops cleanly. When query 2 of `Session::run` fails, the results of query 1 are
      cleared.
- [x] **Multi-partition parity.** A join, a `summarize` and a `sort` with a tie-breaker give the
      same results with `target_partitions: 4` as with `1`.
- [x] **Generated-file edge cases** (generated in the `TempDir`, not checked in): runtime `DS3008`
      past `DEFAULT_INFER_ROWS` for CSV and NDJSON, and one compressed JSON source: a tiny
      `.ndjson.gz` checked into `tests/data/compressed/` (there is no `flate2` dependency to
      generate one). Gate it with `#[cfg(feature = "compression")]`, because M2-30 runs this crate's
      tests with `--no-default-features`; under `#[cfg(not(feature = "compression"))]`, assert the
      `DS3008` "this build cannot read compressed files" error instead.

### Output parity suite (`crates/datascript-engine/tests/output_parity.rs`)
- [x] Drive `printer::stream_results` over a shared edge-type dataset and check every `Format`
      (`table`, `csv`, `json`, `ndjson`, `arrow`, reading Arrow IPC back) agrees on:
      - `real` `NaN`, `±inf`, `-0.0` and `1e20` next to ordinary decimals,
      - `datetime` before 1970 and outside years `0..=9999`, negative `timespan`,
      - strings with wide characters, control characters and CSV-special characters (in data and
        in headers),
      - `dynamic` valid and invalid JSON, and an Arrow-only column (`Binary` or `Struct`).
- [x] 0-row results in every format (a 0-batch Arrow file still has the schema) and a 0-column
      result.
- [x] `write_csv_header` + `write_csv_rows` over several batches is byte-identical to `write_csv`.
- [x] Table output, through `printer::stream_results` with `Format::Table` (`write_table` is
      private): a runtime error during `count_rest` after the truncated table is printed, the
      singular `1 more row` footer, and no footer at exactly `max_rows`.

### Golden `mode: run` cases (`tests/cases/`)
- [x] Null join keys do not match (inner, outer, semi, anti) and an empty side for each kind.
- [x] A chained join in one query (`from a | join b on k | join c on k`), which exercises the
      lowering's per-join qualifiers.

### Out of scope
- Sandbox fixes and tests (M4-01).
- Re-running every golden `run` case under `target_partitions > 1`.
- Exhaustive aggregate and scalar edge-case matrices; M2-15 covered golden `run` gaps. Add a case
  only for a specific branch you suspect.
- Naming the failing file in multi-source runtime `DS3008` errors ([M2-12](M2-12-runtime-source-errors.md)).

## Acceptance criteria
- [x] Both new suites pass on every CI platform.
- [x] New `mode: run` cases are deterministic (`order: any` or a tie-broken `sort`, per
      [TESTING.md](../TESTING.md)).
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
      `cargo test --workspace` pass.

## Follow-ups
- Fixed in passing (the suite found it): a file deleted between `check` and `run` read as an empty
  result, because DataFusion lists a missing file as no files. Before planning,
  `DataFusionEngine::execute` now checks that each checked file still exists and gives runtime
  `DS3008` "cannot read `<file>`" at the scan ("this file was moved or deleted after the program
  was checked"); other I/O errors (permission denied) are `DS3008` with their own label. A source
  that fails when an engine reads it again to run is now `DS3008` too, not internal `DS9001`. A
  file deleted after that check but before DataFusion opens it still reads as no rows; carried
  over to M2-12.
- `compressed/events.ndjson.gz` is generated by `datascript-fixtures` with the hand-built
  stored-block `gzip` it already uses for `orders.csv.gz`, rather than checked in by hand, so the
  fixture check keeps covering it.
- Null join keys were unspecified; LANGUAGE.md §3.2 (`join`) now says they match nothing (SQL), which is
  what the new golden cases pin down.

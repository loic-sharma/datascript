# M2-29 — Engine and DataFusion integration tests

**Status:** todo
**Depends on:** M2-15
**Crates:** `datascript-datafusion`, `datascript-engine`, `datascript-diagnostics`, `datascript-tests`
**Diagnostic codes:** none new (`DS3007` is promoted from `Planned` to `Active` in `crates/datascript-diagnostics/src/codes.rs` and `docs/DIAGNOSTICS.md`; `DS3008`, `DS4003` and `DS4004` are already active).

## Goal
`datascript-engine`'s `Session` tests currently run only against an in-memory `FakeEngine`, neither
`datascript-engine` nor `datascript-datafusion` has a `tests/` integration suite, `DS3007` sandbox
enforcement has latent bugs and zero end-to-end tests, and `printer::stream_results` / `writers`
have no cross-format parity coverage on edge types. Fix the verified sandbox bugs, promote `DS3007`
to `Active`, and add focused integration suites and golden `mode: run` edge-case coverage across
the engine and DataFusion backend.

## Scope

### Sandbox fixes and `DS3007` promotion
- [ ] Fix missing-file-under-symlinked-directory bypass in `check_sandbox`
      (`crates/datascript-datafusion/src/sources.rs:545-572`): when `std::fs::canonicalize(path)`
      fails (missing target), resolve the deepest existing ancestor with `canonicalize` before
      appending and lexically normalizing the remaining components so intermediate symlinks cannot
      escape the sandbox.
- [ ] Fix 0-match escaping globs in `glob_files`
      (`crates/datascript-datafusion/src/sources.rs:486-532`): validate the glob's non-wildcard
      prefix against `check_sandbox` before returning `SourceError::NoMatches` (`DS3004`), so an
      escaping pattern such as `../*.nonexistent` or `/outside/*.csv` fails with
      `SourceError::OutsideSandbox` (`DS3007`) even when it matches zero files.
- [ ] Promote `DS3007` from `CodeStatus::Planned` to `CodeStatus::Active` in
      `crates/datascript-diagnostics/src/codes.rs:673-678` and update the corresponding row in
      `docs/DIAGNOSTICS.md:249` so the registry and spec stay in sync. (If [M2-30](M2-30-compiler-harness-ci-test-gaps.md)'s
      golden active-code coverage test has already landed, add `DS3007` to its justified allowlist
      noting that it is covered end-to-end by `crates/datascript-datafusion/tests/integration.rs`.)

### `Session` + `DataFusionEngine` integration suite (`crates/datascript-datafusion/tests/integration.rs`)
Add `tempfile.workspace = true` to `[dev-dependencies]` in `crates/datascript-datafusion/Cargo.toml`
and build `crates/datascript-datafusion/tests/integration.rs` using `tempfile::TempDir`:
- [ ] **End-to-end `DS3007` sandbox matrix**: run `Session::check` and `Session::run` against
      `DataFusionEngine` with `ExecContext::sandbox` covering:
      - symlink inside sandbox pointing to an existing file outside (`inside/link.csv -> ../outside.csv`),
      - directory symlink inside sandbox pointing outside (existing file and missing target),
      - escaping globs with 0 matches, all-outside matches, and mixed inside/outside matches
        (including a glob inside the sandbox matching an escaping symlink),
      - sibling-prefix directory paths (e.g. `sandbox2/a.csv` against `sandbox`),
      - absolute paths inside and outside the sandbox,
      - host tables (`DataFusionEngine::with_table`) whose registered path lies inside vs outside
        the sandbox (`sources.rs:151-183`).
- [ ] **Schema cache and `DataFusionEngine::resolved` invalidation/drift**:
      - `Session::invalidate_source(&SourceRef)` (`crates/datascript-engine/src/session.rs:210-212`)
        and `Session::invalidate_schemas()` (`session.rs:205-207`) followed by `Session::run`,
        verifying both schema and data refresh through `Session.schemas` and
        `DataFusionEngine::resolved` (`crates/datascript-datafusion/src/lib.rs:75-86, 132-162`).
      - Cached failures (`DS3001`) clearing after file creation + invalidation.
      - File mutation or deletion between `Session::check` and `Session::run`
        (`crates/datascript-datafusion/src/lib.rs:198-211`).
- [ ] **Streaming parity, `row_limit`, cancellation, and multi-query rollback**:
      - `Session::start` (`RunStream` / `QueryStream`) vs `Session::run` equivalence on
        multi-batch (`batch_size = 2`) and multi-query programs (schemas, rows, `render` specs,
        and runtime diagnostics).
      - `ExecContext::row_limit` (`crates/datascript-engine/src/lib.rs:106-134, 264`,
        `session.rs:422`) with `Some(0)` (empty vs non-empty stream), `Some(1)`, exact batch/result
        match (`truncated == false`), mid-batch cut (`truncated == true`), and multi-query programs
        where each query truncates independently.
      - In-process real-engine cancellation: drop a multi-batch `DataFusionEngine` `QueryStream`
        mid-stream and drop a `RunStream` between queries, verifying clean stop without hanging or
        panicking.
      - Multi-query `Session::run` rollback when query 2 fails (`session.rs:427-431` clears earlier
        `output.results`).
- [ ] **Multi-partition parity (`target_partitions = 4` vs `1`)**:
      - Run 5–6 representative queries (`sort` with tie-breaker, `summarize`, `join`, `distinct`,
        `top`, `union`) with `DataFusionConfig { target_partitions: 4, .. }` and assert results
        match `target_partitions: 1` (sorting unordered outputs).
- [ ] **Dynamic `TempDir` source edge cases**:
      - Runtime `DS3008` past `DEFAULT_INFER_ROWS = 1000` (`sources.rs:33`, `errors.rs:204-238`)
        for both CSV (`ArrowError::CsvError` / `ParseError`) and NDJSON (`ArrowError::JsonError`).
      - `read_csv(..., escape = '\\')` (`sources.rs:77, 102, 222`).
      - `.bz2` / `compression = 'bzip2'`, `.xz` / `compression = 'xz'`, explicit
        `compression = 'zstd'`, and compressed JSON (`.ndjson.gz` / `read_json(..., compression = 'gzip')`,
        exercising `is_json_array` at `sources.rs:325-343`).
      - `read_csv` / `read_json` / `read_parquet` invoked with a glob pattern (`sources.rs:171`),
        including `read_csv(..., null_value = ...)` over non-uniform files (`lower.rs:392-426`).

### Multi-format output parity suite (`crates/datascript-engine/tests/output_parity.rs`)
- [ ] Add `crates/datascript-engine/tests/output_parity.rs` driving `printer::stream_results`
      (`crates/datascript-engine/src/printer.rs:140-400`), `table::format_table`
      (`table.rs:38-100`), and `writers::*` (`writers.rs:26-147`) over a shared edge-type dataset:
      - Verify consistent representation across `Format::Table`, `Csv`, `Json`, `Ndjson`, and
        `Arrow` IPC (reading Arrow IPC back via `arrow_ipc::reader::FileReader`) for:
        - `bool`, `int`, `long`, `real` including `NaN`, `+inf`, `-inf`, `-0.0`, and `1e20` mixed
          with normal decimals (`table.rs:152-158, 185-193`, `writers.rs:133-136`),
        - `datetime` (pre-1970, fractional µs, years outside `0..=9999` at `table.rs:298-302`) and
          `timespan` (negative, `i64::MIN`),
        - `string` with Unicode/CJK/combining marks/emoji (`table.rs:208-210`), control characters
          (`\r`, `\n`, `\t`, `\x00`, `\x1b` at `table.rs:268-285`), and CSV-special characters
          (including column headers containing `,`, `"`, `\n`),
        - `Type::Dynamic` valid JSON and invalid-JSON fallback (`writers.rs:137-140`, plus Arrow
          IPC `arrow.json` extension metadata preservation),
        - Arrow-only columns (`Binary`, `Struct`) across all 5 formats (including `printer.rs:410`
          `arrow:...` type names in JSON `columns` metadata).
      - Verify 0-row results across all 5 formats (including valid 0-batch Arrow IPC file with
        schema) and 0-column results (`(no columns)`).
      - Verify `write_csv_header` + `write_csv_rows` multi-batch byte-for-byte parity with
        `write_csv` (`writers.rs:31-57`).
      - Verify `printer::write_table` runtime error during `count_rest` *after* the truncated table
        has already been printed (`printer.rs:284-287`), singular `"1 more row"` footer
        (`printer.rs:297`), exact `max_rows` boundary (no footer), and `Sink::lazy` staying
        unopened on 0-query programs across `Json`/`Arrow`/`Ndjson`/`Table` (`printer.rs:189`).

### Targeted golden `mode: run` cases (`tests/cases/`)
- [ ] **Empty and all-null aggregates**: cover all 12 aggregates (`count`, `countif`, `sum`,
      `sumif`, `avg` including `avg(timespan)`, `min`, `max`, `dcount`, `percentile` including
      `percentile(x, null)`, `stdev`, `variance`, `take_any`) over 0-row and all-null inputs.
- [ ] **Join edge cases**:
      - Null join keys (`null == null` does not match) across inner, outer, semi, and anti joins.
      - Empty left or right join side across join kinds.
      - Chained multi-join in a single query (`from a | join b on k | join c on k`, exercising
        `self.joins += 1` qualifier isolation at `crates/datascript-datafusion/src/lower.rs:312-316`).
- [ ] **Operator and expression null/edge cases**:
      - `distinct` over columns containing nulls (`tests/cases/operators/distinct/`).
      - `top N by ... asc/desc nulls first/last` with nulls in the sort key.
      - `in` / `!in` / `in~` / `!in~` with `null` in the LHS or list.
      - `union` with an empty input and chained `union`s; `from '../../data/edge/empty.csv' | getschema`
        (`lower.rs:271-279`).
      - Scalar edge cases: `bin` with negative size or null (`udf.rs:426, 433-438`),
        `tobool(0.0 / 0.0)`, `tolong`/`toint` on `NaN`/`inf` (`udf.rs:852-855, 907`), `extract`
        with negative group (`udf.rs:808`), `indexof` with `length < -1` (`udf.rs:768`), real
        modulo (`lower.rs:624`), `datetime_diff` for `'minute'`/`'dayofyear'`, and `now(-1h)` /
        `ago(-1h)`.
- [ ] **Missing `DS4003` / `DS4004` runtime error branches** (`tests/cases/errors/runtime/`):
      - `DS4003`: `bin` overflow (`udf.rs:433-437`), `round(long, -digits)` overflow (`udf.rs:489`),
        `datetime_diff('microsecond', ...)` overflow (`calendar.rs:176`), `now(offset)` /
        `ago(offset)` overflow (`lower.rs:725-726`), `long` subtraction overflow (`k_sub`).
      - `DS4004`: `percentile` with negative `p` and `NaN` (`udaf.rs:317-324`).

### Out of scope
- Adding a `sandbox:` header to the `.dstest` golden harness (the `TempDir` matrix in
  `crates/datascript-datafusion/tests/integration.rs` covers `DS3007` cleanly without coupling
  golden files to OS symlink semantics).
- Re-running all 155 golden `mode: run` cases under `target_partitions > 1` (the 5–6 query
  multi-partition parity matrix covers parallel plan boundaries without multiplying CI runtime).
- Checking large (>1000-row) or wide binary/compressed fixtures into `tests/data/` (generate them
  dynamically inside `TempDir` tests instead so `tests/data/` stays small).
- Naming the failing file in multi-source runtime `DS3008` errors (owned by [M2-12](M2-12-runtime-source-errors.md)).

## Acceptance criteria
- [ ] Both `check_sandbox` bugs are fixed and `DS3007` is `CodeStatus::Active` in `codes.rs` and
      `docs/DIAGNOSTICS.md`.
- [ ] `crates/datascript-datafusion/tests/integration.rs` and
      `crates/datascript-engine/tests/output_parity.rs` pass on all CI platforms (Unix symlink
      tests gated with `#[cfg(unix)]` where needed).
- [ ] Every new `mode: run` `.dstest` case passes with deterministic output (`order: any` or
      tie-broken `sort` per `docs/TESTING.md`).
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, and
      `cargo check -p datascript-syntax -p datascript-compiler -p datascript-engine --target wasm32-unknown-unknown`
      all pass.

## Follow-ups

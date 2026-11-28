# M2-33 — Custom JSON/NDJSON reader with `arrow.json` columns

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** none new (reuses `DS2041`, `DS3003`, `DS3005`, `DS3008`; richer JSON read diagnostics are in [M2-34](M2-34-json-read-diagnostics.md); D-018)

## Goal
Read real-world semi-structured JSON and NDJSON files without failing on heterogeneous nested
fields or blowing up schemas on map-like objects.

Today `datascript-datafusion` delegates JSON/NDJSON reading to DataFusion's `JsonFormat`, which
uses `arrow-json`'s strict recursive schema inference and `ReaderBuilder`:
- If a nested field has incompatible JSON types across rows within `DEFAULT_INFER_ROWS` (for
  example, a string `"scripts.yaml"` in one record and an object `{"build": "..."}` in another),
  `arrow-json` aborts with `Expected object json type, found: Scalar({Utf8})`.
- Even if the schema were overridden to `Utf8`, `arrow-json`'s `StringArrayDecoder` cannot decode
  JSON objects or arrays into strings (it either errors or drops them to `NULL`).
- Map-like objects (such as package dependency maps) create a static Arrow `Struct` child column
  for every distinct key seen during inference.

Replace DataFusion's `JsonFormat` with a custom JSON/NDJSON `FileFormat` and `FileSource` in
`datascript-datafusion` that infers **top-level columns** and stores nested (object/array) and
mixed-type columns as `Utf8` with the `arrow.json` extension type (raw JSON text). This aligns
directly with DataScript's `dynamic` representation ([LANGUAGE.md](../LANGUAGE.md) §5,
[M2-03](M2-03-dynamic-type.md)) and matches DuckDB's ability to query polymorphic NDJSON files.

> **Open risk (Q-003):** This task builds on `Utf8` + `arrow.json` as the storage representation
> (`LANGUAGE.md` §5, D-024). If [M2-03](M2-03-dynamic-type.md) resolves Q-003 in favor of Arrow
> Variant instead, the JSON reader's output type for nested/mixed columns will need to be updated
> there.

## Scope
- **Custom JSON/NDJSON `FileFormat` and `FileSource`** (a self-contained module, e.g.
  `crates/datascript-datafusion/src/json.rs`):
  - **Top-level schema inference** over the first `infer_rows` records (default
    `DEFAULT_INFER_ROWS = 1000`):
    - `Null` + any type `T` → `T` (all-null or absent in the sample infers `Null`).
    - `Int64` + `Float64` → `Float64`.
    - Integers outside the `i64` range → `Float64` (matches `arrow-json` and avoids surprising
      `DS4003` errors on JSON numbers).
    - Consistent scalar columns infer `Boolean`, `Int64`, `Float64`, or `Utf8`.
    - Any top-level column that contains a JSON **object**, a JSON **array**, or **any other mix
      of JSON types** (including mixed scalars such as `Int64` + `Utf8` or `Boolean` + `Utf8`, as
      well as scalar + object/array) is inferred as `Utf8` with extension type `arrow.json`
      (`datascript_compiler::arrow::JSON_EXTENSION`). Widening mixed scalars to `arrow.json`
      rather than plain `Utf8` preserves JSON types losslessly (`1` vs. `"1"`) for `dynamic`/KQL.
    - Duplicate keys within one JSON object: **last wins** (matches `serde_json` and
      `JSON.parse`).
    - Supports both NDJSON (`.ndjson`, `.jsonl`, `read_json(..., format = "ndjson")`, auto-detected
      `.json`) and JSON arrays (`[{...}, ...]`, `read_json(..., format = "array")`, auto-detected
      `.json`), plus compressed files (`.gz`, `.bz2`, `.xz`, `.zst` behind
      `#[cfg(feature = "compression")]`).
    - Note: streaming a top-level JSON array (`[{...}, ...]`) is not a
      `serde_json::StreamDeserializer` one-liner — implement a small bracket-, comma-, and
      string-aware array-to-record scanner (similar to DataFusion's `JsonArrayToNdjsonReader`) so
      large array files stream without buffering the whole document.
    - Read samples and detect array-vs-NDJSON through the DataFusion `ObjectStore` instead of
      `std::fs` (removing `is_json_array`'s `std::fs` call, which builds for `wasm32` but would
      fail at runtime in the browser; `single_file`/`glob_files` still use `std::fs` until
      [M5-01](M5-01-wasm-build.md)).
  - **Streaming row decoding and projection pushdown:**
    - **Encoding inside `arrow.json` columns:** every non-null value—including scalars—is stored
      verbatim as raw JSON text (a string row is stored with quotes, e.g. `"\"scripts.yaml\""`; a
      number keeps its original JSON text; booleans as `"true"`/`"false"`; objects and arrays as
      raw JSON slices). JSON `null` and missing keys become Arrow `null`.
    - **Type changes past `infer_rows`:** every scalar-inferred column (including plain `Utf8`)
      fails with a structured type-mismatch error if a later row past `infer_rows` contains an
      incompatible JSON kind (for example, an object, array, number, or boolean in a `Utf8`
      column) rather than silently coercing; `read_json(..., infer_rows = n)` is the escape hatch.
    - Parse only top-level keys per JSON object record (e.g. via `serde_json::value::RawValue` or
      a lightweight streaming scanner), slicing nested objects/arrays verbatim for `arrow.json`
      columns without recursing into nested keys.
    - Implement `FileSource::try_pushdown_projection` so unprojected columns (including large
      nested JSON blobs) are skipped during scanning.
    - Respect `batch_size`, fill missing keys with `null`, and ignore keys absent from the
      inferred schema.
    - Carry a structured error type through `DataFusionError::External` (recording 1-based line
      number, 1-based record index for JSON arrays, column name, and failure kind: syntax error,
      non-object record, or type mismatch) so [M2-34](M2-34-json-read-diagnostics.md) can render
      plain-language `DS3008` diagnostics without parsing error strings.
- **Preserve field metadata in schema merging (`crates/datascript-datafusion/src/sources.rs`):**
  - `Merged::Arrow` currently stores only `DataType` and strips field metadata in `Merged::field`.
    Preserve field metadata (in particular `arrow.json`) across `Merged::of`, `Merged::merge` and
    `Merged::field` so `Utf8` + `arrow.json` columns do not silently degrade into `Type::String`.
  - **Glob merging (`Typed` vs. `arrow.json`):** keep `Merged::Typed(_)` vs.
    `Merged::Arrow(Utf8, arrow.json)` across different files in a glob as a `DS3005` conflict
    (a plain Arrow `Utf8` → `Utf8` cast would leave scalar strings unquoted and produce invalid
    JSON), while `Merged::Null` + `Merged::Arrow(Utf8, arrow.json)` merges to `arrow.json`.
  - Until [M2-03](M2-03-dynamic-type.md) lands, `from_arrow_source` continues to report `Utf8`
    with `arrow.json` as an unsupported pass-through type (`DS2008` when used in expressions;
    allowed in `project`, `project-away`, `take`, etc.).
- **`infer_rows` option on `read_json`:**
  - Add `infer_rows` (positive `long`, default 1,000, matching `read_csv`) to `read_json` in
    `crates/datascript-compiler/src/source.rs` (`READERS`), wire it into `Reader::Json` in
    `crates/datascript-datafusion/src/sources.rs`, and update any reader-option tests/completions.
- **Docs, hand-offs, and existing tests:**
  - Update `tests/cases/sources/glob_schema.dstest` (where `props` in `events/*.ndjson` now
    reports `Utf8 (extension type \`arrow.json\`)` instead of `Struct(...)`), and verify existing
    JSON tests (`glob_ndjson_run.dstest`, `reader_json.dstest`, `path_formats.dstest`,
    `path_formats_run.dstest`, `host_table_real.dstest`).
  - Update [LANGUAGE.md](../LANGUAGE.md) (§3.1 `read_json` options, §5.1 JSON nested/mixed
    columns) and record the top-level `arrow.json` inference rules (including mixed scalars →
    `arrow.json`, out-of-range integers → `Float64`, duplicate keys → last wins, and glob
    `DS3005`) in [DECISIONS.md](../DECISIONS.md).
  - Update [M5-01](M5-01-wasm-build.md)'s carried-over bullet from M2-13 to remove `is_json_array`
    once it reads through `ObjectStore` (keeping "Test JSON, NDJSON and globs on wasm32").
  - Add a carried-over line to [M2-03](M2-03-dynamic-type.md): `(from M2-33) the JSON reader now
    produces Utf8 + arrow.json for nested and mixed columns, never Struct/List; mapping
    arrow.json to dynamic covers JSON sources`.

## Acceptance criteria
- [ ] Golden `check` and `run` cases covering:
      - NDJSON with a polymorphic nested field (string in one row, object in another),
      - NDJSON with polymorphic top-level fields (string vs. object—projecting the column to
        verify strings are stored quoted as raw JSON text—scalar vs. array, int vs. float, and
        mixed scalars like int vs. string → `arrow.json`),
      - JSON array (`[{...}, ...]`) with nested and heterogeneous fields (including a minified
        single-line array),
      - glob merging: `Null` + `arrow.json` merges cleanly, and `string` vs. `arrow.json` across
        files reports `DS3005`,
      - compressed `.ndjson.gz` (gated by `#[cfg(feature = "compression")]`) and existing JSON
        tests (`glob_schema.dstest`, `glob_ndjson_run.dstest`, `reader_json.dstest`,
        `path_formats.dstest`, `path_formats_run.dstest`),
      - `read_json(..., infer_rows = n)` happy path and invalid option values (`DS2041`).
- [ ] Unit tests for the custom JSON schema inference and streaming decoder (empty files, blank
      lines, integers outside `i64`, duplicate keys last-wins, missing/extra keys, type change
      past `infer_rows` on `Utf8` and other scalar columns, multi-batch streaming, streaming JSON
      array scanner, and projection pushdown skipping unprojected columns).
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`,
      `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`,
      and `cargo check -p datascript-datafusion --no-default-features --features wasm --target wasm32-unknown-unknown`
      pass.

## Follow-ups

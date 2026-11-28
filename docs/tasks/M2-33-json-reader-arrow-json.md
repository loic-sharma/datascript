# M2-33 — Custom JSON/NDJSON reader with `arrow.json` columns

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** none new (reuses `DS2041`, `DS3003`, `DS3008`; richer JSON read diagnostics are in [M2-34](M2-34-json-read-diagnostics.md); D-018)

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

## Scope
- **Custom JSON/NDJSON `FileFormat` and `FileSource`** (a self-contained module, e.g.
  `crates/datascript-datafusion/src/json.rs`):
  - **Top-level schema inference** over the first `infer_rows` records (default
    `DEFAULT_INFER_ROWS = 1000`):
    - Top-level columns that hold only compatible scalars infer `Boolean`, `Int64`, `Float64`
      (`Int64` + `Float64` widens to `Float64`), `Utf8` (other mixed scalar types such as
      `Int64` + `Utf8` widen to `Utf8`), or `Null` (all-null or absent in the sample).
    - Any top-level column that contains a JSON **object**, a JSON **array**, or a mix of
      scalar and object/array values is inferred as `Utf8` with extension type `arrow.json`
      (`datascript_compiler::arrow::JSON_EXTENSION`).
    - Supports both NDJSON (`.ndjson`, `.jsonl`, `read_json(..., format = "ndjson")`, auto-detected
      `.json`) and JSON arrays (`[{...}, ...]`, `read_json(..., format = "array")`, auto-detected
      `.json`), plus compressed files (`.gz`, `.bz2`, `.xz`, `.zst` behind
      `#[cfg(feature = "compression")]`).
    - Reads samples and detects array-vs-NDJSON through the DataFusion `ObjectStore` (not
      `std::fs`), keeping `wasm32-unknown-unknown` builds working and removing the `std::fs`
      dependency in `is_json_array` (coordinate with [M5-01](M5-01-wasm-build.md)).
  - **Streaming row decoding and projection pushdown:**
    - Parse only top-level keys per JSON object record (e.g. via `serde_json::value::RawValue` or
      a lightweight streaming scanner), storing raw JSON slices verbatim for `arrow.json` columns
      without recursing into nested keys.
    - Implement `FileSource::try_pushdown_projection` so unprojected columns (including large
      nested JSON blobs) are skipped during scanning.
    - Respect `batch_size`, fill missing keys with `null`, and ignore keys absent from the
      inferred schema.
    - Carry a structured error type through `DataFusionError::External` (recording line/record
      number, column name, and failure kind: syntax error, non-object record, or type mismatch)
      so [M2-34](M2-34-json-read-diagnostics.md) can render plain-language `DS3008` diagnostics
      without parsing error strings.
- **Preserve field metadata in schema merging (`crates/datascript-datafusion/src/sources.rs`):**
  - `Merged::Arrow` currently stores only `DataType` and strips field metadata in `Merged::field`.
    Preserve field metadata (in particular `arrow.json`) across `Merged::of`, `Merged::merge` and
    `Merged::field` so `Utf8` + `arrow.json` columns do not silently degrade into `Type::String`.
  - Decide and test glob merging when one file infers a scalar column and another infers
    `arrow.json` (either widen to `arrow.json` across JSON files or report `DS3005`, matching
    `LANGUAGE.md` §3.1).
  - Until [M2-03](M2-03-dynamic-type.md) lands, `from_arrow_source` continues to report `Utf8`
    with `arrow.json` as an unsupported pass-through type (`DS2008` when used in expressions;
    allowed in `project`, `project-away`, `take`, etc.).
- **`infer_rows` option on `read_json`:**
  - Add `infer_rows` (positive `long`, default 1,000, matching `read_csv`) to `read_json` in
    `crates/datascript-compiler/src/source.rs` (`READERS`), wire it into `Reader::Json` in
    `crates/datascript-datafusion/src/sources.rs`, and update any reader-option tests/completions.
- **Docs and existing tests:**
  - Update `tests/cases/sources/glob_schema.dstest` (where `props` in `events/*.ndjson` now
    reports `Utf8 (extension type \`arrow.json\`)` instead of `Struct(...)`).
  - Update [LANGUAGE.md](../LANGUAGE.md) (§3.1 `read_json` options, §5.1 JSON nested/mixed
    columns) and record the top-level `arrow.json` inference decision in
    [DECISIONS.md](../DECISIONS.md).

## Acceptance criteria
- [ ] Golden `check` and `run` cases covering:
      - NDJSON with a polymorphic nested field (string in one row, object in another),
      - NDJSON with polymorphic top-level fields (string vs object, scalar vs array, int vs float,
        int vs string),
      - JSON array (`[{...}, ...]`) with nested and heterogeneous fields,
      - compressed `.ndjson.gz` (gated by `#[cfg(feature = "compression")]`) and existing NDJSON
        glob tests (`glob_schema.dstest`, `glob_ndjson_run.dstest`),
      - `read_json(..., infer_rows = n)` happy path and invalid option values (`DS2041`).
- [ ] Unit tests for the custom JSON schema inference and streaming decoder (empty files, blank
      lines, missing/extra keys, multi-batch streaming, and projection pushdown skipping
      unprojected columns).
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`,
      `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`,
      and `cargo check -p datascript-datafusion --no-default-features --features wasm --target wasm32-unknown-unknown`
      pass.

## Follow-ups

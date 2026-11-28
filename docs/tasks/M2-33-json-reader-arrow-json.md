# M2-33 — Custom JSON/NDJSON reader with `arrow.json` columns

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler` (only the `read_json` entry in `READERS`; `arrow::from_arrow_source` stays unchanged, mapping `arrow.json` to `dynamic` is [M2-03](M2-03-dynamic-type.md)'s), `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** none new (reuses `DS2041`, `DS3003`, `DS3008`; JSON globs no longer report `DS3005`; richer JSON read diagnostics are in [M2-34](M2-34-json-read-diagnostics.md); D-018)

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
    - `Null` + any type `T` → `T`.
    - `Int64` + `Float64` → `Float64`.
    - A JSON number infers `Int64` only when it is an integer in the `i64` range with no fraction
      or exponent; any other number (`1.0`, `1.5`, `1e3`) infers `Float64`.
    - Integers outside the `i64` range (for example `u64` IDs and hashes) → `Utf8` +
      `arrow.json`, which keeps their exact digits. This deliberately differs from `arrow-json`,
      which infers `Float64` and silently rounds them; it also avoids surprising `DS4003` errors.
    - Numbers whose magnitude overflows `f64` (for example `1e400`) → `Utf8` + `arrow.json` for
      the same reason, rather than storing infinity.
    - Consistent scalar columns infer `Boolean`, `Int64`, `Float64`, or `Utf8`.
    - Any top-level column that contains a JSON **object**, a JSON **array**, or **any other mix
      of JSON types** (including mixed scalars such as `Int64` + `Utf8` or `Boolean` + `Utf8`, as
      well as scalar + object/array) is inferred as `Utf8` with extension type `arrow.json`
      (`datascript_compiler::arrow::JSON_EXTENSION`). Widening mixed scalars to `arrow.json`
      rather than plain `Utf8` preserves JSON types losslessly (`1` vs. `"1"`) for `dynamic`/KQL.
    - **All-null columns:** a key whose sampled values are all JSON `null` infers Arrow `Null` for
      that file (so glob merging lets another file's type win, as for CSV, D-044). If no file of
      the source gives it a type, the merged column is `Utf8` + `arrow.json` (not `string` as
      for CSV), so any later value can be stored losslessly instead of failing the type check.
    - **Keys first seen past `infer_rows`** are not in the schema and are dropped silently (as
      `arrow-json` and DuckDB do). Document this in LANGUAGE.md §3.1 next to `infer_rows`.
    - Duplicate keys within one **top-level** JSON object: **last wins** (matches `serde_json`
      and `JSON.parse`). Nested objects are stored as written (see "Encoding" below), so
      duplicate keys inside them are kept; resolving them is [M2-03](M2-03-dynamic-type.md)'s
      job when it parses `dynamic` values.
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
    - **Detect array-vs-NDJSON per file** for auto-detected `.json` sources, both when sampling
      and when the `FileSource` opens each file. Today `sources.rs` checks only `paths[0]` and
      applies its answer to the whole glob; once one table reads every file, a `*.json` glob
      mixing NDJSON and array files must still read each file in its own layout. An explicit
      `format = "ndjson"`/`"array"` (or a `.ndjson`/`.jsonl` extension) still applies to every
      file.
    - **Skip a leading UTF-8 BOM** (`EF BB BF`) before detection and decoding. `serde_json`
      rejects it, and today `is_json_array` treats it as the first non-whitespace character, so
      a BOM-prefixed array file is misdetected as NDJSON. Line numbers are unaffected (the BOM
      is on line 1).
  - **Schema-driven streaming decoder and projection pushdown:**
    - The decoder decodes records into **any target schema it is given**, matching keys by name,
      not only the schema it inferred for that file. `lower.rs::scan` passes the merged
      (widened) schema for JSON sources, so every file of a glob is read by one table with one
      schema and no post-read casts are needed. This also lets [M2-09](M2-09-schema-overrides.md)
      plug a user-declared `schema = (...)` straight into the reader.
    - **Encoding inside `arrow.json` columns:** every non-null value—including scalars—is stored
      as its JSON text: a string row is stored with quotes (e.g. `"\"scripts.yaml\""`), a number
      keeps its original JSON spelling, booleans are `true`/`false`, and objects and arrays are
      raw JSON slices with **insignificant whitespace removed** (a cheap, string-aware byte pass,
      so pretty-printed array files do not store newlines and indentation). Escapes, number
      spelling, key order and nested duplicate keys are kept as written; the reader does not
      parse nested values. JSON `null` and missing keys become Arrow `null`. Record this in
      DECISIONS.md: `dynamic` equality and grouping in M2-03 must compare parsed values (or
      canonicalize) rather than raw text, since `"\u00e9"` and `"é"` are stored differently.
    - **Type checks past `infer_rows`** (the reader never coerces silently;
      `read_json(..., infer_rows = n)` is the escape hatch). For each target column type:
      | Target type | Accepted JSON values | Anything else |
      |---|---|---|
      | `Boolean` | `true`, `false` | type mismatch (including the strings `"true"`/`"false"`) |
      | `Int64` | integers in the `i64` range, with no fraction or exponent | type mismatch (including `1.0`, `1.5`, `1e3` and integers outside `i64`) |
      | `Float64` | any JSON number except integers outside the `i64` range and numbers that overflow `f64` (in-range integers are converted) | type mismatch (including integers outside `i64`, which would otherwise be rounded silently, and numbers such as `1e400`, which would become infinity; inference stores both as `arrow.json` for the same reason) |
      | `Utf8` (no extension) | strings | type mismatch (objects, arrays, numbers, booleans) |
      | `Utf8` + `arrow.json` | anything | — |
      `null` is accepted in every column.
    - Parse only top-level keys per JSON object record (e.g. via `serde_json::value::RawValue` or
      a lightweight streaming scanner), slicing nested objects/arrays for `arrow.json` columns
      without recursing into nested keys.
    - Implement `FileSource::try_pushdown_projection` so unprojected columns (including large
      nested JSON blobs) are skipped during scanning. Type checks run only on projected
      columns, so a type mismatch in a column the query never reads (or in any column, for a
      query such as `count` that projects none) is not an error; syntax errors and non-object
      records are still reported for every record. Record this in DECISIONS.md and LANGUAGE.md
      §3.1 (DuckDB behaves the same way).
    - Respect `batch_size`, fill missing keys with `null`, and ignore keys absent from the
      target schema.
    - **Read each file whole, from byte 0:** override `FileSource::supports_repartitioning` to
      return `false`. DataFusion's default (`true`) splits large uncompressed files into byte
      ranges read in parallel; a range that starts inside a JSON array cannot be parsed, and a
      reader that starts mid-file cannot know its 1-based line number, which the structured
      error and [M2-34](M2-34-json-read-diagnostics.md) need. Parallelism across the files of a
      glob is unaffected. Record this in DECISIONS.md.
    - Carry a structured error type through `DataFusionError::External` (recording the file's
      path, taken from the `PartitionedFile` being opened, 1-based line
      number, 1-based record index for JSON arrays, column name, and failure kind: syntax error,
      non-object record, or type mismatch with the target type and the JSON kind found) so
      [M2-34](M2-34-json-read-diagnostics.md) can render plain-language `DS3008` diagnostics
      without parsing error strings. The file path matters because a JSON glob is now read by
      one table: [M2-12](M2-12-runtime-source-errors.md) cannot fall back to one
      `ListingTable` per file to find which file failed, so it names the file from this field.
  - **Keep `DS3008` working until M2-34 lands.** `errors.rs::read_problem` only recognizes
    `ArrowError::{CsvError, JsonError, ParseError}` and Parquet errors; anything else falls
    through to `DS4002 "the query failed while running"`. Add a minimal match for the new error
    type in `crates/datascript-datafusion/src/errors.rs` so runtime JSON errors stay `DS3008` at
    the scan: label `a value in this source is not valid JSON` for syntax and non-object errors,
    and `a value in this source does not match its column's type` for type mismatches, with a
    help that names `read_json(..., infer_rows = n)` (not `read_csv`). The note may still carry
    the error's own text; M2-34 replaces it with structured notes.
- **Schema merging (`crates/datascript-datafusion/src/sources.rs`):**
  - `Merged::Arrow` currently stores only `DataType` and strips field metadata in `Merged::field`.
    Add a dedicated `Merged::Json` variant: `Merged::of` maps a `Utf8` + `arrow.json` field to it
    **before** calling `from_arrow_source`, and `Merged::field` turns it back into `Utf8` with the
    `arrow.json` metadata, so these columns do not silently degrade into `Type::String`. Checking
    the extension first keeps the rules below working after [M2-03](M2-03-dynamic-type.md) makes
    `from_arrow_source` map `arrow.json` to `Type::Dynamic`; otherwise `Typed(String)` +
    `Typed(Dynamic)` would go through `Type::common` and turn working globs back into `DS3005`.
  - **Glob merging for JSON sources** (the merge needs to know the source is JSON):
    - `Merged::Null` + `Merged::Json` → `Merged::Json`.
    - `Merged::Typed(_)` + `Merged::Json` → `Merged::Json`.
    - `Merged::Typed(a)` + `Merged::Typed(b)` → `Type::common(a, b)` when there is one (e.g.
      `long` + `real` → `real`), else `Merged::Json`, never `DS3005`. This matches what one file
      holding both types in its first `infer_rows` records infers. Merging only ever widens
      (`long` → `real` → `arrow.json`) and every wider type accepts the narrower one's values,
      so adding a file never turns a working read into a failing one (unless the new file is
      itself malformed). The converse does not hold, because each file is sampled on its own:
      a file whose column changes type past `infer_rows` fails alone but can read once another
      file widens that column.
    - These are safe because the decoder reads every file into the merged schema and encodes
      scalars as JSON text (strings quoted), rather than casting a file's column after the read.
      Real globs (daily log files where one file holds only strings in a field and another holds
      numbers or objects) then work.
    - `Merged::Null` with no other type → `Merged::Json` (see "All-null columns").
    - CSV, TSV and Parquet merging is unchanged (`Type::common`, else `DS3005`).
  - With one schema for all JSON files, `Resolved::uniform` / the per-file `UNION ALL` path in
    `lower.rs::scan` is no longer needed for JSON sources (keep it for CSV).
  - Until [M2-03](M2-03-dynamic-type.md) lands, `from_arrow_source` continues to report `Utf8`
    with `arrow.json` as an unsupported pass-through type (`DS2008` when used in expressions;
    allowed in `project`, `project-away`, `take`, etc.).
- **`infer_rows` option on `read_json`:**
  - Add `infer_rows` (positive `long`, default 1,000, matching `read_csv`) to `read_json` in
    `crates/datascript-compiler/src/source.rs` (`READERS`), wire it into `Reader::Json` in
    `crates/datascript-datafusion/src/sources.rs`, and update the reader-option tests and
    completions.
  - Fix the doc comment on `DEFAULT_INFER_ROWS` in `sources.rs`, which says it is only
    `read_csv`'s default.
- **Docs, hand-offs, and existing tests:**
  - Update `tests/cases/sources/glob_schema.dstest`: `props` in `events/*.ndjson` now reports
    `Utf8 (extension type \`arrow.json\`)` instead of `Struct(...)`; also update the header comment
    ("`props` is a JSON object…") and check the DS2008 note ("nested, JSON and GUID columns…")
    still reads right.
  - Add `infer_rows` cases to `tests/cases/sources/reader_options.dstest` (where the `read_json`
    option errors live).
  - Re-bless `tests/cases/sources/unreadable.dstest` (`broken.json`), whose note text changes
    with the new reader.
  - Verify the other existing JSON tests: `glob_ndjson_run.dstest`, `reader_json.dstest`,
    `path_formats.dstest`, `path_formats_run.dstest`, `host_table_real.dstest`, and
    `tests/cases/operators/join/run_multi_key.dstest` (reads `events/*.ndjson`).
  - Update [LANGUAGE.md](../LANGUAGE.md) (§3.1 `read_json` options, including `infer_rows` and
    dropped late keys; §5.1 JSON nested/mixed columns, and the §5.1 type-mapping row
    "Null → `string`", which must say that for JSON sources an all-null column becomes
    `Utf8` + `arrow.json` instead) and record the inference, encoding and
    merging rules in [DECISIONS.md](../DECISIONS.md): mixed scalars → `arrow.json`, out-of-range
    integers and `f64`-overflowing numbers → `arrow.json` (unlike `arrow-json`), all-null
    columns, per-file array-vs-NDJSON detection, leading BOM skipped, top-level duplicate keys →
    last wins, whitespace-only normalization of nested values, the type-check table, type checks
    only on projected columns, files read whole (no byte-range splitting), and the JSON glob
    merge rules (`Typed` + `arrow.json` and `Typed(a)` + `Typed(b)` with no common type both →
    `arrow.json`, never `DS3005`). Amend D-044, which says globs with differing schemas are read
    file by file and combined with `UNION ALL`: from M2-33 that holds only for CSV and TSV, and
    JSON globs are read by one table with the merged schema.
  - Update [M5-01](M5-01-wasm-build.md)'s carried-over bullet from M2-13 to remove `is_json_array`
    once it reads through `ObjectStore` (keeping "Test JSON, NDJSON and globs on wasm32").
  - Add a carried-over line to [M2-03](M2-03-dynamic-type.md): `(from M2-33) the JSON reader now
    produces Utf8 + arrow.json for nested and mixed columns, never Struct/List (Parquet still
    produces Struct/List, so mapping those stays in scope); nested values are stored as written
    minus whitespace, so dynamic equality, grouping and duplicate-key handling must work on parsed
    values (see M2-33's DECISIONS.md entry); the glob merge checks for arrow.json (Merged::Json)
    before from_arrow_source, so keep that order when from_arrow_source starts mapping arrow.json
    to dynamic`.
  - Add a carried-over line to [M2-12](M2-12-runtime-source-errors.md): `(from M2-33) a JSON
    glob is read by one table, not one ListingTable per file; name the failing JSON file from
    the path in M2-33's structured reader error instead`.
  - Add a carried-over line to [M2-09](M2-09-schema-overrides.md): `(from M2-33) the JSON reader
    decodes into any target schema by key name; pass the declared schema to it for read_json
    rather than casting after the read, and use the M2-33 type-check table for values that do not
    match a declared type`.

## Acceptance criteria
- [ ] Golden `check` and `run` cases covering:
      - NDJSON with a polymorphic nested field (string in one row, object in another),
      - NDJSON with polymorphic top-level fields (string vs. object—projecting the column to
        verify strings are stored quoted as raw JSON text—scalar vs. array, int vs. float, and
        mixed scalars like int vs. string → `arrow.json`),
      - JSON array (`[{...}, ...]`) with nested and heterogeneous fields (including a minified
        single-line array and a pretty-printed multi-line array, whose nested values come out
        without whitespace),
      - glob merging: `Null` + `arrow.json`, `string` + `arrow.json` and `long` + `string`
        all merge to `arrow.json` (with the `string` file's values quoted in the output and the
        `long` file's numbers unquoted), and `long` + `real` merges to `real`,
      - integers outside `i64` (e.g. `18446744073709551615`) inferred as `arrow.json` with
        their exact digits,
      - a `*.json` glob mixing an NDJSON file and a JSON-array file (each read in its own
        layout), and BOM-prefixed NDJSON and JSON-array files (the array auto-detected as an
        array),
      - a type mismatch past `infer_rows` in a column the query does not project is not an
        error,
      - a column that is all `null` in the sample and holds an object past `infer_rows`
        (stored as `arrow.json`),
      - a runtime type mismatch past `infer_rows` reported as `DS3008` (not `DS4002`) with the
        `read_json(..., infer_rows = n)` help,
      - compressed `.ndjson.gz` (gated by `#[cfg(feature = "compression")]`) and the existing
        JSON tests listed above,
      - `read_json(..., infer_rows = n)` happy path and invalid option values (`DS2041`).
- [ ] Unit tests for the custom JSON schema inference and streaming decoder (empty files, blank
      lines, a leading BOM, integers outside `i64` and `1e400` → `arrow.json`,
      `1.0`/`1e3` → `Float64`, top-level
      duplicate keys last-wins, nested duplicate keys
      kept, whitespace removal outside strings only, missing/extra keys, keys first seen past
      `infer_rows` dropped, every row of the type-check table, decoding into a wider target
      schema than the file's own, multi-batch streaming, streaming JSON array scanner,
      projection pushdown skipping unprojected columns, and a file larger than
      `repartition_file_min_size` (or with that threshold lowered) read as one range, with
      correct line numbers in errors past the first range's size).
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`,
      `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`,
      and `cargo check -p datascript-datafusion --no-default-features --features wasm --target wasm32-unknown-unknown`
      pass.

## Follow-ups

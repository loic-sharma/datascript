# M2-34 — Accurate `DS3008` diagnostics for JSON/NDJSON errors

**Status:** todo
**Depends on:** M2-33
**Crates:** `datascript-compiler`, `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** none new expected (reuses `DS3008`); DS3075–DS3079 if needed (D-018)

## Goal
Give clear, actionable `DS3008` diagnostics when reading a JSON or NDJSON file fails during schema
inference or execution, instead of blaming file format syntax for every failure and printing raw
`Arrow error: Json error: ...` strings.

Before [M2-33](M2-33-json-reader-arrow-json.md), any non-IO schema-inference failure was mapped to
`ReadProblem::Malformed` (`crates/datascript-datafusion/src/sources.rs`) and rendered by
`Binder::unreadable` (`crates/datascript-compiler/src/binder/sources.rs`) with the
format-dependent label ``this file is not valid {format}`` (for example `this file is not valid
NDJSON`, or `this file is not valid JSON` as in `tests/cases/sources/unreadable.dstest`), even
when the file was valid NDJSON with a type conflict across rows, and runtime errors emitted raw
Arrow/serde text without naming the column, location, or fix.

M2-33 keeps its reader's runtime errors on `DS3008` with a minimal match in
`crates/datascript-datafusion/src/errors.rs` (generic labels, a `read_json(..., infer_rows = n)`
help, and the error's own text in the note). This task replaces that interim mapping with the
structured diagnostics below.

## Scope
- **Structured classification in `datascript-datafusion` and `datascript-compiler`:**
  - Map the structured JSON reader error from [M2-33](M2-33-json-reader-arrow-json.md) in both
    schema inference (`crates/datascript-datafusion/src/sources.rs` → `ReadProblem` in
    `crates/datascript-compiler/src/source.rs` → `Binder::unreadable` in
    `crates/datascript-compiler/src/binder/sources.rs`) and runtime execution
    (`crates/datascript-datafusion/src/errors.rs`), following [DIAGNOSTICS.md](../DIAGNOSTICS.md).
  - **Location reporting:** report the 1-based **line number** for NDJSON files, and the 1-based
    **record index** (along with the 1-based line number where the record starts) for JSON-array
    files, so single-line minified JSON arrays still pinpoint the offending record.
  - Distinguish the three failure modes clearly:
    1. **Malformed JSON syntax** (inference or runtime): report the location in plain words
       (e.g. `"line 42 is not valid JSON"` for NDJSON, or `"record 3 (line 1) is not valid JSON"`
       for JSON arrays), with the concise parser reason in the note (no `Arrow error: Json error:`
       wrapper noise).
    2. **Non-object top-level record** (inference or runtime):
       - When the file's first non-whitespace character (after any UTF-8 BOM) is `[` while
         reading as NDJSON (e.g. a `.jsonl`/`.ndjson` file or
         `read_json(..., format = "ndjson")`), explain that the file holds a JSON array rather
         than one object per line and add a `help` suggesting `read_json(..., format = "array")`.
       - Otherwise report which line/record is not a JSON object and what JSON kind was found
         (e.g. a string, number, boolean, or array).
    3. **Runtime type conflict past `infer_rows`:** when a top-level column inferred as a scalar
       type in the first `infer_rows` rows encounters an incompatible value later (as defined by
       the type-check table in [M2-33](M2-33-json-reader-arrow-json.md), e.g. a plain `Utf8`
       column seeing an object, array, number, or boolean, or an `Int64` column seeing `1.5` or
       an integer outside `i64`):
       - Primary label: `"a value in this source does not match its column's type"` (matching
         CSV's `tests/cases/errors/runtime/ds3008_malformed_row.dstest`).
       - Note naming the column, location (line number, plus record index for JSON arrays),
         inferred type, and found JSON value/kind.
       - `help`: suggest `read_json(..., infer_rows = n)` (tailored to `read_json`, not
         `read_csv`).
- **Keep single-source vs. multi-source runtime behavior aligned** with
  `crates/datascript-datafusion/src/errors.rs` (multi-source file attribution itself belongs to
  [M2-12](M2-12-runtime-source-errors.md)).
- **Update existing golden tests** whose error wording improves (such as
  `tests/cases/sources/unreadable.dstest` for `broken.json`).

## Acceptance criteria
- [ ] Golden `check` and `run` `.dstest` cases covering:
      - malformed JSON syntax during schema inference and past `infer_rows` at runtime (reports
        the line number for NDJSON and record index + line number for JSON arrays),
      - a JSON array file read as NDJSON (explains why and suggests
        `read_json(..., format = "array")`),
      - a bare scalar or array line mid-file in NDJSON and a non-object element in a minified
        single-line JSON array (reports location and kind found),
      - a top-level column whose type changes past `infer_rows` at runtime (including a plain
        `Utf8` column seeing an object; names the column, location, inferred vs. found type, and
        suggests `read_json(..., infer_rows = n)`).
- [ ] No raw `Arrow error: Json error:` or `Scalar({Utf8})` strings appear in user-facing
      JSON/NDJSON diagnostics.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, and
      `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`
      pass.

## Follow-ups

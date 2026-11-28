# M2-34 — Accurate `DS3008` diagnostics for JSON/NDJSON errors

**Status:** todo
**Depends on:** M2-33
**Crates:** `datascript-compiler`, `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** none new expected (reuses `DS3008`). If a new code is needed, reserve a block in [DIAGNOSTICS.md](../DIAGNOSTICS.md) first (D-018).

## Goal
Give clear, actionable `DS3008` diagnostics when reading a JSON or NDJSON file fails during schema
inference or execution, instead of blaming NDJSON syntax for every failure and printing raw
`Arrow error: Json error: ...` strings.

Before [M2-33](M2-33-json-reader-arrow-json.md), any non-IO schema-inference failure was mapped to
`ReadProblem::Malformed` (`crates/datascript-datafusion/src/sources.rs`) and rendered as
`"this file is not valid NDJSON"` (`crates/datascript-compiler/src/binder/sources.rs`), even when
the file was valid NDJSON with a type conflict across rows, and runtime errors emitted raw
Arrow/serde text without naming the column, line, or fix.

## Scope
- **Structured classification in `datascript-datafusion` and `datascript-compiler`:**
  - Map the structured JSON reader error from [M2-33](M2-33-json-reader-arrow-json.md) in both
    schema inference (`crates/datascript-datafusion/src/sources.rs` → `ReadProblem` in
    `crates/datascript-compiler/src/source.rs` → `Binder::unreadable` in
    `crates/datascript-compiler/src/binder/sources.rs`) and runtime execution
    (`crates/datascript-datafusion/src/errors.rs`), following [DIAGNOSTICS.md](../DIAGNOSTICS.md).
  - Distinguish the three failure modes clearly:
    1. **Malformed JSON syntax** (inference or runtime): report the line number in plain words
       (e.g. `"line 42 is not valid JSON"`), with the concise parser reason in the note (no
       `Arrow error: Json error:` wrapper noise).
    2. **Non-object top-level record** (inference or runtime):
       - When line 1 starts with `[` while reading as NDJSON (e.g. a `.jsonl`/`.ndjson` file or
         `read_json(..., format = 'ndjson')`), explain that the file holds a JSON array rather
         than one object per line and add a `help` suggesting `read_json(..., format = 'array')`.
       - Otherwise report which line/record is not a JSON object and what JSON kind was found
         (e.g. a string, number, boolean, or array).
    3. **Runtime type conflict past `infer_rows`:** when a top-level column inferred as a scalar
       type in the first `infer_rows` rows encounters an incompatible value later (an object, an
       array, or a value that does not match the inferred scalar type):
       - Primary label: `"a value in this source does not match its column's type"` (matching
         CSV's `tests/cases/errors/runtime/ds3008_malformed_row.dstest`).
       - Note naming the column, line/record number, inferred type, and found JSON value/kind.
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
        the line number),
      - a JSON array file read as NDJSON (explains why and suggests
        `read_json(..., format = 'array')`),
      - a bare scalar or array line mid-file in NDJSON (reports line number and kind found),
      - a top-level column whose type changes past `infer_rows` at runtime (names the column,
        line number, inferred vs. found type, and suggests `read_json(..., infer_rows = n)`).
- [ ] No raw `Arrow error: Json error:` or `Scalar({Utf8})` strings appear in user-facing
      JSON/NDJSON diagnostics.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, and
      `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`
      pass.

## Follow-ups

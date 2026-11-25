# M2-22 — CSV sniffing

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-datafusion`
**Diagnostic codes:** DS3070–DS3074 (D-018)

## Goal
Read a CSV file correctly without reader options when its dialect is not the default, as DuckDB's
sniffer does. Today `.csv` means comma-separated with a header row, and anything else needs
`read_csv(…, delimiter = ';', header = false)`.

Keep it small: DataFusion has no sniffer, but it already reads any dialect it is told about and
infers column types itself (`CsvFormat` and `infer_schema` in
`crates/datascript-datafusion/src/sources.rs`). The sniffer only chooses the `CsvReader` settings
before that code runs. It does not parse rows or infer types.

## Scope
- **What is detected**, from a sample of the first `infer_rows` lines:
  - the delimiter, from `,` `\t` `;` `|`: the candidate that splits the most sample rows into the
    same number of fields, with more than one field. Ties go to the first in that order, which is
    documented; there is no warning for a close call.
  - whether the first row is a header: it is when some column's first value is text while the rest
    of that column reads as numbers, dates or booleans. A file of text columns only keeps the
    default (a header row).
  - the quote character, `"` or `'`: `'` only when it wraps fields that contain the delimiter.
- **When it runs:** for `.csv`, `.tsv` and `read_csv(...)`. An option the user writes always wins,
  and detection fills in only the options left out. `.tsv` still defaults to `\t` unless the data
  clearly disagrees (decide, and write it down).
- **One dialect per source.** Each file of a glob is sniffed, and all must agree. If two files
  disagree, that is an error from this task's block naming both files and their delimiters, with
  help to write `read_csv(…, delimiter = …)`. `Resolved` keeps a single `Reader`, so
  `Resolved::uniform()` and the scan code do not change.
- **Reading the sample** goes through the object store, not `std::fs`, so the same code works on
  wasm32 (unlike `is_json_array`, which M5-01 already has to fix). Compressed files (`.csv.gz`, …)
  are decompressed before sniffing.
- **A self-contained module** (`sniff.rs` or similar) with a pure function over bytes, plus a small
  change where `CsvReader` is built. In `datascript-compiler`, only `SourceError` (a new case for
  disagreeing dialects, `crates/datascript-compiler/src/source.rs`), the binder's mapping to this
  task's code and the code registry change. `Engine::schema_of` and `SchemaLookup` do not.
- Update LANGUAGE.md §3.1 (reader options and defaults, the tie order) and record the rules in
  DECISIONS.md.

Not in this task (informational; add a task only if users ask): showing the detected dialect in
`explain` or `.schema`, a warning for an ambiguous delimiter (both need `schema_of` to carry
metadata and warnings, a trait change across every backend), and globs whose files use different
dialects (needs a reader per file).

## Acceptance criteria
- [ ] Golden `run` cases over new fixtures: semicolon-delimited, pipe-delimited, tab-delimited
      with a `.csv` extension, no header row, single-quoted fields, a gzip-compressed
      semicolon-delimited file, and a glob of two semicolon-delimited files.
- [ ] A golden `check` case for a glob whose files use different delimiters (this task's error).
- [ ] Writing `delimiter`, `header` or `quote` turns off detection for that option (golden case).
- [ ] The sniffer has unit tests over byte slices, including the tie order and a single-column
      file, and `datascript-datafusion` still passes its wasm32 check.

## Follow-ups

# M2-22 — CSV sniffing

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-engine`, `datascript-compiler`, `datascript-datafusion`, `datascript-cli`
**Diagnostic codes:** DS3070–DS3074 (D-018)

## Goal
Read a CSV file correctly without reader options when its dialect is not the default, as DuckDB's
sniffer does. Today `.csv` means comma-separated with a header row, and anything else needs
`read_csv(…, delimiter = ';', header = false)`.

## Scope
- Detect, from the first `infer_rows` lines: the delimiter (`,` `;` `\t` `|`), the quote character
  (`"` or `'`) and whether the first row is a header (for example: the first row is all strings and
  some later column is not).
- Runs by default for `.csv`, `.tsv` and `read_csv(...)`. An option the user writes always wins
  over what is detected; detection fills in only the options left out. `.tsv` still defaults to
  `\t` unless the data clearly disagrees (decide, and write it down).
- Each file of a glob is sniffed on its own, before glob schema merging (LANGUAGE.md §3.1). Files
  with different dialects are fine; the merge is by column name as today.
- DataFusion has no sniffer, so this is our own code. It runs on bytes already read for schema
  inference and must build for wasm32 (no `std::fs` in the detection logic).
- Show what was detected: `explain` and the REPL's `.schema` name the dialect used.
- **Carrying the result back.** `Engine::schema_of` (`crates/datascript-engine/src/lib.rs`)
  returns only `Result<SchemaRef, SourceError>`, and `SchemaLookup::Ready`
  (`crates/datascript-compiler/src/source.rs`) carries only a `SchemaRef`. Extend both to carry
  source metadata (the detected dialect) and warnings, so `check` can report the ambiguity warning
  at the source's span and `explain` and the REPL can print the dialect.
- **Per-file readers.** `Resolved` in `crates/datascript-datafusion/src/sources.rs` stores one
  `Reader` for a whole glob. Mixed-dialect globs need a `CsvReader` per file, and
  `Resolved::uniform()` must also compare dialects.
- **Compression.** Sniff compressed files (`.csv.gz`, …) after decompressing the bytes read for
  inference.
- Update LANGUAGE.md §3.1 (reader options and defaults) and record the rules in DECISIONS.md.

## Acceptance criteria
- [ ] Golden `run` cases over new fixtures: semicolon-delimited, pipe-delimited, tab-delimited
      with a `.csv` extension, no header row, single-quoted fields, and a glob mixing two dialects.
- [ ] A file where two delimiters fit equally well gives a warning (from this task's block) that
      names the chosen delimiter and the `read_csv(…, delimiter = …)` that would override it.
- [ ] Writing `delimiter`, `header` or `quote` turns off detection for that option (golden case).
- [ ] The core of the sniffer has unit tests over byte slices, and `datascript-datafusion` still
      passes its wasm32 check.

## Follow-ups

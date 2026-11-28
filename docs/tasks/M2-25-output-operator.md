# M2-25 — `output` operator

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-engine`, `datascript-datafusion`, `datascript-cli`, `datascript-lsp`, `datascript-tests`, `datascript-diagnostics`
**Diagnostic codes:** DS0295–DS0299 (syntax), DS3090–DS3092 (sources), DS4065–DS4069 (engine) (D-018)

## Goal
Write a query's result to a file from the language, inspired by DuckDB's `COPY … TO`. A sink that
mirrors sources: `from 'x.csv'` on the way in, `output 'x.csv'` on the way out, with the same path
rules and formats by extension.

```ds
from 'orders.csv'
| summarize revenue = sum(amount) by customerId
| output 'out/revenue.csv';                          // format from the extension

from 'orders.csv' | output 'out/orders.ndjson';     // .ndjson and .jsonl: newline-delimited
from 'orders.csv' | output 'out/orders.json';       // .json: newline-delimited too, as DuckDB
```

This task writes uncompressed CSV, TSV and newline-delimited JSON with default settings. Parquet,
the check that a query does not overwrite its own input, and the check for reading a file back in
the same program are M2-27's. Writer functions with options
(`output write_csv('x.csv', delimiter = ';')`) and compressed output (`'x.csv.gz'`) are M2-26's.

The statement's result is a one-row summary (as DuckDB's `COPY` returns a row count), printed
like any other result:

```
path               | format  | rows
string             | string  | long
-------------------+---------+------
out/revenue.csv    | csv     |  113
```

KQL has no such operator (its `.export` is a management command, LANGUAGE.md §9), so this is
≠ KQL. Record the design in DECISIONS.md.

## Scope

### Syntax and binding
- `output <target>`, where `<target>` is a path string or a scalar `let` bound to one. The parser
  accepts any expression, as after `from` (D-029); the binder rejects the rest. A value that is not
  a string (`output 5`) reuses DS2040, and a non-constant path reuses DS3006; reword both codes'
  summaries in the DIAGNOSTICS.md registry and `codes.rs` to cover output targets. M2-26 adds
  writer calls as targets.
- `output` must be the last operator, and only in a query statement, each with an error from this
  task's syntax block. A query cannot have both `output` and `render`, and no separate error is
  needed: `… | render … | output …` is DS0120 (`render` is not last), and `… | output … | render …`
  is this task's "`output` is not last" error.
- Paths follow the source rules (LANGUAGE.md §3.1): resolved against the same base directory per
  host, constant (DS3006), format from the extension (`.csv`, `.tsv`, `.json`, `.ndjson`, `.jsonl`),
  unknown extension DS3002. Until M2-27 and M2-26 land, `.parquet` and a compression suffix
  (`.gz`, `.bz2`, `.xz`, `.zst`) are DS3002 with help saying they are planned.
- Globs in an output path are an error, and so is a URL (`output 'https://…'`, `s3://…`), which
  would otherwise be written as a local path like `./https:/…`. Writing to remote stores is not
  planned (M7-01, M7-02 read only).
- The statement's schema is the summary table above. Its column names (`path`, `format`, `rows`)
  are lowercase, not KQL's PascalCase, following the same naming exception as `describe` (M2-23);
  whichever task lands first writes the DECISIONS.md entry and the other extends it.
- `output` is a new operator in `datascript-syntax`, so this task edits the parser (see the
  parallelism note in the task index).
- LSP completion and hover cover `output`. (The TextMate grammar in `editors/vscode` already
  highlights any identifier after `|` as an operator, so it needs no change.) If M2-07 (formatter) is done, format it there too; otherwise M2-07's "Carried over" item covers
  it.

### Writing
- Defaults: CSV with a header row, comma-separated, empty text for null; TSV likewise with tabs;
  newline-delimited JSON.
- Values are written as the CLI's CSV and JSON writers print them today (`datascript_engine::writers`,
  D-043, streaming since D-055), so `ds run --format csv` and `output 'x.csv'` give the same bytes.
- Lower to a sink node in the IR; the DataFusion backend streams batches into the writer (D-055),
  so a large result is never buffered. `Capabilities` gains the formats a backend can write; the
  SQL backend (M6-03) reports none until it can.
- Write to a temporary file in the target directory and rename it when done (DuckDB's
  `USE_TMP_FILE`), so a failed or cancelled query never leaves a half-written file. Create missing
  parent directories (decide, and say so in LANGUAGE.md).
- Single files are overwritten, as DuckDB does, so a script can be re-run. (M2-26 adds an
  `overwrite` option to refuse instead.) Because rows go to a temporary file that replaces the target only at the end, a query
  that reads the file it writes does not corrupt it on Unix; M2-27 turns that case into a clear
  error everywhere.
- I/O failures (permission denied, disk full, a directory in the way) are runtime errors from this
  task's engine block that name the path and point at the `output` operator.
- `check`, `explain`, the language server's background work (diagnostics, completion, hover) and
  REPL completion never write; only running does.
- **Clearing caches.** A written file may be what a cached schema describes, through a path, a
  glob, a reader call or a host table. Rather than matching entries, clear **both** caches entirely
  after any run that wrote a file: the session's schema cache (D-043) and `DataFusionEngine`'s
  `resolved` map. `RunStream` holds no handle to the session's cache today
  (`crates/datascript-engine/src/session.rs`), so give it one (or a callback), and add an `Engine`
  method to clear the backend's cache (default: do nothing). Re-reading schemas on the next query
  is cheap.

### Hosts
- CLI and REPL: write relative to the base directory. `--output` and `--format` stay for printing
  results to standard output or piping; document how they differ from `output`.
- VS Code: the user-triggered Run (`datascript/run`, D-054, D-056, trusted workspaces only) executes
  `output` like the CLI. Background diagnostics, completion and hover never write.
- `ds ui` (M4-01): writes go through D-016's sandbox, so only paths under the root are allowed
  (DS3007). If M4-12 (sandbox hardening) is not done yet, its "Carried over" item covers this
  instead.
- WASM (M5-02): writes go into the registered-file store and the JS API reports them so the UI can
  offer a download. If M5-02 is not done yet, its "Carried over" item covers this instead.
- Python and C API (M7-03, M7-04): no special handling; the summary row comes back as a result.

### Tests and docs
- Golden tests must read a written file back, which cannot happen in the same program
  (`Session::run` checks every statement before running the first; see M2-27). Add a harness
  directive that reads a written file after the run (or runs a second program in the same test),
  and document it in TESTING.md. M2-26, M2-27 and M2-28 reuse it.
- LANGUAGE.md: a new section after §3.1 for sinks, `output` in §3.2, the grammar sketch in §8.
  README: an `output` example in "Getting started".

## Acceptance criteria
- [ ] Golden `run` cases for every example in the Goal, each reading the written file back and
      comparing it with the query result.
- [ ] Golden `check` cases for every new error: `output` not last (including before `render`),
      `output` outside a query statement, unknown extension, `.parquet` before M2-27,
      non-constant path, glob path.
- [ ] A test that a cancelled or failed query leaves no file and no temporary file behind.
- [ ] A test that `check` and the LSP never create the file.
- [ ] `ds run --format csv` and `output 'x.csv'` produce identical bytes for the fixtures.

## Follow-ups

# M2-25 — `output` operator

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-engine`, `datascript-datafusion`, `datascript-cli`, `datascript-tests`
**Diagnostic codes:** DS0295–DS0299 (syntax), DS3090–DS3094 (sources), DS4065–DS4069 (engine) (D-018)

## Goal
Write a query's result to a file from the language, inspired by DuckDB's `COPY … TO`. A sink that
mirrors sources: `from 'x.csv'` on the way in, `output 'x.csv'` on the way out, with the same path
rules, formats by extension and compression suffixes.

```ds
from 'orders.csv'
| summarize revenue = sum(amount) by customerId
| output 'out/revenue.csv';                          // format from the extension

from 'logs/*.json' | where level == "error" | output 'out/errors.parquet';
from 'orders.csv' | output 'out/orders.ndjson';     // .ndjson and .jsonl: newline-delimited
from 'orders.csv' | output 'out/orders.json';       // .json: newline-delimited too, as DuckDB
from 'orders.csv' | output 'out/orders.csv.gz';     // compression from the second suffix
```

This task writes with default settings only. Writer functions with options
(`output write_csv('x.csv', delimiter = ';')`) are M2-26's.

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
  accepts any expression, as after `from` (D-029); the binder rejects the rest. M2-26 adds writer
  calls as targets.
- `output` must be the last operator (like `render`, DS0120), only in a query statement, and a
  query cannot have both `output` and `render`. Each gets an error from this task's blocks.
- Paths follow the source rules (LANGUAGE.md §3.1): resolved against the same base directory per
  host, constant (DS3006), format from the extension (`.csv`, `.tsv`, `.parquet`, `.json`,
  `.ndjson`, `.jsonl`, plus `.gz`/`.bz2`/`.xz`/`.zst` for text formats), unknown extension DS3002.
  Globs in an output path are an error.
- The statement's schema is the summary table above. Its column names (`path`, `format`, `rows`)
  are lowercase, not KQL's PascalCase, following the same naming exception as `describe` (M2-23);
  whichever task lands first writes the DECISIONS.md entry and the other extends it.
- `output` is a new operator in `datascript-syntax`, so this task edits the parser (see the
  parallelism note in the task index).
- LSP completion and hover cover `output`, and the TextMate grammar in `editors/vscode` learns it.
  If M2-07 (formatter) is done, format it there too; otherwise M2-07's "Carried over" item covers
  it.

### Writing
- Defaults: CSV with a header row, comma-separated, empty text for null; Parquet with a default
  compression (decide: snappy as DuckDB, or zstd, and write it down); newline-delimited JSON.
- Values are written as the CLI's CSV and JSON writers print them today (`datascript_engine::writers`,
  D-043, streaming since D-055), so `ds run --format csv` and `output 'x.csv'` give the same bytes.
  Extension-type metadata (D-024) is kept in the Parquet schema. Parquet is a new writer.
- Lower to a sink node in the IR; the DataFusion backend streams batches into the writer (D-055),
  so a large result is never buffered. `Capabilities` gains the formats a backend can write; the
  SQL backend (M6-03) reports none until it can.
- Write to a temporary file in the target directory and rename it when done (DuckDB's
  `USE_TMP_FILE`), so a failed or cancelled query never leaves a half-written file. Create missing
  parent directories (decide, and say so in LANGUAGE.md).
- Single files are overwritten, as DuckDB does, so a script can be re-run. (M2-26 adds a write-mode
  option.)
- Writing to a file the same query reads (`from 'x.csv' | … | output 'x.csv'`) is an error before
  anything is written. The check runs in the **engine**, which compares the canonicalized paths of
  every resolved input (globs expanded, symlinks resolved, host tables included) with the
  canonicalized target before opening it; `datascript-compiler` cannot touch the filesystem.
- I/O failures (permission denied, disk full, a directory in the way) are runtime errors from this
  task's engine block that name the path and point at the `output` operator.
- `check`, `explain`, the language server's background work (diagnostics, completion, hover) and
  REPL completion never write; only running does.
- **Reading a file written earlier in the same program.** `Session::run` checks every statement
  before running the first, so in

  ```ds
  from 'orders.csv' | output 'out.parquet';
  from 'out.parquet' | count
  ```

  the second statement fails with DS3001 if `out.parquet` does not exist yet, or binds against a
  stale cached schema if it does. This task does not support reading back within one program.
  Instead: DS3001 for a path that an earlier `output` in the same program writes gets help saying
  so and suggesting a separate run; the session invalidates its schema cache for a path after
  writing it, so the next REPL statement or run sees the new file. Record the decision in
  DECISIONS.md.

### Hosts
- CLI and REPL: write relative to the base directory. `--output` and `--format` stay for printing
  results to standard output or piping; document how they differ from `output`.
- VS Code: the user-triggered Run (`datascript/run`, D-054, D-056, trusted workspaces only) executes
  `output` like the CLI. Background diagnostics, completion and hover never write.
- `ds ui` (M4-01): writes go through D-016's sandbox, so only paths under the root are allowed
  (DS3007). If M4-01 is not done yet, its "Carried over" item covers this instead.
- WASM (M5-02): writes go into the registered-file store and the JS API reports them so the UI can
  offer a download. If M5-02 is not done yet, its "Carried over" item covers this instead.
- Python and C API (M7-03, M7-04): no special handling; the summary row comes back as a result.

### Tests and docs
- Golden tests must read a written file back, which cannot happen in the same program (see
  above). Add a harness directive that reads a written file after the run (or runs a second
  program in the same test), and document it in TESTING.md.
- LANGUAGE.md: a new section after §3.1 for sinks, `output` in §3.2, the grammar sketch in §8.
  README: an `output` example in "Getting started".

## Acceptance criteria
- [ ] Golden `run` cases for every example in the Goal, each reading the written file back and
      comparing it with the query result.
- [ ] Golden `check` cases for every new error: `output` not last, `output` with `render`,
      unknown extension, non-constant path, glob path.
- [ ] A golden `run` case for writing to a file the query reads (engine check), and one for the
      DS3001 help when a later statement reads a file an earlier `output` writes.
- [ ] A test that a cancelled or failed query leaves no file and no temporary file behind.
- [ ] A test that `check` and the LSP never create the file.
- [ ] `ds run --format csv` and `output 'x.csv'` produce identical bytes for the fixtures.

## Follow-ups

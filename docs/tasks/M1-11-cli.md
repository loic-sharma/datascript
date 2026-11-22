# M1-11 — CLI: `ds check`, `ds run`, `ds explain`

**Status:** done — `ds check|run|explain` with every format, `-e`/stdin, `--table`, `--statement`, `--max-rows`, `--color`, Ctrl-C (exit 130) and `assert_cmd` tests (D-046).
**Depends on:** M1-09

Extends the skeleton from M1-15 rather than starting from nothing.
**Crates:** `datascript-cli`

## Goal
The command-line compiler and runner.

## Scope
- `ds check <files...>`: diagnostics only; `--format human|json`; exit code 0 (clean),
  1 (errors), 2 (usage/internal error). Warnings don't fail unless `--deny-warnings`.
- `ds run <file> | -e '<query>' | -` (stdin): `--format table|csv|json|ndjson|arrow`, `--output <path>`,
  `--table name=path` (repeatable), `--max-rows` for table output (default 100 with a "… N more rows" footer).
- `ds explain <file>`: prints the IR.
- `--color auto|always|never`, respecting `NO_COLOR`. Terminal diagnostics renderer when on a TTY.
- Multiple query statements print multiple tables with a header naming the statement's line.
- Friendly handling of a nonexistent script path, an empty program, and Ctrl-C during execution.

Design-review additions — several query statements in machine formats:
- `table` prints every result (as above). `json` prints an array with one
  `{ "statement": <line>, "columns": [...], "rows": [...] }` per query statement.
- `csv`, `ndjson` and `arrow` need exactly one result. With more, exit 2 with a message suggesting
  `--statement <n>` (1-based index of the query statement), which runs only that statement and the
  lets it needs.

## Acceptance criteria
- [x] Integration tests with `assert_cmd` for exit codes, formats, and stdin.
- [ ] `ds run examples/top_customers.ds` works. *Blocked on `render` (M1-10): the CLI side is done and the test is written but ignored; see Follow-ups.*
- [x] `ds --help` and each subcommand's help have examples.

## Carried over
- [x] (from M1-15) The CLI accepts more than one file, `-e`, stdin, `--format`, `--color` and `--table` (the skeleton takes one file per command).
- [x] (from M1-07) Use `datascript_engine::writers` (`write_csv`, `write_ndjson`, `json_rows` for the `{statement, columns, rows}` objects) for `--format csv|ndjson|json`, and run through `Session`, whose methods already turn panics into DS9001.
- [x] (from M1-08) `--table name=path`: check the path's format with `datascript_compiler::source::format_of_path` (a diagnostic, not a bare string, when it has none), make the path absolute against the current directory (the engine resolves relative host-table paths against the script's directory), and register it with `DataFusionEngine::with_table(name, SourceRef::for_path(path))` (D-044).

## Follow-ups
- `examples/top_customers.ds` ends in `render timechart`, which still fails with DS9001. Handed to
  M1-10 (un-ignore `run_the_readme_example` and print DS5001 from the CLI).
- Results are collected before printing, not streamed. Handed to M1-13.
- Informational: `QueryPlan` gained `span` (the whole statement) and `QueryResult` gained
  `statement`; `Session::run_statement` runs one query of a checked program. `check` accepts `-`
  for stdin but not `-e`. `--table` also works with `check` and `explain`.

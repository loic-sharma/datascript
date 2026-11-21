# M1-11 — CLI: `ds check`, `ds run`, `ds explain`

**Status:** todo
**Depends on:** M1-09
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

## Acceptance criteria
- [ ] Integration tests with `assert_cmd` for exit codes, formats, and stdin.
- [ ] `ds run examples/top_customers.ds` works.
- [ ] `ds --help` and each subcommand's help have examples.

## Follow-ups

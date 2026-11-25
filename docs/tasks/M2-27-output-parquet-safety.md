# M2-27 — `output`: Parquet, self-overwrite check and read-back help

**Status:** todo
**Depends on:** M2-25
**Crates:** `datascript-compiler`, `datascript-engine`, `datascript-datafusion`
**Diagnostic codes:** DS3093–DS3099 (D-018)

## Goal
Finish the `output` operator M2-25 starts:

```ds
from 'logs/*.json' | where level == "error" | output 'out/errors.parquet';
```

plus two safety rails: a query that would overwrite its own input is an error, and reading a file
that an earlier statement of the same program writes gets a helpful message instead of a bare
"file not found".

## Scope

### Parquet
- `.parquet` as an output extension, with a new streaming Parquet writer in
  `datascript_engine::writers` (D-043, D-055). Pick a default compression (snappy as DuckDB, or
  zstd) and write it down in LANGUAGE.md. Keep extension-type metadata (D-024) in the Parquet
  schema, so a file written and read back has the same DataScript types.
- Remove M2-25's "planned" help for `.parquet`.

### Writing to a file the query reads
- `from 'x.csv' | … | output 'x.csv'` is an error before anything is written.
- The check runs in the **DataFusion backend** (`datascript-datafusion`), where the resolved files
  are (`sources::Resolved`): it compares the canonicalized path of every input file (globs
  expanded, symlinks resolved, file-backed host tables included) with the canonicalized target
  before opening it. The `Engine` trait does not change; `datascript-compiler` cannot touch the
  filesystem. The error names the path and points at both the `output` and the source that reads
  it.

### Reading a file written earlier in the same program
`Session::run` checks every statement before running the first, so in

```ds
from 'orders.csv' | output 'out.parquet';
from 'out.parquet' | count
```

the second statement fails with DS3001 if `out.parquet` does not exist yet, or binds against a
stale cached schema if it does. Reading back within one program is not supported. Instead, DS3001
for a path that an earlier `output` in the same program writes gets help saying so and suggesting
a separate run. Record the decision in DECISIONS.md.

## Acceptance criteria
- [ ] Golden `run` cases writing Parquet from CSV and JSON fixtures, reading each back with the
      M2-25 harness directive and comparing types and values.
- [ ] Golden `run` cases for the self-overwrite error: a plain path, a glob that matches the
      target, and a symlink to the target.
- [ ] A golden case for the DS3001 help when a later statement reads a file an earlier `output`
      writes.

## Follow-ups

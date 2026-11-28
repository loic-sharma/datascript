# M2-27 — `output`: Parquet, self-overwrite check and read-back check

**Status:** todo
**Depends on:** M2-25
**Crates:** `datascript-compiler`, `datascript-engine`, `datascript-datafusion`
**Diagnostic codes:** DS3093–DS3099 (D-018)

## Goal
Finish the `output` operator M2-25 starts:

```ds
from 'logs/*.json' | where level == "error" | output 'out/errors.parquet';
```

plus two safety rails: a query that would overwrite its own input is an error, and so is reading
a file that an earlier statement of the same program writes, which would otherwise read a missing
or stale file.

## Scope

### Parquet
- `.parquet` as an output extension, with a new streaming Parquet writer in
  `datascript_engine::writers` (D-043, D-055). The default compression is **snappy**, as in DuckDB:
  it is pure Rust, so the wasm32 build needs no C code (zstd would pull in `zstd-sys`, which needs a
  wasm-capable clang, M0-05). Other codecs and their cargo feature are M2-26's. Write the default
  down in LANGUAGE.md. Keep extension-type metadata (D-024) in the Parquet
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

the second statement fails with DS3001 if `out.parquet` does not exist yet. Worse, on a re-run it
does exist: the second statement binds against the old file's schema and reads data from before
the first statement replaced it, with no error at all.

So reading back within one program is an **error, always**, found by the binder before anything
runs: a source whose constant path matches an earlier `output` target in the same program. The
binder cannot touch the filesystem, so it compares path text only:
- exact matches after normalizing `./`, `..` and separators, both resolved against the same base
  directory;
- a glob source whose pattern matches the target path (`from 'out/*.parquet'` after
  `output 'out/x.parquet'`), using the same glob matcher as source resolution, on text.

The error points at both the source and the `output`, and its help suggests a separate run.
Symlinks, host tables and other aliases are not detected. Write those limits down in
LANGUAGE.md; such a case still reads correct data on the next run, because M2-25 clears every
cache after a run that writes. Record the decision in DECISIONS.md.

### Hosts
- If M4-14 is done, its export (D-066) offers Parquet through the same writer. (M4-14's "Carried
  over" item covers it if M4-14 lands after this task.)

## Acceptance criteria
- [ ] Golden `run` cases writing Parquet from CSV and JSON fixtures, reading each back with the
      M2-25 harness directive and comparing types and values.
- [ ] Golden `run` cases for the self-overwrite error: a plain path, a glob that matches the
      target, and a symlink to the target.
- [ ] Golden `check` cases for the read-back error: the same path, the same path written
      differently (`./out/x.csv` and `out/x.csv`), a glob that matches the target, and a `let`
      bound to the path. One test runs the program twice to show the second run is also refused.

## Follow-ups

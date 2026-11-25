# M2-27 — `output`: partitioning, file splitting and the remaining DuckDB options

**Status:** todo
**Depends on:** M2-26, M2-23
**Crates:** `datascript-compiler`, `datascript-engine`, `datascript-datafusion`
**Diagnostic codes:** DS3095–DS3099 (D-018)

## Goal
Cover the rest of DuckDB's `COPY … TO` options in the writer catalog M2-26 adds, so DataScript
can write everything DuckDB can. No new syntax: each option is a row in the catalog. Column-list
values such as `partition_by = (year, month)` use the option syntax M2-26 adds for
`force_quote = (col, …)`.

```ds
from orders
| extend year = getyear(timestamp), month = getmonth(timestamp)
| output write_parquet('out/orders/', partition_by = (year, month));
// out/orders/year=2024/month=1/data_0.parquet, …
```

## Scope
- `partition_by = (col, …)`: Hive-style directories (`key=value`), named exactly as M2-23 reads
  them, so a partitioned write reads back with the same columns and types. Partition columns are
  left out of the files, as DuckDB does. The target is a directory; a directory that already has
  files needs the M2-26 overwrite option.
- `filename_pattern` (with `{i}` and `{uuid}`, as DuckDB) and `file_extension`.
- `per_thread_output` and `file_size_bytes` (split output into several files). Output must be
  deterministic for golden tests: file names and contents must not depend on thread scheduling,
  or the tests must say why they are excluded.
- Parquet: `row_group_size_bytes`, `field_ids`, `kv_metadata`, and any newer DuckDB options
  (check DuckDB's current `COPY` documentation when starting, and list what is skipped and why).
- **Summary rows.** For an output that writes several files (`partition_by`, `per_thread_output`,
  `file_size_bytes`), the statement's summary table (`path`, `format`, `rows`, M2-26) has one row
  per file written, sorted by path, so users see where the rows went.
- Directory writes are atomic per file, as in M2-26; a failure part-way through says which files
  were written.
- Update LANGUAGE.md's sink section and the option table.

## Acceptance criteria
- [ ] Golden `run` cases for each option, reading the output back (partitioned output through
      M2-23's reader).
- [ ] An overwrite test for directory outputs.
- [ ] A table in LANGUAGE.md mapping every DuckDB `COPY` option to its DataScript name, or saying
      why it is not supported.

## Follow-ups

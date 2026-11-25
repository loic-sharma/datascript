# M2-27 — `output`: partitioned output

**Status:** todo
**Depends on:** M2-26
**Crates:** `datascript-compiler`, `datascript-engine`, `datascript-datafusion`
**Diagnostic codes:** DS3095–DS3099 (D-018)

## Goal
Split an `output` into Hive-style directories, and name the files written, as DuckDB's
`PARTITION_BY` and `FILENAME_PATTERN` do:

```ds
from orders
| extend year = getyear(timestamp), month = getmonth(timestamp)
| output write_parquet('out/orders/', partition_by = (year, month));
// out/orders/year=2024/month=1/data_0.parquet, …
```

No new syntax: both are rows in M2-26's writer catalog, and `partition_by` uses the column-list
option syntax M2-26 adds for `force_quote`.

## Scope
- `partition_by = (col, …)` on every writer: Hive-style directories (`key=value`), values written
  so other tools (DuckDB, Spark, Polars) read them back as partitions. Partition columns are left
  out of the files, as DuckDB does. The target is a directory; a directory that already has files
  needs M2-26's overwrite option. DataScript itself does not read partitions back as columns yet
  (Q-006 is open), so say so in LANGUAGE.md.
- `filename_pattern` (with `{i}` and `{uuid}`, as DuckDB). File names must be deterministic for
  golden tests unless `{uuid}` is used.
- **Summary rows.** The statement's summary table (`path`, `format`, `rows`, M2-25) has one row per
  file written, sorted by path, so users see where the rows went.
- Directory writes are atomic per file, as in M2-25; a failure part-way through says which files
  were written.
- **A support table instead of "every option".** Add a table to LANGUAGE.md's sink section listing
  every DuckDB `COPY … TO` option (check DuckDB's current documentation) with its DataScript name,
  or "not supported" and why. Options not supported today, such as `per_thread_output`,
  `file_size_bytes` (file layouts that depend on scheduling), `file_extension`, and Parquet's
  `row_group_size_bytes`, `field_ids` and `kv_metadata`, are added only when someone asks for them.

## Acceptance criteria
- [ ] Golden `run` cases: one and two partition columns, a partition column with nulls, and
      `filename_pattern`, checking the files and directories written and reading each file back.
- [ ] An overwrite test for directory outputs.
- [ ] The DuckDB option table in LANGUAGE.md.

## Follow-ups

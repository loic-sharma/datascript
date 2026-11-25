# M2-23 — `describe` operator

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`, `datascript-cli`
**Diagnostic codes:** DS0290–DS0294 (syntax) (D-018). No type errors are expected: arguments are a
syntax error, and `describe` after `render` is DS0120.

## Goal
One operator that summarizes every column of its input with statistics, like pandas' and Polars'
`describe()` and DuckDB's `SUMMARIZE`: `from 'orders.parquet' | describe`. KQL has no equivalent,
so this is ≠ KQL; record it in DECISIONS.md.

`describe` means two things in other tools. In pandas, Polars and Spark DataFrames it gives
statistics; in SQL (DuckDB, MySQL, Spark SQL, Snowflake) `DESCRIBE` gives column names and types
from metadata alone, which DataScript calls `getschema`. This operator follows pandas. Its output
includes the column names and types, so a SQL user still sees what they expect, but it reads every
row. The docs and hover must say so (see Scope).

## Scope
- Output: one row per input column, in input order, with these columns (snake_case, names follow
  DuckDB's `SUMMARIZE` where one exists):

  | Column | Type | Meaning |
  |---|---|---|
  | `column_name` | `string` | |
  | `column_type` | `string` | as `getschema`'s `DataType` |
  | `count` | `long` | non-null values |
  | `null_percentage` | `real` | 0–100 |
  | `approx_unique` | `long` | approximate distinct count (HyperLogLog), so it is cheap on large inputs. Order-independent, so results are deterministic |
  | `min`, `max` | see below | |
  | `avg`, `std` | `real` | numbers only (`avg` of a timespan or datetime: decide, as for `avg`) |
  | `q25`, `q50`, `q75` | see below | approximate (t-digest, DataFusion's `approx_percentile_cont`), as DuckDB's `SUMMARIZE` |

  Cells that make no sense for a column's type (`avg` of a string) are null. On an empty input
  (0 rows), `null_percentage` is null, not `NaN`, and `count` and `approx_unique` are 0.
- **Why approximate quantiles.** `percentile` is exact and keeps every value in memory
  (`PercentileAccumulator` in `crates/datascript-datafusion/src/udaf.rs`). Three exact percentiles
  per column would hold roughly every numeric and datetime value of the input, on exactly the large
  unfamiliar files people run `describe` on. t-digest uses bounded memory. Its results can vary
  slightly with how input is partitioned: golden tests either run with one partition or use inputs
  small enough that t-digest is exact; say which in TESTING.md. Datetime quantiles go through
  their integer value.
- DataFusion's `approx_distinct` does not accept every Arrow type (check `Float64`, `Boolean`,
  `Timestamp` and `Duration`); cast such columns to a type it accepts, or compute the count
  another way, and test every DataScript type.
- `min`, `max` and `q25`…`q75` hold values of different types in different rows. Decide how:
  `string` columns holding the formatted value (simple, what DuckDB does), or `dynamic` once M2-03
  lands. Record the choice.
- **Naming.** Every other generated column name in DataScript is PascalCase, following KQL
  (`getschema`'s `ColumnName`, `count`'s `Count`). These are snake_case on purpose, to match
  DuckDB and pandas users' expectations. Record the exception in DECISIONS.md, and add a note to
  LANGUAGE.md so later operators know which convention to follow. `output`'s summary columns
  (M2-25) follow the same exception; whichever task lands first writes the entry and the other
  extends it.
- **Cost.** Lower it as one aggregate pass where possible (an unpivot of per-column aggregates),
  not one query per column. LANGUAGE.md, hover and completion docs say that `describe` reads every
  row and that `getschema` gives column names and types without reading data.
- Columns with no DataScript type (DS2008) appear with only `count` and `null_percentage`.
- Takes no arguments in this task. `describe a, b` (a column subset) is a follow-up if wanted.
- REPL: `.describe <source>` runs `from <source> | describe`. (The REPL's `.schema` stays the
  cheap one.)
- `describe` is a new operator in `datascript-syntax` (`OPERATORS`, AST, grammar), so this task
  edits the parser (see the parallelism note in the task index).
- Add it to LANGUAGE.md §3.2, LSP completion and hover, and the TextMate grammar in
  `editors/vscode`. If M2-07 (formatter) is done, format `describe` there too; otherwise M2-07's
  "Carried over" item covers it.

## Acceptance criteria
- [ ] Golden `run` cases over `orders.csv`, a Parquet file with every DataScript type, an empty
      table and a table with an all-null column.
- [ ] Error cases: arguments given (`describe x`) and `describe` after `render`.
- [ ] A test that `approx_unique` gives the same value across runs and `target_partitions`
      settings, and how close it is to `dcount` on the fixtures (whether it is exact on small
      inputs depends on the implementation; check, and document what golden cases can rely on).
- [ ] A REPL test for `.describe`.

## Carried over
- [ ] (from planning, 2026-11-24) If M4-04 is done, add a "Describe" action to the web UI's schema
      browser that runs `from <file> | describe`. If M4-04 is not done, its "Carried over" item
      covers this instead.

## Follow-ups

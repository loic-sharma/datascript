# M2-23 — `describe` operator

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`, `datascript-cli`, `datascript-lsp`, `datascript-tests`
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
  | `avg`, `std` | `real` | numbers only (`avg` of a timespan or datetime: decide, as for `avg`). `std` is the sample standard deviation, as `stdev`; null for fewer than 2 values |
  | `q25`, `q50`, `q75` | see below | approximate (t-digest, DataFusion's `approx_percentile_cont`), as DuckDB's `SUMMARIZE` |

  Cells that make no sense for a column's type (`avg` of a string) are null. On an empty input
  (0 rows), `null_percentage` is null, not `NaN`, and `count` and `approx_unique` are 0.
- **Why approximate quantiles.** `percentile` is exact and keeps every value in memory
  (`PercentileAccumulator` in `crates/datascript-datafusion/src/udaf.rs`). Three exact percentiles
  per column would hold roughly every numeric and datetime value of the input, on exactly the large
  unfamiliar files people run `describe` on. t-digest uses bounded memory. Its results can vary
  slightly with how input is partitioned, but golden tests already run with one partition (D-017),
  so they are deterministic. Datetime quantiles go through their integer value.
- DataFusion's `approx_distinct` does not accept every Arrow type (check `Float64`, `Boolean`,
  `Timestamp` and `Duration`); cast such columns to a type it accepts, or compute the count
  another way, and test every DataScript type.
- `min`, `max` and `q25`…`q75` hold values of different types in different rows, so they are
  `string` columns holding the formatted value, as in DuckDB, formatted as `tostring` formats
  them. Not `dynamic`: this task does not depend on M2-03.
- **Naming.** Every other generated column name in DataScript is PascalCase, following KQL
  (`getschema`'s `ColumnName`, `count`'s `Count`). These are snake_case on purpose, to match
  DuckDB and pandas users' expectations. Record the exception in DECISIONS.md, and add a note to
  LANGUAGE.md so later operators know which convention to follow. `output`'s summary columns
  (M2-25) follow the same exception; whichever task lands first writes the entry and the other
  extends it.
- **IR.** `describe` gets its own IR node (`PlanNode::Describe` or similar), because the IR has no
  unpivot and no approximate aggregates to build it from. `explain` shows it as one step, and the
  SQL backend (M6-03) can lower it to DuckDB's `SUMMARIZE`. `Capabilities` says whether a backend
  supports it (DS4001 otherwise).
- **Row order.** Rows come out in input-column order, so the output order is defined: update
  LANGUAGE.md §3.4 and the golden driver's `output_order_defined`
  (`crates/datascript-tests/src/pipeline.rs`) so `describe` results compare in order.
- **Cost.** The DataFusion backend lowers it as one aggregate pass (an unpivot of per-column
  aggregates), not one query per column. LANGUAGE.md, hover and completion docs say that `describe` reads every
  row and that `getschema` gives column names and types without reading data.
- Columns with no DataScript type (DS2008) appear with only `count` and `null_percentage`.
- Takes no arguments in this task. `describe a, b` (a column subset) is a follow-up if wanted.
- REPL: `.describe <source>` runs `from <source> | describe` through the normal query path, so
  Ctrl-C cancels it as it cancels any query (`crates/datascript-cli/src/repl/editor.rs`). Add it to
  `.help` and to dot-command completion. (The REPL's `.schema` stays the cheap one.)
- `describe` is a new operator in `datascript-syntax` (`OPERATORS`, AST, grammar), so this task
  edits the parser (see the parallelism note in the task index).
- Add it to LANGUAGE.md §3.2 and LSP completion and hover. (The TextMate grammar in
  `editors/vscode` already highlights any identifier after `|` as an operator, so it needs no
  change.) If M2-07 (formatter) is done, format `describe` there too; otherwise M2-07's
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

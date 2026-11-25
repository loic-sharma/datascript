# M2-26 — `output`: writer functions and options

**Status:** todo
**Depends on:** M2-25
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-engine`, `datascript-datafusion`
**Diagnostic codes:** DS2330–DS2339 (types) (D-018)

## Goal
Writer functions as `output` targets, mirroring the reader functions, with the same option names
so a file written with some options reads back with the same options:

```ds
from 'orders.csv' | output write_csv('out/orders.csv', delimiter = ';');
from 'orders.csv' | output write_csv('out/orders.csv', header = false);
from 'logs/*.json' | output write_parquet('out/errors.parquet', compression = "zstd", row_group_size = 100000);
from 'orders.csv' | top 10 by amount desc | output write_json('out/top10.json', format = "array");
```

## Scope

### Binding
- `output write_csv(…)`, `write_parquet(…)`, `write_json(…)`. Writer options live in a writer
  catalog next to `source::READERS` (D-041), checked by the binder with the same codes as reader
  options (DS3003 unknown option with did-you-mean, DS2041 wrong value, DS2043 given twice, DS2042
  not exactly one path). Reword those codes' summaries in the DIAGNOSTICS.md registry and
  `codes.rs` from "reader option" to "reader or writer option".
- `datascript-syntax` has its own `READERS` list that a compiler test keeps in step with
  `source::READERS`; add the writer names the same way.
- **Column-list option values.** `force_quote = (a, b)` (and M2-27's `partition_by = (year, month)`)
  needs syntax that does not exist yet: a parenthesized list of column names is not an expression
  in `datascript-syntax`, and `OptionValue` (`crates/datascript-compiler/src/source.rs`) holds only
  `Bool`, `Long` and `String`. Add the syntax for option values only, an `OptionValue` case for a
  column list, and check each name against the input schema (DS1001 with did-you-mean).
- LSP completion, hover and signature help cover the writer functions and their options. If M2-07
  (formatter) is done, format writer calls there too; otherwise M2-07's "Carried over" item covers
  it.

### Options
Reader names win over DuckDB's where a reader option exists.

| Writer | Options |
|---|---|
| `write_csv` | `delimiter`, `header` (default true), `quote`, `escape`, `null_value` (default empty), `force_quote = (col, …)`, `timestamp_format`, `compression` |
| `write_parquet` | `compression`, `compression_level`, `row_group_size` |
| `write_json` | `format = "ndjson" \| "array"` (the `read_json` name; DuckDB's `ARRAY`), `timestamp_format`, `compression` |
| all | `overwrite` / `append`: one write-mode option, named in this task (DuckDB has `OVERWRITE`, `OVERWRITE_OR_IGNORE`, `APPEND`) |

DataScript has no `date` type (`Date32`/`Date64` widen to `datetime`, LANGUAGE.md §5.1), so there
is no `date_format`: `timestamp_format` covers every `datetime` column, and uses the same
specifiers as `format_datetime` (LANGUAGE.md §7), not DuckDB's `strftime` ones.

`write_json(…, format = "array")` needs a streaming JSON array writer: `writers::write_json` still
collects every row into a `Vec<Value>`. Make it stream (sharing the code with `printer.rs`).

Partitioned output is M2-27's. Excel output (`write_xlsx`) is M2-24's.

## Acceptance criteria
- [ ] Golden `run` cases for every example in the Goal and every option, each reading the file
      back (with the matching reader options where they exist).
- [ ] Golden `check` cases: unknown option with did-you-mean, repeated option, wrong value type,
      unknown column in `force_quote`, a writer call with no path.
- [ ] A test that a large `format = "array"` output is streamed, not collected.

## Follow-ups

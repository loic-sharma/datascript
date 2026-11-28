# M2-26 — `output`: writer functions, options and compression

**Status:** todo
**Depends on:** M2-25, M2-27 (`write_parquet` needs its Parquet writer)
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-diagnostics`, `datascript-engine`, `datascript-datafusion`, `datascript-lsp`
**Diagnostic codes:** DS2330–DS2339 (types) (D-018). Most errors reuse existing codes (below).
The new ones are for problems no existing code describes, such as options that conflict
(`compression_level` with a codec that has no levels) or a column-list option naming a column the
writer cannot apply it to. A malformed column list (`force_quote = (a,, b)`) is the generic
DS0101, so no syntax block is needed.

## Goal
Writer functions as `output` targets, mirroring the reader functions, with the same option names
so a file written with some options reads back with the same options, plus compressed output:

```ds
from 'orders.csv' | output write_csv('out/orders.csv', delimiter = ';');
from 'orders.csv' | output write_csv('out/orders.csv', header = false);
from 'orders.csv' | output 'out/orders.csv.gz';     // compression from the second suffix
from 'logs/*.json' | output write_parquet('out/errors.parquet', compression = "zstd", row_group_size = 100000);
from 'orders.csv' | top 10 by amount desc | output write_json('out/top10.json', format = "array");
```

## Scope

### Binding
- `output write_csv(…)`, `write_parquet(…)`, `write_json(…)`. Writer options live in a writer
  catalog next to `source::READERS` (D-041), checked by the binder with the same codes as reader
  options (DS3003 unknown option with did-you-mean, DS2041 wrong value, DS2043 given twice, DS2042
  not exactly one path). Reword those codes' summaries in the DIAGNOSTICS.md registry and
  `codes.rs` (`datascript-diagnostics`) from "reader option" to "reader or writer option".
- No writer list is needed in `datascript-syntax`: its `READERS` exists only so the parser can
  report DS0150 for a pipeline that starts with a reader call
  (`crates/datascript-syntax/src/parser/grammar.rs`), and a writer never starts a pipeline.
- **Column-list option values.** `force_quote = (a, b)` needs syntax that does not exist yet: a
  parenthesized list of column names is not an expression in `datascript-syntax`, and
  `OptionValue` (`crates/datascript-compiler/src/source.rs`) holds only `Bool`, `Long` and
  `String`. Add the syntax for option values only, an `OptionValue` case for a column list, and
  check each name against the input schema (DS1001 with did-you-mean). Specify and test:
  - `force_quote = (a)` with one column, which already parses as a parenthesized expression, so
    the binder must accept that shape as a one-column list;
  - trailing commas, `(a,)` and `(a, b,)`, allowed as in every other list (D-059);
  - `force_quote = *` (every column) is not in this task.
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
| all | `overwrite` (bool, default true): false refuses to replace an existing file, with an error naming it |

- **No `append`.** DuckDB's `APPEND` is for partitioned output, which is not planned (it returns
  with Hive-partitioned directories, Q-006). Appending to a single file would also defeat
  M2-25's temp-file-and-rename and clash with CSV header rows.
- **Compression.** Text writers take `compression` (`"gzip"`, `"bzip2"`, `"xz"`, `"zstd"`, the
  reader values in `source::COMPRESSIONS`) or the second suffix of the path (`.csv.gz`), as
  readers do; remove M2-25's "planned" help for compression suffixes. Parquet has its own codec
  list: `"uncompressed"`, `"snappy"` (the default, M2-27), `"gzip"`, `"zstd"`, `"lz4"`,
  `"brotli"`. `compression_level` applies only to codecs that have levels.
- **Builds.** Compression crates in `datascript-engine` go behind a cargo feature, as reading is
  gated in `datascript-datafusion` (`compression` feature, native only), so the wasm32 build of
  the engine stays free of C code. Where a codec is not built in, writing with it is an error that
  says so, as reading already is.
- **`timestamp_format`.** DataScript has no `date` type (`Date32`/`Date64` widen to `datetime`,
  LANGUAGE.md §5.1), so there is no `date_format`: `timestamp_format` covers every `datetime`
  column and uses `format_datetime`'s specifiers (LANGUAGE.md §7), not DuckDB's `strftime`
  ones. Check the format string at check time, as `format_datetime`'s own argument is checked
  (`crates/datascript-compiler/src/arg_values.rs`), with the same code. **Apply it in the plan, not
  in the writer:** lowering adds a projection before the sink that turns each `datetime` column
  into text with the existing `format_datetime` function. The formatting code lives in
  `datascript-datafusion` and the writers in `datascript-engine`, which does not depend on it, so
  this needs no code moved, and the output matches `format_datetime` exactly.

`write_json(…, format = "array")` needs a streaming JSON array writer: `writers::write_json` still
collects every row into a `Vec<Value>`. Make it stream (sharing the code with `printer.rs`). If
M4-14 is done, add `json` to its export formats (D-064) with a read-back test. (M4-14's "Carried
over" item covers it if M4-14 lands after this task.)

Excel output (`write_xlsx`) is M2-28's. Partitioned output (`partition_by`) is not planned yet;
it comes back together with reading Hive-partitioned directories (Q-006, ROADMAP.md).

**DuckDB option table.** Add a table to LANGUAGE.md's sink section listing every DuckDB
`COPY … TO` option (check DuckDB's current documentation) with its DataScript name, or "not
supported" and why (for example `append`, `partition_by`, `per_thread_output`, `file_size_bytes`,
`filename_pattern`, and Parquet's `row_group_size_bytes`, `field_ids` and `kv_metadata`). Options
not supported are added only when someone asks.

## Acceptance criteria
- [ ] Golden `run` cases for every example in the Goal and every option, each reading the file
      back with the M2-25 harness directive (with the matching reader options where they exist).
- [ ] Golden `check` cases: unknown option with did-you-mean, repeated option, wrong value type,
      unknown column in `force_quote`, a one-column and a trailing-comma `force_quote`, an invalid
      `timestamp_format`, a Parquet codec given to `write_csv`, and a writer call with no path.
- [ ] A golden `run` case for `overwrite = false` on an existing file.
- [ ] A test that a large `format = "array"` output is streamed, not collected.
- [ ] `datascript-engine` still builds for wasm32 without the compression feature.
- [ ] The DuckDB option table in LANGUAGE.md.

## Follow-ups

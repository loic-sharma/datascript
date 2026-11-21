# Testing

DataScript is tested mainly with **golden-file integration tests**: an input program plus its
expected output, where the expected output may be a result table *or* diagnostics. Error messages
are behaviour, and they are reviewed in diffs like any other output.

## Test layers

| Layer | Where | What |
|---|---|---|
| Golden tests | `tests/cases/**/*.dstest`, run by the `datascript-tests` crate | End-to-end: parse → bind → plan → execute, or the diagnostics |
| Unit tests | `#[cfg(test)]` in each crate | Lexer edge cases, type rules, small algorithms |
| Parser snapshots | `tests/cases/syntax/*.dstest` with `mode: syntax` | CST dumps for tricky parses and recovery: one line per node (`KIND@start..end`) or token (`KIND@start..end "text"`), children indented two spaces, trivia included; then the lexical and syntax diagnostics, and, if any has a machine-applicable fix, the query with the fixes applied and the result of parsing it again. Targeted syntax diagnostics live in `tests/cases/errors/syntax/` |
| LSP tests | `crates/datascript-lsp/tests` (M3) | Request/response scripts against an in-process server |
| Web/WASM tests | `web/` (Vitest + Playwright, M4/M5) | UI flows and runtime parity with the CLI |

## The `.dstest` format

A single file with header lines, then sections introduced by `----` lines:

```
# Summarize revenue per customer, sorted.
mode: run
---- query
from '../../data/orders.csv'
| summarize revenue = sum(amount) by customerId
| sort by revenue desc
| take 3
---- expected
customerId | revenue
string     | real
-----------+--------
c-042      | 1250.5
c-007      |  980.0
c-113      |  402.25
```

```
# Typo in a column name gets a did-you-mean.
mode: check
---- query
from '../../data/orders.csv' | summarize sum(amout)
---- expected
error[DS1001]: unknown column `amout`
  --> query:1:46
  |
1 | from '../../data/orders.csv' | summarize sum(amout)
  |                                              ^^^^^ no column named `amout` here
  |
  = help: a column with a similar name exists: `amount`
  = note: columns available at this point: orderId, customerId, amount, timestamp
```

### Header keys

| Key | Values | Default | Meaning |
|---|---|---|---|
| `mode` | `syntax`, `check`, `explain`, `run` | `run` | How far the pipeline goes. `syntax` dumps the CST, `explain` prints the logical plan |
| `order` | `strict`, `any` | `strict` if every query's output order is defined by LANGUAGE.md §3.4 (its last `sort` or `top` is followed only by order-preserving operators such as `take`, `where`, `project`, `extend`, `render`), else `any` | `any` sorts result rows before comparing (D-017) |
| `now` | ISO datetime | `2024-06-01T00:00:00Z` | Fixed clock for `now()`/`ago()` |
| `engine` | `datafusion`, … | `datafusion` | Backend under test. A comma-separated list runs the case on each (see "Several engines") |
| `tables` | `name=path, ...` | | Host-registered tables (simulates `--table`/drag-drop) |
| `schema` | `name(col: type, ...); ...` | | Schemas for a mock `SchemaProvider`, for `check`/`explain` cases before a real backend exists (M1-05). With `schema`, no files are read |
| `show` | `schema` | | `check` cases only: after the diagnostics, print each query's output columns (`name: type`, one per line; `unknown` when an error hides them). Used to test operator schemas (M1-16) |
| `skip` | reason | | Skip with a reason (e.g. known bug, links a task) |
| `eol` | `lf`, `crlf` | `lf` | Line endings of the query. `.dstest` files are always stored with LF (`.gitattributes`), so `crlf` converts every query line ending to `\r\n` before running |

Lines starting with `#` before the first section are a free-form description.

Harness details (M0-03):

- A section header is exactly `---- ` followed by a name (`query`, `expected`,
  `expected[<engine>]`). Table separators such as `-----+---` contain no space, so they are never
  mistaken for one. Unknown keys, sections, modes and engines fail the case with the file, the
  line and a did-you-mean.
- A case without `order:` asks the driver through `Driver::output_order_defined`. The pipeline
  driver parses the query and answers per §3.4; it does not look through `let`s, so
  `from s | take 5` with a sorted `let s` counts as undefined (`any`). A driver that cannot tell
  returns `None`, which means `any`.
- The query is passed with trailing blank lines removed and exactly one final newline. Expected
  sections compare with `\r\n` normalized and trailing blank lines ignored; trailing spaces inside
  lines are significant (`.editorconfig` keeps them in `.dstest` files).
- `schema:` names are identifiers or quoted paths (`'../data/x.csv'(n: long)`); column names are
  identifiers or `['quoted name']`. Types are DataScript type names, mapped to the canonical Arrow
  types by the harness.
- Each case is one trial named by its path under `tests/cases/` and tagged with its mode, so
  `-- operators/where/` and `-- run` both filter. Cases in a mode the engine's driver does not
  support yet, and `skip:` cases, are **ignored**; forcing them with `-- --ignored` fails them
  with the reason rather than passing them.

### Expected output rules

- **Results** print as an ASCII table: header row, type row, separator, data rows. Values are
  formatted deterministically: reals with the shortest round-trip representation, datetimes in
  ISO 8601 UTC, nulls as `null`. Several query statements print several tables separated by blank lines.
- Table layout (`datascript_engine::table::format_table`, shared with the CLI): `real` columns
  are aligned on the decimal point, `int`/`long` right-aligned, everything else left-aligned.
  Columns with no DataScript type show `arrow:<Arrow type>` in the type row. `timespan` values
  print as `[-][d.]hh:mm:ss[.ffffff]`; fractions of datetimes and timespans drop trailing zeros.
  Control characters in strings are escaped (`\n`, `\t`, `\u{1b}`). Trailing spaces are trimmed.
- A `render` adds a line `render: timechart x=month y=[revenue] series=customerId` after the table.
- **Diagnostics** use the *plain* renderer: no color, ASCII only, the file is shown as `query`.
- **Fixes** (`mode: syntax`, M1-03): when a diagnostic carries a machine-applicable suggestion,
  the output ends with `with the machine-applicable fixes applied:`, the fixed query indented four
  spaces, and either `which parses without errors` or `which still has errors:` followed by them
  (the file is shown as `fixed`). `datascript_tests::apply_suggestions` applies the fixes; a
  suggestion that overlaps one already applied is skipped whole. All fixes are applied together in
  one pass, so fixes that collide (`a&&!b` giving `andnot(b)`) show up in the re-parse; at one
  offset a replacement is applied before an insertion, so the insertion lands in front of it. A fix that does not parse shows
  up in review as a diff, so every such suggestion is tested by the case that produces it.
  All diagnostics are printed, errors and warnings, in source order.
- When a `run` program has warnings and still executes, the warnings print first, then the table.
- `DS5001` (chart not drawn) is not printed in golden output, because the `render:` line already
  shows the spec. Every other info diagnostic is printed.

### Determinism

Golden output must be byte-identical across machines and runs (D-017):

- The driver runs DataFusion with `target_partitions = 1` and a fixed batch size, so row order
  without `sort` and float summation order are stable. Cases still must not depend on them: use
  `order: any` or a `sort` with tie-breaking keys.
- `sort` is not stable (LANGUAGE.md §3.4). The fixtures contain ties, so a case that sorts on a
  tied key must add a tie-breaker or use `order: any`.
- Absolute paths in output are rewritten: the test file's directory becomes `<test>/`, the
  repository root `<repo>/`. Directory listings (did-you-mean over nearby files) are sorted.
- No output may depend on the wall clock, the locale, the time zone, or the environment.

### Several engines

With `engine: datafusion, sql-duckdb`, one `---- expected` section applies to all engines. When an
engine legitimately differs, add `---- expected[sql-duckdb]` for it and say why in the description.
`DS_BLESS` writes per-engine sections only when the outputs differ.

### Blessing

```bash
DS_BLESS=1 cargo test -p datascript-tests            # rewrite all expected sections
DS_BLESS=1 cargo test -p datascript-tests -- join/    # only matching cases
```

Review every blessed diff. Do not bless error output you have not read.

### Fixtures

Shared datasets live in `tests/data/` and stay small (well under 1 MB in total), are deterministic,
and are documented in `tests/data/README.md`:

- `orders.csv` / `orders.parquet`: orderId, customerId, amount, timestamp (~50 rows over 6 months)
- `customers.csv`: customerId, name, country, signupDate
- `events/2024-0{1,2,3}.ndjson`: for glob and JSON tests
- `edge/`: empty file, header-only CSV, nulls, unicode, quoted delimiters
- `reordered/*.csv`: CSV files with the same columns in different header orders, plus one file
  with an extra column, for name-based glob merging (LANGUAGE.md §3.1)
- `types.parquet`: `Timestamp(ns)`, `Date32`, `Int16`, `UInt32`, `Float32`, a dictionary string
  column and a decimal column, for the read mapping in LANGUAGE.md §5.1

Paths inside `.dstest` files are relative to the test file, like script paths in real use.

## Coverage expectations per task

Every task that adds language surface adds, at minimum:

1. A happy-path `run` case per operator/function variant.
2. An error case for each diagnostic it introduces, showing the full rendered message.
3. A recovery case: a program with two independent errors reports both.

The case directory mirrors the language: `tests/cases/{lexer,syntax,expressions,let,operators/<op>,functions/<group>,sources,render,errors}/`.
Error cases are grouped by area under `errors/` (`syntax`, `names`, `types`, `recovery`).

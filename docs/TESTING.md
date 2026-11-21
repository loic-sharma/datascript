# Testing

DataScript is tested mainly with **golden-file integration tests**: an input program plus its
expected output, where the expected output may be a result table *or* diagnostics. Error messages
are behaviour, and they are reviewed in diffs like any other output.

## Test layers

| Layer | Where | What |
|---|---|---|
| Golden tests | `tests/cases/**/*.dstest`, run by the `datascript-tests` crate | End-to-end: parse → bind → plan → execute, or the diagnostics |
| Unit tests | `#[cfg(test)]` in each crate | Lexer edge cases, type rules, small algorithms |
| Parser snapshots | `tests/cases/syntax/*.dstest` with `mode: syntax` | CST dumps for tricky parses and recovery |
| LSP tests | `crates/datascript-lsp/tests` (M3) | Request/response scripts against an in-process server |
| Web/WASM tests | `web/` (Vitest + Playwright, M4/M5) | UI flows and runtime parity with the CLI |

## The `.dstest` format

A single file with header lines, then sections introduced by `----` lines:

```
# Summarize revenue per customer, sorted.
mode: run
---- query
'../../data/orders.csv'
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
'../../data/orders.csv' | summarize sum(amout)
---- expected
error[DS1001]: unknown column `amout`
  --> query:1:41
  |
1 | '../../data/orders.csv' | summarize sum(amout)
  |                                         ^^^^^ no column named `amout` here
  |
  = help: a column with a similar name exists: `amount`
  = note: columns available at this point: orderId, customerId, amount, timestamp
```

### Header keys

| Key | Values | Default | Meaning |
|---|---|---|---|
| `mode` | `syntax`, `check`, `explain`, `run` | `run` | How far the pipeline goes. `syntax` dumps the CST, `explain` prints the logical plan |
| `order` | `strict`, `any` | `strict` | `any` sorts result rows before comparing (for queries without a defined order) |
| `now` | ISO datetime | `2024-06-01T00:00:00Z` | Fixed clock for `now()`/`ago()` |
| `engine` | `datafusion`, … | `datafusion` | Backend under test. Later: run the same case on several engines |
| `tables` | `name=path, ...` | | Host-registered tables (simulates `--table`/drag-drop) |
| `skip` | reason | | Skip with a reason (e.g. known bug, links a task) |

Lines starting with `#` before the first section are a free-form description.

### Expected output rules

- **Results** print as an ASCII table: header row, type row, separator, data rows. Values are
  formatted deterministically: reals with the shortest round-trip representation, datetimes in
  ISO 8601 UTC, nulls as `null`. Several query statements print several tables separated by blank lines.
- A `render` adds a line `render: timechart x=month y=[revenue] series=customerId` after the table.
- **Diagnostics** use the *plain* renderer: no color, ASCII only, the file is shown as `query`.
  All diagnostics are printed, errors and warnings, in source order.
- When a `run` program has warnings and still executes, the warnings print first, then the table.

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

Paths inside `.dstest` files are relative to the test file, like script paths in real use.

## Coverage expectations per task

Every task that adds language surface adds, at minimum:

1. A happy-path `run` case per operator/function variant.
2. An error case for each diagnostic it introduces, showing the full rendered message.
3. A recovery case: a program with two independent errors reports both.

The case directory mirrors the language: `tests/cases/{lexer,syntax,operators/<op>,functions/<group>,sources,render,errors}/`.

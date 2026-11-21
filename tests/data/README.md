# Test fixtures

Small, deterministic datasets shared by golden tests (`tests/cases/`), examples and docs.
Paths in `.dstest` files are relative to the test file, e.g. `'../../data/orders.csv'` from
`tests/cases/<dir>/x.dstest`.

**Do not edit these files by hand.** They are written by the `datascript-fixtures` crate from a
fixed seed:

```bash
cargo run -p datascript-fixtures
```

`cargo test -p datascript-fixtures` fails if a committed file differs from what the generator
produces, if a file here is not generated, if this README does not mention a file, or if a
property listed below stops holding. To change a fixture, change the generator, regenerate, and
update this README. Existing golden tests depend on the current rows, so prefer adding a file to
changing one.

All files use LF line endings and UTF-8 (`.gitattributes` keeps Git from converting them).
Timestamps are UTC. Total size is about 16 KB.

## `orders.csv` / `orders.parquet`

50 orders between 2023-12-01 and 2024-05-31 (six months, all before the golden tests' default
`now` of 2024-06-01). Rows are sorted by `timestamp`; `orderId` follows that order. The two files
hold the same rows.

| Column | CSV text | Parquet type | DataScript type |
|---|---|---|---|
| `orderId` | `o-0001` … `o-0050` | Utf8 | `string` |
| `customerId` | `c-042` | Utf8 | `string` |
| `amount` | `183.5` (always has a decimal point; empty = null) | Float64, nullable | `real` |
| `timestamp` | `2024-02-29T23:59:59Z` | Timestamp(µs, "UTC") | `datetime` |

Properties tests rely on:

- **Exact sums.** Every amount is a multiple of 0.25, so sums are exact whatever order rows are
  added in.
- **Revenue ranking** (`sum(amount) by customerId`): `c-042` 1250.5, `c-007` 980.0, `c-113`
  402.25, then a **tie** at 300.0 between `c-001` (4 × 75.0) and `c-019` (3 × 100.0). Every other
  customer is below 300.
- **Tied amounts**: 75.0 four times, 100.0 three times, and a few accidental repeats.
- **A tied timestamp**: `o-0025` and `o-0026` share `2024-02-23T21:33:46Z`. Sort on
  `timestamp` needs a tie-breaker such as `orderId`.
- **Calendar edges**: an order at `2024-02-29T23:59:59Z` (leap day, last second of February) and
  one at exactly `2024-03-01T00:00:00Z` (month boundary).
- **One null amount**: `o-0037` (customer `c-011`).
- **An order whose customer does not exist**: `c-999` has 2 orders and is not in
  `customers.csv` (inner joins drop them, left joins keep them with null customer columns).

## `customers.csv`

13 customers.

| Column | Example | DataScript type |
|---|---|---|
| `customerId` | `c-042` | `string` |
| `name` | `Hana Sato` | `string` |
| `country` | `JP` (empty = null) | `string` |
| `signupDate` | `2023-07-01` (a date, no time) | `datetime` |

Properties:

- **A customer with no orders**: `c-200`.
- **A null country**: `c-077`.
- **A repeated country**: `DE` for `c-113` and `c-128`.
- Every `customerId` is unique (the join key on this side has no duplicates; on the orders side
  every customer has several rows).

## `events/2024-01.ndjson`, `events/2024-02.ndjson`, `events/2024-03.ndjson`

NDJSON, 8 events per file (24 in total), sorted by `ts` within each file. For globs
(`'events/*.ndjson'`) and JSON tests.

| Field | Example | JSON type |
|---|---|---|
| `ts` | `"2024-02-01T08:00:34Z"` | string (ISO 8601) |
| `user` | `"u-1"` … `"u-5"` | string |
| `event` | `"view"`, `"click"`, `"purchase"` | string |
| `duration_ms` | `1335` | integer |
| `props` | `{"page":"/docs","ref":null}` | object; `ref` is `"newsletter"` or null |
| `country` | `"US"` | string, **only in `events/2024-02.ndjson`** |

Properties:

- **A column missing from some files**: `country` exists only in February, so a glob over all
  three files has it null for January and March.
- **A null in one file only**: the first March event has `"duration_ms": null`.
- **A nested object** (`props`) for `dynamic` tests (M2).

## `edge/`

Hand-written files for reader edge cases.

| File | Contents |
|---|---|
| `edge/empty.csv` | 0 bytes: no header, no rows |
| `edge/header_only.csv` | Header `id,name,value`, no rows |
| `edge/nulls.csv` | `id,name,value,when,always_null`, 5 rows. Empty fields scattered over `name`, `value` and `when`; row 5 is empty except `id`; `always_null` is empty in every row (no type can be inferred from it) |
| `edge/unicode.csv` | `id,name,città,unit price`, 7 rows. A non-ASCII column name and one with a space (`['unit price']`); Latin accents, CJK, Cyrillic, Arabic (right-to-left), decomposed accents (`e` + U+0301) and emoji (outside the BMP) |
| `edge/quoted.csv` | `id,text,amount`, 6 rows. A quoted comma (`"a, b"`), a quoted newline (row 2 spans two lines), doubled quotes (`"she said ""hi"""`), an empty quoted field (`""`), and leading/trailing spaces inside quotes |

## `reordered/`

Three CSV files with the same columns in different header orders, for name-based glob merging
(LANGUAGE.md §3.1). `'reordered/*.csv'` reads 8 rows with `id` 1–8.

| File | Header | Rows |
|---|---|---|
| `reordered/a.csv` | `id,name,score` | ids 1–3 |
| `reordered/b.csv` | `score,name,id` | ids 4–6 |
| `reordered/c.csv` | `name,id,score,extra` | ids 7–8; `extra` is `x`, `y` |

`id` and `score` are integers in every file, `name` and `extra` strings, `score` = `id` × 10.
Merged by name, `extra` is null for ids 1–6. Merged by position, the values would land in the
wrong columns, which the tests catch.

## `types.parquet`

4 rows, one column per Arrow type in LANGUAGE.md §5.1. Row 4 is null in every column but `id`.

| Column | Arrow type | Rows 1–3 | Notes |
|---|---|---|---|
| `id` | Int64, non-null | 1, 2, 3 (4) | Sort key |
| `ts_ns` | Timestamp(ns, no timezone) | `2024-01-15T09:30:00.123456789`, `1969-12-31T23:59:59`, `2024-02-29T23:59:59.999999999` | Nanoseconds truncate to µs (row 3 must stay on Feb 29); a pre-1970 value |
| `ts_tz` | Timestamp(µs, "+02:00") | `2024-01-15T09:30:00Z`, `2024-06-30T22:30:00Z`, `1970-01-01T00:00:00Z` | Values are UTC instants; row 2 is July 1 in +02:00 but June 30 in UTC |
| `date` | Date32 | `2024-01-15`, `1969-12-31`, `2024-02-29` | Midnight UTC; a negative day number |
| `i16` | Int16 | -32768, 0, 32767 | Widens to `long` |
| `u32` | UInt32 | 0, 1, 4294967295 | Widens to `long`; the max does not fit `int` |
| `f32` | Float32 | 0.1, -1.5, 3.4028235e38 (`f32::MAX`) | 0.1 widens to `0.10000000149011612` |
| `dict` | Dictionary(Int32, Utf8) | `red`, `green`, `red` | Reads as `string` |
| `dec` | Decimal128(10, 2) | 12345.67, -0.01, 0.00 | `real` in M1 |
| `bin` | Binary | `00 01`, empty, `ff` | No DataScript type: projectable only, `DS2008` in expressions |

## Source test files (M1-08)

Small files for reading sources: glob merging, compression, JSON layouts, reader options and
errors.

| File | Contents | Used for |
|---|---|---|
| `widen/a.csv`, `widen/b.csv` | `id,score`; `score` is an integer in `a.csv` (10, 20) and a real in `b.csv` (30.5, 40.25) | `'widen/*.csv'` widens `score` to `real` |
| `conflict/a.csv`, `conflict/b.csv` | `id,when`; `when` is a timestamp in `a.csv` and text (`soon`, `later`) in `b.csv` | `DS3005`: no common type |
| `compressed/orders.csv.gz` | `orders.csv` in a gzip file (one stored block, so the bytes never depend on the compression library; no file name, zero mtime) | Compression from the suffix |
| `compressed/orders.tsv.zst` | `orders.csv` with tabs for commas, zstd level 3 | TSV and zstd |
| `compressed/orders.dat` | Same bytes as `orders.csv.gz` | `read_csv(..., compression = 'gzip')` on a name with no known extension |
| `json/array.json` | A JSON array of 3 objects `{id, name, score}`; `score` is null in the second | `.json` holding an array (detected) |
| `json/lines.json` | 2 lines of NDJSON `{id, name}` in a `.json` file | `.json` holding NDJSON (detected) |
| `options/semicolon.txt` | 3 rows, no header, `;` as delimiter, `'` as quote, `NA` for null: `1;'a;b';1.5`, `2;NA;2.5`, `3;plain;NA` | `read_csv` options |
| `bad/broken.json` | One good line, then a truncated object | `DS3008`: malformed JSON |
| `bad/not_parquet.parquet` | Plain text | `DS3008`: not a Parquet file |
| `uint64.parquet` | `id` Int64 (1–3); `big` UInt64: 1, `i64::MAX`, `u64::MAX` | `DS4003`: `UInt64` above `i64::MAX` |

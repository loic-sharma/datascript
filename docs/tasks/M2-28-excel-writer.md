# M2-28 — Excel writer

**Status:** todo
**Depends on:** M2-24, M2-26
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-engine`, `datascript-datafusion`
**Diagnostic codes:** DS3085–DS3089 (D-018)

## Goal
`| output 'report.xlsx'` and `| output write_xlsx('report.xlsx', sheet = "Summary")` write Excel
workbooks, as DuckDB's excel extension does.

## Scope
- `.xlsx` as an output extension (M2-25) and `write_xlsx` in M2-26's writer catalog (and in
  `datascript-syntax`'s writer list). Options: `sheet` (default `Sheet1`) and `header` (default
  true).
- Use `rust_xlsxwriter` (pure Rust). Check its license and turn off default features that are not
  needed. Its default timestamp in the workbook's metadata calls `SystemTime::now()`, which
  compiles on wasm32 but panics there at run time, so the wasm32 build check would not catch it:
  set the creation time explicitly (core code reads no clock; use the session's injected clock),
  and test a write on wasm32 if M5-01 is done.
- `datetime` and `timespan` columns get Excel number formats.
- Values Excel cannot hold (a string over 32 767 characters, more than 1 048 576 rows) are an error
  from this task's block naming the limit, never silent truncation.
- If M4-01 is done, its export (Q-007) offers `.xlsx` through the same writer.

## Acceptance criteria
- [ ] Golden `run` cases that write a workbook with `output` and read it back with `read_xlsx`
      (M2-24), including dates, a custom sheet name and no header.
- [ ] Golden error cases for each new code, including the row limit.

## Follow-ups

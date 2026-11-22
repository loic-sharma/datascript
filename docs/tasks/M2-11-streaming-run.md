# M2-11 — Stream results in `ds run`

**Status:** done — `Session::start`/`RunStream`/`QueryStream` stream results; `ds run` and the REPL write them batch by batch, and cut tables stop reading (D-055).
**Depends on:** M1-13
**Crates:** `datascript-engine`, `datascript-datafusion`, `datascript-cli`
**Diagnostic codes:** none new expected; reserve a block if needed

## Goal
`ds run` collects each result fully before printing (`Session::run`). A large result should
stream instead, so output starts at once and memory stays flat.

## Scope
- A streaming variant of `Session::run` behind the same panic boundary (D-045, D-046), yielding
  record batches as the engine produces them.
- `--format table`: print rows as batches arrive. Column widths need the rows first, so buffer up
  to `--max-rows` rows, print the table, then stop reading (which cancels the query). Count the
  remaining rows only if it is cheap; otherwise say "more rows" without a number.
- `--format csv`/`ndjson`/`arrow`: write batch by batch.
- The REPL uses the same path, with `.max-rows`.
- Ctrl-C still cancels mid-stream with exit code 130.

## Acceptance criteria
- [x] A CLI test shows output arriving before a slow query finishes (or the result is not
      collected: e.g. a query over a large generated file stays under a memory bound).
      `csv_is_written_batch_by_batch_before_the_query_finishes` (rows written before the last
      batch fails) and `a_cut_table_stops_reading_and_says_at_least` (a row that would fail is
      never read) in `crates/datascript-cli/tests/cli.rs`.
- [x] Table output with `--max-rows` is unchanged for the existing golden and CLI cases, except
      where the footer can no longer count the rest (document the new wording). New wording
      `… at least N more rows (show them all with --max-rows 0)` in D-055.
- [x] CSV/NDJSON/Arrow output is byte-identical to the collected version. JSON too. Compared
      with the M1 binary over every fixture in `tests/data` and a generated 50,000-row file:
      stdout, stderr and exit codes match, except the table footer for results past the
      counting budget.

## Carried over
- [x] (from planning, 2026-11-22) M4-01 also needs streaming and cancellation for long queries over WebSocket. Whichever task comes first builds the streaming `Session` API in `datascript-engine` for both; the other one reuses it.
- [x] (from M1-11) `ds run` collects each result fully before printing (`Session::run`). Large
      results should stream: print table rows as batches arrive and stop reading (which cancels
      the query) once `--max-rows` is reached, counting the rest only if cheap; write
      CSV/NDJSON/Arrow batch by batch. This needs a streaming variant of `Session::run` behind the
      same panic boundary.

## Follow-ups
- The REPL streams through the same `output::stream_results` (with `.max-rows`).
- M4-01 reuses `Session::start` (handed over in its "Carried over").
- M3-05 must print what `ds run` prints; the streaming printer lives in `datascript-cli` for now
  (handed over in M3-05's "Carried over").

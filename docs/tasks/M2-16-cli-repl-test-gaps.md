# M2-16 — CLI and REPL integration test gaps

**Status:** done — added 17 CLI and REPL end-to-end integration tests covering `ds check` (`--deny-warnings`, `--format json`), `ds run` (broken pipe, streaming/mid-stream failure for `ndjson`/`json`/`arrow`, `--statement` on no-query program, `--output` formats, multiple/glob `--table`), `ds explain` (file, stdin, multi-query, `render`), and REPL (`--max-rows`, runtime error `<repl:N>`, deleted-file saved `let` `<repl:1>`, `.max-rows` and extra-arg meta mistakes), and ticked M1-11's README example box.
**Depends on:** M2-11
**Crates:** `datascript-cli` (`tests/cli.rs`, `tests/repl.rs`)
**Diagnostic codes:** none new expected (tests only). If a test finds a bug whose fix needs a
code, write a new task for the fix instead of allocating one here (D-018).

## Goal
A review of the integration tests (November 2026) found CLI and REPL behaviour that only unit
tests cover, or that nothing tests. Add end-to-end tests through the `ds` binary.

## Scope

### `ds check`
- [x] `--deny-warnings`: exit 1 when a warning is reported, exit 0 when only info diagnostics
  (such as `DS1007`) are. Today only `check_exit_code` in `crates/datascript-cli/src/main.rs` has a
  unit test.
- [x] `--format json` with several files, with warnings, and with a missing file (exit 2).

### `ds run`
- [x] A closed stdout pipe (`ds run … | head -1`) exits 0 and stops reading
  (`OutputError::BrokenPipe` in `src/output.rs` and `src/main.rs`).
- [x] Multi-batch streaming and a mid-stream failure for `--format ndjson`, `json` and `arrow`, as
  `csv_is_written_batch_by_batch_before_the_query_finishes` and
  `a_runtime_error_keeps_the_results_already_printed` do for `csv` and `table`. For `json`, check
  the output after a failure (the unclosed-array handling in
  `crates/datascript-engine/src/printer.rs`).
- [x] `--statement 1` on a program with no query (`let x = 1;`), covering `statement_help` in
  `src/main.rs`.
- [x] `--output` with `table`, `json` and `ndjson` (only `csv` and `arrow` are tested).
- [x] Several `--table` flags in one command, and a `--table` whose path is a glob.

### `ds explain`
- [x] A script file, and `-` (stdin).
- [x] A program with several queries, and a query that ends in `render`.

### REPL
- [x] Starting with `ds --max-rows N` (only `.max-rows` inside a session is tested).
- [x] A runtime error (for example `DS4003`): the span says `<repl:N>`, the exit code is 1, and
  saved `let`s and later inputs still work.
- [x] A saved `let` that breaks later (its file is removed after it was saved), through piped `ds`;
  the error points at `<repl:1>`. Today only `src/repl/session.rs` unit tests cover this.
- [x] `.max-rows` with no argument and with a non-numeric argument; extra arguments to commands
  that take none (`.help x`, `.tables x`, `.quit x`).

Out of scope: Ctrl-C, Ctrl-D, continuation prompts and history need a PTY and stay manual
(M1-12).

### Docs
- [x] Tick the `ds run examples/top_customers.ds` acceptance box in
  [M1-11-cli.md](M1-11-cli.md): `run_the_readme_example` in `tests/cli.rs` is no longer ignored.

## Acceptance criteria
- [x] Every item above has a test, or a note under "Follow-ups" saying why it cannot.
- [x] Any wrong behaviour a new test exposes gets its own task file and index row; the test is
      added with `#[ignore = "<task id>"]`.

## Follow-ups
- Informational: all new tests passed against the current CLI and engine; no bug tasks were needed.

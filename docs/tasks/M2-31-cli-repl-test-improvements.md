# M2-31 — CLI and REPL bug fixes and test hygiene

**Status:** done — Fixed `ds check --format json` (`BrokenPipe` via `output::write_stdout` and JSON `DS3002` on bad `--table`) and last-wins `--table` deduplication in `ReplSession`/`add_host_tables`; cleaned up `cli.rs`/`repl.rs` test hygiene and added CLI plumbing and piped REPL integration tests.
**Depends on:** M2-16, M3-07
**Crates:** `datascript-cli`
**Diagnostic codes:** none expected. If a fix needs a new code, reserve a block in
[DIAGNOSTICS.md](../DIAGNOSTICS.md) first (D-018).

## Goal
Fix two CLI/REPL bugs found by the November 2026 audit, remove the flaky and non-portable bits of
`crates/datascript-cli/tests/{cli,repl}.rs`, and test the CLI's plumbing (exit codes, stream
separation, lazy output) across formats. Format rendering itself is tested at the engine level in
[M2-29](M2-29-engine-datafusion-integration-tests.md); don't repeat it here.

## Scope

### Bug fixes
- [x] **`ds check --format json`:**
  - It prints with `println!`, so piping to a reader that closes early panics on `BrokenPipe`.
    Write through `output::write_stdout`, like `ds run` and `ds explain`.
  - A bad `--table` calls `reporter.fail_usage` on stderr. In JSON mode, emit the `DS3002`
    diagnostic as a JSON report on stdout (still exit 2), like unreadable scripts.
- [x] **Duplicate `--table` in the REPL.** Deduplicate host tables last-wins (as
      `DataFusionEngine::with_table` does) before storing them in `ReplSession`, and fix
      `add_host_tables` in `repl/editor.rs` to check `extra` rather than `items`, so `.tables` and
      Tab completion show each table once with its current path.

### CLI plumbing (`tests/cli.rs`)
- [x] For each format: a 0-row result is valid output, and a `let`-only program writes nothing and
      creates no `--output` file.
- [x] A non-`DS5001` warning exits 0, goes to stderr only, and leaves stdout parseable in `csv`,
      `json`, `ndjson` and `arrow`.
- [x] `--statement N` with `json` keeps the statement's line number, and selecting a clean
      statement succeeds when an unselected one would fail at runtime.
- [x] A runtime failure after `--max-rows` rows were printed (during `count_rest`) leaves the
      partial table, no footer, the error on stderr, and exit 1. A mid-stream failure while
      writing to `--output <path>`.
- [x] `BrokenPipe` exits 0 with clean stderr for `table`, `json`, `ndjson` and `arrow` (`csv` is
      covered), and for `ds check --format json` and `ds explain`.
- [x] Duplicate `--table` flags: last wins in `run`, `check` and `explain`. A missing `--table`
      file: unused exits 0, referenced exits 1 with `DS3008` ("the file registered for this table
      does not exist"; see Follow-ups).
- [x] One non-UTF-8 script file gives `DS3008` with exit 2.

### Test hygiene
- [x] `ctrl_c_stops_a_running_query` waits for the first row on stdout before sending `SIGINT`
      instead of sleeping 1 second (the pipe-sync pattern in `tests/repl.rs`).
- [x] `single_result_formats_need_one_query` writes to a `TempDir` path, not `/dev/null`.
- [x] One command-builder helper strips `NO_COLOR` for every `ds` invocation, replacing the raw
      `Command::new(env!("CARGO_BIN_EXE_ds"))` calls.

### Piped REPL (`tests/repl.rs`)
- [x] A warning on a saved `let` is reported once, not repeated on later inputs.
- [x] `diagnostics_are_plain_even_with_color_always_requested_by_auto` actually passes
      `--color always` (renamed to `diagnostics_are_plain_even_with_color_always`).
- [x] `.schema` on a `--table` host table and on an Arrow-only column type.
- [x] `.format`: switching `csv` → `table`, a mid-stream runtime error under `csv`, and rejecting
      `ndjson`/`arrow`.
- [x] A failed `let` redefinition rolls back end-to-end (unit-tested today), and the plural
      message for removing several dependent `let`s.

### Out of scope
- An interactive `reedline` PTY suite (manual per M1-12 and M2-16).
- An `is_terminal` seam for unit-testing TTY and colour branches.
- Re-testing format rendering of edge types (M2-29).

## Acceptance criteria
- [x] Both bugs are fixed with regression tests.
- [x] `tests/cli.rs` has no fixed sleep and no hardcoded `/dev/null`.
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
      `cargo test --workspace` pass.

## Follow-ups
- Note: a referenced `--table` pointing at a missing file produces `DS3008` ("the file registered
  for this table does not exist") rather than `DS3001`, by design in `datascript-datafusion`
  (`sources.rs`), because `SourceError::NotFound` on `SourceRef::Table` maps to `DS1002` (unknown
  table name). The test asserts `DS3008` and the explanatory note.

# M2-31 — CLI and REPL test improvements and bug fixes

**Status:** todo
**Depends on:** M2-16, M3-07
**Crates:** `datascript-cli`
**Diagnostic codes:** none expected. If a fix turns out to need a new code, reserve a block in
[DIAGNOSTICS.md](../DIAGNOSTICS.md) first (D-018).

## Goal
Close the remaining CLI and piped-REPL integration test gaps found after M2-16 and M3-07, fix two
verified CLI/REPL bugs uncovered by the audit, and tighten signal-test determinism and command
hygiene in `crates/datascript-cli/tests/{cli,repl}.rs`.

## Scope

### Verified CLI and REPL bug fixes
- [ ] **`ds check --format json` broken-pipe and `--table` error handling:**
  - Write JSON output through `output::write_stdout` instead of `println!` in
    `crates/datascript-cli/src/main.rs:182`, so piping `ds check --format json` to a reader that
    closes early exits 0 instead of panicking on `BrokenPipe` (matching `ds run` and `ds explain`).
  - When `session(&args.tables.tables)` fails in `crates/datascript-cli/src/main.rs:157-160` and
    `args.format == CheckFormat::Json`, emit the `DS3002` diagnostic as a JSON report on standard
    output (still exiting 2) instead of calling `reporter.fail_usage` on standard error, matching
    missing-file exit-2 handling in `main.rs:167-185`.
- [ ] **Duplicate `--table` deduplication in the REPL:**
  - Deduplicate host tables with last-wins semantics (matching `DataFusionEngine::with_table` in
    `crates/datascript-datafusion/src/lib.rs:113-116`) before storing them in `ReplSession`
    (`crates/datascript-cli/src/repl/mod.rs:48`) and guard `add_host_tables`
    (`crates/datascript-cli/src/repl/editor.rs:435-462`, which currently checks `items.iter()`
    rather than `extra.iter()`), so `.tables` (`crates/datascript-cli/src/repl/session.rs:693-696`)
    and Tab completion do not emit duplicate rows or duplicate completion items with stale paths.

### CLI format-contract matrix and test hygiene (`crates/datascript-cli/tests/cli.rs`)
- [ ] **Table-driven 5-format (`table`, `csv`, `json`, `ndjson`, `arrow`) contract matrix:**
  - **0-row results** (`where amount < 0`): valid empty table, header-only CSV, JSON with
    `"rows": []`, empty NDJSON, and a valid 0-row Arrow IPC file with schema.
  - **0-query / `let`-only / empty programs** across all 5 formats, including verifying lazy
    `--output` creation (`crates/datascript-engine/src/printer.rs:54-93, 189` writes nothing and
    creates no file).
  - **`--statement N`** across formats: `--format json` (1-element array preserving original
    statement line number), `ndjson`, and `arrow`; selecting a clean statement when an unselected
    statement would fail at runtime; and selecting a non-chart query when another query has
    `render barchart` so `run.select` suppresses `DS5001` (`crates/datascript-cli/src/main.rs:267,
    284`).
  - **Warnings + clean stdout/stderr separation**: non-`DS5001` warnings (such as `DS2103`) exit 0,
    write warnings strictly to stderr, and keep stdout clean and parseable in `csv`, `json`,
    `ndjson`, and `arrow`; `--max-rows` is ignored for `json`/`ndjson`/`arrow` and cuts multiple
    `table` queries independently.
  - **Mid-stream failure edge cases**:
    - Table failure *inside* `COUNT_BUDGET` (`10_000`, `crates/datascript-engine/src/printer.rs:27,
      284-287`) during `count_rest` after `--max-rows` rows were already printed (partial table on
      stdout, no footer, `DS4003` on stderr, exit 1).
    - Mid-stream failure when writing to `--output <path>` (contrasting with the existing
      first-batch test `a_failed_run_creates_no_output_file` at `tests/cli.rs:767-777`).
  - **`BrokenPipe` (exit 0, clean stderr)** across `table`, `json`, `ndjson`, and `arrow` (beyond
    existing `csv` at `tests/cli.rs:512-538`), plus `ds check --format json` and `ds explain`
    (`crates/datascript-cli/src/main.rs:215-218`).
- [ ] **`--table` and unreadable-input edge cases:**
  - Duplicate `--table name=path` flags (verifying last wins in `ds run`, `ds check`, `ds explain`,
    and the REPL).
  - Missing `--table` target files with valid extensions (`--table o=missing.csv`): unused exits 0;
    referenced exits 1 with `DS3001` (not exit 2).
  - Non-UTF-8 script files (`crates/datascript-cli/src/script.rs:94-95`, `DS3008`), non-UTF-8 stdin
    for `run -`, `check -`, `explain -` (`script.rs:59-60`) and piped REPL (`repl/mod.rs:62-65`),
    missing parent directory note (`script.rs:129-131`), `ds explain` exit-2 unreadable and bad
    `--table` errors (`main.rs:204, 208`), and `ds check` (human and `--format json`) on a
    directory or non-UTF-8 file (`DS3008`, exit 2).
- [ ] **Signal determinism, portability, and env hygiene:**
  - Replace the hardcoded `Duration::from_secs(1)` sleep in `ctrl_c_stops_a_running_query`
    (`crates/datascript-cli/tests/cli.rs:1255-1296`) with a deterministic first-row stdout
    readiness marker before sending `SIGINT` (matching the pipe-sync pattern in
    `crates/datascript-cli/tests/repl.rs:323-389`).
  - Replace Unix-only `"-o", "/dev/null"` in `single_result_formats_need_one_query`
    (`crates/datascript-cli/tests/cli.rs:365`) with a portable `TempDir` path.
  - Centralize command-builder helpers so every `ds` invocation strips `NO_COLOR` (covering the raw
    `Command::new(env!("CARGO_BIN_EXE_ds"))` calls at `tests/cli.rs:101, 1267, 1332` and
    `tests/repl.rs:124, 199, 261`).

### Piped REPL integration tests (`crates/datascript-cli/tests/repl.rs`)
- [ ] **Saved-`let` warning suppression (`crates/datascript-cli/src/repl/session.rs:153-159`):**
  verify that a warning (such as `DS2103`) on a saved `let` is reported once on `<repl:1>` and not
  repeated on later inputs, and that a warning-emitting REPL query exits 0 with the warning on
  stderr and the table on stdout.
- [ ] **Fix piped `--color always` test (`crates/datascript-cli/tests/repl.rs:271-273`):** update
  `diagnostics_are_plain_even_with_color_always_requested_by_auto` to actually pass
  `--color always` and verify `repl/mod.rs:39-43` still uses `Reporter::plain()` in piped mode.
- [ ] **`.schema` coverage:** `.schema <host_table>` (from `--table`), `.schema` on a scalar `let`
  (`crates/datascript-cli/src/repl/session.rs:680-683`), `.schema` on an Arrow-only column type
  (`session.rs:916-918`, e.g. `tests/data/types.parquet`), and post-create/edit schema-cache
  invalidation (`session.rs:656`).
- [ ] **`.format` coverage:** multi-query `.format json` in one input, switching `.format csv` /
  `json` back to `.format table`, mid-stream runtime error under `.format csv`/`json`
  (`session.rs:524-533`), did-you-mean (`.format jsno`), and rejecting unsupported CLI formats
  (`.format ndjson`, `.format arrow`) and extra arguments.
- [ ] **`let` redefinition, charts, and submit-rule end-to-end coverage:** failed `let`
  redefinition rollback (`let x = 1; let y = x; let x = nope;` keeps both, currently unit-only at
  `session.rs:1114`), transitive multi-user removal (`x -> y -> z`, plural `"them"` in
  `repl/mod.rs:146-163`), silent redefinition of an unused `let`, quoted-identifier `let`
  redefinition (`session.rs:739-742`), two charts in one input yielding one `DS5001` with note,
  and piped submit-rule edge cases (`;` in strings/comments, EOF flush without `;`).

### Optional small `is_terminal: bool` seam (unit/in-process)
- [ ] Extract a small `is_terminal: bool` seam for binary Arrow TTY refusal
  (`crates/datascript-cli/src/main.rs:237-242`, `output.rs:44-46`) and `Reporter::new` color
  selection (`crates/datascript-cli/src/script.rs:142-154`) so unit tests cover TTY refusal,
  TTY + `--color auto` with `NO_COLOR` unset vs `NO_COLOR=""` (keeps color per `script.rs:145`)
  vs `NO_COLOR=1` (`Renderer::terminal(false)`), and TTY + `--color never`
  (`Renderer::terminal(false)`).

### Out of scope
- Interactive `reedline` PTY test suite (Ctrl-C at prompt / mid-query, Ctrl-D, continuation
  prompts, interactive Tab completion, persistent history file): stays manual per M1-12 and M2-16.
- `style_of` syntax-highlighting style unit tests (`crates/datascript-cli/src/repl/editor.rs:270-282`).

## Acceptance criteria
- [ ] Both verified bugs (`ds check --format json` broken-pipe/`--table` output and REPL duplicate
      `--table` deduplication) are fixed and covered by regression tests.
- [ ] Every scoped item above has a passing test in `datascript-cli`.
- [ ] `ctrl_c_stops_a_running_query` no longer relies on a fixed sleep, and `tests/cli.rs` has no
      hardcoded `/dev/null` path.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
      `cargo test --workspace` pass.

## Follow-ups
